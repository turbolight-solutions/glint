//! The committed errata data on disk: `<dir>/ledger.tsv` and `<dir>/patches/rfcN/<errata_id>.patch`, in Glint
//! `corpus/errata`. Symbolic links are refused everywhere, and every read has a size limit.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::feed::ascii_number;
use crate::ledger::{Decision, LedgerError, Row, format_ledger, parse_ledger};
use crate::patch::{MAX_PATCH_BYTES, SeriesFailure, SourceFormat};

pub const LEDGER_FILE: &str = "ledger.tsv";
pub const PATCHES_DIR: &str = "patches";
/// The largest ledger accepted (about 30 KB in 2026).
const MAX_LEDGER_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
    #[error("{0}: {1}")]
    Ledger(PathBuf, LedgerError),
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.to_owned(),
        source,
    }
}

fn invalid(path: &Path, message: impl Into<String>) -> StoreError {
    StoreError::Invalid {
        path: path.to_owned(),
        message: message.into(),
    }
}

/// Reads `path`, which must be a regular file (not a symbolic link) of at most `max_bytes`.
pub fn read_regular_file(path: &Path, max_bytes: usize) -> Result<Vec<u8>, StoreError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error(path))?;
    if !metadata.file_type().is_file() {
        return Err(invalid(
            path,
            "not a regular file (symbolic links are refused)",
        ));
    }
    let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    let mut data = Vec::new();
    fs::File::open(path)
        .map_err(io_error(path))?
        .take(limit.saturating_add(1))
        .read_to_end(&mut data)
        .map_err(io_error(path))?;
    if data.len() > max_bytes {
        return Err(invalid(path, format!("larger than {max_bytes} bytes")));
    }
    Ok(data)
}

fn read_utf8(path: &Path, max_bytes: usize) -> Result<String, StoreError> {
    String::from_utf8(read_regular_file(path, max_bytes)?).map_err(|_| invalid(path, "not UTF-8"))
}

/// Creates `path` as a directory if it is missing; refuses it if it is anything else, a symbolic link included.
fn real_dir(path: &Path) -> Result<(), StoreError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_dir() => Ok(()),
        Ok(_) => Err(invalid(
            path,
            "not a directory (symbolic links are refused)",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(io_error(path))
        }
        Err(e) => Err(io_error(path)(e)),
    }
}

/// Replaces `path` with `data` in one step: a new temporary file in the same directory (created exclusively, so a
/// planted symbolic link is not followed), synced, then renamed over `path` (which replaces a link, never follows
/// it). The directory must exist.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), StoreError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut f| f.write_all(data).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map_err(io_error(path))
}

pub fn ledger_path(dir: &Path) -> PathBuf {
    dir.join(LEDGER_FILE)
}

pub fn patch_path(dir: &Path, rfc: u32, errata_id: u32) -> PathBuf {
    dir.join(PATCHES_DIR)
        .join(format!("rfc{rfc}"))
        .join(format!("{errata_id}.patch"))
}

/// The ledger rows; a missing ledger is an empty one.
pub fn read_ledger(dir: &Path) -> Result<Vec<Row>, StoreError> {
    let path = ledger_path(dir);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        _ => {}
    }
    let text = read_utf8(&path, MAX_LEDGER_BYTES)?;
    parse_ledger(&text).map_err(|e| StoreError::Ledger(path, e))
}

pub fn write_ledger(dir: &Path, rows: &[Row]) -> Result<(), StoreError> {
    real_dir(dir)?;
    let path = ledger_path(dir);
    let text = format_ledger(rows).map_err(|e| StoreError::Ledger(path.clone(), e))?;
    atomic_write(&path, text.as_bytes())
}

/// A plain number in a file name: ASCII digits without leading zeros.
fn name_number(text: &str) -> Option<u32> {
    let n = ascii_number(text)?;
    (n.to_string() == text).then_some(n)
}

