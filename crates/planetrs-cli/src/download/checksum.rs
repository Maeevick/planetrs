//! The md5 a planet is published with, and the md5 of what arrived.

use std::io::{self, Read};
use std::path::Path;

use md5::{Digest, Md5};

/// A published checksum line, as `md5sum` writes it: `<hex>  <file name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// The md5 in lower-case hexadecimal.
    pub md5: String,
    /// The file it is the checksum of, when the line names one.
    pub file_name: Option<String>,
}

/// Why a checksum file could not be read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("the checksum file does not start with an md5 in hexadecimal: {0:?}")]
pub struct NotAChecksum(pub String);

/// Reads the first line of a checksum file, tolerating `md5sum`'s binary marker `*`.
pub fn parse(text: &str) -> Result<Published, NotAChecksum> {
    let line = text.lines().next().unwrap_or_default().trim();
    let mut parts = line.split_whitespace();
    let md5 = parts.next().unwrap_or_default().to_ascii_lowercase();
    if md5.len() != 32 || !md5.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(NotAChecksum(line.chars().take(80).collect()));
    }
    let file_name = parts
        .next()
        .map(|name| name.trim_start_matches('*'))
        .filter(|name| !name.is_empty() && !name.contains('/'))
        .map(str::to_string);
    Ok(Published { md5, file_name })
}

/// The md5 of everything `reader` yields, in lower-case hexadecimal, with `progress` told how many
/// bytes have been read after each block.
pub fn md5_of(mut reader: impl Read, progress: &dyn Fn(u64)) -> io::Result<String> {
    let mut hasher = Md5::new();
    let mut block = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let read = match reader.read(&mut block) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        hasher.update(&block[..read]);
        done += read as u64;
        progress(done);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// The md5 of a whole file.
pub fn md5_of_file(path: &Path, progress: &dyn Fn(u64)) -> io::Result<String> {
    md5_of(std::fs::File::open(path)?, progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_line_as_md5sum_writes_it_when_parsed_then_the_checksum_and_the_file_are_read() {
        assert_eq!(
            parse("D41D8CD98F00B204E9800998ECF8427E  planet-260907.osm.pbf\n"),
            Ok(Published {
                md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
                file_name: Some("planet-260907.osm.pbf".into())
            })
        );
    }

    #[test]
    fn given_the_binary_marker_when_parsed_then_it_is_not_part_of_the_name() {
        assert_eq!(
            parse("d41d8cd98f00b204e9800998ecf8427e *monaco-latest.osm.pbf").map(|found| found.file_name),
            Ok(Some("monaco-latest.osm.pbf".into()))
        );
    }

    #[test]
    fn given_a_checksum_alone_when_parsed_then_no_file_is_named() {
        assert_eq!(
            parse("d41d8cd98f00b204e9800998ecf8427e").map(|found| found.file_name),
            Ok(None)
        );
    }

    #[test]
    fn given_an_html_page_instead_of_a_checksum_when_parsed_then_it_is_refused() {
        assert!(parse("<html><body>Not Found</body></html>").is_err());
    }

    #[test]
    fn given_known_bytes_when_hashed_then_the_md5_is_the_published_one() {
        assert_eq!(
            md5_of(&b""[..], &|_| {}).unwrap(),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
        assert_eq!(
            md5_of(&b"The quick brown fox jumps over the lazy dog"[..], &|_| {}).unwrap(),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
    }
}
