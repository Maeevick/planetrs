//! The outer framing of a PBF file: a plain sequence of blobs, with no index and no table of
//! contents. Every blob is
//!
//! ```text
//! u32 big-endian   length of the BlobHeader that follows
//! BlobHeader       { type: string = 1, datasize: int32 = 3 }
//! payload          datasize bytes, a Blob message holding raw or compressed bytes
//! ```
//!
//! Walking the framing therefore costs three reads per blob and no decompression, which is what
//! makes a census of a 94 GB file affordable.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::PbfError;
use crate::proto::{Reader, Value};

/// The spec caps a BlobHeader at 64 KiB and a payload at 32 MiB. Anything larger means the cursor
/// has drifted out of alignment, and refusing early turns that into one error rather than a long
/// walk through nonsense.
pub const MAX_HEADER: u32 = 64 * 1024;
pub const MAX_PAYLOAD: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobKind {
    Header,
    Data,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    /// Offset of the payload itself, past the framing, so it can be read back on its own.
    pub offset: u64,
    pub kind: BlobKind,
    pub data_size: usize,
}

/// Decodes one BlobHeader. Pure: the caller does the reading.
pub fn blob_header(bytes: &[u8]) -> Result<(BlobKind, usize), PbfError> {
    let mut reader = Reader::new(bytes);
    let mut kind = BlobKind::Other;
    let mut data_size = None;

    while let Some(field) = reader.next_field()? {
        match (field.number, field.value) {
            (1, Value::Bytes(b"OSMHeader")) => kind = BlobKind::Header,
            (1, Value::Bytes(b"OSMData")) => kind = BlobKind::Data,
            (3, Value::Varint(size)) => data_size = Some(size as usize),
            _ => {}
        }
    }

    match data_size {
        Some(size) if size > MAX_PAYLOAD => Err(PbfError::PayloadTooLarge {
            size,
            limit: MAX_PAYLOAD,
        }),
        Some(size) => Ok((kind, size)),
        None => Err(PbfError::MissingDataSize),
    }
}

/// A sequential walk of the framing, reading headers and stepping over payloads.
pub struct Framing {
    file: BufReader<File>,
    at: u64,
    end: u64,
    header: Vec<u8>,
}

impl Framing {
    pub fn open(path: &Path) -> Result<Self, PbfError> {
        let file = File::open(path)?;
        let end = file.metadata()?.len();
        Ok(Self {
            file: BufReader::with_capacity(1 << 16, file),
            at: 0,
            end,
            header: vec![0; MAX_HEADER as usize],
        })
    }

    pub fn file_size(&self) -> u64 {
        self.end
    }

    /// Reads the next blob's framing and leaves the cursor past its payload, without touching it.
    pub fn next_location(&mut self) -> Result<Option<Location>, PbfError> {
        if self.at >= self.end {
            return Ok(None);
        }
        let started_at = self.at;

        let mut length = [0u8; 4];
        self.file
            .read_exact(&mut length)
            .map_err(|error| PbfError::from(error).at_blob(started_at))?;
        let length = u32::from_be_bytes(length);
        if length > MAX_HEADER {
            return Err(PbfError::HeaderTooLarge {
                length,
                limit: MAX_HEADER,
            }
            .at_blob(started_at));
        }

        let header = &mut self.header[..length as usize];
        self.file
            .read_exact(header)
            .map_err(|error| PbfError::from(error).at_blob(started_at))?;
        let (kind, data_size) = blob_header(header).map_err(|error| error.at_blob(started_at))?;

        let offset = started_at + 4 + u64::from(length);
        self.at = offset + data_size as u64;
        self.file
            .seek(SeekFrom::Start(self.at))
            .map_err(|error| PbfError::from(error).at_blob(started_at))?;

        Ok(Some(Location {
            offset,
            kind,
            data_size,
        }))
    }
}

impl Iterator for Framing {
    type Item = Result<Location, PbfError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_location().transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_the_header_names_a_data_blob_then_its_payload_size_comes_with_it() {
        assert_eq!(
            blob_header(b"\x0a\x07OSMData\x18\x80\x02").unwrap(),
            (BlobKind::Data, 256)
        );
    }

    #[test]
    fn when_the_header_names_the_file_header_then_it_is_told_apart_from_data() {
        assert_eq!(
            blob_header(b"\x0a\x09OSMHeader\x18\x2a").unwrap(),
            (BlobKind::Header, 42)
        );
    }

    /// `indexdata` (field 2) is optional and unused by planet files, but skipping an unknown field
    /// has to work or the cursor lands mid-message and every blob after it is garbage.
    #[test]
    fn when_the_header_carries_a_field_we_ignore_then_the_rest_still_decodes() {
        assert_eq!(
            blob_header(b"\x0a\x07OSMData\x12\x03abc\x18\x2a").unwrap(),
            (BlobKind::Data, 42)
        );
    }

    #[test]
    fn when_the_header_has_no_size_then_it_is_refused_rather_than_assumed_zero() {
        assert!(matches!(
            blob_header(b"\x0a\x07OSMData"),
            Err(PbfError::MissingDataSize)
        ));
    }

    /// A size past the spec's ceiling means the cursor has drifted, and reserving on it would hand
    /// a number taken from the file straight to the allocator.
    #[test]
    fn when_the_header_claims_more_than_the_spec_allows_then_it_is_refused() {
        let oversized = [
            b"\x0a\x07OSMData\x18".as_slice(),
            b"\x80\x80\x80\x80\x01".as_slice(),
        ]
        .concat();

        assert!(matches!(
            blob_header(&oversized),
            Err(PbfError::PayloadTooLarge { .. })
        ));
    }
}
