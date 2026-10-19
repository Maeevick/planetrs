//! A dataset's files held in memory, and the contract every way of reading files must honour.
//!
//! The fake stands in for a directory or a web server in the tests of whatever reads a dataset, and
//! the contract is run against it and against every real adapter, so the fake cannot drift from
//! what the adapters do.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use crate::files::{ForReadingFiles, Unreachable, within};

/// A dataset's files held in memory, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilesInMemory {
    files: BTreeMap<String, Vec<u8>>,
}

impl FilesInMemory {
    /// The same files, with one more.
    pub fn with(mut self, name: &str, bytes: &[u8]) -> Self {
        self.files.insert(name.to_string(), bytes.to_vec());
        self
    }
}

impl FilesInMemory {
    fn bytes_of(&self, file: &str) -> Result<&[u8], Unreachable> {
        self.files
            .get(file)
            .map(Vec::as_slice)
            .ok_or_else(|| Unreachable::Missing {
                file: file.to_string(),
            })
    }
}

impl ForReadingFiles for FilesInMemory {
    fn length(&self, file: &str) -> Result<u64, Unreachable> {
        self.bytes_of(file).map(|bytes| bytes.len() as u64)
    }

    fn read(&self, file: &str, range: Range<u64>) -> Result<Vec<u8>, Unreachable> {
        let bytes = self.bytes_of(file)?;
        within(file, &range, bytes.len() as u64)?;
        Ok(bytes[range.start as usize..range.end as usize].to_vec())
    }
}

/// A web server on this machine serving files by range, as a dataset's host does, for the tests of
/// whatever reads a dataset over HTTP.
///
/// It answers HEAD with a length and a strong ETag, honours `Range` and `If-Match` on GET, and can
/// replace a file while running, the way a host republishing a dataset would.
pub struct RangeServer {
    address: SocketAddr,
    files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl RangeServer {
    /// A server holding `files`, listening on a free port of this machine until the process ends.
    pub fn serving(files: &[(&str, &[u8])]) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let files = Arc::new(Mutex::new(
            files
                .iter()
                .map(|(name, bytes)| (name.to_string(), bytes.to_vec()))
                .collect::<BTreeMap<_, _>>(),
        ));
        let served = Arc::clone(&files);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let served = Arc::clone(&served);
                std::thread::spawn(move || answer(stream, &served));
            }
        });
        Ok(Self { address, files })
    }

    /// The address the files are served under, ending with a slash.
    pub fn url(&self) -> String {
        format!("http://{}/", self.address)
    }

    /// Replaces a file, or adds it, as a host republishing it would.
    pub fn replace(&self, name: &str, bytes: &[u8]) {
        if let Ok(mut files) = self.files.lock() {
            files.insert(name.to_string(), bytes.to_vec());
        }
    }
}

/// A strong validator that changes whenever the bytes do.
pub fn etag_of(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("\"{hash:016x}\"")
}

fn answer(stream: TcpStream, files: &Mutex<BTreeMap<String, Vec<u8>>>) {
    let Ok(reading) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reading);
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let name = parts
        .next()
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_string();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if line.trim().is_empty() => break,
            Ok(_) => {
                if let Some((key, value)) = line.split_once(':') {
                    headers.push((key.trim().to_lowercase(), value.trim().to_string()));
                }
            }
        }
    }
    let header = |wanted: &str| {
        headers
            .iter()
            .find(|(key, _)| key == wanted)
            .map(|(_, value)| value.clone())
    };
    let held = files.lock().ok().and_then(|files| files.get(&name).cloned());
    let reply = match held {
        None => respond("404 Not Found", &[], b""),
        Some(bytes) => {
            let etag = etag_of(&bytes);
            let length = bytes.len();
            let tagged = [("ETag", etag.clone()), ("Accept-Ranges", "bytes".to_string())];
            match (method.as_str(), header("range")) {
                ("HEAD", _) => respond_with_length("200 OK", &tagged, length),
                (_, _) if header("if-match").is_some_and(|asked| asked != etag) => {
                    respond("412 Precondition Failed", &tagged, b"")
                }
                (_, Some(range)) => match asked_range(&range, length) {
                    Some((first, last)) => respond(
                        "206 Partial Content",
                        &[
                            tagged[0].clone(),
                            ("Content-Range", format!("bytes {first}-{last}/{length}")),
                        ],
                        &bytes[first..=last],
                    ),
                    None => respond(
                        "416 Range Not Satisfiable",
                        &[("Content-Range", format!("bytes */{length}"))],
                        b"",
                    ),
                },
                (_, None) => respond("200 OK", &tagged, &bytes),
            }
        }
    };
    let mut writing = stream;
    let _ = writing.write_all(&reply);
}

fn asked_range(range: &str, length: usize) -> Option<(usize, usize)> {
    let (first, last) = range.strip_prefix("bytes=")?.split_once('-')?;
    let first: usize = first.parse().ok()?;
    let last: usize = last.parse::<usize>().ok()?.min(length.checked_sub(1)?);
    (first <= last).then_some((first, last))
}

fn respond_with_length(status: &str, headers: &[(&str, String)], length: usize) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n");
    for (key, value) in headers {
        head.push_str(&format!("{key}: {value}\r\n"));
    }
    head.push_str("\r\n");
    head.into_bytes()
}

fn respond(status: &str, headers: &[(&str, String)], body: &[u8]) -> Vec<u8> {
    [respond_with_length(status, headers, body.len()), body.to_vec()].concat()
}

/// What every way of reading files must do, checked against one built to hold `holding`.
///
/// Panics, as a test does, at the first behaviour the files do not honour.
pub fn honours_for_reading_files<F: ForReadingFiles>(holding: impl Fn(&[(&str, &[u8])]) -> F) {
    let manifest: &[u8] = br#"{"format":"planetrs dataset 1"}"#;
    let files = holding(&[("manifest.json", manifest), ("empty.parquet", b"")]);

    assert_eq!(
        files.length("manifest.json"),
        Ok(manifest.len() as u64),
        "a file's length is its count of bytes"
    );
    assert_eq!(
        files.read("manifest.json", 2..10),
        Ok(manifest[2..10].to_vec()),
        "a range reads exactly its bytes"
    );
    assert_eq!(
        files.read("manifest.json", 20..manifest.len() as u64),
        Ok(manifest[20..].to_vec()),
        "a range ending at the end reads the tail"
    );
    assert_eq!(
        files.length("empty.parquet"),
        Ok(0),
        "an empty file has length zero"
    );
    assert_eq!(
        files.read("manifest.json", 20..40),
        Err(Unreachable::PastTheEnd {
            file: "manifest.json".into(),
            range: 20..40,
            length: manifest.len() as u64,
        }),
        "a range past the end is refused rather than shortened"
    );
    assert_eq!(
        files.length("gazetteer.parquet"),
        Err(Unreachable::Missing {
            file: "gazetteer.parquet".into()
        }),
        "a file the dataset lacks is missing"
    );
    assert_eq!(
        files.read("gazetteer.parquet", 0..4),
        Err(Unreachable::Missing {
            file: "gazetteer.parquet".into()
        }),
        "a range of a file the dataset lacks is missing too"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_files_held_in_memory_when_read_then_they_honour_the_contract() {
        honours_for_reading_files(|holding| {
            holding
                .iter()
                .fold(FilesInMemory::default(), |files, (name, bytes)| {
                    files.with(name, bytes)
                })
        });
    }
}
