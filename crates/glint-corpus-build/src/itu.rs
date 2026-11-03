//! ITU-T modules that corpus modules import (design spec, "Non-RFC modules").

use std::collections::HashSet;
use std::path::Path;

use glint_corpus::SourceKind;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub name: String,
    pub url: String,
    pub recommendation: String,
    pub edition: String,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    module: Vec<Entry>,
}

pub fn read_manifest(path: &Path) -> Result<Vec<Entry>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest: Manifest =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(manifest.module)
}

/// A downloaded module with the source record it will carry.
pub struct Fetched {
    pub name: String,
    pub text: String,
    pub kind: SourceKind,
}

/// Fetches (or reads from `cache`) every manifest entry whose name is in `needed`.
pub fn fetch_needed(
    manifest: &[Entry],
    cache: &Path,
    needed: &HashSet<String>,
    already: &HashSet<String>,
) -> (Vec<Fetched>, Vec<String>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    let _ = std::fs::create_dir_all(cache);
    for entry in manifest
        .iter()
        .filter(|e| needed.contains(&e.name) && !already.contains(&e.name))
    {
        let file = cache.join(format!("{}.asn", entry.name));
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => Some(text),
            Err(_) => download(&entry.url).inspect(|body| {
                let _ = std::fs::write(&file, body);
            }),
        };
        match text {
            Some(text) => out.push(Fetched {
                name: entry.name.clone(),
                text,
                kind: SourceKind::Itu {
                    url: entry.url.clone(),
                    recommendation: entry.recommendation.clone(),
                    edition: entry.edition.clone(),
                },
            }),
            None => problems.push(format!("could not fetch {} from {}", entry.name, entry.url)),
        }
    }
    (out, problems)
}

fn download(url: &str) -> Option<String> {
    let response = reqwest::blocking::get(url).ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}
