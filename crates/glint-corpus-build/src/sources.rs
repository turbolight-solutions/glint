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
/// the OID, so several entries may share a name. A module whose header gives no OID (13.1 allows an empty
/// "DefinitiveIdentification") is listed without one and identified by its document and name.
#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub body: String,
    pub document: String,
    pub edition: Option<String>,
    pub name: String,
    pub oid: Option<Oid>,
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
    let mut seen_without: HashSet<(&str, &str)> = HashSet::new();
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
        match &entry.oid {
            Some(oid) if !seen.insert(oid) => {
                return Err(format!(
                    "{}: OID {oid} is listed twice (second time as {})",
                    path.display(),
                    entry.name
                ));
            }
            None if !seen_without.insert((&entry.document, &entry.name)) => {
                return Err(format!(
                    "{}: {} of {} is listed twice without an OID",
                    path.display(),
                    entry.name,
                    entry.document
                ));
            }
            _ => {}
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
/// An entry without an OID is taken only by an import without one: an import that gives an OID names a module
/// that has one.
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
            .map(|oid| {
                on_demand()
                    .filter(|e| e.oid.as_ref() == Some(oid))
                    .collect()
            })
            .unwrap_or_default();
        let candidates: Vec<&Entry> = if exact.is_empty() {
            if defined.names.contains(&need.name) {
                Vec::new()
            } else {
                on_demand()
                    .filter(|e| e.name == need.name && (e.oid.is_some() || need.oid.is_none()))
                    .collect()
            }
        } else {
            exact
        };
        for entry in candidates {
            let defined_already = entry
                .oid
                .as_ref()
                .is_some_and(|oid| defined.oids.contains(oid));
            if !defined_already && !chosen.iter().any(|c| std::ptr::eq(*c, entry)) {
                chosen.push(entry);
            }
        }
    }
    chosen.sort_by(|a, b| (&a.oid, &a.document, &a.name).cmp(&(&b.oid, &b.document, &b.name)));
    chosen
}

/// A downloaded module with the source record it will carry.
pub struct Fetched {
    pub name: String,
    pub oid: Option<Oid>,
    pub text: String,
    pub source: ModuleSource,
}

/// Fetches (or reads from `cache`) each of `entries`. A file is used, and cached, only when it is the module the
/// entry lists: a module whose definitive identifier is the entry's OID (13.8; the module of the entry's name if the
/// file has several). Anything else (an HTML page,
/// an error page served with status 200, another edition) is a problem naming the URL, and a cached file that is
/// not the module is downloaded again.
pub fn fetch(entries: &[&Entry], cache: &Path) -> (Vec<Fetched>, Vec<String>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    let _ = std::fs::create_dir_all(cache);
    for entry in entries {
        let file = cache.join(cache_name(entry));
        let cached = std::fs::read(&file)
            .ok()
            .map(|bytes| decode(&bytes))
            .filter(|text| not_the_module(text, entry).is_none());
        let text = match cached {
            Some(text) => Ok(text),
            None => download(&entry.url).and_then(|(bytes, content_type)| {
                let text = decode(&bytes);
                match not_the_module(&text, entry) {
                    None => {
                        let _ = std::fs::write(&file, &bytes);
                        Ok(text)
                    }
                    Some(why) => Err(format!("{why} (content type {content_type})")),
                }
            }),
        };
        match text {
            Ok(text) => out.push(Fetched {
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
                    errata: Vec::new(),
                },
            }),
            Err(why) => problems.push(format!("{} from {}: {why}", entry_label(entry), entry.url)),
        }
    }
    (out, problems)
}

/// The cache file of `entry`: by name and OID, or by document and name for an entry without an OID.
fn cache_name(entry: &Entry) -> String {
    match &entry.oid {
        Some(oid) => format!("{}-{oid}.asn", entry.name),
        None => {
            let document: String = entry
                .document
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '.' {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            format!("{document}-{}.asn", entry.name)
        }
    }
}

/// `entry` in a problem: its name and OID, or its name and document when it has no OID.
fn entry_label(entry: &Entry) -> String {
    match &entry.oid {
        Some(oid) => format!("{} {oid}", entry.name),
        None => format!("{} of {}", entry.name, entry.document),
    }
}

/// Why `text` is not the module `entry` lists, or `None` when it is. With an OID, the module whose definitive
/// identifier is that OID (13.8; the module of the entry's name if the file has several). Without one, a module of
/// the entry's name whose header gives no OID: a module that has one is listed with it.
fn not_the_module(text: &str, entry: &Entry) -> Option<String> {
    let Some((name, found)) = crate::build::module_identity(text, &entry.name) else {
        return Some("not an ASN.1 module".to_owned());
    };
    match (&entry.oid, found) {
        (Some(listed), Some(oid)) if &oid == listed => None,
        (Some(_), Some(oid)) => Some(format!("the module's identifier is {oid}")),
        (Some(_), None) => Some(format!(
            "module {name} has no identifier; list it without `oid`"
        )),
        (None, Some(oid)) => Some(format!(
            "the module's identifier is {oid}; list it with `oid`"
        )),
        (None, None) if name != entry.name => Some(format!("the module is called {name}")),
        (None, None) => None,
    }
}

/// The body of a successful download and its content type.
fn download(url: &str) -> Result<(Vec<u8>, String), String> {
    let response = reqwest::blocking::get(url).map_err(|e| e.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("not given")
        .to_owned();
    let bytes = response.bytes().map_err(|e| e.to_string())?;
    Ok((bytes.to_vec(), content_type))
}

/// The text of a downloaded module. Older modules on ITU-T's site are Windows-1252, not UTF-8: the ISO/IEC ones
/// say "Copyright \u{a9} ISO/IEC" with byte 0xA9. A lossy UTF-8 reading would replace the copyright sign in the
/// notice the module is published with, so a file that is not UTF-8 is read as Windows-1252.
fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| windows_1252(b)).collect(),
    }
}

