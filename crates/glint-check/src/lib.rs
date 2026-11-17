//! The check pipeline: extract blocks from a document, parse them, run the semantic checks and map every finding
//! back to document coordinates.

pub mod artifacts;
mod closure;
mod config;
mod decode;
mod pipeline;
mod source;

pub use closure::{
    ArcFill, ArcOrigin, BuiltinDefinition, BuiltinImport, Closure, ClosureInput, ClosureModule,
    ClosureOptions, FIRST_AUTO_ARC, ModuleRename, OidKey, Problem, Rename, Role, Warning, closure,
};
pub use config::{Config, ConfigError, ExamplesConfig, SeveritySetting};
pub use decode::{
    CorpusOracle, find_corpus_module, lower_corpus_root, lower_corpus_type, lower_user_type,
};
pub use pipeline::{
    BlockSummary, CheckOptions, ImportSummary, ModuleSummary, OtherDocument, Report,
    check_document, check_extraction, document_id,
};
pub use source::{
    DocumentRef, RefError, draft_urls, latest_revision_url, revision_from_datatracker, rfc_urls,
    split_revision, valid_draft_name, valid_rev,
};
