//! The check pipeline: extract blocks from a document, parse them, run the semantic checks and map every finding
//! back to document coordinates.

mod config;
mod pipeline;
mod source;

pub use config::{Config, ConfigError, SeveritySetting};
pub use pipeline::{
    BlockSummary, CheckOptions, ImportSummary, ModuleSummary, Report, check_document, document_id,
};
pub use source::{
    DocumentRef, RefError, draft_urls, latest_revision_url, revision_from_datatracker, rfc_urls,
    split_revision, valid_draft_name, valid_rev,
};
