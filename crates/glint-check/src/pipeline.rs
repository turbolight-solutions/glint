use std::collections::HashMap;
use std::ops::Range;

use glint_corpus::CorpusProvider;
use glint_diag::{Diagnostic, FileId, Files, Label, LineIndex, Severity};
use glint_extract::{Block, BlockKind, Extraction, Format, extract};
use glint_sema::{
    DraftSource, Finding, FragmentSource, Loc, Matched, Options, Origin, SnippetOptions,
    analyze_with_fragments, is_ignored, reverse_check,
};
use glint_syntax::{ParseOptions, parse_source_with};

use crate::Config;

/// Everything a check needs besides the document and the corpus.
#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    pub config: Config,
    /// Extra pins from the command line, applied over the configuration.
    pub pins: Vec<(String, String)>,
    /// Do not compare the snippets in the text with the modules.
    pub no_snippets: bool,
}

/// How one import of a draft module was resolved.
#[derive(Debug, Clone)]
pub struct ImportSummary {
    pub module: String,
    /// `rfc5280#PKIX1Explicit88`, or the draft's own module name; `None` when opaque.
    pub resolved_to: Option<String>,
    pub symbols: Vec<String>,
}

/// A module found in the document.
#[derive(Debug, Clone)]
pub struct ModuleSummary {
    pub name: String,
    pub oid: Option<String>,
    pub section: Option<String>,
    /// Byte range in the document of the block the module was extracted from.
    pub span: Range<usize>,
    pub imports: Vec<ImportSummary>,
}

/// A piece of ASN.1 that the extractor found in the document.
#[derive(Debug, Clone)]
pub struct BlockSummary {
    pub kind: BlockKind,
    pub section: Option<String>,
    pub title: Option<String>,
    /// Byte range in the document.
    pub span: Range<usize>,
    /// 1-based line and column of the start.
    pub line_col: (u32, u32),
}

/// The outcome of checking one document.
#[derive(Debug)]
pub struct Report {
    pub files: Files,
    pub document: FileId,
    pub format: Format,
    pub blocks: Vec<BlockSummary>,
    pub diagnostics: Vec<Diagnostic>,
    /// For the text of an RFCXML document checked with the blocks its XML marks ([`Extraction::blocks_declared`]): the
    /// findings about that marking (`W201`, `W202`, `W203`, `I200`, `I201`: an element's `type`, or what an element
    /// marked as ASN.1 holds). They are about the XML, not the text, so they are kept out of `diagnostics`; a check of
    /// the XML gives them on its elements.
    pub markup_findings: Vec<Diagnostic>,
    pub modules: Vec<ModuleSummary>,
    /// The build id of the data the check used.
    pub data_build: String,
}

fn convert_severity(severity: glint_syntax::Severity) -> Severity {
    match severity {
        glint_syntax::Severity::Error => Severity::Error,
        glint_syntax::Severity::Warning => Severity::Warning,
        glint_syntax::Severity::Info => Severity::Info,
    }
}

fn extract_severity(severity: glint_extract::Severity) -> Severity {
    match severity {
        glint_extract::Severity::Error => Severity::Error,
        glint_extract::Severity::Warning => Severity::Warning,
        glint_extract::Severity::Info => Severity::Info,
    }
}

struct Mapper<'a> {
    files: &'a mut Files,
    document: FileId,
    document_index: LineIndex,
    blocks: &'a [Block],
    /// For each draft source id, the index of its block.
    block_of_source: Vec<usize>,
    corpus_files: HashMap<usize, (FileId, LineIndex)>,
    corpus: &'a dyn CorpusProvider,
}

