//! The RFC module corpus: records, indexes, the [`CorpusProvider`] trait and the bundle file format.
//!
//! The crate does no network access. `glint-corpus-build` produces bundles; every other crate only reads them.

mod bundle;
mod corpus;
mod oid;
mod record;

pub use bundle::{BUNDLE_FORMAT_VERSION, BundleError, read_bundle, write_bundle};
pub use corpus::{Corpus, CorpusProvider, EmptyCorpus, preferred};
pub use oid::{Oid, OidParseError};
pub use record::{
    Bundle, CorpusInfo, Exports, ModuleRecord, ModuleSource, OidAssignment, OverrideTier,
    ParseStatus, SourceKind, SyntaxEra,
};
