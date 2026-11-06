//! What the web page asks of Glint, answered from the published data: checks, documents by name, and corpus queries.
//! The answers are the JSON the page's `api.ts` reads.

use std::collections::BTreeMap;
use std::path::Path;

use glint_check::{
    CheckOptions, Config, DocumentRef, ImportSummary, Report, SeveritySetting, check_document,
};
use glint_corpus::{
    Corpus, CorpusData, CorpusProvider, DataError, Documents, LabelEvidence, LabelProblem,
    ModuleRecord, Oid, OidAssignment, ParseStatus, SCHEMA, SyntaxEra, document_title, draft_name,
    preferred, unique_label,
};
use glint_diag::{Diagnostic, FileId};
use glint_extract::{BlockKind, Format, detect_format};
use serde::{Deserialize, Serialize};

/// The version of the code, reported beside the data build in every result.
pub const GLINT_VERSION: &str = env!("CARGO_PKG_VERSION");
const RESPONSE_SCHEMA: u32 = 1;
const MODULE_LIST_DEFAULT: usize = 50;
const MODULE_LIST_MAX: usize = 500;
const OID_CHILDREN_MAX: usize = 500;

/// An error the page shows; `code` lets it pick its own wording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceError {
    pub code: &'static str,
    pub message: String,
}

impl ServiceError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new("bad_request", message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }
}

