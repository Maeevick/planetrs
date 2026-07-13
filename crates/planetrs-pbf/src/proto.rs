//! Protobuf wire format, limited to the subset the OSM PBF format uses.
//!
//! Hand-rolled rather than generated: the schema is two dozen fields that have not changed since
//! 2010, and a code generator would pull in a build step and a dependency tree for it.

use crate::error::PbfError;

#[derive(Debug, PartialEq, Eq)]
pub enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed32([u8; 4]),
    Fixed64([u8; 8]),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Field<'a> {
    pub number: u32,
    pub value: Value<'a>,
}

/// Undoes protobuf's `sint64` encoding, which maps signed values onto unsigned ones so that small
/// negative numbers stay short. Every coordinate and every id delta in a PBF file arrives this way.
pub fn zigzag(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

/// A cursor over one protobuf message. Fields come back in wire order, borrowed from the input.
pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    pub fn exhausted(&self) -> bool {
        self.at >= self.bytes.len()
    }

    pub fn varint(&mut self) -> Result<u64, PbfError> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = *self.bytes.get(self.at).ok_or(PbfError::Truncated)?;
            self.at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift >= 64 {
                return Err(PbfError::VarintTooLong);
            }
        }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], PbfError> {
        let end = self.at.checked_add(count).ok_or(PbfError::Truncated)?;
        let slice = self.bytes.get(self.at..end).ok_or(PbfError::Truncated)?;
        self.at = end;
        Ok(slice)
    }

    pub fn next_field(&mut self) -> Result<Option<Field<'a>>, PbfError> {
        if self.exhausted() {
            return Ok(None);
        }
        let key = self.varint()?;
        let number = (key >> 3) as u32;
        let value = match (key & 7) as u8 {
            0 => Value::Varint(self.varint()?),
            1 => Value::Fixed64(self.take(8)?.try_into().expect("take yielded 8 bytes")),
            2 => {
                let length = self.varint()? as usize;
                Value::Bytes(self.take(length)?)
            }
            5 => Value::Fixed32(self.take(4)?.try_into().expect("take yielded 4 bytes")),
            other => return Err(PbfError::UnknownWireType(other)),
        };
        Ok(Some(Field { number, value }))
    }
}

/// A packed repeated varint field: ids, coordinates and way refs all arrive as one of these.
pub struct Packed<'a>(Reader<'a>);

impl<'a> Packed<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self(Reader::new(bytes))
    }
}

impl Iterator for Packed<'_> {
    type Item = Result<u64, PbfError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.exhausted() {
            return None;
        }
        Some(self.0.varint())
    }
}

/// The running total of a packed delta-coded array, which is how every id and coordinate is stored.
pub struct Undeltaed<'a> {
    packed: Packed<'a>,
    running: i64,
}

impl<'a> Undeltaed<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            packed: Packed::new(bytes),
            running: 0,
        }
    }
}

impl Iterator for Undeltaed<'_> {
    type Item = Result<i64, PbfError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.packed.next()? {
            Ok(delta) => {
                self.running += zigzag(delta);
                Some(Ok(self.running))
            }
            Err(error) => Some(Err(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_a_varint_spans_several_bytes_then_the_groups_are_little_endian() {
        let mut reader = Reader::new(&[0xac, 0x02]);

        assert_eq!(reader.varint().unwrap(), 300);
    }

    #[test]
    fn when_a_varint_never_terminates_then_it_is_truncated_rather_than_panicking() {
        let mut reader = Reader::new(&[0xac]);

        assert!(matches!(reader.varint(), Err(PbfError::Truncated)));
    }

    #[test]
    fn when_a_length_prefix_overruns_the_buffer_then_it_is_truncated() {
        let mut reader = Reader::new(&[0x0a, 0x05, b'a', b'b']);

        assert!(matches!(reader.next_field(), Err(PbfError::Truncated)));
    }

    #[test]
    fn when_a_message_holds_a_string_then_it_is_borrowed_not_copied() {
        let mut reader = Reader::new(b"\x0a\x09OSMHeader");

        assert_eq!(
            reader.next_field().unwrap(),
            Some(Field {
                number: 1,
                value: Value::Bytes(b"OSMHeader")
            })
        );
    }

    #[test]
    fn when_a_message_is_exhausted_then_no_further_field_comes_back() {
        let mut reader = Reader::new(b"\x0a\x09OSMHeader");
        reader.next_field().unwrap();

        assert_eq!(reader.next_field().unwrap(), None);
    }

    #[test]
    fn when_values_are_zigzagged_then_small_negatives_stay_small() {
        assert_eq!(
            [zigzag(0), zigzag(1), zigzag(2), zigzag(4294967294)],
            [0, -1, 1, 2147483647]
        );
    }

    #[test]
    fn when_a_field_is_packed_then_every_varint_is_yielded_in_order() {
        let values: Result<Vec<_>, _> = Packed::new(&[0x01, 0xac, 0x02, 0x03]).collect();

        assert_eq!(values.unwrap(), vec![1, 300, 3]);
    }

    /// Ids and coordinates are stored as differences from the previous entry, so a value is
    /// meaningless without every value before it in the same array.
    #[test]
    fn when_an_array_is_delta_coded_then_each_value_is_the_running_total() {
        let values: Result<Vec<_>, _> = Undeltaed::new(&[0x02, 0x04, 0x01]).collect();

        assert_eq!(values.unwrap(), vec![1, 3, 2]);
    }

    #[test]
    fn when_a_delta_coded_array_is_empty_then_nothing_is_yielded() {
        let values: Result<Vec<_>, _> = Undeltaed::new(&[]).collect();

        assert_eq!(values.unwrap(), Vec::<i64>::new());
    }
}
