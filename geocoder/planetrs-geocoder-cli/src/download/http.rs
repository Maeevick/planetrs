//! The planet over HTTP: rustls with bundled roots, no OpenSSL, proxies read from the environment.

use std::io::{self, Write};
use std::time::Duration;

use ureq::Agent;
use ureq::http::StatusCode;

use super::ranges::{content_range, range_header};
use super::{Remote, Reply, Source};

/// Who is asking, as the OpenStreetMap Foundation's usage policy wants every client to say.
pub const USER_AGENT: &str = concat!(
    "planetrs-geocoder/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/maeevick/planetrs)"
);

const CHECKSUM_LIMIT: u64 = 64 * 1024;

/// A [`Source`] over HTTP and HTTPS.
pub struct Http {
    agent: Agent,
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Http {
    /// An HTTP source with the timeouts a planet needs: a range of 64 MiB must arrive within a
    /// quarter of an hour, about 75 kB/s, and a slower one counts as a broken request to resume.
    pub fn new() -> Self {
        let agent = Agent::config_builder()
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(Duration::from_secs(15 * 60)))
            .build()
            .into();
        Self { agent }
    }
}

fn answered(url: &str, status: StatusCode) -> io::Error {
    io::Error::other(format!("{url} answered {status}"))
}

fn header(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

impl Source for Http {
    fn text(&self, url: &str) -> io::Result<String> {
        let mut response = self.agent.get(url).call().map_err(ureq::Error::into_io)?;
        if response.status() != StatusCode::OK {
            return Err(answered(url, response.status()));
        }
        response
            .body_mut()
            .with_config()
            .limit(CHECKSUM_LIMIT)
            .read_to_string()
            .map_err(ureq::Error::into_io)
    }

    fn describe(&self, url: &str) -> io::Result<Remote> {
        let response = self.agent.head(url).call().map_err(ureq::Error::into_io)?;
        if !response.status().is_success() {
            return Err(answered(url, response.status()));
        }
        let length = header(&response, "content-length")
            .and_then(|length| length.parse().ok())
            .ok_or_else(|| io::Error::other(format!("{url} does not say how long it is")))?;
        let validator = header(&response, "etag")
            .filter(|tag| !tag.starts_with("W/"))
            .or_else(|| header(&response, "last-modified"));
        Ok(Remote { length, validator })
    }

    fn range(
        &self,
        url: &str,
        from: u64,
        to: u64,
        validator: Option<&str>,
        sink: &mut dyn Write,
    ) -> io::Result<Reply> {
        let mut request = self.agent.get(url).header("Range", range_header(from, to));
        if let Some(validator) = validator {
            request = request.header("If-Range", validator);
        }
        let mut response = request.call().map_err(ureq::Error::into_io)?;
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                let starts_where_asked = header(&response, "content-range")
                    .and_then(|range| content_range(&range))
                    .is_some_and(|range| range.first == from);
                if !starts_where_asked {
                    return Err(io::Error::other(format!(
                        "{url} sent another range than the one asked for"
                    )));
                }
                io::copy(&mut response.body_mut().as_reader(), sink)?;
                Ok(Reply::Range)
            }
            StatusCode::OK => Ok(Reply::Whole),
            StatusCode::RANGE_NOT_SATISFIABLE => Ok(Reply::PastTheEnd),
            other => Err(answered(url, other)),
        }
    }

    fn whole(&self, url: &str, sink: &mut dyn Write) -> io::Result<()> {
        let mut response = self
            .agent
            .get(url)
            .config()
            .timeout_recv_body(None)
            .build()
            .call()
            .map_err(ureq::Error::into_io)?;
        if response.status() != StatusCode::OK {
            return Err(answered(url, response.status()));
        }
        io::copy(&mut response.body_mut().as_reader(), sink)?;
        Ok(())
    }
}
