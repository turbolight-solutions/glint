//! The `/api/v1` endpoints (design spec, "Web: API and frontend").

use std::collections::BTreeMap;
use std::path::Path;

use axum::extract::{DefaultBodyLimit, FromRequest, Path as UrlPath, Query, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use glint_check::{
    CheckOptions, Config, DocumentRef, RefError, Report, SeveritySetting, check_document,
    draft_urls, latest_revision_url, revision_from_datatracker, rfc_urls,
};
use glint_corpus::{
    CorpusProvider, ModuleRecord, Oid, OidAssignment, ParseStatus, SyntaxEra, preferred,
    unique_label,
};
use glint_diag::{Diagnostic, FileId};
use glint_extract::{BlockKind, Format, detect_format};
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::{PeerIpKeyExtractor, SmartIpKeyExtractor};
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::error::ApiError;
use crate::fetch::FetchError;
use crate::state::AppState;

const SCHEMA_VERSION: u32 = 1;

/// The page loads Bootstrap from jsDelivr (pinned, with integrity hashes) and nothing else from outside.
/// CodeMirror adds `<style>` elements at run time, hence `'unsafe-inline'` for styles only.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self' https://cdn.jsdelivr.net; \
style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; font-src 'self' https://cdn.jsdelivr.net; \
img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; \
frame-ancestors 'none'";
const MODULE_LIST_DEFAULT: usize = 50;
const MODULE_LIST_MAX: usize = 500;
const OID_CHILDREN_MAX: usize = 500;

#[derive(FromRequest)]
#[from_request(via(Json), rejection(ApiError))]
struct ApiJson<T>(T);

/// Builds the application: the API, `/health`, and the static frontend when a directory is configured.
pub fn router(state: AppState) -> Router {
    let settings = state.settings.clone();
    let limited = Router::new()
        .route("/check", post(check))
        .route("/check/draft", post(check_draft))
        .route("/modules", get(search_modules))
        .route("/modules/{source}/{name}", get(module_detail))
        .route("/oids/{dotted}", get(oid_detail))
        .route("/corpus", get(corpus_info));
    let limited = apply_rate_limit(limited, &settings);
    let api = limited
        .route("/health", get(health))
        .fallback(|| async { ApiError::not_found("no such endpoint") });
    let mut app = Router::new()
        .nest("/api/v1", api)
        .route("/health", get(health));
    if let Some(dir) = &settings.static_dir {
        let index = ServeFile::new(dir.join("index.html"));
        app = app.fallback_service(ServeDir::new(dir).not_found_service(index));
    }
    app.layer(
        ServiceBuilder::new()
            .layer(TraceLayer::new_for_http())
            .layer(CompressionLayer::new())
            .layer(security_header(
                header::CONTENT_SECURITY_POLICY,
                CONTENT_SECURITY_POLICY,
            ))
            .layer(security_header(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
            .layer(security_header(header::REFERRER_POLICY, "no-referrer"))
            .layer(TimeoutLayer::with_status_code(
                StatusCode::GATEWAY_TIMEOUT,
                settings.request_timeout,
            ))
            .layer(RequestBodyLimitLayer::new(settings.max_body))
            .layer(DefaultBodyLimit::max(settings.max_body)),
    )
    .with_state(state)
}

fn security_header(name: HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value))
}

fn apply_rate_limit(router: Router<AppState>, settings: &crate::Settings) -> Router<AppState> {
    let per_minute = settings.rate_limit_per_minute;
    if per_minute == 0 {
        return router;
    }
    let interval_ms = (60_000_u64).checked_div(u64::from(per_minute)).unwrap_or(1);
    let interval = std::time::Duration::from_millis(interval_ms.max(1));
    if settings.trust_proxy {
        let config = GovernorConfigBuilder::default()
            .period(interval)
            .burst_size(per_minute)
            .key_extractor(SmartIpKeyExtractor)
            .finish();
        match config {
            Some(config) => router.layer(GovernorLayer::new(config)),
            None => router,
        }
    } else {
        let config = GovernorConfigBuilder::default()
            .period(interval)
            .burst_size(per_minute)
            .key_extractor(PeerIpKeyExtractor)
            .finish();
        match config {
            Some(config) => router.layer(GovernorLayer::new(config)),
            None => router,
        }
    }
}

async fn health() -> StatusCode {
    StatusCode::OK
}

// ---- check ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckRequest {
    document: DocumentInput,
    #[serde(default)]
    options: OptionsInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentInput {
    format: Option<FormatName>,
    name: Option<String>,
    content: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FormatName {
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
struct OptionsInput {
    /// Module name to `rfcNNNN`.
    pins: BTreeMap<String, String>,
    severity: BTreeMap<String, SeveritySetting>,
    /// Compare the snippets in the text with the modules (default true).
    snippets: Option<bool>,
}

impl OptionsInput {
    fn into_check_options(self) -> Result<CheckOptions, ApiError> {
        let mut pins = Vec::new();
        for (name, rfc) in self.pins {
            let valid = !name.is_empty()
                && rfc
                    .strip_prefix("rfc")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if !valid {
                return Err(ApiError::bad_request(format!(
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

#[derive(Serialize)]
struct CheckResponse {
    schema_version: u32,
    corpus_version: String,
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
    /// Only for `/check/draft`, where the client has not got the text.
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
struct ModuleRef {
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
struct ImportInfo {
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

fn import_infos(imports: &[glint_check::ImportSummary]) -> Vec<ImportInfo> {
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

fn to_response(report: Report, name: String, content: Option<String>) -> CheckResponse {
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
        schema_version: SCHEMA_VERSION,
        corpus_version: report.data_build,
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

async fn run_check(
    state: &AppState,
    name: String,
    content: String,
    format: Format,
    options: CheckOptions,
    echo_content: bool,
) -> Result<Json<CheckResponse>, ApiError> {
    let response = state
        .run_blocking(move |corpus| {
            let report = check_document(&name, &content, format, corpus, &options);
            to_response(report, name, echo_content.then_some(content))
        })
        .await?;
    Ok(Json(response))
}

async fn check(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<CheckRequest>,
) -> Result<Json<CheckResponse>, ApiError> {
    let name = request
        .document
        .name
        .unwrap_or_else(|| "document".to_owned());
    let format = request.document.format.map_or_else(
        || detect_format(Some(Path::new(&name)), &request.document.content),
        Format::from,
    );
    let options = request.options.into_check_options()?;
    run_check(
        &state,
        name,
        request.document.content,
        format,
        options,
        false,
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftRequest {
    draft: Option<String>,
    rev: Option<String>,
    rfc: Option<u32>,
    #[serde(default)]
    options: OptionsInput,
}

fn fetch_error(error: FetchError) -> ApiError {
    match error {
        FetchError::NotFound => ApiError::not_found("the document was not found"),
        FetchError::TooLarge => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "the document is larger than the limit",
        ),
        FetchError::HostNotAllowed(host) => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "fetch_failed",
            format!("host `{host}` is not allowed"),
        ),
        FetchError::Upstream(message) => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "fetch_failed",
            format!("could not fetch the document: {message}"),
        ),
    }
}

async fn fetch_first(state: &AppState, urls: &[String]) -> Result<(String, Format), ApiError> {
    let mut last = ApiError::not_found("the document was not found");
    for url in urls {
        match state.fetcher.fetch(url).await {
            Ok(content) => {
                let format = if url.ends_with(".xml") {
                    Format::Xml
                } else {
                    Format::Text
                };
                return Ok((content, format));
            }
            Err(FetchError::NotFound) => {}
            Err(error) => last = fetch_error(error),
        }
    }
    Err(last)
}

async fn latest_revision(state: &AppState, draft: &str) -> Result<String, ApiError> {
    let body = state
        .fetcher
        .fetch(&latest_revision_url(draft))
        .await
        .map_err(fetch_error)?;
    revision_from_datatracker(&body).ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "fetch_failed",
            "could not find the latest revision",
        )
    })
}

async fn check_draft(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<DraftRequest>,
) -> Result<Json<CheckResponse>, ApiError> {
    let options = request.options.into_check_options()?;
    let reference = match (request.draft, request.rfc) {
        (Some(draft), None) => DocumentRef::draft(&draft, request.rev.as_deref()),
        (None, Some(number)) => DocumentRef::rfc(number),
        _ => {
            return Err(ApiError::bad_request(
                "give exactly one of `draft` (with optional `rev`) and `rfc` (1 to 99999)",
            ));
        }
    }
    .map_err(|e: RefError| ApiError::bad_request(e.to_string()))?;
    let (name, urls) = match reference {
        DocumentRef::Draft { name, rev } => {
            let rev = match rev {
                Some(rev) => rev,
                None => latest_revision(&state, &name).await?,
            };
            (format!("{name}-{rev}"), draft_urls(&name, &rev))
        }
        DocumentRef::Rfc(number) => (format!("rfc{number}"), rfc_urls(number)),
    };
    let (content, format) = fetch_first(&state, &urls).await?;
    run_check(&state, name, content, format, options, true).await
}

// ---- corpus ----

#[derive(Serialize)]
struct CorpusResponse {
    version: String,
    notices: Vec<String>,
    modules: usize,
    by_status: BTreeMap<&'static str, usize>,
    oid_assignments: usize,
}

fn status_name(status: ParseStatus) -> &'static str {
    match status {
        ParseStatus::Clean => "clean",
        ParseStatus::Warnings => "warnings",
        ParseStatus::Errors => "errors",
        ParseStatus::Unparseable => "unparseable",
    }
}

async fn corpus_info(State(state): State<AppState>) -> Json<CorpusResponse> {
    let corpus = &state.corpus;
    let mut by_status = BTreeMap::new();
    for module in corpus.modules() {
        let count: &mut usize = by_status
            .entry(status_name(module.parse_status))
            .or_default();
        *count = count.saturating_add(1);
    }
    let info = corpus.info();
    Json(CorpusResponse {
        version: info.build.clone(),
        notices: info.notices.clone(),
        modules: corpus.modules().len(),
        by_status,
        oid_assignments: corpus.assignments_under(&Oid::new(Vec::new())).len(),
    })
}

#[derive(Serialize)]
struct ModuleListItem {
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

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
    limit: Option<usize>,
    offset: Option<usize>,
}

#[derive(Serialize)]
struct ModulePage {
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

async fn search_modules(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Json<ModulePage> {
    let offset = params.offset.unwrap_or(0);
    let query = params.q.trim().to_ascii_lowercase();
    let oid_query = query.trim_matches('.').to_owned();
    let limit = params
        .limit
        .unwrap_or(MODULE_LIST_DEFAULT)
        .min(MODULE_LIST_MAX);
    let mut hits: Vec<(u8, &ModuleRecord)> = state
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
            .then_with(|| b.source.recency().cmp(&a.source.recency()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Json(ModulePage {
        total: hits.len(),
        offset,
        limit,
        modules: hits
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(_, module)| ModuleListItem::new(module))
            .collect(),
    })
}

#[derive(Serialize)]
struct ModuleDetail {
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

#[derive(Deserialize)]
struct DetailParams {
    /// Picks one of several modules of the same name in a source.
    oid: Option<String>,
}

async fn module_detail(
    State(state): State<AppState>,
    UrlPath((source, name)): UrlPath<(String, String)>,
    Query(params): Query<DetailParams>,
) -> Result<Json<ModuleDetail>, ApiError> {
    let wanted_oid = params
        .oid
        .as_deref()
        .map(|text| {
            text.parse::<Oid>()
                .map_err(|_| ApiError::bad_request(format!("`{text}` is not a dotted OID")))
        })
        .transpose()?;
    let candidates: Vec<usize> = state
        .corpus
        .by_name(&name)
        .iter()
        .copied()
        .filter(|&i| {
            state
                .corpus
                .modules()
                .get(i)
                .is_some_and(|m| source_matches(m, &source))
        })
        .filter(|&i| {
            wanted_oid.as_ref().is_none_or(|oid| {
                state
                    .corpus
                    .modules()
                    .get(i)
                    .is_some_and(|m| m.oid.as_ref() == Some(oid))
            })
        })
        .collect();
    let record = preferred(state.corpus.as_ref(), &candidates)
        .and_then(|i| state.corpus.modules().get(i))
        .ok_or_else(|| ApiError::not_found(format!("no module `{name}` in `{source}`")))?
        .clone();
    let text = record.text.clone();
    let label = unique_label(state.corpus.as_ref(), &record);
    let imports = state
        .run_blocking(move |corpus| {
            let options = CheckOptions {
                no_snippets: true,
                ..CheckOptions::default()
            };
            let report = check_document(&label, &text, Format::Bare, corpus, &options);
            report
                .modules
                .iter()
                .find(|m| m.name == record_name(&label))
                .or(report.modules.first())
                .map(|m| import_infos(&m.imports))
                .unwrap_or_default()
        })
        .await?;
    Ok(Json(ModuleDetail {
        summary: ModuleListItem::new(&record),
        rfc_status: record.rfc_status,
        syntax_era: record.syntax_era,
        source_kind: record.source.kind,
        exports: record.exports,
        sha256: record.sha256,
        text: record.text,
        imports,
    }))
}

fn record_name(label: &str) -> &str {
    let name = label.split_once('#').map_or(label, |(_, name)| name);
    name.split_once('@').map_or(name, |(name, _)| name)
}

#[derive(Serialize)]
struct OidResponse {
    oid: String,
    parent: Option<String>,
    module: Option<ModuleRef>,
    assignments: Vec<OidAssignment>,
    children: Vec<OidAssignment>,
    truncated: bool,
}

async fn oid_detail(
    State(state): State<AppState>,
    UrlPath(dotted): UrlPath<String>,
) -> Result<Json<OidResponse>, ApiError> {
    let oid: Oid = dotted
        .parse()
        .map_err(|e: glint_corpus::OidParseError| ApiError::bad_request(e.to_string()))?;
    let corpus = &state.corpus;
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
    Ok(Json(OidResponse {
        oid: oid.to_string(),
        parent: oid.parent().map(|p| p.to_string()),
        module,
        assignments,
        children,
        truncated,
    }))
}