/// One Windows-1252 byte as a character (WHATWG Encoding Standard, index-windows-1252): 0x80-0x9F are
/// typographic characters, the five bytes the code page leaves unassigned map to the C1 control of that value,
/// and every other byte is the Latin-1 character of that value.
fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8d}', '\u{17d}',
        '\u{8f}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}',
        '\u{2014}', '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}',
        '\u{178}',
    ];
    match byte
        .checked_sub(0x80)
        .and_then(|i| HIGH.get(usize::from(i)))
    {
        Some(&c) => c,
        None => char::from(byte),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, oid: &str, edition: &str) -> Entry {
        Entry {
            name: name.to_owned(),
            oid: Some(oid.parse().unwrap()),
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
            .map(|e| {
                e.oid
                    .as_ref()
                    .map_or_else(|| e.name.clone(), ToString::to_string)
            })
            .collect()
    }

    #[test]
    fn only_the_listed_module_is_the_module() {
        let entry = entry("Standalone", "1.2.3.4", "2012");
        assert_eq!(
            not_the_module("Standalone { 1 2 3 4 } DEFINITIONS ::= BEGIN END", &entry),
            None
        );
        assert_eq!(
            not_the_module("Standalone { 1 2 3 5 } DEFINITIONS ::= BEGIN END", &entry).as_deref(),
            Some("the module's identifier is 1.2.3.5")
        );
        let page =
            "<HTML><PRE>Standalone { 1 2 3 4 } <B>DEFINITIONS</B> ::= <B>BEGIN</B></PRE></HTML>";
        assert_eq!(
            not_the_module(page, &entry).as_deref(),
            Some("not an ASN.1 module")
        );
        assert_eq!(
            not_the_module("Standalone DEFINITIONS ::= BEGIN END", &entry).as_deref(),
            Some("module Standalone has no identifier; list it without `oid`")
        );
    }

    fn without_oid(name: &str) -> Entry {
        Entry {
            oid: None,
            document: "T.101".to_owned(),
            ..entry(name, "1", "1994")
        }
    }

    #[test]
    fn an_entry_without_an_oid_is_a_module_of_its_name_without_one() {
        let entry = without_oid("Videotex");
        assert_eq!(
            not_the_module("Videotex DEFINITIONS ::= BEGIN END", &entry),
            None
        );
        assert_eq!(
            not_the_module("Videotex { 1 2 3 } DEFINITIONS ::= BEGIN END", &entry).as_deref(),
            Some("the module's identifier is 1.2.3; list it with `oid`")
        );
        assert_eq!(
            not_the_module("Teletex DEFINITIONS ::= BEGIN END", &entry).as_deref(),
            Some("the module is called Teletex")
        );
        assert_eq!(cache_name(&entry), "T.101-Videotex.asn");
        assert_eq!(
            cache_name(&Entry {
                document: "ISO/IEC 9541-2".to_owned(),
                ..without_oid("Font")
            }),
            "ISO-IEC-9541-2-Font.asn"
        );
    }

    #[test]
    fn only_an_import_without_an_oid_takes_an_entry_without_one() {
        let manifest = vec![without_oid("Videotex")];
        let by_name = picked(&manifest, &[need("Videotex", None)], &Defined::default());
        assert_eq!(by_name, ["Videotex"]);
        let by_oid = picked(
            &manifest,
            &[need("Videotex", Some("1.2.3"))],
            &Defined::default(),
        );
        assert!(by_oid.is_empty());
        let defined = Defined {
            names: HashSet::from(["Videotex".to_owned()]),
            ..Defined::default()
        };
        assert!(picked(&manifest, &[need("Videotex", None)], &defined).is_empty());
    }

    #[test]
    fn a_module_that_is_not_utf8_is_read_as_windows_1252() {
        // ISO/IEC 8571-4's module on ITU-T's site: a copyright sign and curly quotes.
        let text = decode(b"-- Copyright \xa9 ISO/IEC 1988. \x93No Value Available\x94 \x81");
        assert_eq!(
            text,
            "-- Copyright \u{a9} ISO/IEC 1988. \u{201c}No Value Available\u{201d} \u{81}"
        );
    }

    #[test]
    fn a_utf8_module_is_read_as_it_is() {
        assert_eq!(
            decode("-- Copyright \u{a9} ITU-T".as_bytes()),
            "-- Copyright \u{a9} ITU-T"
        );
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
