//! A dataset's files over HTTP, range by range, as data.gouv.fr and any static host serve them.
//!
//! Each file is described once, by a HEAD that gives its length and its ETag. Every later range is
//! asked for with that ETag in `If-Match`, so that a file republished while it is being read is
//! refused rather than stitched together from two versions.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Mutex;
use std::time::Duration;

use ureq::Agent;
use ureq::http::{Response, StatusCode};

use crate::files::{ForReadingFiles, Unreachable, within};

/// Who is asking, as hosts of open data like every client to say.
pub const USER_AGENT: &str = concat!(
    "planetrs-dataset/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/maeevick/planetrs)"
);

/// A dataset's files under one web address.
pub struct HttpFiles {
    base: String,
    agent: Agent,
    described: Mutex<HashMap<String, Described>>,
}

/// What a HEAD said about a file: how long it is and which version of it was described.
#[derive(Debug, Clone)]
struct Described {
    length: u64,
    etag: Option<String>,
}

impl HttpFiles {
    /// The files of the dataset published under `base`, by name relative to it. A name that is
    /// itself a web address, as a published manifest gives, is fetched as it stands.
    pub fn at(base: &str) -> Self {
        let agent = Agent::config_builder()
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(Duration::from_secs(5 * 60)))
            .build()
            .into();
        let base = if base.ends_with('/') {
            base.to_string()
        } else {
            format!("{base}/")
        };
        Self {
            base,
            agent,
            described: Mutex::new(HashMap::new()),
        }
    }

    fn address_of(&self, file: &str) -> String {
        if file.starts_with("http://") || file.starts_with("https://") {
            file.to_string()
        } else {
            format!("{}{file}", self.base)
        }
    }

    fn described(&self, file: &str) -> Result<Described, Unreachable> {
        let known = self
            .described
            .lock()
            .ok()
            .and_then(|described| described.get(file).cloned());
        if let Some(known) = known {
            return Ok(known);
        }
        let response = self
            .agent
            .head(&self.address_of(file))
            .call()
            .map_err(|error| failed(file, error.to_string()))?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => {
                return Err(Unreachable::Missing {
                    file: file.to_string(),
                });
            }
            other => return Err(failed(file, format!("the server answered {other}"))),
        }
        let length = header(&response, "content-length")
            .and_then(|length| length.parse().ok())
            .ok_or_else(|| failed(file, "the server does not say how long the file is".to_string()))?;
        let found = Described {
            length,
            etag: header(&response, "etag").filter(|etag| !etag.starts_with("W/")),
        };
        if let Ok(mut described) = self.described.lock() {
            described.insert(file.to_string(), found.clone());
        }
        Ok(found)
    }
}

impl ForReadingFiles for HttpFiles {
    fn length(&self, file: &str) -> Result<u64, Unreachable> {
        self.described(file).map(|described| described.length)
    }

    fn read(&self, file: &str, range: Range<u64>) -> Result<Vec<u8>, Unreachable> {
        let described = self.described(file)?;
        within(file, &range, described.length)?;
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let asked = format!("bytes={}-{}", range.start, range.end - 1);
        let request = self.agent.get(&self.address_of(file)).header("Range", &asked);
        let request = match &described.etag {
            Some(etag) => request.header("If-Match", etag),
            None => request,
        };
        let response = request.call().map_err(|error| failed(file, error.to_string()))?;
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {}
            StatusCode::NOT_FOUND => {
                return Err(Unreachable::Missing {
                    file: file.to_string(),
                });
            }
            other => return Err(failed(file, format!("the server answered {other} to {asked}"))),
        }
        let starts_where_asked = header(&response, "content-range")
            .is_some_and(|served| served.starts_with(&format!("bytes {}-", range.start)));
        if !starts_where_asked {
            return Err(failed(
                file,
                format!("the server sent another range than {asked}"),
            ));
        }
        exactly(file, &asked, response, range.end - range.start)
    }
}

/// The body of a range response, refused unless it holds exactly the `wanted` bytes.
///
/// ureq refuses a body that reaches its limit without ending, so the limit sits one byte past the
/// range: a server sending more than was asked is still stopped there, and the count says so.
fn exactly(
    file: &str,
    asked: &str,
    mut response: Response<ureq::Body>,
    wanted: u64,
) -> Result<Vec<u8>, Unreachable> {
    let bytes = response
        .body_mut()
        .with_config()
        .limit(wanted + 1)
        .read_to_vec()
        .map_err(|error| failed(file, error.to_string()))?;
    if bytes.len() as u64 != wanted {
        return Err(failed(file, format!("{} bytes came for {asked}", bytes.len())));
    }
    Ok(bytes)
}

fn failed(file: &str, detail: String) -> Unreachable {
    Unreachable::Failed {
        file: file.to_string(),
        detail,
    }
}

fn header(response: &Response<ureq::Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}