impl Mapper<'_> {
    fn draft_label(&self, block_index: usize, range: Range<usize>) -> Label {
        let Some(block) = self.blocks.get(block_index) else {
            return Label::new(self.document, 0..0, &self.document_index);
        };
        let mapped = block
            .original_range(range)
            .unwrap_or_else(|| block.origin.span.clone());
        let mut label = Label::new(self.document, mapped, &self.document_index);
        if let Some(section) = &block.origin.section {
            label = label.with_section(section.to_string());
        }
        label
    }

    fn label_for(&mut self, graph: &glint_sema::ModuleGraph<'_>, loc: Loc) -> Label {
        let range = usize::from(loc.range.start())..usize::from(loc.range.end());
        let Some(unit) = graph.unit(loc.source) else {
            return Label::new(self.document, 0..0, &self.document_index);
        };
        match unit.origin {
            Origin::Builtin => Label::new(self.document, 0..0, &self.document_index),
            Origin::Draft => {
                let block = usize::try_from(loc.source.0)
                    .ok()
                    .and_then(|i| self.block_of_source.get(i))
                    .copied()
                    .unwrap_or(0);
                self.draft_label(block, range)
            }
            Origin::Corpus { record } => {
                if !self.corpus_files.contains_key(&record)
                    && let Some(module) = self.corpus.modules().get(record)
                {
                    let entry = self.files.add(
                        glint_corpus::unique_label(self.corpus, module),
                        module.text.clone(),
                    );
                    self.corpus_files.insert(record, entry);
                }
                match self.corpus_files.get(&record) {
                    Some((file, index)) => Label::new(*file, range, index),
                    None => Label::new(self.document, 0..0, &self.document_index),
                }
            }
        }
    }
}

/// Checks `text` as a document of `format` against `corpus`.
pub fn check_document(
    name: &str,
    text: &str,
    format: Format,
    corpus: &dyn CorpusProvider,
    options: &CheckOptions,
) -> Report {
    check_extraction(name, text, extract(text, format), corpus, options)
}

