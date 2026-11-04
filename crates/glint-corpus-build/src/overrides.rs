//! Curated overrides (design spec, "Override policy").

use std::path::Path;

use glint_corpus::{Oid, OverrideTier, document_title, rfc_number};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TierName {
    Extraction,
    ErrataVerified,
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
                TierName::ErrataVerified => OverrideTier::ErrataVerified,
                TierName::Correction => OverrideTier::Correction,
            };
            if matches!(tier, OverrideTier::Correction) && entry.justification.is_none() {
                return Err(format!(
                    "{}: a `correction` override of {} needs a justification",
                    meta_path.display(),
                    entry.module
                ));
            }
            if matches!(tier, OverrideTier::ErrataVerified) && entry.errata_ids.is_empty() {
                return Err(format!(
                    "{}: an `errata-verified` override of {} must cite an erratum",
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
            "extraction override of {} in {} changes tokens; use the `correction` or `errata-verified` tier",
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
    #[serde(rename = "errata_status_code")]
    status: String,
}

/// Checks that every erratum cited by an `errata-verified` override exists, belongs to the RFC and is Verified.
/// Only RFCs have errata, so the tier is refused for modules of other documents.
pub fn check_errata(items: &[Override], errata_json: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(errata_json)
        .map_err(|e| format!("{}: {e}", errata_json.display()))?;
    let all: Vec<Erratum> = serde_json::from_str(&text)
        .map_err(|e| format!("{}: unexpected errata format: {e}", errata_json.display()))?;
    for item in items
        .iter()
        .filter(|i| matches!(i.tier, OverrideTier::ErrataVerified))
    {
        let Some(number) = rfc_number(&item.document) else {
            return Err(format!(
                "`errata-verified` override of {} in {}: only RFCs have errata",
                item.module, item.document
            ));
        };
        for cited in &item.errata_ids {
            let found = all.iter().find(|e| {
                e.id.to_string().trim_matches('"') == cited
                    && e.doc_id.eq_ignore_ascii_case(&format!("RFC{number}"))
            });
            match found {
                Some(e) if e.status.eq_ignore_ascii_case("Verified") => {}
                Some(e) => {
                    return Err(format!(
                        "erratum {cited} for RFC {number} has status {}, not Verified",
                        e.status
                    ));
                }
                None => {
                    return Err(format!("erratum {cited} is not an erratum of RFC {number}"));
                }
            }
        }
    }
    Ok(())
}
