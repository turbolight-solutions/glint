//! Curated overrides (design spec, "Override policy").

use std::path::Path;

use glint_corpus::OverrideTier;
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
    rfc: u32,
    module: String,
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
    pub rfc: u32,
    pub module: String,
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
    let mut rfc_dirs: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    rfc_dirs.sort();
    for rfc_dir in rfc_dirs.into_iter().filter(|p| p.is_dir()) {
        let meta_path = rfc_dir.join("meta.toml");
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
            let asn_path = rfc_dir.join(format!("{}.asn", entry.module));
            let text = std::fs::read_to_string(&asn_path)
                .map_err(|e| format!("{}: {e}", asn_path.display()))?;
            out.push(Override {
                rfc: entry.rfc,
                module: entry.module,
                tier,
                errata_ids: entry.errata_ids,
                justification: entry.justification,
                text,
            });
        }
    }
    Ok(out)
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
            "extraction override of {} in RFC {} has no extracted module to compare with",
            item.module, item.rfc
        ));
    };
    if significant_tokens(extracted) == significant_tokens(&item.text) {
        Ok(())
    } else {
        Err(format!(
            "extraction override of {} in RFC {} changes tokens; use the `correction` or `errata-verified` tier",
            item.module, item.rfc
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
pub fn check_errata(items: &[Override], errata_json: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(errata_json)
        .map_err(|e| format!("{}: {e}", errata_json.display()))?;
    let all: Vec<Erratum> = serde_json::from_str(&text)
        .map_err(|e| format!("{}: unexpected errata format: {e}", errata_json.display()))?;
    for item in items
        .iter()
        .filter(|i| matches!(i.tier, OverrideTier::ErrataVerified))
    {
        for cited in &item.errata_ids {
            let found = all.iter().find(|e| {
                e.id.to_string().trim_matches('"') == cited
                    && e.doc_id.eq_ignore_ascii_case(&format!("RFC{}", item.rfc))
            });
            match found {
                Some(e) if e.status.eq_ignore_ascii_case("Verified") => {}
                Some(e) => {
                    return Err(format!(
                        "erratum {cited} for RFC {} has status {}, not Verified",
                        item.rfc, e.status
                    ));
                }
                None => {
                    return Err(format!(
                        "erratum {cited} is not an erratum of RFC {}",
                        item.rfc
                    ));
                }
            }
        }
    }
    Ok(())
}
