//! Glint's code generators (Codegen and Decoder Design, Codegen).
//!
//! A backend turns the [`IrSet`] of one root type into source for one library. The work every target shares lives in
//! the middle layer ([`plan`]): inline types are hoisted and named, names pass through the backend's [`Namer`] and a
//! collision pass, constraints become language-neutral check programs, open types become dispatch enums keyed by
//! their object set's table, and the component relations that select them are anchored to the record that resolves
//! them. A backend prints the resulting [`EmitPlan`] and ships a runtime file that implements the runtime contract:
//! check-program interpretation, open-type resolution, contents constraints and the normalized value tree the
//! differential tests compare against the decoder's.
//!
//! Nothing outside this crate names a backend: the CLI, the web and the tests read [`backends`].

pub mod naming;
pub mod plan;
mod rasn;

use glint_diag::{Severity, code_info};
use glint_ir::IrSet;
use serde::Serialize;

pub use naming::Namer;
pub use plan::EmitPlan;

/// The language a backend writes, for file extensions and editor modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    Python,
    Go,
}

impl Language {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Rust => "rs",
            Self::Python => "py",
            Self::Go => "go",
        }
    }
}

/// What a backend is.
#[derive(Debug, Clone, Serialize)]
pub struct BackendMeta {
    /// `rasn`, `der`, `pyasn1`, `go-cryptobyte`.
    pub id: &'static str,
    pub display_name: &'static str,
    pub language: Language,
    /// The library the output uses.
    pub library: &'static str,
    /// The release of the library the output is tested against.
    pub library_version: &'static str,
}

/// What a target library handles natively; the middle layer covers the rest.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Capabilities {
    /// A record component, list element or alternative can carry a tag of its own; otherwise a tagged element is
    /// hoisted into a named type.
    pub element_tags: bool,
    /// How many tag layers one use of a type can carry after collapsing (X.680 31.2); deeper chains are hoisted.
    pub max_use_tags: usize,
    /// A named `SEQUENCE` or `SET` can carry an implicit tag of its own; other tagged constructed types are split into
    /// a tagged newtype over an untagged one.
    pub tagged_records: bool,
    /// The library checks `PATTERN` constraints, or the runtime does.
    pub pattern: bool,
    /// The library has `REAL`.
    pub real: bool,
}

/// A generated file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct File {
    /// Relative to the output directory.
    pub path: String,
    pub contents: String,
}

/// Something the target cannot express, and what the output does instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub severity: Severity,
    /// The type, component or object set it concerns (`PKIX1Explicit-2009.TBSCertificate.extensions`).
    pub at: String,
    /// One line, no trailing period.
    pub message: String,
}

impl Finding {
    pub fn new(code: &'static str, at: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: code_info(code).map_or(Severity::Warning, |info| info.severity),
            at: at.into(),
            message: message.into(),
        }
    }
}

/// What a backend wrote.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Output {
    /// One file per ASN.1 module, a file that declares them, and the runtime file.
    pub files: Vec<File>,
    pub findings: Vec<Finding>,
}

/// What goes into every file header.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The Glint version that generated the code.
    pub glint_version: String,
    /// The corpus data build the modules came from, when they came from the corpus.
    pub data_build: Option<String>,
}

/// One target.
pub trait Backend: Sync {
    fn meta(&self) -> &BackendMeta;
    fn capabilities(&self) -> &Capabilities;
    fn namer(&self) -> &dyn Namer;
    /// Prints the plan. The runtime file is part of the output.
    fn emit(&self, plan: &EmitPlan, opts: &Options) -> Output;
    /// The vendored runtime file.
    fn runtime(&self) -> File;
}

static BACKENDS: &[&dyn Backend] = &[&rasn::RASN];

/// Every backend, in the order the CLI and the web list them.
pub fn backends() -> &'static [&'static dyn Backend] {
    BACKENDS
}

/// The backend with `id`.
pub fn backend(id: &str) -> Option<&'static dyn Backend> {
    BACKENDS.iter().copied().find(|b| b.meta().id == id)
}

/// Generates the code of `set`'s root type for `backend`: the middle layer's plan, printed.
pub fn generate(set: &IrSet, backend: &dyn Backend, opts: &Options) -> Output {
    let plan = plan::build(set, backend.namer(), backend.capabilities());
    let mut output = backend.emit(&plan, opts);
    let mut findings = plan.findings.clone();
    findings.append(&mut output.findings);
    output.findings = findings;
    output
}
