//! Identifies the type of a PEM example (Codegen and Decoder Design, Example artifacts: Identification): the roots its
//! label suggests, then the top-level types of its document's own modules, then the common corpus roots, decoded in
//! that order until one fits.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use glint_corpus::CorpusProvider;
use glint_decode::{DecodeFinding, Decoded, SchemaState, Tlv, common_roots, decode, label_roots};
use glint_diag::Severity;
use glint_ir::{IrSet, Kind};
use glint_sema::{ModuleGraph, ModuleId, SymbolKind};

use crate::decode::{CorpusOracle, lower_corpus_root, with_corpus_module};

pub use glint_corpus::Confidence;

/// A type an example may be: its name (`rfc5912#PKIX1Explicit-2009.Certificate`, or `Module.Type` for a draft's
/// own module) and its IR.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub root: String,
    pub set: Rc<IrSet>,
}

/// The type chosen for an example.
#[derive(Debug, Clone)]
pub struct Identification {
    /// The chosen type; for [`Confidence::Unknown`], the closest candidate, if any was tried.
    pub root: Option<String>,
    pub confidence: Confidence,
    /// The findings of the decode as `root`.
    pub findings: Vec<DecodeFinding>,
}

impl Identification {
    /// The distinct codes of the findings, in order of first appearance.
    pub fn codes(&self) -> Vec<&'static str> {
        let mut codes: Vec<&'static str> = Vec::new();
        for finding in &self.findings {
            if !codes.contains(&finding.code) {
                codes.push(finding.code);
            }
        }
        codes
    }

    /// The first finding that kept the closest candidate from fitting, for [`Confidence::Unknown`].
    pub fn first_mismatch(&self) -> Option<&DecodeFinding> {
        self.findings.iter().find(|f| is_mismatch(f))
    }
}

/// Findings that mean the octets are not of the type: the shape, not the values, is wrong.
fn is_mismatch(finding: &DecodeFinding) -> bool {
    finding.severity == Severity::Error
        && matches!(finding.code, "E400" | "E403" | "E404" | "E405" | "E409")
}

/// How well a decode fits its type.
struct Fit {
    /// No error at all.
    clean: bool,
    /// The structure matches; errors, if any, are about values (contents, constraints).
    fits: bool,
    /// TLVs decoded with a schema, for choosing the closest of several candidates that do not fit.
    matched: usize,
}

fn fit(decoded: &Decoded) -> Fit {
    let findings = decoded.all_findings();
    let matched = decoded
        .tlvs
        .iter()
        .flat_map(|t| t.walk())
        .filter(|t| !matches!(t.schema, SchemaState::Unmatched))
        .count();
    let mut outer = Vec::new();
    for tlv in &decoded.tlvs {
        outer_findings(tlv, &mut outer);
    }
    outer.extend(&decoded.findings);
    Fit {
        clean: !findings.iter().any(|f| f.severity == Severity::Error),
        fits: !outer.iter().any(|f| is_mismatch(f)),
        matched,
    }
}

/// The findings of `tlv` and its subtree, leaving out what lies in the decoded contents of an octet or bit string
/// (`CONTAINING`, an open type, a hint): an example whose extension value is broken is still of the type, with an
/// error in a value.
fn outer_findings<'t>(tlv: &'t Tlv, out: &mut Vec<&'t DecodeFinding>) {
    out.extend(&tlv.findings);
    let holds_contents = tlv.tag.is_some_and(|t| !t.constructed);
    if holds_contents {
        return;
    }
    for child in &tlv.children {
        outer_findings(child, out);
    }
}

fn findings_of(decoded: &Decoded) -> Vec<DecodeFinding> {
    decoded.all_findings().into_iter().cloned().collect()
}

/// Identifies examples against the corpus, lowering each candidate root once.
pub struct Identifier<'c> {
    corpus: &'c dyn CorpusProvider,
    oracle: CorpusOracle<'c>,
    lowered: RefCell<HashMap<String, Option<Rc<IrSet>>>>,
}

impl<'c> Identifier<'c> {
    pub fn new(corpus: &'c dyn CorpusProvider) -> Self {
        Self {
            corpus,
            oracle: CorpusOracle::new(corpus),
            lowered: RefCell::new(HashMap::new()),
        }
    }