/// The committed patch files, `{(rfc, errata_id): path}`, and every entry of `patches/` that is not one: names other
/// than `rfc<N>/<N>.patch`, symbolic links, anything but directories and regular files.
pub fn scan_patches(dir: &Path) -> (BTreeMap<(u32, u32), PathBuf>, Vec<String>) {
    let root = dir.join(PATCHES_DIR);
    let mut found = BTreeMap::new();
    let mut problems = Vec::new();
    let entries =
        |path: &Path, problems: &mut Vec<String>| -> Vec<(String, PathBuf, fs::FileType)> {
            let mut list: Vec<(String, PathBuf, fs::FileType)> = match fs::read_dir(path) {
                Ok(read) => read
                    .filter_map(Result::ok)
                    .filter_map(|e| {
                        Some((
                            e.file_name().to_string_lossy().into_owned(),
                            e.path(),
                            e.file_type().ok()?,
                        ))
                    })
                    .collect(),
                Err(e) => {
                    problems.push(format!("{}: {e}", path.display()));
                    Vec::new()
                }
            };
            list.sort_by(|a, b| a.0.cmp(&b.0));
            list
        };
    match fs::symlink_metadata(&root) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (found, problems),
        Ok(m) if m.file_type().is_dir() => {}
        _ => {
            return (
                found,
                vec![format!("{} is not a directory", root.display())],
            );
        }
    }
    for (name, path, kind) in entries(&root, &mut problems) {
        let Some(rfc) = name.strip_prefix("rfc").and_then(name_number) else {
            problems.push(format!("{}: not an rfc<N> directory", path.display()));
            continue;
        };
        if !kind.is_dir() {
            problems.push(format!(
                "{}: not a directory (symbolic links are refused)",
                path.display()
            ));
            continue;
        }
        for (file, file_path, file_kind) in entries(&path, &mut problems) {
            let Some(id) = file.strip_suffix(".patch").and_then(name_number) else {
                problems.push(format!(
                    "{}: not an <errata_id>.patch file",
                    file_path.display()
                ));
                continue;
            };
            if !file_kind.is_file() {
                problems.push(format!(
                    "{}: not a regular file (symbolic links are refused)",
                    file_path.display()
                ));
                continue;
            }
            found.insert((rfc, id), file_path);
        }
    }
    (found, problems)
}

pub fn read_patch(path: &Path) -> Result<String, StoreError> {
    read_utf8(path, MAX_PATCH_BYTES)
}

/// The `patches/` and `patches/rfcN/` directories, each refused unless it is a real directory.
fn patch_dirs(dir: &Path, rfc: u32) -> [PathBuf; 2] {
    let patches = dir.join(PATCHES_DIR);
    let rfc_dir = patches.join(format!("rfc{rfc}"));
    [patches, rfc_dir]
}

pub fn write_patch(dir: &Path, rfc: u32, errata_id: u32, patch: &str) -> Result<(), StoreError> {
    real_dir(dir)?;
    for parent in patch_dirs(dir, rfc) {
        real_dir(&parent)?;
    }
    atomic_write(&patch_path(dir, rfc, errata_id), patch.as_bytes())
}

/// Removes a patch file, and its RFC's directory when that is left empty.
pub fn remove_patch(dir: &Path, rfc: u32, errata_id: u32) -> Result<(), StoreError> {
    let path = patch_path(dir, rfc, errata_id);
    for parent in patch_dirs(dir, rfc) {
        match fs::symlink_metadata(&parent) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Ok(m) if m.file_type().is_dir() => {}
            _ => {
                return Err(invalid(
                    &parent,
                    "not a directory (symbolic links are refused)",
                ));
            }
        }
    }
    match fs::remove_file(&path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_error(&path)(e)),
        _ => {}
    }
    if let Some(parent) = path.parent() {
        let _ = fs::remove_dir(parent);
    }
    Ok(())
}

/// The format a patch's headers name (`--- a/rfcN.xml` or `.txt`), if they name one.
pub fn patch_format(patch: &str) -> Option<SourceFormat> {
    let first = patch.lines().next()?;
    if first.ends_with(".xml") {
        Some(SourceFormat::Xml)
    } else if first.ends_with(".txt") {
        Some(SourceFormat::Text)
    } else {
        None
    }
}

