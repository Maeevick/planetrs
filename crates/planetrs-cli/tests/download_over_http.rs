//! The download through real HTTP, against a small server on this machine that honours ranges and
//! can drop a connection midway, the way a long download over the Internet does.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use planetrs_cli::download::checksum::md5_of;
use planetrs_cli::download::http::{Http, USER_AGENT};
use planetrs_cli::download::{Outcome, Settings, Surroundings, download};

const ETAG: &str = "\"v1\"";

struct Served {
    content: Vec<u8>,
    checksum: String,
    drops: Mutex<VecDeque<usize>>,
    seen: Mutex<Vec<Seen>>,
}

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    range: Option<String>,
    if_range: Option<String>,
    user_agent: Option<String>,
}

fn serve(served: Arc<Served>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            answer(stream, &served);
        }
    });
    format!("http://{address}")
}

fn answer(mut stream: TcpStream, served: &Served) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut parts = first.split_whitespace();
    let (method, path) = (
        parts.next().unwrap_or_default().to_string(),
        parts.next().unwrap_or_default().to_string(),
    );
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_lowercase(), value.trim().to_string()));
        }
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    served.seen.lock().unwrap().push(Seen {
        method: method.clone(),
        path: path.clone(),
        range: header("range"),
        if_range: header("if-range"),
        user_agent: header("user-agent"),
    });
    let length = served.content.len();
    let head = |status: &str, extra: &str, body_length: usize| {
        format!("HTTP/1.1 {status}\r\nContent-Length: {body_length}\r\nConnection: close\r\n{extra}\r\n")
    };
    match (method.as_str(), path.as_str()) {
        ("GET", "/pbf/planet-latest.osm.pbf.md5") => {
            let _ = stream.write_all(head("200 OK", "", served.checksum.len()).as_bytes());
            let _ = stream.write_all(served.checksum.as_bytes());
        }
        ("HEAD", "/pbf/planet-260907.osm.pbf") => {
            let _ = stream.write_all(head("200 OK", &format!("ETag: {ETAG}\r\n"), length).as_bytes());
        }
        ("GET", "/pbf/planet-260907.osm.pbf") => {
            let asked = header("range").and_then(|range| {
                let (first, last) = range.strip_prefix("bytes=")?.split_once('-')?;
                Some((first.parse::<usize>().ok()?, last.parse::<usize>().ok()?))
            });
            let Some((first, last)) = asked else {
                let _ = stream.write_all(head("200 OK", &format!("ETag: {ETAG}\r\n"), length).as_bytes());
                let _ = stream.write_all(&served.content);
                return;
            };
            let last = last.min(length - 1);
            let body = &served.content[first..=last];
            let extra = format!("ETag: {ETAG}\r\nContent-Range: bytes {first}-{last}/{length}\r\n");
            let _ = stream.write_all(head("206 Partial Content", &extra, body.len()).as_bytes());
            match served.drops.lock().unwrap().pop_front() {
                Some(after) => {
                    let _ = stream.write_all(&body[..after.min(body.len())]);
                    let _ = stream.flush();
                    let _ = stream.shutdown(Shutdown::Both);
                }
                None => {
                    let _ = stream.write_all(body);
                }
            }
        }
        _ => {
            let _ = stream.write_all(head("404 Not Found", "", 0).as_bytes());
        }
    }
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("planetrs-http-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn planet() -> Vec<u8> {
    (0..20_000u32)
        .map(|at| (at.wrapping_mul(31) % 253) as u8)
        .collect()
}

fn served(drops: &[usize]) -> Arc<Served> {
    Arc::new(Served {
        checksum: format!(
            "{}  planet-260907.osm.pbf\n",
            md5_of(&planet()[..], &|_| {}).unwrap()
        ),
        content: planet(),
        drops: Mutex::new(drops.iter().copied().collect()),
        seen: Mutex::new(Vec::new()),
    })
}

fn settings() -> Settings {
    Settings {
        step: 4_096,
        attempts: 3,
        first_pause: Duration::ZERO,
        margin: 0,
    }
}

#[test]
fn given_a_connection_dropped_midway_when_downloaded_over_http_then_the_planet_arrives_whole() {
    let server = served(&[0, 1_000]);
    let base = serve(server.clone());
    let directory = scratch("dropped");
    let surroundings = Surroundings {
        free_space: &|_| Ok(u64::MAX),
        pause: &|_| {},
        report: &|_| {},
    };

    let outcome = download(
        &Http::new(),
        &format!("{base}/pbf/planet-latest.osm.pbf"),
        &directory,
        &settings(),
        &surroundings,
    )
    .unwrap();

    let saved = directory.join("planet-260907.osm.pbf");
    assert_eq!(outcome, Outcome::Downloaded(saved.clone()));
    assert_eq!(std::fs::read(&saved).unwrap(), planet());
    let ranges: Vec<String> = server
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|seen| seen.range.clone())
        .collect();
    assert_eq!(
        &ranges[..4],
        [
            "bytes=0-4095",
            "bytes=0-4095",
            "bytes=1000-5095",
            "bytes=5096-9191"
        ]
    );
}

#[test]
fn given_any_request_when_downloading_over_http_then_it_names_the_tool_and_ranges_carry_the_validator() {
    let server = served(&[]);
    let base = serve(server.clone());
    let surroundings = Surroundings {
        free_space: &|_| Ok(u64::MAX),
        pause: &|_| {},
        report: &|_| {},
    };

    download(
        &Http::new(),
        &format!("{base}/pbf/planet-latest.osm.pbf"),
        &scratch("polite"),
        &settings(),
        &surroundings,
    )
    .unwrap();

    let seen = server.seen.lock().unwrap().clone();
    assert!(
        seen.iter()
            .all(|request| request.user_agent.as_deref() == Some(USER_AGENT))
    );
    assert!(
        seen.iter()
            .filter(|request| request.method == "GET" && request.range.is_some())
            .all(|request| request.if_range.as_deref() == Some(ETAG)
                && request.path == "/pbf/planet-260907.osm.pbf")
    );
}
