//! The arithmetic of a resumable download: which file, which bytes next, and what a reply means.

/// The bytes a `Content-Range` header says a reply carries: `bytes 0-99/1000`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    /// The first byte, counted from zero.
    pub first: u64,
    /// The last byte, inclusive.
    pub last: u64,
    /// The whole file's length, when the server says it.
    pub length: Option<u64>,
}

/// Reads a `Content-Range` header of a 206 reply.
pub fn content_range(header: &str) -> Option<ContentRange> {
    let span = header.trim().strip_prefix("bytes ")?;
    let (bytes, length) = span.split_once('/')?;
    let (first, last) = bytes.split_once('-')?;
    let (first, last) = (first.trim().parse().ok()?, last.trim().parse().ok()?);
    let length = match length.trim() {
        "*" => None,
        known => Some(known.parse().ok()?),
    };
    (first <= last).then_some(ContentRange { first, last, length })
}

/// The `Range` header asking for bytes `from` up to, not including, `to`.
pub fn range_header(from: u64, to: u64) -> String {
    format!("bytes={from}-{}", to.saturating_sub(1))
}

/// The last segment of a URL's path, `planet-latest.osm.pbf`.
pub fn file_name_of(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next()?;
    path.rsplit('/').next().filter(|name| !name.is_empty())
}

/// The URL of the file a checksum names, beside the one asked for.
///
/// `planet-latest.osm.pbf` is a link the server moves every week, and its checksum names the dated
/// planet it currently points to. Downloading that dated file instead means a download resumed days
/// later still fetches the same bytes.
pub fn named_by_checksum(url: &str, named: &str) -> String {
    let path = &url[..url.find(['?', '#']).unwrap_or(url.len())];
    match path.rsplit_once('/') {
        Some((directory, asked)) if !asked.is_empty() && asked != named => format!("{directory}/{named}"),
        _ => url.to_string(),
    }
}

/// The ranges a file of `length` bytes is fetched in, from `offset`, `step` bytes each.
pub fn next_range(offset: u64, length: u64, step: u64) -> Option<(u64, u64)> {
    (offset < length).then(|| (offset, offset.saturating_add(step).min(length)))
}

/// What a resumable download remembers beside its partial file, so a later run resumes only the
/// same bytes from the same source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartMeta {
    /// The URL being fetched.
    pub url: String,
    /// The md5 the whole file must have.
    pub md5: String,
    /// The file's length.
    pub length: u64,
    /// The server's validator for the file, an ETag or a date, when it gave one.
    pub validator: Option<String>,
}

impl PartMeta {
    /// The meta as written beside the partial file: one `key value` per line.
    pub fn to_text(&self) -> String {
        let mut text = format!("url {}\nmd5 {}\nlength {}\n", self.url, self.md5, self.length);
        if let Some(validator) = &self.validator {
            text.push_str(&format!("validator {validator}\n"));
        }
        text
    }

    /// Reads what [`PartMeta::to_text`] wrote, or nothing when it is not that.
    pub fn from_text(text: &str) -> Option<Self> {
        let value = |key: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix(' '))
                .map(str::to_string)
        };
        Some(Self {
            url: value("url")?,
            md5: value("md5")?,
            length: value("length")?.parse().ok()?,
            validator: value("validator"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_content_range_when_read_then_its_bytes_and_length_come_out() {
        assert_eq!(
            content_range("bytes 0-99/1000"),
            Some(ContentRange {
                first: 0,
                last: 99,
                length: Some(1_000)
            })
        );
        assert_eq!(content_range("bytes 5-9/*").and_then(|range| range.length), None);
    }

    #[test]
    fn given_a_content_range_that_is_not_one_when_read_then_there_is_none() {
        assert_eq!(content_range("bytes */1000"), None);
        assert_eq!(content_range("bytes 9-5/1000"), None);
        assert_eq!(content_range("items 0-9/10"), None);
    }

    #[test]
    fn given_a_span_when_asked_for_then_the_range_header_is_inclusive() {
        assert_eq!(range_header(0, 100), "bytes=0-99");
    }

    #[test]
    fn given_the_latest_planet_when_its_checksum_names_a_dated_one_then_the_dated_one_is_fetched() {
        assert_eq!(
            named_by_checksum(
                "https://planet.openstreetmap.org/pbf/planet-latest.osm.pbf",
                "planet-260907.osm.pbf"
            ),
            "https://planet.openstreetmap.org/pbf/planet-260907.osm.pbf"
        );
    }

    #[test]
    fn given_a_checksum_naming_the_file_asked_for_when_followed_then_the_url_is_unchanged() {
        let url = "https://download.geofabrik.de/europe/monaco-latest.osm.pbf";

        assert_eq!(named_by_checksum(url, "monaco-latest.osm.pbf"), url);
    }

    #[test]
    fn given_a_file_when_cut_into_ranges_then_the_last_one_stops_at_its_end() {
        assert_eq!(next_range(0, 250, 100), Some((0, 100)));
        assert_eq!(next_range(200, 250, 100), Some((200, 250)));
        assert_eq!(next_range(250, 250, 100), None);
    }

    #[test]
    fn given_a_meta_when_written_and_read_back_then_it_is_the_same() {
        let meta = PartMeta {
            url: "https://example.org/planet-260907.osm.pbf".into(),
            md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
            length: 94_746_421_723,
            validator: Some("\"5f0-61c\"".into()),
        };

        assert_eq!(PartMeta::from_text(&meta.to_text()), Some(meta));
    }
}
