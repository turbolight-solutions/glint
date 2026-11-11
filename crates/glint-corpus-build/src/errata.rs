//! The RFC errata the build applies (`corpus/errata`, design spec "Errata"): each RFC's `patched` errata as a series of
//! patches to the RFC's source, applied before extraction.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use glint_errata::store::{
    patch_format, patched_series, read_ledger, read_regular_file, scan_patches,
};
use glint_errata::{
    Decision, MAX_FEED_BYTES, SeriesFailure, Source, SourceFormat, apply_patch, forbidden_in_patch,
    parse_feed,
};

/// One RFC's patches, `(errata_id, patch)` in id order, or why they cannot be read.
pub type Series = Result<Vec<(u32, String)>, SeriesFailure>;

#[derive(Debug, Default)]
pub struct Errata {
    pub series: BTreeMap<u32, Series>,
}

/// Reads and checks `dir`. Malformed errata data fails the build: the ledger, a stray or unreadable entry under
/// `patches/`, a patch without a `patched` row, an added line with a control or bidi character, and, with `feed`
/// (`errata.json`), a patched erratum that is not a Verified erratum of its RFC. A patch that does not apply is no
/// error here: that RFC is built as published and the report says why.
pub fn read_errata(dir: &Path, feed: Option<&Path>) -> Result<Errata, String> {
    let rows = read_ledger(dir).map_err(|e| e.to_string())?;
    let (files, problems) = scan_patches(dir);
    if !problems.is_empty() {
        return Err(format!("errata patches:\n  {}", problems.join("\n  ")));
    }
    let patched: BTreeSet<(u32, u32)> = rows
        .iter()
        .filter(|r| r.decision == Decision::Patched)
        .map(|r| r.key())
        .collect();
    let orphans: Vec<String> = files
        .iter()
        .filter(|(key, _)| !patched.contains(key))
        .map(|(_, path)| path.display().to_string())
        .collect();
    if !orphans.is_empty() {
        return Err(format!(
            "errata patches without a `patched` ledger row:\n  {}",
            orphans.join("\n  ")
        ));
    }
    if let Some(feed_path) = feed {
        check_verified(&patched, feed_path)?;
    }
    let series = patched_series(dir, &rows, &files);
    for (rfc, list) in &series {
        for (id, patch) in list.iter().flatten() {
            if let Some(problem) = forbidden_in_patch(patch) {
                return Err(format!("the patch of RFC {rfc} erratum {id}: {problem}"));
            }
        }
    }
    Ok(Errata { series })
}

fn check_verified(patched: &BTreeSet<(u32, u32)>, feed_path: &Path) -> Result<(), String> {
    let raw = read_regular_file(feed_path, MAX_FEED_BYTES).map_err(|e| e.to_string())?;
    let feed = parse_feed(&raw).map_err(|e| format!("{}: {e}", feed_path.display()))?;
    let wrong: Vec<String> = patched
        .iter()
        .filter_map(|&(rfc, id)| match feed.errata.get(&id) {
            Some(e) if e.rfc == rfc && e.is_verified() => None,
            Some(e) if e.rfc != rfc => {
                Some(format!("erratum {id} is of RFC {}, not RFC {rfc}", e.rfc))
            }
            Some(e) => Some(format!(
                "RFC {rfc} erratum {id} has status {}, not Verified",
                e.status
            )),
            None => Some(format!(
                "RFC {rfc} erratum {id} is not in {}",
                feed_path.display()
            )),
        })
        .collect();
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "patched errata that are not Verified errata of their RFC:\n  {}",
            wrong.join("\n  ")
        ))
    }
}

/// The texts of `published` after each patch of `series`, in order, or why the series cannot be applied to a source
/// of `format`: a patch for the other format, or a patch that does not apply. All or nothing.
pub fn apply(
    rfc: u32,
    format: SourceFormat,
    published: &str,
    series: &Series,
) -> Result<Vec<(u32, String)>, SeriesFailure> {
    let list = series.as_ref().map_err(Clone::clone)?;
    let source = Source { rfc, format };
    let mut texts: Vec<(u32, String)> = Vec::with_capacity(list.len());
    let mut current = published.to_owned();
    for (id, patch) in list {
        if patch_format(patch) != Some(format) {
            return Err(SeriesFailure {
                errata_id: *id,
                reason: format!(
                    "the patch is not against {}, which the corpus reads",
                    source.file_name()
                ),
            });
        }
        current = apply_patch(&current, patch, source).map_err(|e| SeriesFailure {
            errata_id: *id,
            reason: e.0,
        })?;
        texts.push((*id, current.clone()));
    }
    Ok(texts)
}