/// Checks `text` against `corpus` with the ASN.1 blocks `extraction` found in it (from [`extract`], or
/// [`glint_extract::extract_declared`] for the text of an RFCXML document with the blocks its XML marks).
pub fn check_extraction(
    name: &str,
    text: &str,
    extraction: Extraction,
    corpus: &dyn CorpusProvider,
    options: &CheckOptions,
) -> Report {
    let mut files = Files::new();
    let (document, document_index) = files.add_document(name, text);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let notes = extraction.notes.iter().map(|note| {
        Diagnostic::new(
            note.code,
            note.message.clone(),
            Label::new(document, note.span.clone(), &document_index),
        )
        .with_severity(extract_severity(note.severity))
    });
    // Extraction from declared blocks notes only what the XML marks: the markup findings.
    let markup_findings = if extraction.blocks_declared {
        let mut markup = apply_config(notes.collect(), &options.config);
        markup.sort_by_key(|d| d.primary.span.start);
        markup
    } else {
        diagnostics.extend(notes);
        Vec::new()
    };

    let mut sources = Vec::new();
    let mut fragments = Vec::new();
    let mut module_blocks = Vec::new();
    let mut fragment_blocks = Vec::new();
    for (index, block) in extraction.blocks.iter().enumerate() {
        let is_fragment = block.kind == BlockKind::Fragment;
        // A bare ASN.1 file has no prose to quote ASN.1 in: a block of it without a module header (a header that does
        // not parse) is not a snippet, so it is checked even with snippets switched off.
        if is_fragment && options.no_snippets && extraction.format != Format::Bare {
            continue;
        }
        let parse = parse_source_with(&block.text, ParseOptions::corpus());
        if is_ignored(&parse.syntax()) {
            continue;
        }
        for error in glint_sema::without_legacy_name_errors(parse.errors(), &parse.syntax()) {
            let range = usize::from(error.range.start())..usize::from(error.range.end());
            let mapped = block
                .original_range(range)
                .unwrap_or_else(|| block.origin.span.clone());
            let mut label = Label::new(document, mapped, &document_index);
            if let Some(section) = &block.origin.section {
                label = label.with_section(section.to_string());
            }
            // A snippet a text yields is a guess; one its RFCXML source marks is not.
            let guessed = extraction.format == Format::Text && !extraction.blocks_declared;
            let severity = if is_fragment && guessed {
                Severity::Warning
            } else {
                convert_severity(error.severity)
            };
            diagnostics.push(
                Diagnostic::new(error.code, error.message.clone(), label).with_severity(severity),
            );
        }
        if is_fragment {
            fragments.push(FragmentSource {
                text: block.text.clone(),
                parse,
            });
            fragment_blocks.push(index);
        } else {
            sources.push(DraftSource {
                text: block.text.clone(),
                parse,
            });
            module_blocks.push(index);
        }
    }
    let mut block_of_source = module_blocks;
    block_of_source.extend(fragment_blocks);

    let mut sema_options = Options {
        pins: options.config.pins.clone().into_iter().collect(),
        placeholders: options.config.oid.placeholders.clone(),
        oid_synonyms: options.config.oid.synonym_labels,
        own_document: document_id(name),
        ..Options::default()
    };
    for (module, pin) in &options.pins {
        sema_options.pins.insert(module.clone(), pin.clone());
    }
    let snippet_options = SnippetOptions {
        elision_comments: options.config.snippets.elision_comments.clone(),
    };
    let analysis =
        analyze_with_fragments(sources, fragments, &snippet_options, corpus, &sema_options);
    let mut findings = analysis.findings.clone();
    if options.config.snippets.reverse_check && !options.no_snippets {
        let mentioned = mentioned_names(text, &extraction.blocks);
        findings.extend(reverse_check(&analysis.graph, &analysis.drafts, &mentioned));
    }

    let mut mapper = Mapper {
        files: &mut files,
        document,
        document_index,
        blocks: &extraction.blocks,
        block_of_source,
        corpus_files: HashMap::new(),
        corpus,
    };
    for finding in &findings {
        diagnostics.push(to_diagnostic(&mut mapper, &analysis.graph, finding));
    }

    if options.config.examples.check {
        diagnostics.extend(example_diagnostics(
            &mut mapper,
            text,
            &extraction,
            &analysis,
            corpus,
            options.config.examples.identified,
        ));
    }

    let line_index = LineIndex::for_document(text);
    let blocks = extraction
        .blocks
        .iter()
        .map(|block| BlockSummary {
            kind: block.kind,
            section: block.origin.section.as_ref().map(ToString::to_string),
            title: block.origin.title.clone(),
            span: block.origin.span.clone(),
            line_col: line_index.line_col(block.origin.span.start),
        })
        .collect();
    let modules = summarize(
        &analysis,
        &extraction.blocks,
        &mapper.block_of_source,
        corpus,
    );
    let diagnostics = apply_config(diagnostics, &options.config);
    let mut diagnostics = diagnostics;
    diagnostics.sort_by_key(|d| (d.primary.file, d.primary.span.start));
    Report {
        files,
        document,
        format: extraction.format,
        blocks,
        diagnostics,
        markup_findings,
        modules,
        data_build: corpus.info().build.clone(),
    }
}

