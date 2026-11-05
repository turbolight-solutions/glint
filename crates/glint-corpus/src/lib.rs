//! The RFC module corpus: records, indexes, the [`CorpusProvider`] trait and the published data files.
//!
//! The crate does no I/O. `glint-corpus-build` produces the data; every other crate only reads it.

mod corpus;
mod data;
mod labels;
mod oid;
mod record;

pub use corpus::{Corpus, CorpusProvider, EmptyCorpus, preferred, unique_label};
pub use data::{
    CORPUS_FILE, DOCUMENTS_FILE, DataError, DocumentEntry, Documents, Pointer, SCHEMA, build_date,
    build_dir, latest_path, schema_dir, valid_build_id,
};
pub use labels::{AllowedOid, LabelEvidence, LabelProblem, OidLabels, OidNode};
pub use oid::{Oid, OidParseError};
pub use record::{
    BODY_RFC, CorpusData, CorpusInfo, DocumentRecord, Exports, ModuleRecord, ModuleSource,
    OidAssignment, OverrideTier, ParseStatus, SourceKind, SyntaxEra, document_title, rfc_number,
};