    pub fn oracle(&self) -> &CorpusOracle<'c> {
        &self.oracle
    }

    /// The IR of a corpus root written `doc#Module.Type`, lowered once; `None` when the corpus lacks it.
    pub fn corpus_root(&self, root: &str) -> Option<Candidate> {
        if let Some(found) = self.lowered.borrow().get(root) {
            return found.clone().map(|set| Candidate {
                root: root.to_owned(),
                set,
            });
        }
        let set = lower_corpus_root(self.corpus, root).ok().map(Rc::new);
        self.lowered
            .borrow_mut()
            .insert(root.to_owned(), set.clone());
        set.map(|set| Candidate {
            root: root.to_owned(),
            set,
        })
    }

    /// Decodes `der` as `candidate`.
    pub fn decode(&self, der: &[u8], candidate: &Candidate) -> Decoded {
        decode(
            der,
            Some((&candidate.set, candidate.set.root)),
            &self.oracle,
        )
    }

    /// Chooses the type of an example with PEM label `label`, trying `own` (the top-level types of its document's
    /// modules) after the label's roots and before the common corpus roots.
    pub fn identify(&self, der: &[u8], label: &str, own: &[Candidate]) -> Identification {
        // The first candidate that fits with errors, and the closest one that does not fit.
        let mut fitting: Option<(Confidence, String, Decoded)> = None;
        let mut closest: Option<(usize, String, Decoded)> = None;
        let mut tried: HashSet<String> = HashSet::new();
        let label_candidates = label_roots(label)
            .iter()
            .filter_map(|root| self.corpus_root(root));
        let others = own.iter().cloned().chain(
            common_roots()
                .iter()
                .filter_map(|root| self.corpus_root(root)),
        );
        let ordered = label_candidates
            .map(|c| (Confidence::Label, c))
            .chain(others.map(|c| (Confidence::Decoded, c)));
        for (confidence, candidate) in ordered {
            if !tried.insert(candidate.root.clone()) {
                continue;
            }
            let decoded = self.decode(der, &candidate);
            let fit = fit(&decoded);
            if fit.clean {
                return Identification {
                    root: Some(candidate.root),
                    confidence,
                    findings: findings_of(&decoded),
                };
            }
            if fit.fits {
                if fitting.is_none() {
                    fitting = Some((confidence, candidate.root, decoded));
                }
            } else if closest.as_ref().is_none_or(|(m, ..)| fit.matched > *m) {
                closest = Some((fit.matched, candidate.root, decoded));
            }
        }
        if let Some((confidence, root, decoded)) = fitting {
            return Identification {
                root: Some(root),
                confidence,
                findings: findings_of(&decoded),
            };
        }
        match closest {
            Some((_, root, decoded)) => Identification {
                root: Some(root),
                confidence: Confidence::Unknown,
                findings: findings_of(&decoded),
            },
            None => Identification {
                root: None,
                confidence: Confidence::Unknown,
                findings: Vec::new(),
            },
        }
    }
}

/// Types a module defines that could be an example's root: constructed, not parameterized, and not used by another
/// type of the module.
const MAX_TYPES: usize = 400;

/// The top-level types of `module` of `graph`, each named `prefix` + `Module.Type`.
pub fn module_roots(graph: &ModuleGraph<'_>, module: ModuleId, prefix: &str) -> Vec<Candidate> {
    let Some(loaded) = graph.module(module) else {
        return Vec::new();
    };
    let model = &loaded.model;
    let lowered: Vec<(String, IrSet)> = model
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Type && !s.is_parameterized())
        .take(MAX_TYPES)
        .filter_map(|s| {
            glint_sema::ir::lower(graph, module, &s.name).map(|set| (s.name.clone(), set))
        })
        .collect();
    let referenced: HashSet<&str> = lowered
        .iter()
        .flat_map(|(name, set)| {
            set.types
                .iter()
                .filter(move |t| t.origin.module == model.name && t.origin.asn1_name != *name)
                .map(|t| t.origin.asn1_name.as_str())
        })
        .collect();
    lowered
        .iter()
        .filter(|(name, set)| {
            let constructed = set.get(set.root).is_some_and(|def| {
                matches!(
                    set.resolve(&def.ty).kind,
                    Kind::Sequence(_)
                        | Kind::Set(_)
                        | Kind::Choice(_)
                        | Kind::SequenceOf(_)
                        | Kind::SetOf(_)
                )
            });
            constructed && !referenced.contains(name.as_str())
        })
        .map(|(name, set)| Candidate {
            root: format!("{prefix}{}.{name}", model.name),
            set: Rc::new(set.clone()),
        })
        .collect()
}

/// The top-level types of the corpus modules of `document`, named `document#Module.Type`.
pub fn document_roots(corpus: &dyn CorpusProvider, document: &str) -> Vec<Candidate> {
    let indices: Vec<usize> = corpus
        .modules()
        .iter()
        .enumerate()
        .filter(|(_, m)| m.source.document.eq_ignore_ascii_case(document))
        .map(|(i, _)| i)
        .collect();
    let prefix = format!("{document}#");
    indices
        .into_iter()
        .flat_map(|index| {
            with_corpus_module(corpus, index, |analysis, name| {
                let id = analysis.drafts.iter().copied().find(|&id| {
                    analysis
                        .graph
                        .module(id)
                        .is_some_and(|m| m.model.name == name)
                })?;
                Some(module_roots(&analysis.graph, id, &prefix))
            })
            .unwrap_or_default()
        })
        .collect()
}