/// Decodes the PEM examples of the document as the type each is identified as, its own modules' types tried before the
/// common corpus roots (Codegen and Decoder Design, Example artifacts: Examples in a checked draft).
fn example_diagnostics(
    mapper: &mut Mapper<'_>,
    text: &str,
    extraction: &Extraction,
    analysis: &glint_sema::Analysis<'_>,
    corpus: &dyn CorpusProvider,
    identified: bool,
) -> Vec<Diagnostic> {
    // The text of an RFCXML document checked with its declared blocks is plain text.
    let format = if extraction.blocks_declared {
        Format::Text
    } else {
        extraction.format
    };
    let scan = glint_extract::artifacts::pem_blocks(text, format);
    if scan.blocks.is_empty() && scan.findings.is_empty() {
        return Vec::new();
    }
    let label = |mapper: &Mapper<'_>, place: &glint_extract::artifacts::Place| {
        let mut label = Label::new(mapper.document, place.span.clone(), &mapper.document_index);
        if let Some(section) = &place.section {
            label = label.with_section(section.to_string());
        }
        label
    };
    let mut out = Vec::new();
    for finding in &scan.findings {
        out.push(Diagnostic::new(
            "E410",
            finding.message.clone(),
            label(mapper, &finding.place),
        ));
    }
    if scan.blocks.is_empty() {
        return out;
    }
    let own: Vec<crate::artifacts::Candidate> = analysis
        .drafts
        .iter()
        .flat_map(|&id| crate::artifacts::module_roots(&analysis.graph, id, ""))
        .collect();
    let identifier = crate::artifacts::Identifier::new(corpus);
    for block in &scan.blocks {
        let found = identifier.identify(&block.der, &block.label, &own);
        let at = label(mapper, &block.place);
        let Some(root) = &found.root else {
            out.push(Diagnostic::new(
                "W411",
                format!("no type decodes the `{}` example", block.label),
                at,
            ));
            continue;
        };
        if found.confidence == crate::artifacts::Confidence::Unknown {
            let mut diagnostic = Diagnostic::new(
                "W411",
                format!(
                    "no type decodes the `{}` example; the closest is {root}",
                    block.label
                ),
                at,
            );
            if let Some(mismatch) = found.first_mismatch() {
                diagnostic =
                    diagnostic.with_note(format!("{}: {}", mismatch.code, mismatch.message));
            }
            out.push(diagnostic);
            continue;
        }
        // A label whose types the corpus has, decoded only as another type.
        let label_types: Vec<&str> = glint_decode::label_roots(&block.label)
            .iter()
            .copied()
            .filter(|r| identifier.corpus_root(r).is_some())
            .collect();
        if found.confidence == crate::artifacts::Confidence::Decoded && !label_types.is_empty() {
            out.push(
                Diagnostic::new(
                    "W415",
                    format!(
                        "the `{}` example is a {root}, not a type its label names",
                        block.label
                    ),
                    at.clone(),
                )
                .with_note(format!(
                    "`{}` names {}",
                    block.label,
                    label_types.join(", ")
                )),
            );
        }
        let errors: Vec<_> = found
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .collect();
        if let Some(first) = errors.first() {
            let mut diagnostic = Diagnostic::new(
                "W412",
                format!(
                    "the `{}` example has errors as {root}: {}",
                    block.label, first.message
                ),
                at.clone(),
            );
            for more in errors.iter().skip(1).take(4) {
                diagnostic = diagnostic.with_note(format!("{}: {}", more.code, more.message));
            }
            out.push(
                diagnostic.with_help(format!("`glint decode` shows every TLV ({})", first.code)),
            );
        }
        let der: Vec<_> = found.findings.iter().filter(|f| f.code == "W402").collect();
        if let Some(first) = der.first() {
            out.push(Diagnostic::new(
                "I413",
                format!(
                    "the `{}` example is not DER: {}{}",
                    block.label,
                    first.message,
                    if der.len() > 1 {
                        format!(" (and {} more)", der.len().saturating_sub(1))
                    } else {
                        String::new()
                    }
                ),
                at.clone(),
            ));
        }
        if identified {
            out.push(Diagnostic::new(
                "I414",
                format!(
                    "the `{}` example is a {root} ({})",
                    block.label,
                    found.confidence.as_str()
                ),
                at,
            ));
        }
    }
    out
}

fn to_diagnostic(
    mapper: &mut Mapper<'_>,
    graph: &glint_sema::ModuleGraph<'_>,
    finding: &Finding,
) -> Diagnostic {
    let primary = mapper.label_for(graph, finding.primary);
    let mut diagnostic = Diagnostic::new(finding.code, finding.message.clone(), primary)
        .with_severity(finding.severity);
    for (loc, text) in &finding.secondary {
        let label = mapper.label_for(graph, *loc).with_text(text.clone());
        diagnostic = diagnostic.with_secondary(label);
    }
    for note in &finding.notes {
        diagnostic = diagnostic.with_note(note.clone());
    }
    if let Some(help) = &finding.help {
        diagnostic = diagnostic.with_help(help.clone());
    }
    diagnostic
}

