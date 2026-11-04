//! Standalone modules that standards bodies publish as files of their own (design spec, "Non-RFC modules"): the
//! sources manifest, which of its entries a build needs, and fetching them.

use std::collections::HashSet;
use std::path::Path;

use glint_corpus::{BODY_RFC, ModuleSource, Oid, SourceKind};
use serde::Deserialize;

/// When a module is bundled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Include {
    /// Only when a corpus module imports it and nothing in the corpus satisfies the import.
    #[default]
    WhenImported,
    Always,
}

/// A standards body and the notice that credits it in the bundle.
#[derive(Debug, Clone, Deserialize)]
pub struct Body {
    pub id: String,
    pub notice: String,
}

/// One downloadable module. The OID is the identity: editions of a specification keep the module name and change
/// the OID, so several entries may share a name.
#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub body: String,
    pub document: String,
    pub edition: Option<String>,
    pub name: String,
    pub oid: Oid,
    pub url: String,
    #[serde(default)]
    pub include: Include,
}

#[derive(Debug, Default, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub body: Vec<Body>,
    #[serde(default)]
    pub module: Vec<Entry>,
}

pub fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest: Manifest =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut seen: HashSet<&Oid> = HashSet::new();
    for entry in &manifest.module {
        if entry.body == BODY_RFC {
            return Err(format!(
                "{}: {} is listed under body `{BODY_RFC}`; RFC modules come from the RFC mirror",
                path.display(),
                entry.name
            ));
        }
        if !manifest.body.iter().any(|b| b.id == entry.body) {
            return Err(format!(
                "{}: {} names body `{}`, which has no [[body]] entry with a notice",
                path.display(),
                entry.name,
                entry.body
            ));
        }
        if !seen.insert(&entry.oid) {
            return Err(format!(
                "{}: OID {} is listed twice (second time as {})",
                path.display(),
                entry.oid,
                entry.name
            ));
        }
    }
    Ok(manifest)
}

impl Manifest {
    /// The entries to bundle in every build.
    pub fn always(&self) -> Vec<&Entry> {
        self.module
            .iter()
            .filter(|e| e.include == Include::Always)
            .collect()
    }
}

/// An import that no module in the corpus satisfies yet.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Need {
    pub name: String,
    /// The OID of the import's `AssignedIdentifier`, when it is a literal.
    pub oid: Option<Oid>,
}

/// What the corpus defines already.
#[derive(Debug, Default)]
pub struct Defined {
    pub names: HashSet<String>,
    pub oids: HashSet<Oid>,
}

/// The `when-imported` entries to download for `needs`. An import with an OID takes the entry with that OID.
/// Otherwise, or when the manifest has no such OID, it takes the entries of that name, but only while no module of
/// the name is defined at all, so a name-only import never pulls in a second edition of a module the corpus has.
pub fn select<'a>(
    manifest: &'a [Entry],
    needs: &HashSet<Need>,
    defined: &Defined,
) -> Vec<&'a Entry> {
    let on_demand = || {
        manifest
            .iter()
            .filter(|e| e.include == Include::WhenImported)
    };
    let mut chosen: Vec<&Entry> = Vec::new();
    for need in needs {
        let exact: Vec<&Entry> = need
            .oid
            .as_ref()
            .map(|oid| on_demand().filter(|e| &e.oid == oid).collect())
            .unwrap_or_default();
        let candidates: Vec<&Entry> = if exact.is_empty() {
            if defined.names.contains(&need.name) {
                Vec::new()
            } else {
                on_demand().filter(|e| e.name == need.name).collect()
            }
        } else {
            exact
        };
        for entry in candidates {
            if !defined.oids.contains(&entry.oid) && !chosen.iter().any(|c| c.oid == entry.oid) {
                chosen.push(entry);
            }
        }
    }
    chosen.sort_by(|a, b| a.oid.cmp(&b.oid));
    chosen
}

/// A downloaded module with the source record it will carry.
pub struct Fetched {
    pub name: String,
    pub oid: Oid,
    pub text: String,
    pub source: ModuleSource,
}

