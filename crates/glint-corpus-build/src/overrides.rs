//! Curated overrides (design spec, "Override policy").

use std::collections::HashMap;
use std::path::Path;

use glint_corpus::{AllowedOid, Oid, OverrideTier, document_title};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TierName {
    Extraction,
    Correction,
}

#[derive(Debug, Deserialize)]
struct Entry {
    tier: TierName,
    document: String,
    module: String,
    oid: Option<String>,
    #[serde(default)]
    errata_ids: Vec<String>,
    justification: Option<String>,
    #[allow(dead_code)]
    author: Option<String>,
    #[allow(dead_code)]
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Meta {
    #[serde(rename = "override")]
    overrides: Vec<Entry>,
}

#[derive(Debug, Clone)]
pub struct Override {
    /// The document id of the module's source: `rfc4998`, `X.509`.
    pub document: String,
    pub module: String,
    /// Definitive identifier of the module this replaces, as extracted. Needed when the RFC defines several
    /// modules with this name.
    pub oid: Option<Oid>,
    pub tier: OverrideTier,
    pub errata_ids: Vec<String>,
    pub justification: Option<String>,
    pub text: String,
}

pub fn read_overrides(dir: &Path) -> Result<Vec<Override>, String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(out);
    };
    let mut document_dirs: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    document_dirs.sort();
    for document_dir in document_dirs.into_iter().filter(|p| p.is_dir()) {
        let meta_path = document_dir.join("meta.toml");
        let meta_text = std::fs::read_to_string(&meta_path)
            .map_err(|e| format!("{}: {e}", meta_path.display()))?;
        let meta: Meta =
            toml::from_str(&meta_text).map_err(|e| format!("{}: {e}", meta_path.display()))?;
        for entry in meta.overrides {
            let tier = match entry.tier {
                TierName::Extraction => OverrideTier::Extraction,
                TierName::Correction => OverrideTier::Correction,
            };
            if matches!(tier, OverrideTier::Correction) && entry.justification.is_none() {
                return Err(format!(
                    "{}: a `correction` override of {} needs a justification",
                    meta_path.display(),
                    entry.module
                ));
            }
            let oid = entry
                .oid
                .as_deref()
                .map(|text| {
                    text.parse::<Oid>().map_err(|e| {
                        format!("{}: oid of {}: {e}", meta_path.display(), entry.module)
                    })
                })
                .transpose()?;
            let asn_path = document_dir.join(override_file_name(&entry.module, oid.as_ref()));
            let text = std::fs::read_to_string(&asn_path)
                .map_err(|e| format!("{}: {e}", asn_path.display()))?;
            out.push(Override {
                document: entry.document,
                module: entry.module,
                oid,
                tier,
                errata_ids: entry.errata_ids,
                justification: entry.justification,
                text,
            });
        }
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowlistEntry {
    oid: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Allowlist {
    #[serde(default)]
    oid: Vec<AllowlistEntry>,
}

/// The file of wrong but deployed OIDs, in the overrides directory.
pub const ALLOWLIST_FILE: &str = "oid-allowlist.toml";

/// Reads `oid-allowlist.toml` of the overrides directory; no file is an empty list.
pub fn read_allowlist(dir: &Path) -> Result<Vec<AllowedOid>, String> {
    let path = dir.join(ALLOWLIST_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let list: Allowlist = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    list.oid
        .into_iter()
        .map(|entry| {
            let oid = entry
                .oid
                .parse::<Oid>()
                .map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(AllowedOid {
                oid,
                reason: entry.reason,
            })
        })
        .collect()
}

/// `<Module>.asn`, or `<Module>-<oid>.asn` when the entry names the OID of the module it replaces.
fn override_file_name(module: &str, oid: Option<&Oid>) -> String {
    match oid {
        Some(oid) => format!("{module}-{oid}.asn"),
        None => format!("{module}.asn"),
    }
}

fn significant_tokens(text: &str) -> Vec<String> {
    let (tokens, _) = glint_syntax::lex(text);
    tokens
        .iter()
        .filter(|t| !t.kind.is_trivia())
        .filter_map(|t| text.get(usize::from(t.range.start())..usize::from(t.range.end())))
        .map(str::to_owned)
        .collect()
}

/// An `extraction` override may only change layout: its tokens must equal those of the extracted module
/// (design spec, "Override policy").
pub fn check_extraction_tier(item: &Override, extracted: Option<&str>) -> Result<(), String> {
    if !matches!(item.tier, OverrideTier::Extraction) {
        return Ok(());
    }
    let Some(extracted) = extracted else {
        return Err(format!(
            "extraction override of {} in {} has no module to compare with",
            item.module,
            document_title(&item.document)
        ));
    };
    if significant_tokens(extracted) == significant_tokens(&item.text) {
        Ok(())
    } else {
        Err(format!(
            "extraction override of {} in {} changes tokens; use the `correction` tier, or an errata patch (`corpus/errata`) for a Verified erratum",
            item.module,
            document_title(&item.document)
        ))
    }
}

#[derive(Debug, Deserialize)]
struct Erratum {
    #[serde(alias = "errata_id")]
    id: serde_json::Value,
    #[serde(rename = "doc-id")]
    doc_id: String,
}

impl Erratum {
    fn id(&self) -> String {
        self.id.to_string().trim_matches('"').to_owned()
    }
}

fn read_errata(errata_json: &Path) -> Result<Vec<Erratum>, String> {
    let text = std::fs::read_to_string(errata_json)
        .map_err(|e| format!("{}: {e}", errata_json.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("{}: unexpected errata format: {e}", errata_json.display()))
}

/// The document id (`rfc6210`) each erratum is reported against, by erratum id.
pub fn errata_documents(errata_json: &Path) -> Result<HashMap<String, String>, String> {
    Ok(read_errata(errata_json)?
        .into_iter()
        .map(|e| (e.id(), e.doc_id.to_ascii_lowercase()))
        .collect())
}