/// The committed patches of every `patched` row, by RFC, as `(errata_id, patch)` in id order. An RFC whose patch is
/// missing or unreadable maps to the failure of its lowest such erratum instead.
pub fn patched_series(
    dir: &Path,
    rows: &[Row],
    files: &BTreeMap<(u32, u32), PathBuf>,
) -> BTreeMap<u32, Result<Vec<(u32, String)>, SeriesFailure>> {
    let mut series: BTreeMap<u32, Result<Vec<(u32, String)>, SeriesFailure>> = BTreeMap::new();
    let mut patched: Vec<&Row> = rows
        .iter()
        .filter(|r| r.decision == Decision::Patched)
        .collect();
    patched.sort_by_key(|r| r.key());
    for row in patched {
        let entry = series.entry(row.rfc).or_insert_with(|| Ok(Vec::new()));
        let Ok(list) = entry else { continue };
        let read = match files.get(&row.key()) {
            None => Err(format!(
                "{} is missing",
                patch_path(dir, row.rfc, row.errata_id).display()
            )),
            Some(path) => read_patch(path).map_err(|e| e.to_string()),
        };
        match read {
            Ok(patch) => list.push((row.errata_id, patch)),
            Err(reason) => {
                *entry = Err(SeriesFailure {
                    errata_id: row.errata_id,
                    reason,
                })
            }
        }
    }
    series
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{Decider, Rule};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glint-errata-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn row(rfc: u32, id: u32, decision: Decision) -> Row {
        Row {
            rfc,
            errata_id: id,
            decision,
            by: Decider::Claude,
            fingerprint: "0123456789abcdef".into(),
            rule: Rule::Asn1,
            note: "x".into(),
        }
    }

    #[test]
    fn ledger_and_patches_round_trip() {
        let dir = temp_dir("roundtrip");
        assert!(read_ledger(&dir).is_ok_and(|rows| rows.is_empty()));
        let rows = vec![
            row(5, 1, Decision::Patched),
            row(5, 2, Decision::Patched),
            row(6, 3, Decision::Patched),
        ];
        assert!(write_ledger(&dir, &rows).is_ok());
        assert!(read_ledger(&dir).is_ok_and(|read| read == rows));
        assert!(write_patch(&dir, 5, 1, "p1").is_ok());
        assert!(write_patch(&dir, 5, 2, "p2").is_ok());
        let (files, problems) = scan_patches(&dir);
        assert!(problems.is_empty());
        let series = patched_series(&dir, &rows, &files);
        assert_eq!(
            series.get(&5),
            Some(&Ok(vec![(1, "p1".to_owned()), (2, "p2".to_owned())]))
        );
        assert!(
            series
                .get(&6)
                .is_some_and(|s| s.as_ref().is_err_and(|f| f.errata_id == 3))
        );
        assert!(remove_patch(&dir, 5, 1).is_ok() && remove_patch(&dir, 5, 2).is_ok());
        assert!(!dir.join("patches/rfc5").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn odd_entries_are_problems_never_patches() {
        let dir = temp_dir("odd");
        let patches = dir.join(PATCHES_DIR);
        let _ = fs::create_dir_all(patches.join("rfc05"));
        let _ = fs::create_dir_all(patches.join("rfc7"));
        let _ = fs::write(patches.join("rfc7/01.patch"), "x");
        let _ = fs::write(patches.join("rfc7/notes.txt"), "x");
        let _ = fs::write(patches.join("README"), "x");
        #[cfg(unix)]
        let _ = std::os::unix::fs::symlink("/etc/hostname", patches.join("rfc7/2.patch"));
        let (files, problems) = scan_patches(&dir);
        assert!(files.is_empty());
        let joined = problems.join("\n");
        for expected in [
            "rfc05: not an rfc<N>",
            "README: not an rfc<N>",
            "01.patch: not an",
            "notes.txt: not an",
        ] {
            assert!(joined.contains(expected), "{expected}: {joined}");
        }
        #[cfg(unix)]
        assert!(joined.contains("2.patch: not a regular file"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_refuse_symlinks_and_large_files() {
        let dir = temp_dir("reads");
        let big = dir.join("big");
        let _ = fs::write(&big, vec![b'x'; 11]);
        assert!(
            read_regular_file(&big, 10).is_err_and(|e| e.to_string().contains("larger than 10"))
        );
        assert!(read_regular_file(&big, 11).is_ok());
        #[cfg(unix)]
        {
            let link = dir.join("link");
            let _ = std::os::unix::fs::symlink(&big, &link);
            assert!(
                read_regular_file(&link, 100)
                    .is_err_and(|e| e.to_string().contains("not a regular file"))
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn writes_never_go_through_symbolic_links() {
        let dir = temp_dir("links");
        let elsewhere = temp_dir("links-target");
        let _ = fs::create_dir_all(dir.join(PATCHES_DIR));
        let _ = std::os::unix::fs::symlink(&elsewhere, dir.join(PATCHES_DIR).join("rfc5"));
        assert!(
            write_patch(&dir, 5, 1, "p").is_err_and(|e| e.to_string().contains("symbolic links"))
        );
        assert!(remove_patch(&dir, 5, 1).is_err());
        let ledger = dir.join(LEDGER_FILE);
        let _ = std::os::unix::fs::symlink(elsewhere.join("victim"), &ledger);
        let temporary = dir.join(format!(".{LEDGER_FILE}.tmp-{}", std::process::id()));
        let _ = std::os::unix::fs::symlink(elsewhere.join("victim2"), &temporary);
        assert!(write_ledger(&dir, &[]).is_err());
        let _ = fs::remove_file(&temporary);
        assert!(write_ledger(&dir, &[]).is_ok());
        assert!(fs::symlink_metadata(&ledger).is_ok_and(|m| m.file_type().is_file()));
        assert!(fs::read_dir(&elsewhere).is_ok_and(|mut d| d.next().is_none()));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&elsewhere);
    }

    #[test]
    fn patch_format_comes_from_the_header() {
        assert_eq!(patch_format("--- a/rfc9881.xml\n"), Some(SourceFormat::Xml));
        assert_eq!(
            patch_format("--- a/rfc5280.txt\n"),
            Some(SourceFormat::Text)
        );
        assert_eq!(patch_format("garbage"), None);
    }
}
