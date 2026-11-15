//! The RFC module corpus: records, indexes, the [`CorpusProvider`] trait and the published data files.
//!
//! The crate does no I/O. `glint-corpus-build` produces the data; every other crate only reads it.

mod artifacts;
mod corpus;
mod data;
mod labels;
mod oid;
mod record;

pub use artifacts::{ARTIFACTS_FILE, Artifact, ArtifactsData, BrokenArtifact, Confidence};
pub use corpus::{
    Corpus, CorpusProvider, EmptyCorpus, IanaVerdict, iana_conflict, iana_verdict, preferred,
    unique_label,
};
pub use data::{
    BodyRange, CORPUS_FILE, DOCUMENTS_FILE, DataError, DocumentBlock, DocumentEntry, DocumentText,
    Documents, Pointer, SCHEMA, build_date, build_dir, latest_path, schema_dir, valid_build_id,
};
pub use labels::{AllowedOid, LabelEvidence, LabelProblem, OidLabels, OidNode};
pub use oid::{Oid, OidParseError};
pub use record::{
    AppliedErratum, BODY_DRAFT, BODY_RFC, CorpusData, CorpusInfo, DocumentRecord, ErrataFailure,
    ErratumPatch, Exports, HunkLines, IanaModuleId, ModuleRecord, ModuleSource, OidAssignment,
    OidHint, OverrideTier, ParseStatus, SourceKind, SyntaxEra, document_title, draft_name,
    rfc_number, same_document,
};
