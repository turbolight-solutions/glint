//! Downloads for `glint check --draft` and `glint corpus update`.

use std::io::Read;
use std::time::Duration;

use glint_check::{
    DocumentRef, draft_urls, latest_revision_url, revision_from_datatracker, rfc_urls,
};
use glint_extract::Format;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;

const MAX_DOCUMENT_BYTES: u64 = 5 * 1024 * 1024;
const MAX_DATA_BYTES: u64 = 256 * 1024 * 1024;
const IETF_HOSTS: [&str; 3] = ["www.ietf.org", "datatracker.ietf.org", "www.rfc-editor.org"];

pub fn client(redirect_hosts: Option<&'static [&'static str]>) -> Result<Client, String> {
    let policy = Policy::custom(move |attempt| {
        let host_ok = redirect_hosts.is_none_or(|hosts| {
            attempt
                .url()
                .host_str()
                .is_some_and(|host| hosts.contains(&host))
        });
        if attempt.previous().len() >= 5 || attempt.url().scheme() != "https" || !host_ok {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    Client::builder()
        .redirect(policy)
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("glint/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .build()
        .map_err(|e| e.to_string())
}

/// `Ok(None)` when the server says 404.
pub fn get(client: &Client, url: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let response = client.get(url).send().map_err(|e| format!("{url}: {e}"))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("{url}: status {}", response.status()));
    }
    let mut bytes = Vec::new();
    response
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{url}: {e}"))?;
    if u64::try_from(bytes.len()).is_ok_and(|len| len > limit) {
        return Err(format!("{url}: larger than {limit} bytes"));
    }
    Ok(Some(bytes))
}

/// A fetched document: its display name, text and format.
pub struct Fetched {
    pub name: String,
    pub text: String,
    pub format: Format,
}

/// Fetches a draft (latest revision unless the name gives one) or an RFC, preferring RFCXML to text.
pub fn fetch_document(reference: &DocumentRef) -> Result<Fetched, String> {
    let client = client(Some(&IETF_HOSTS))?;
    let (name, urls) = match reference {
        DocumentRef::Draft { name, rev } => {
            let rev = match rev {
                Some(rev) => rev.clone(),
                None => latest_revision(&client, name)?,
            };
            (format!("{name}-{rev}"), draft_urls(name, &rev))
        }
        DocumentRef::Rfc(number) => (format!("rfc{number}"), rfc_urls(*number)),
    };
    for url in &urls {
        if let Some(bytes) = get(&client, url, MAX_DOCUMENT_BYTES)? {
            let format = if url.ends_with(".xml") {
                Format::Xml
            } else {
                Format::Text
            };
            let extension = if matches!(format, Format::Xml) {
                "xml"
            } else {
                "txt"
            };
            return Ok(Fetched {
                name: format!("{name}.{extension}"),
                text: String::from_utf8_lossy(&bytes).into_owned(),
                format,
            });
        }
    }
    Err(format!("{name} was not found at the IETF archive"))
}

fn latest_revision(client: &Client, draft: &str) -> Result<String, String> {
    let body = get(client, &latest_revision_url(draft), 1024 * 1024)?
        .ok_or_else(|| format!("the datatracker does not know {draft}"))?;
    revision_from_datatracker(&String::from_utf8_lossy(&body))
        .ok_or_else(|| format!("could not find the latest revision of {draft}"))
}

/// Downloads a data file of the site; `None` when it is not there.
pub fn fetch_data(url: &str) -> Result<Option<Vec<u8>>, String> {
    let client = client(None)?;
    get(&client, url, MAX_DATA_BYTES)
}