impl From<DataError> for ServiceError {
    fn from(error: DataError) -> Self {
        let code = match error {
            DataError::Schema { .. } => "data_schema",
            DataError::BuildMismatch { .. } => "data_build",
            DataError::Damaged(_) => "data_damaged",
        };
        Self::new(code, error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ServiceError>;

/// The loaded data. A page starts with none; checks of pasted text work as soon as the corpus is in.
#[derive(Default)]
pub struct Service {
    corpus: Corpus,
    documents: Option<Documents>,
}

#[derive(Serialize)]
pub struct CorpusResponse {
    glint_version: &'static str,
    schema: u32,
    build: String,
    notices: Vec<String>,
    modules: usize,
    documents: usize,
    by_status: BTreeMap<&'static str, usize>,
    oid_assignments: usize,
    oid_nodes: usize,
}

impl Service {
    /// Replaces the corpus with `corpus.json`. Documents of another build are dropped with it.
    pub fn load_corpus(&mut self, json: &[u8]) -> Result<CorpusResponse> {
        let data = CorpusData::from_json(json)?;
        self.corpus = Corpus::from_data(data);
        if self
            .documents
            .as_ref()
            .is_some_and(|d| d.build() != self.corpus.info().build)
        {
            self.documents = None;
        }
        Ok(self.corpus_info())
    }

    /// Takes `documents.bin`, which must be from the build of the corpus.
    pub fn load_documents(&mut self, bytes: Vec<u8>) -> Result<()> {
        let documents = Documents::read(bytes)?;
        documents.require_build(&self.corpus.info().build)?;
        self.documents = Some(documents);
        Ok(())
    }

    pub fn has_documents(&self) -> bool {
        self.documents.is_some()
    }

    pub fn corpus(&self) -> &Corpus {
        &self.corpus
    }

    pub fn corpus_info(&self) -> CorpusResponse {
        let corpus = &self.corpus;
        let mut by_status = BTreeMap::new();
        for module in corpus.modules() {
            let count: &mut usize = by_status
                .entry(status_name(module.parse_status))
                .or_default();
            *count = count.saturating_add(1);
        }
        let info = corpus.info();
        CorpusResponse {
            glint_version: GLINT_VERSION,
            schema: SCHEMA,
            build: info.build.clone(),
            notices: info.notices.clone(),
            modules: corpus.modules().len(),
            documents: corpus.documents().len(),
            by_status,
            oid_assignments: corpus.assignments_under(&Oid::new(Vec::new())).len(),
            oid_nodes: corpus.oid_labels().nodes().len(),
        }
    }

    // ---- check ----

    pub fn check(&self, request: CheckRequest) -> Result<CheckResponse> {
        let name = request.name.unwrap_or_else(|| "document".to_owned());
        let format = request.format.map_or_else(
            || detect_format(Some(Path::new(&name)), &request.content),
            Format::from,
        );
        let options = request.options.into_check_options()?;
        let report = check_document(&name, &request.content, format, &self.corpus, &options);
        Ok(self.to_response(report, name, None))
    }

    /// Checks a draft or RFC by name from `documents.bin`: `draft-ietf-lamps-foo`, `draft-ietf-lamps-foo-03`,
    /// `rfc5280`. The answer carries the text, which the page has not got.
    pub fn check_document(&self, request: DocumentRequest) -> Result<CheckResponse> {
        let options = request.options.into_check_options()?;
        let reference = match (request.draft, request.rfc) {
            (Some(draft), None) => DocumentRef::draft(&draft, request.rev.as_deref()),
            (None, Some(number)) => DocumentRef::rfc(number),
            _ => {
                return Err(ServiceError::bad_request(
                    "give exactly one of `draft` (with optional `rev`) and `rfc` (1 to 99999)",
                ));
            }
        }
        .map_err(|e| ServiceError::bad_request(e.to_string()))?;
        let documents = self.documents.as_ref().ok_or_else(|| {
            ServiceError::new(
                "documents_loading",
                "the document texts are still loading; try again in a moment",
            )
        })?;
        let id = self.document_id(&reference)?;
        let (text, format) = documents
            .get(&id)
            .ok_or_else(|| self.not_in_corpus(&reference))?;
        let format = if format == "xml" {
            Format::Xml
        } else {
            Format::Text
        };
        let report = check_document(&id, text, format, &self.corpus, &options);
        Ok(self.to_response(report, id, Some(text.to_owned())))
    }

    /// The document id that `documents.bin` has for a reference: the RFC, or the revision of the draft the corpus
    /// has, which must be the one asked for when a revision is given.
    fn document_id(&self, reference: &DocumentRef) -> Result<String> {
        match reference {
            DocumentRef::Rfc(number) => Ok(format!("rfc{number}")),
            DocumentRef::Draft { name, rev } => {
                let current = self
                    .corpus
                    .documents()
                    .iter()
                    .find(|d| draft_name(&d.id) == Some(name.as_str()))
                    .map(|d| d.id.clone())
                    .ok_or_else(|| self.not_in_corpus(reference))?;
                match rev {
                    Some(rev) if current != format!("{name}-{rev}") => {
                        Err(ServiceError::not_found(format!(
                            "{name}-{rev} is not the current revision; the corpus has {current}"
                        )))
                    }
                    _ => Ok(current),
                }
            }
        }
    }

    fn not_in_corpus(&self, reference: &DocumentRef) -> ServiceError {
        let name = match reference {
            DocumentRef::Rfc(number) => format!("RFC {number}"),
            DocumentRef::Draft {
                name,
                rev: Some(rev),
            } => format!("{name}-{rev}"),
            DocumentRef::Draft { name, rev: None } => name.clone(),
        };
        ServiceError::not_found(format!(
            "{name} is not in the corpus, which has every RFC and active Internet-Draft with an ASN.1 module \
             (build {}); paste or open the document instead",
            self.corpus.info().build
        ))
    }

    fn to_response(&self, report: Report, name: String, content: Option<String>) -> CheckResponse {
        let files = report
            .files
            .iter()
            .map(|(id, file_name)| {
                let is_document = id == report.document;
                FileInfo {
                    id,
                    name: file_name.to_owned(),
                    kind: if is_document { "document" } else { "corpus" },
                    module: if is_document {
                        None
                    } else {
                        ModuleRef::from_label(file_name)
                    },
                }
            })
            .collect();
        let blocks = report
            .blocks
            .iter()
            .map(|block| BlockInfo {
                kind: block_kind_name(block.kind),
                section: block.section.clone(),
                title: block.title.clone(),
                start: block.span.start,
                end: block.span.end,
                line: block.line_col.0,
                column: block.line_col.1,
            })
            .collect();
        let modules = report
            .modules
            .iter()
            .map(|module| ModuleInfo {
                name: module.name.clone(),
                oid: module.oid.clone(),
                section: module.section.clone(),
                start: module.span.start,
                end: module.span.end,
                imports: import_infos(&module.imports),
            })
            .collect();
        CheckResponse {
            schema_version: RESPONSE_SCHEMA,
            glint_version: GLINT_VERSION,
            data_build: report.data_build,
            document: DocumentInfo {
                name,
                format: format_name(report.format),
                content,
            },
            files,
            blocks,
            modules,
            diagnostics: report.diagnostics,
        }
    }

    // ---- corpus ----

    pub fn modules(&self, query: &str, offset: usize, limit: Option<usize>) -> ModulePage {
        let query = query.trim().to_ascii_lowercase();
        let oid_query = query.trim_matches('.').to_owned();
        let limit = limit.unwrap_or(MODULE_LIST_DEFAULT).min(MODULE_LIST_MAX);
        let mut hits: Vec<(u8, &ModuleRecord)> = self
            .corpus
            .modules()
            .iter()
            .filter_map(|module| {
                if query.is_empty() {
                    Some((0, module))
                } else {
                    match_rank(module, &query, &oid_query).map(|rank| (rank, module))
                }
            })
            .collect();
        hits.sort_by(|(ra, a), (rb, b)| {
            ra.cmp(rb)
                .then_with(|| b.rank().cmp(&a.rank()))
                .then_with(|| a.name.cmp(&b.name))
        });
        ModulePage {
            total: hits.len(),
            offset,
            limit,
            modules: hits
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|(_, module)| ModuleListItem::new(module))
                .collect(),
        }
    }

    pub fn module(&self, source: &str, name: &str, oid: Option<&str>) -> Result<ModuleDetail> {
        let wanted_oid = oid
            .map(|text| {
                text.parse::<Oid>()
                    .map_err(|_| ServiceError::bad_request(format!("`{text}` is not a dotted OID")))
            })
            .transpose()?;
        let corpus = &self.corpus;
        let candidates: Vec<usize> = corpus
            .by_name(name)
            .iter()
            .copied()
            .filter(|&i| {
                corpus.modules().get(i).is_some_and(|m| {
                    source_matches(m, source)
                        && wanted_oid
                            .as_ref()
                            .is_none_or(|oid| m.oid.as_ref() == Some(oid))
                })
            })
            .collect();
        let record = preferred(corpus, &candidates)
            .and_then(|i| corpus.modules().get(i))
            .ok_or_else(|| ServiceError::not_found(format!("no module `{name}` in `{source}`")))?;
        let label = unique_label(corpus, record);
        let options = CheckOptions {
            no_snippets: true,
            ..CheckOptions::default()
        };
        let report = check_document(&label, &record.text, Format::Bare, corpus, &options);
        let imports = report
            .modules
            .iter()
            .find(|m| m.name == record.name)
            .or(report.modules.first())
            .map(|m| import_infos(&m.imports))
            .unwrap_or_default();
        Ok(ModuleDetail {
            summary: ModuleListItem::new(record),
            rfc_status: record.rfc_status.clone(),
            syntax_era: record.syntax_era,
            source_kind: record.source.kind.clone(),
            exports: record.exports.clone(),
            sha256: record.sha256.clone(),
            text: record.text.clone(),
            imports,
        })
    }

    /// Looks up `dotted`. `names` are the names written for its arcs, in order, with an empty string for an arc
    /// written as a bare number; each is compared with the labels the corpus has for that arc.
    pub fn oid(&self, dotted: &str, names: &[String]) -> Result<OidResponse> {
        let oid: Oid = dotted
            .parse()
            .map_err(|e: glint_corpus::OidParseError| ServiceError::bad_request(e.to_string()))?;
        let corpus = &self.corpus;
        let assignments = corpus.assignments_of(&oid).to_vec();
        let below: Vec<&OidAssignment> = corpus
            .assignments_under(&oid)
            .into_iter()
            .filter(|a| a.oid != oid)
            .collect();
        let truncated = below.len() > OID_CHILDREN_MAX;
        let children = below.into_iter().take(OID_CHILDREN_MAX).cloned().collect();
        let module = corpus
            .by_oid(&oid)
            .and_then(|i| corpus.modules().get(i))
            .map(|m| ModuleRef {
                source: m.source.label(),
                name: m.name.clone(),
                oid: m.oid.as_ref().map(ToString::to_string),
            });
        let allowlisted = corpus.oid_labels().allowed(&oid).map(|a| a.reason.clone());
        Ok(OidResponse {
            oid: oid.to_string(),
            parent: oid.parent().map(|p| p.to_string()),
            module,
            labels: corpus.oid_labels().labels_of(&oid).to_vec(),
            label_notes: label_notes(corpus, &oid, names),
            allowlisted,
            assignments,
            children,
            truncated,
        })
    }
}

fn document_list(documents: &[String]) -> String {
    const SHOWN: usize = 3;
    let titles: Vec<String> = documents
        .iter()
        .take(SHOWN)
        .map(|d| document_title(d))
        .collect();
    let rest = documents.len().saturating_sub(SHOWN);
    if rest > 0 {
        format!("{} and {rest} more", titles.join(", "))
    } else {
        titles.join(", ")
    }
}

/// The names written for the arcs of `oid`, checked as the checker checks labels (W142, W143, I145): against the
/// labels the corpus has for each arc. The arcs after a wrongly placed one are under the wrong parent too, so the
/// first such problem ends the list; an arc the corpus calls by another name does not.
fn label_notes(corpus: &Corpus, oid: &Oid, names: &[String]) -> Vec<String> {
    let table = corpus.oid_labels();
    if table.allowed(oid).is_some() {
        return Vec::new();
    }
    let mut notes = Vec::new();
    for (length, name) in (1..=oid.len()).zip(names) {
        if name.is_empty() {
            continue;
        }
        let arc = Oid::new(oid.arcs().get(..length).unwrap_or_default().to_vec());
        let number = arc.last().unwrap_or_default();
        let parent = arc.parent().map_or_else(String::new, |p| p.to_string());
        let defined = corpus.by_oid(&arc).is_some();
        let problem = if length == 1 {
            // The table does not check top-level arcs, whose labels never vary in practice; a name nobody writes
            // for one is still worth a note.
            let others: Vec<String> = table
                .labels_of(&arc)
                .iter()
                .map(|e| e.label.clone())
                .collect();
            (!others.is_empty() && !others.contains(name))
                .then_some(LabelProblem::Synonym { others })
        } else {
            table.check(&arc, name, None, defined)
        };
        match problem {
            None => {}
            Some(LabelProblem::Synonym { others }) => {
                notes.push(format!(
                    "The corpus calls {arc} {}, not {name}.",
                    others.join(" or ")
                ));
            }
            Some(LabelProblem::WrongNumber {
                expected,
                documents,
            }) => {
                notes.push(format!(
                    "{name}({number}): under {parent} the corpus numbers {name} {expected} ({}).",
                    document_list(&documents)
                ));
                break;
            }
            Some(LabelProblem::WrongParent {
                expected_parent,
                documents,
            }) => {
                notes.push(format!(
                    "{name}({number}) is under {expected_parent} in the corpus, not under {parent} ({}). Check for a missing or extra arc before it.",
                    document_list(&documents)
                ));
                break;
            }
        }
    }
    notes
}

// ---- requests ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRequest {
    pub content: String,
    pub name: Option<String>,
    pub format: Option<FormatName>,
    #[serde(default)]
    pub options: OptionsInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentRequest {
    pub draft: Option<String>,
    pub rev: Option<String>,
    pub rfc: Option<u32>,
    #[serde(default)]
    pub options: OptionsInput,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FormatName {
    Xml,
    Markdown,
    Text,
    Bare,
}

impl From<FormatName> for Format {
    fn from(name: FormatName) -> Self {
        match name {
            FormatName::Xml => Self::Xml,
            FormatName::Markdown => Self::Markdown,
            FormatName::Text => Self::Text,
            FormatName::Bare => Self::Bare,
        }
    }
}

fn format_name(format: Format) -> &'static str {
    match format {
        Format::Xml => "xml",
        Format::Markdown => "markdown",
        Format::Text => "text",
        Format::Bare => "bare",
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OptionsInput {
    /// Module name to `rfcNNNN`.
    pins: BTreeMap<String, String>,
    severity: BTreeMap<String, SeveritySetting>,
    /// Compare the snippets in the text with the modules (default true).
    snippets: Option<bool>,
}

impl OptionsInput {
    fn into_check_options(self) -> Result<CheckOptions> {
        let mut pins = Vec::new();
        for (name, rfc) in self.pins {
            let valid = !name.is_empty()
                && rfc
                    .strip_prefix("rfc")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if !valid {
                return Err(ServiceError::bad_request(format!(
                    "pin `{name}` must be `rfcNNNN`, got `{rfc}`"
                )));
            }
            pins.push((name, rfc));
        }
        Ok(CheckOptions {
            config: Config {
                severity: self.severity,
                ..Config::default()
            },
            pins,
            no_snippets: self.snippets == Some(false),
        })
    }
}

// ---- responses ----

#[derive(Serialize)]
pub struct CheckResponse {
    schema_version: u32,
    glint_version: &'static str,
    data_build: String,
    document: DocumentInfo,
    files: Vec<FileInfo>,
    blocks: Vec<BlockInfo>,
    modules: Vec<ModuleInfo>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Serialize)]
struct DocumentInfo {
    name: String,
    format: &'static str,
    /// Only for a document checked by name, whose text the page has not got.
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

#[derive(Serialize)]
struct FileInfo {
    id: FileId,
    name: String,
    kind: &'static str,
    /// For corpus files, the module they are.
    #[serde(skip_serializing_if = "Option::is_none")]
    module: Option<ModuleRef>,
}

#[derive(Serialize, Clone)]
pub struct ModuleRef {
    source: String,
    name: String,
    /// Tells apart modules of one name in one source.
    #[serde(skip_serializing_if = "Option::is_none")]
    oid: Option<String>,
}

impl ModuleRef {
    fn from_label(label: &str) -> Option<Self> {
        let (source, rest) = label.split_once('#')?;
        let (name, oid) = match rest.split_once('@') {
            Some((name, oid)) => (name, Some(oid.to_owned())),
            None => (rest, None),
        };
        Some(Self {
            source: source.to_owned(),
            name: name.to_owned(),
            oid,
        })
    }
}

#[derive(Serialize)]
struct BlockInfo {
    kind: &'static str,
    section: Option<String>,
    title: Option<String>,
    start: usize,
    end: usize,
    line: u32,
    column: u32,
}

#[derive(Serialize)]
struct ModuleInfo {
    name: String,
    oid: Option<String>,
    section: Option<String>,
    start: usize,
    end: usize,
    imports: Vec<ImportInfo>,
}

#[derive(Serialize)]
pub struct ImportInfo {
    module: String,
    resolved_to: Option<String>,
    target: Option<ModuleRef>,
    symbols: Vec<String>,
}

fn block_kind_name(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Module => "module",
        BlockKind::ModuleList => "module-list",
        BlockKind::Fragment => "fragment",
    }
}

fn import_infos(imports: &[ImportSummary]) -> Vec<ImportInfo> {
    imports
        .iter()
        .map(|import| ImportInfo {
            module: import.module.clone(),
            resolved_to: import.resolved_to.clone(),
            target: import
                .resolved_to
                .as_deref()
                .and_then(ModuleRef::from_label),
            symbols: import.symbols.clone(),
        })
        .collect()
}

pub fn status_name(status: ParseStatus) -> &'static str {
    match status {
        ParseStatus::Clean => "clean",
        ParseStatus::Warnings => "warnings",
        ParseStatus::Errors => "errors",
        ParseStatus::Unparseable => "unparseable",
    }
}

#[derive(Serialize)]
pub struct ModuleListItem {
    name: String,
    oid: Option<String>,
    body: String,
    document: String,
    edition: Option<String>,
    url: Option<String>,
    section: Option<String>,
    status: ParseStatus,
    obsoleted_by: Vec<u32>,
}

impl ModuleListItem {
    fn new(module: &ModuleRecord) -> Self {
        Self {
            name: module.name.clone(),
            oid: module.oid.as_ref().map(ToString::to_string),
            body: module.source.body.clone(),
            document: module.source.document.clone(),
            edition: module.source.edition.clone(),
            url: module.source.url.clone(),
            section: module.source.section.clone(),
            status: module.parse_status,
            obsoleted_by: module.obsoleted_by.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct ModulePage {
    /// Modules that match, over all pages.
    total: usize,
    offset: usize,
    limit: usize,
    modules: Vec<ModuleListItem>,
}

/// How well `query` matches a module: lower is better, `None` is no match.
fn match_rank(module: &ModuleRecord, query: &str, oid_query: &str) -> Option<u8> {
    let name = module.name.to_ascii_lowercase();
    if name == query {
        return Some(0);
    }
    if name.starts_with(query) {
        return Some(1);
    }
    if name.contains(query) {
        return Some(2);
    }
    let oid = module.oid.as_ref().map(ToString::to_string)?;
    if !oid_query.is_empty() && oid.contains(oid_query) {
        return Some(3);
    }
    None
}

#[derive(Serialize)]
pub struct ModuleDetail {
    #[serde(flatten)]
    summary: ModuleListItem,
    rfc_status: Option<String>,
    syntax_era: SyntaxEra,
    source_kind: glint_corpus::SourceKind,
    exports: glint_corpus::Exports,
    sha256: String,
    text: String,
    imports: Vec<ImportInfo>,
}

fn source_matches(module: &ModuleRecord, source: &str) -> bool {
    let label = module.source.label();
    if label.eq_ignore_ascii_case(source) {
        return true;
    }
    source.bytes().all(|b| b.is_ascii_digit())
        && !source.is_empty()
        && label.eq_ignore_ascii_case(&format!("rfc{source}"))
}

#[derive(Serialize)]
pub struct OidResponse {
    oid: String,
    parent: Option<String>,
    module: Option<ModuleRef>,
    /// The `name(number)` labels documents write for this arc.
    labels: Vec<LabelEvidence>,
    /// Where the names written for the arcs disagree with the labels documents write for them.
    label_notes: Vec<String>,
    /// Why the OID is on the allowlist of wrong but deployed OIDs, when it is.
    allowlisted: Option<String>,
    assignments: Vec<OidAssignment>,
    children: Vec<OidAssignment>,
    truncated: bool,
}