/// Fetches (or reads from `cache`) each of `entries`.
pub fn fetch(entries: &[&Entry], cache: &Path) -> (Vec<Fetched>, Vec<String>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    let _ = std::fs::create_dir_all(cache);
    for entry in entries {
        let file = cache.join(format!("{}-{}.asn", entry.name, entry.oid));
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => Some(text),
            Err(_) => download(&entry.url).inspect(|body| {
                let _ = std::fs::write(&file, body);
            }),
        };
        match text {
            Some(text) => out.push(Fetched {
                name: entry.name.clone(),
                oid: entry.oid.clone(),
                text,
                source: ModuleSource {
                    body: entry.body.clone(),
                    document: entry.document.clone(),
                    edition: entry.edition.clone(),
                    url: Some(entry.url.clone()),
                    section: None,
                    kind: SourceKind::Standalone,
                },
            }),
            None => problems.push(format!(
                "could not fetch {} {} from {}",
                entry.name, entry.oid, entry.url
            )),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, oid: &str, edition: &str) -> Entry {
        Entry {
            name: name.to_owned(),
            oid: oid.parse().unwrap(),
            url: format!("https://example.test/{name}/{edition}.asn"),
            body: "itu-t".to_owned(),
            document: "X.509".to_owned(),
            edition: Some(edition.to_owned()),
            include: Include::WhenImported,
        }
    }

    fn need(name: &str, oid: Option<&str>) -> Need {
        Need {
            name: name.to_owned(),
            oid: oid.map(|o| o.parse().unwrap()),
        }
    }

    fn manifest() -> Vec<Entry> {
        vec![
            entry("AuthenticationFramework", "2.5.1.7.5", "2005"),
            entry("AuthenticationFramework", "2.5.1.7.7", "2012"),
            entry("InformationFramework", "2.5.1.1.7", "2012"),
        ]
    }

    fn picked(manifest: &[Entry], needs: &[Need], defined: &Defined) -> Vec<String> {
        let needs: HashSet<Need> = needs.iter().cloned().collect();
        select(manifest, &needs, defined)
            .iter()
            .map(|e| e.oid.to_string())
            .collect()
    }

    #[test]
    fn an_import_with_an_oid_takes_exactly_that_edition() {
        let manifest = manifest();
        let got = picked(
            &manifest,
            &[need("AuthenticationFramework", Some("2.5.1.7.5"))],
            &Defined::default(),
        );
        assert_eq!(got, ["2.5.1.7.5"]);
    }

    #[test]
    fn editions_with_one_name_are_separate_entries() {
        let manifest = manifest();
        let got = picked(
            &manifest,
            &[
                need("AuthenticationFramework", Some("2.5.1.7.5")),
                need("AuthenticationFramework", Some("2.5.1.7.7")),
            ],
            &Defined::default(),
        );
        assert_eq!(got, ["2.5.1.7.5", "2.5.1.7.7"]);
    }

    #[test]
    fn an_import_without_an_oid_takes_every_edition_while_the_name_is_undefined() {
        let manifest = manifest();
        let got = picked(
            &manifest,
            &[need("AuthenticationFramework", None)],
            &Defined::default(),
        );
        assert_eq!(got, ["2.5.1.7.5", "2.5.1.7.7"]);
    }

    #[test]
    fn a_name_only_import_is_satisfied_by_any_defined_module_of_that_name() {
        let manifest = manifest();
        let defined = Defined {
            names: ["AuthenticationFramework".to_owned()].into(),
            oids: ["2.5.1.7.5".parse().unwrap()].into(),
        };
        let got = picked(
            &manifest,
            &[need("AuthenticationFramework", None)],
            &defined,
        );
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn an_oid_import_still_fetches_its_edition_when_another_edition_is_defined() {
        let manifest = manifest();
        let defined = Defined {
            names: ["AuthenticationFramework".to_owned()].into(),
            oids: ["2.5.1.7.5".parse().unwrap()].into(),
        };
        let got = picked(
            &manifest,
            &[need("AuthenticationFramework", Some("2.5.1.7.7"))],
            &defined,
        );
        assert_eq!(got, ["2.5.1.7.7"]);
    }

    #[test]
    fn an_oid_the_manifest_lacks_falls_back_to_the_name() {
        let manifest = manifest();
        let got = picked(
            &manifest,
            &[need("InformationFramework", Some("2.5.1.1.3"))],
            &Defined::default(),
        );
        assert_eq!(got, ["2.5.1.1.7"]);
    }

    #[test]
    fn nothing_is_fetched_twice() {
        let manifest = manifest();
        let defined = Defined {
            names: HashSet::new(),
            oids: ["2.5.1.1.7".parse().unwrap()].into(),
        };
        let got = picked(&manifest, &[need("InformationFramework", None)], &defined);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn an_always_entry_is_never_selected_for_an_import() {
        let mut manifest = manifest();
        manifest[2].include = Include::Always;
        let got = picked(
            &manifest,
            &[need("InformationFramework", None)],
            &Defined::default(),
        );
        assert!(got.is_empty(), "{got:?}");
    }
}
