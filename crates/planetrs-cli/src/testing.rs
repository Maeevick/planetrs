//! A web server on this machine serving files over HTTP as a planet's host does, for the tests of
//! whatever downloads a planet.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;

/// A web server on this machine serving files by range, as planet.openstreetmap.org and its mirrors
/// do: it answers HEAD with a length and a strong ETag, and honours `Range` and `If-Match` on GET.
pub struct RangeServer {
    address: SocketAddr,
}

/// A file as the server holds it: its bytes, shared between requests, and its ETag, computed once.
#[derive(Clone)]
struct Served {
    bytes: Arc<Vec<u8>>,
    etag: String,
}

impl Served {
    fn of(bytes: &[u8]) -> Self {
        Self {
            etag: etag_of(bytes),
            bytes: Arc::new(bytes.to_vec()),
        }
    }
}

impl RangeServer {
    /// A server holding `files`, listening on a free port of this machine until the process ends.
    pub fn serving(files: &[(&str, &[u8])]) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let files: Arc<BTreeMap<String, Served>> = Arc::new(
            files
                .iter()
                .map(|(name, bytes)| (name.to_string(), Served::of(bytes)))
                .collect(),
        );
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let files = Arc::clone(&files);
                std::thread::spawn(move || answer(stream, &files));
            }
        });
        Ok(Self { address })
    }

    /// The address the files are served under, ending with a slash.
    pub fn url(&self) -> String {
        format!("http://{}/", self.address)
    }
}

/// A strong validator that changes whenever the bytes do.
fn etag_of(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("\"{hash:016x}\"")
}

fn answer(stream: TcpStream, files: &BTreeMap<String, Served>) {
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
    let reply = match files.get(&name).cloned() {
        None => respond("404 Not Found", &[], b""),
        Some(Served { bytes, etag }) => {
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
