//! Semantic analysis of ASN.1 modules: symbol tables, import resolution against the corpus, and the checks that
//! need name resolution.
//!
//! The crate does no I/O. A caller hands over parsed draft modules and a [`glint_corpus::CorpusProvider`] and gets
//! back [`Finding`]s in source coordinates.

mod checks;
mod eval;
mod finding;
mod graph;
mod model;
mod names;
mod sem;
mod snippet;
pub mod typemodel;
mod view;

use std::collections::HashMap;

pub use eval::{Evaluator, OidEval, OidProblem, OidProblemKind};
pub use finding::{Finding, Loc, SourceId};
pub use graph::{
    Found, LoadedModule, Lookup, Matched, ModuleGraph, ModuleId, Origin, Peeled, QualifiedError,
    ResolvedImport, SourceUnit,
};
pub use model::{
    AssignedId, ExportsModel, ImportClause, ImportedSymbol, ModuleModel, Selection, Symbol,
    SymbolKind, syntax_era,
};
pub use names::{is_registered_anywhere, registered_arc, validate_arcs};
pub use sem::{ClassInfo, ClassRes, Field, FieldKind, Kind, Sem, SymRef};
pub use snippet::{SnippetOptions, is_ignored, reverse_check};

use glint_corpus::CorpusProvider;
use glint_syntax::Parse;

/// Settings that change how a check runs.
#[derive(Debug, Clone)]
pub struct Options {
    /// `Name -> document id` (`rfcNNNN`, `X.509`) or `Name -> dotted OID` (design spec, "Import resolution order",
    /// rule 4).
    pub pins: HashMap<String, String>,
    /// Extra spellings that count as an OID placeholder, besides `TBD`, `TBDn` and `id-TBD` (I141).
    pub placeholders: Vec<String>,
    /// Compare the draft's OIDs with the corpus (W140). Off when the checked module is itself from the corpus.
    pub oid_collisions: bool,
    /// Report `I103` and `I105` notes about how imports were resolved.
    pub resolution_notes: bool,
    /// The document the checked modules are published in, when checking a corpus module: imports prefer modules of
    /// the same document.
    pub home_document: Option<String>,
    /// Report Glint's own style rules for new text (`W170`, `W171`, `E172`, `W173`); off for published modules.
    pub policy_checks: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pins: HashMap::new(),
            placeholders: Vec::new(),
            oid_collisions: true,
            resolution_notes: true,
            home_document: None,
            policy_checks: true,
        }
    }
}

/// A parsed block of the draft.
pub struct DraftSource {
    pub text: String,
    pub parse: Parse,
}

/// A fragment of the draft: assignments quoted in the text, compared with the modules by [`analyze_with_fragments`].
pub struct FragmentSource {
    pub text: String,
    pub parse: Parse,
}

/// The loaded modules and everything found in the draft's modules.
pub struct Analysis<'c> {
    pub graph: ModuleGraph<'c>,
    /// The draft's own modules.
    pub drafts: Vec<ModuleId>,
    /// The source id of each fragment, in the order given.
    pub fragments: Vec<SourceId>,
    pub findings: Vec<Finding>,
}

/// Loads `sources`, resolves their imports against `corpus` and runs every semantic check on the draft's modules.
pub fn analyze<'c>(
    sources: Vec<DraftSource>,
    corpus: &'c dyn CorpusProvider,
    options: &'c Options,
) -> Analysis<'c> {
    analyze_with_fragments(
        sources,
        Vec::new(),
        &SnippetOptions::default(),
        corpus,
        options,
    )
}

/// Like [`analyze`], and compares the assignments of `fragments` with the module definitions of the same name.
/// Source ids of the fragments follow those of `sources`.
pub fn analyze_with_fragments<'c>(
    sources: Vec<DraftSource>,
    fragments: Vec<FragmentSource>,
    snippets: &SnippetOptions,
    corpus: &'c dyn CorpusProvider,
    options: &'c Options,
) -> Analysis<'c> {
    let mut graph = ModuleGraph::new(corpus, options);
    let mut drafts = Vec::new();
    for source in sources {
        drafts.extend(graph.add_draft(source.text, &source.parse));
    }
    let first_fragment = graph.units.len();
    for fragment in fragments {
        graph.add_draft(fragment.text, &fragment.parse);
    }
    let fragment_ids: Vec<SourceId> = (first_fragment..graph.units.len())
        .filter_map(|i| u32::try_from(i).ok().map(SourceId))
        .collect();
    graph.resolve_all();
    for name in snippet::names_needing_corpus(&graph, &fragment_ids) {
        graph.load_corpus_defining(&name);
    }
    graph.resolve_all();
    graph.load_builtin();
    let mut findings = Vec::new();
    for &module in &drafts {
        checks::check_module(&graph, module, &mut findings);
    }
    snippet::check(&graph, &fragment_ids, snippets, &mut findings);
    Analysis {
        graph,
        drafts,
        fragments: fragment_ids,
        findings,
    }
}
