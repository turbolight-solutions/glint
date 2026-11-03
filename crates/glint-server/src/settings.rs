//! Server configuration, read from `GLINT_*` environment variables.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Everything the server is configured with.
#[derive(Debug, Clone)]
pub struct Settings {
    pub bind: SocketAddr,
    pub corpus: Option<PathBuf>,
    pub max_body: usize,
    pub fetch_allowlist: Vec<String>,
    pub max_fetch_bytes: usize,
    pub check_timeout: Duration,
    pub request_timeout: Duration,
    pub max_concurrent_checks: usize,
    /// Requests per minute and client; 0 turns the limit off.
    pub rate_limit_per_minute: u32,
    /// Take the client address from `X-Forwarded-For` and similar headers (only behind a trusted proxy).
    pub trust_proxy: bool,
    pub static_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 8080)),
            corpus: None,
            max_body: 5 * 1024 * 1024,
            fetch_allowlist: ["www.ietf.org", "datatracker.ietf.org", "www.rfc-editor.org"]
                .map(str::to_owned)
                .to_vec(),
            max_fetch_bytes: 5 * 1024 * 1024,
            check_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            max_concurrent_checks: std::thread::available_parallelism().map_or(2, usize::from),
            rate_limit_per_minute: 120,
            trust_proxy: false,
            static_dir: None,
        }
    }
}

/// An environment variable with an unusable value.
#[derive(Debug, thiserror::Error)]
#[error("{variable}: {reason}")]
pub struct SettingsError {
    variable: &'static str,
    reason: String,
}

fn var(name: &'static str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn parsed<T: std::str::FromStr>(name: &'static str) -> Result<Option<T>, SettingsError>
where
    T::Err: std::fmt::Display,
{
    var(name)
        .map(|value| {
            value.trim().parse().map_err(|e: T::Err| SettingsError {
                variable: name,
                reason: e.to_string(),
            })
        })
        .transpose()
}

impl Settings {
    /// Reads `GLINT_BIND`, `GLINT_CORPUS`, `GLINT_MAX_BODY`, `GLINT_FETCH_ALLOWLIST` (comma separated hosts),
    /// `GLINT_CHECK_TIMEOUT_SECS`, `GLINT_REQUEST_TIMEOUT_SECS`, `GLINT_MAX_CONCURRENT_CHECKS`,
    /// `GLINT_RATE_LIMIT_PER_MINUTE`, `GLINT_TRUST_PROXY` and `GLINT_STATIC_DIR`.
    pub fn from_env() -> Result<Self, SettingsError> {
        let mut settings = Self::default();
        if let Some(bind) = parsed("GLINT_BIND")? {
            settings.bind = bind;
        }
        settings.corpus = var("GLINT_CORPUS").map(PathBuf::from);
        if let Some(max) = parsed("GLINT_MAX_BODY")? {
            settings.max_body = max;
        }
        if let Some(list) = var("GLINT_FETCH_ALLOWLIST") {
            settings.fetch_allowlist = list
                .split(',')
                .map(|host| host.trim().to_ascii_lowercase())
                .filter(|host| !host.is_empty())
                .collect();
        }
        if let Some(secs) = parsed("GLINT_CHECK_TIMEOUT_SECS")? {
            settings.check_timeout = Duration::from_secs(secs);
        }
        if let Some(secs) = parsed("GLINT_REQUEST_TIMEOUT_SECS")? {
            settings.request_timeout = Duration::from_secs(secs);
        }
        if let Some(max) = parsed::<usize>("GLINT_MAX_CONCURRENT_CHECKS")? {
            settings.max_concurrent_checks = max.max(1);
        }
        if let Some(limit) = parsed("GLINT_RATE_LIMIT_PER_MINUTE")? {
            settings.rate_limit_per_minute = limit;
        }
        if let Some(trust) = parsed("GLINT_TRUST_PROXY")? {
            settings.trust_proxy = trust;
        }
        settings.static_dir = var("GLINT_STATIC_DIR").map(PathBuf::from);
        Ok(settings)
    }
}
