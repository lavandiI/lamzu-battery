//! HTTP access for the updater.
//!
//! Redirects are followed manually so every hop can be checked against
//! [`UrlPolicy`], and responses are streamed so a large download never has to
//! fit in memory. Both decisions mirror the security properties of the Python
//! implementation.

use std::io::Read;
use std::time::Duration;

use crate::updater::UpdateError;

/// Metadata of one HTTP exchange.
#[derive(Debug, Clone, Default)]
pub struct HttpMeta {
    /// Status code.
    pub status: u16,
    /// `Location` header of a redirect.
    pub location: Option<String>,
}

/// Minimal HTTP surface used by the updater.
pub trait HttpTransport: Send + Sync {
    /// Perform a GET request and pass body chunks to `sink`.
    fn get_stream(
        &self,
        url: &str,
        accept: &str,
        timeout: Duration,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), UpdateError>,
    ) -> Result<HttpMeta, UpdateError>;

    /// Perform a GET request and return at most `max` body bytes.
    fn get_bytes(
        &self,
        url: &str,
        accept: &str,
        timeout: Duration,
        max: u64,
    ) -> Result<Vec<u8>, UpdateError> {
        let mut body = Vec::new();
        let mut overflow = false;
        let meta = self.get_stream(url, accept, timeout, &mut |chunk| {
            if body.len() as u64 + chunk.len() as u64 > max {
                overflow = true;
                return Err(UpdateError::network("response is larger than expected"));
            }
            body.extend_from_slice(chunk);
            Ok(())
        })?;
        if overflow {
            return Err(UpdateError::network("response is larger than expected"));
        }
        if !(200..300).contains(&meta.status) {
            return Err(UpdateError::network(format!(
                "server returned HTTP {}",
                meta.status
            )));
        }
        Ok(body)
    }
}

/// Which URLs the updater is allowed to talk to.
#[derive(Debug, Clone)]
pub struct UrlPolicy {
    /// Also accept any HTTPS host (tests and local servers only).
    pub allow_any_https: bool,
    /// Repository prefix inside `github.com`.
    pub repository_prefix: String,
}

impl Default for UrlPolicy {
    fn default() -> Self {
        Self::github()
    }
}

impl UrlPolicy {
    /// Policy that only trusts this repository's release assets on GitHub.
    #[must_use]
    pub fn github() -> Self {
        Self {
            allow_any_https: false,
            repository_prefix: "/lavandii/lamzu-battery/releases/download/".to_owned(),
        }
    }

    /// Policy used by tests with a local HTTP server.
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            allow_any_https: true,
            repository_prefix: "/".to_owned(),
        }
    }

    /// A release asset URL as returned by the GitHub API.
    #[must_use]
    pub fn trusted_asset(&self, url: &str) -> bool {
        let Some((scheme, host, path)) = split_url(url) else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("https") && !self.allow_any_https {
            return false;
        }
        if self.allow_any_https {
            return true;
        }
        let host = host.to_ascii_lowercase();
        if host != "github.com" && host != "www.github.com" {
            return false;
        }
        path.to_ascii_lowercase()
            .starts_with(&self.repository_prefix.to_ascii_lowercase())
    }

    /// A URL that a release asset download may be redirected to.
    #[must_use]
    pub fn trusted_download(&self, url: &str) -> bool {
        let Some((scheme, host, _path)) = split_url(url) else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("https") && !self.allow_any_https {
            return false;
        }
        if self.allow_any_https {
            return true;
        }
        let host = host.to_ascii_lowercase();
        if host == "github.com" || host == "www.github.com" {
            return self.trusted_asset(url);
        }
        host == "githubusercontent.com" || host.ends_with(".githubusercontent.com")
    }
}

/// Split an absolute URL into `(scheme, host, path)` without taking a URL
/// parser dependency; user info and a port are stripped from the host.
#[must_use]
pub fn split_url(url: &str) -> Option<(&str, &str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty() || rest.is_empty() {
        return None;
    }
    let (authority, path) = rest
        .find('/')
        .map_or((rest, "/"), |index| rest.split_at(index));
    let host = authority.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then_some((scheme, host, path))
}

/// `ureq`-backed transport.
#[derive(Debug, Clone, Copy, Default)]
pub struct UreqTransport;

fn agent(timeout: Duration) -> Result<ureq::Agent, UpdateError> {
    let config = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(timeout))
        .build();
    Ok(config.into())
}

impl HttpTransport for UreqTransport {
    fn get_stream(
        &self,
        url: &str,
        accept: &str,
        timeout: Duration,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), UpdateError>,
    ) -> Result<HttpMeta, UpdateError> {
        let agent = agent(timeout)?;
        let response = agent
            .get(url)
            .header("Accept", accept)
            .header("User-Agent", crate::USER_AGENT)
            .call()
            .map_err(|error| UpdateError::network(format!("request failed: {error}")))?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if (300..400).contains(&status) {
            return Ok(HttpMeta { status, location });
        }
        let mut reader = response.into_body().into_reader();
        let mut buffer = vec![0_u8; crate::DOWNLOAD_CHUNK];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| UpdateError::network(format!("read failed: {error}")))?;
            if read == 0 {
                break;
            }
            sink(&buffer[..read])?;
        }
        Ok(HttpMeta { status, location })
    }
}
