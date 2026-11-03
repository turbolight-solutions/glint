//! Fetching drafts and RFCs from the IETF hosts.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::redirect::Policy;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("not found")]
    NotFound,
    #[error("the document is larger than the limit")]
    TooLarge,
    #[error("host `{0}` is not allowed")]
    HostNotAllowed(String),
    #[error("upstream error: {0}")]
    Upstream(String),
}

/// Reads a document by URL. The server uses [`HttpFetcher`]; tests supply their own.
pub trait Fetcher: Send + Sync {
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, FetchError>>;
}

/// Fetches over HTTPS from an allowlist of hosts, following redirects only within it.
pub struct HttpFetcher {
    client: reqwest::Client,
    allowlist: Vec<String>,
    max_bytes: usize,
}

fn allowed(allowlist: &[String], url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| allowlist.iter().any(|a| a.eq_ignore_ascii_case(host)))
}

impl HttpFetcher {
    pub fn new(allowlist: Vec<String>, max_bytes: usize) -> Result<Self, reqwest::Error> {
        let redirect_allowlist = allowlist.clone();
        let policy = Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 || !allowed(&redirect_allowlist, attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        let client = reqwest::Client::builder()
            .redirect(policy)
            .timeout(Duration::from_secs(10))
            .user_agent("glint-server")
            .build()?;
        Ok(Self {
            client,
            allowlist,
            max_bytes,
        })
    }
}

impl Fetcher for HttpFetcher {
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, FetchError>> {
        Box::pin(async move {
            let parsed =
                reqwest::Url::parse(url).map_err(|e| FetchError::Upstream(e.to_string()))?;
            if !allowed(&self.allowlist, &parsed) {
                return Err(FetchError::HostNotAllowed(
                    parsed.host_str().unwrap_or_default().to_owned(),
                ));
            }
            let mut response = self
                .client
                .get(parsed)
                .send()
                .await
                .map_err(|e| FetchError::Upstream(e.to_string()))?;
            let status = response.status();
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(FetchError::NotFound);
            }
            if !status.is_success() {
                return Err(FetchError::Upstream(format!("status {status}")));
            }
            let mut bytes: Vec<u8> = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| FetchError::Upstream(e.to_string()))?
            {
                if bytes.len().saturating_add(chunk.len()) > self.max_bytes {
                    return Err(FetchError::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        })
    }
}
