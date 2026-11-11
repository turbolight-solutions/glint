//! `corpus-build`: builds the Glint data files (`corpus.json`, `documents.bin`, `latest.json`) and the report from a
//! local RFC mirror. The library is what the `corpus-build` binary runs, and what `glint errata prove` builds the corpus
//! with on both sides of an edit.

mod build;
mod collect;
pub mod errata;
mod iana;
mod index;
mod overrides;
mod report;
mod sources;

use std::path::PathBuf;

pub use build::run;

/// What to build from, and where to.
pub struct Settings {
    pub rfc_dir: PathBuf,
    pub drafts_dir: Option<PathBuf>,
    pub overrides: Option<PathBuf>,
    pub out: PathBuf,
    pub date: String,
    pub only: Vec<u32>,
    pub dump_findings: Option<PathBuf>,
    pub sources: Option<PathBuf>,
    pub cache: PathBuf,
    pub errata: Option<PathBuf>,
    /// `None` with `--no-errata`.
    pub errata_dir: Option<PathBuf>,
    pub iana: Option<PathBuf>,
}