fn apply_config(diagnostics: Vec<Diagnostic>, config: &Config) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .filter_map(|mut d| match config.severity.get(d.code) {
            Some(setting) => {
                d.severity = setting.severity()?;
                Some(d)
            }
            None => Some(d),
        })
        .collect()
}

fn summarize(
    analysis: &glint_sema::Analysis<'_>,
    blocks: &[Block],
    block_of_source: &[usize],
    corpus: &dyn CorpusProvider,
) -> Vec<ModuleSummary> {
    analysis
        .drafts
        .iter()
        .filter_map(|&id| analysis.graph.module(id))
        .map(|module| {
            let block = usize::try_from(module.source.0)
                .ok()
                .and_then(|i| block_of_source.get(i))
                .and_then(|&b| blocks.get(b));
            let section = block
                .and_then(|b| b.origin.section.as_ref())
                .map(ToString::to_string);
            let span = block.map_or(0..0, |b| b.origin.span.clone());
            let imports = module
                .model
                .imports
                .iter()
                .zip(&module.imports)
                .map(|(clause, resolved)| {
                    let resolved_to =
                        resolved
                            .target
                            .and_then(|t| analysis.graph.module(t))
                            .map(|t| match t.origin {
                                Origin::Corpus { record } => {
                                    corpus.modules().get(record).map_or_else(
                                        || t.model.name.clone(),
                                        |r| glint_corpus::unique_label(corpus, r),
                                    )
                                }
                                Origin::Draft | Origin::Builtin => t.model.name.clone(),
                            });
                    let _ = Matched::Opaque;
                    ImportSummary {
                        module: clause.module_name.clone(),
                        resolved_to,
                        symbols: clause.symbols.iter().map(|s| s.name.clone()).collect(),
                    }
                })
                .collect();
            ModuleSummary {
                name: module.model.name.clone(),
                oid: module.oid.as_ref().map(ToString::to_string),
                section,
                span,
                imports,
            }
        })
        .collect()
}

/// The document id (`rfc5280`, `draft-ietf-lamps-foo-03`) that a file name names, if it names one: `rfc5280.txt`,
/// `path/to/draft-ietf-lamps-foo-03.xml`.
pub fn document_id(name: &str) -> Option<String> {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = file
        .rsplit_once('.')
        .filter(|(_, ext)| matches!(*ext, "txt" | "xml" | "md"))
        .map_or(file, |(stem, _)| stem)
        .to_ascii_lowercase();
    let is_rfc = glint_corpus::rfc_number(&stem).is_some();
    let is_draft = stem.starts_with("draft-")
        && stem
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    (is_rfc || is_draft).then_some(stem)
}

/// Every name written in the document outside its modules (the prose and the snippets).
fn mentioned_names(text: &str, blocks: &[Block]) -> std::collections::HashSet<String> {
    let mut masked = text.to_owned();
    for block in blocks.iter().filter(|b| b.kind != BlockKind::Fragment) {
        let span = block.origin.span.clone();
        if let Some(len) = masked.get(span.clone()).map(str::len) {
            masked.replace_range(span, &" ".repeat(len));
        }
    }
    masked
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::document_id;

    #[test]
    fn file_names_name_documents() {
        assert_eq!(document_id("rfc5280.txt").as_deref(), Some("rfc5280"));
        assert_eq!(document_id("RFC5280").as_deref(), Some("rfc5280"));
        assert_eq!(
            document_id("a/b/draft-ietf-lamps-foo-03.xml").as_deref(),
            Some("draft-ietf-lamps-foo-03")
        );
        assert_eq!(document_id("mine.asn"), None);
        assert_eq!(document_id("document"), None);
    }
}
