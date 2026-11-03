#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use glint_corpus::{
    Bundle, Corpus, CorpusInfo, Exports, ModuleRecord, ModuleSource, OidAssignment, ParseStatus,
    SourceKind, SyntaxEra,
};
use glint_server::{AppState, BoxFuture, FetchError, Fetcher, HttpFetcher, Settings, router};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const BASE: &str = "Base { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN\n\
    Name ::= INTEGER\n\
    id-base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 }\n\
    END\n";

const USER: &str = "User { 1 3 6 1 4 1 99 2 } DEFINITIONS ::= BEGIN\n\
    IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 };\n\
    Thing ::= SEQUENCE { n Name }\n\
    END\n";

const BROKEN: &str = "Broken DEFINITIONS ::= BEGIN\n\
    Thing ::= SEQUENCE { n Missing }\n\
    END\n";

fn record(name: &str, rfc: u32, oid: Option<&str>, text: &str) -> ModuleRecord {
    ModuleRecord {
        name: name.to_owned(),
        oid: oid.map(|o| o.parse().unwrap()),
        source: ModuleSource {
            rfc: Some(rfc),
            section: Some("Appendix A".to_owned()),
            kind: SourceKind::Extracted,
        },
        rfc_status: Some("proposed standard".to_owned()),
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: text.to_owned(),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: "abc".to_owned(),
    }
}

fn corpus() -> Corpus {
    Corpus::from_bundle(Bundle {
        info: CorpusInfo {
            version: "2026-01-01".to_owned(),
            notices: vec!["test corpus".to_owned()],
        },
        modules: vec![
            record("Base", 1000, Some("1.3.6.1.4.1.99.1"), BASE),
            record("User", 1001, Some("1.3.6.1.4.1.99.2"), USER),
        ],
        oid_assignments: vec![
            OidAssignment {
                oid: "1.3.6.1.4.1.99".parse().unwrap(),
                name: "id-base".to_owned(),
                module: "Base".to_owned(),
                rfc: Some(1000),
            },
            OidAssignment {
                oid: "1.3.6.1.4.1.99.7".parse().unwrap(),
                name: "id-seven".to_owned(),
                module: "Base".to_owned(),
                rfc: Some(1000),
            },
        ],
    })
}

#[derive(Default)]
struct FakeFetcher {
    documents: HashMap<String, String>,
    requested: Mutex<Vec<String>>,
}

impl Fetcher for FakeFetcher {
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, FetchError>> {
        Box::pin(async move {
            self.requested.lock().unwrap().push(url.to_owned());
            self.documents.get(url).cloned().ok_or(FetchError::NotFound)
        })
    }
}

fn app_with(settings: Settings, documents: &[(&str, &str)]) -> (Router, Arc<FakeFetcher>) {
    let fetcher = Arc::new(FakeFetcher {
        documents: documents
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        requested: Mutex::new(Vec::new()),
    });
    let state = AppState::new(corpus(), fetcher.clone(), settings);
    (router(state), fetcher)
}

fn settings() -> Settings {
    Settings {
        rate_limit_per_minute: 0,
        ..Settings::default()
    }
}

fn app() -> Router {
    app_with(settings(), &[]).0
}

async fn send(app: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn post(uri: &str, body: &Value) -> Request<Body> {
    let bytes = serde_json::to_vec(body).unwrap();
    Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(Body::from(bytes))
        .unwrap()
}

fn check_body(content: &str) -> Value {
    json!({ "document": { "format": "bare", "name": "x.asn", "content": content } })
}

#[tokio::test]
async fn health_answers_ok() {
    let (status, _) = send(app(), get("/api/v1/health")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(app(), get("/health")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn check_reports_diagnostics_blocks_and_modules() {
    let (status, body) = send(app(), post("/api/v1/check", &check_body(BROKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["corpus_version"], "2026-01-01");
    assert_eq!(body["document"]["name"], "x.asn");
    assert_eq!(body["document"]["format"], "bare");
    assert!(body["document"].get("content").is_none());
    let codes: Vec<&str> = body["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"E110"), "{codes:?}");
    assert_eq!(body["modules"][0]["name"], "Broken");
    assert_eq!(body["blocks"][0]["kind"], "module");
    assert_eq!(body["blocks"][0]["line"], 1);
    assert_eq!(body["files"][0]["kind"], "document");
}

#[tokio::test]
async fn check_links_imports_to_corpus_modules() {
    let (status, body) = send(app(), post("/api/v1/check", &check_body(USER))).await;
    assert_eq!(status, StatusCode::OK);
    let import = &body["modules"][0]["imports"][0];
    assert_eq!(import["module"], "Base");
    assert_eq!(import["resolved_to"], "rfc1000#Base");
    assert_eq!(
        import["target"],
        json!({ "source": "rfc1000", "name": "Base" })
    );
    assert_eq!(import["symbols"], json!(["Name"]));
}

#[tokio::test]
async fn check_detects_the_format_from_the_name_when_none_is_given() {
    let body = json!({ "document": { "name": "m.asn", "content": BROKEN } });
    let (status, body) = send(app(), post("/api/v1/check", &body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["document"]["format"], "bare");
}

#[tokio::test]
async fn check_applies_severity_overrides_and_pins() {
    let body = json!({
        "document": { "format": "bare", "content": BROKEN },
        "options": { "severity": { "E110": "off" } },
    });
    let (status, body) = send(app(), post("/api/v1/check", &body)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "E110")
    );

    let body = json!({
        "document": { "format": "bare", "content": USER },
        "options": { "pins": { "Base": "rfc1000" } },
    });
    let (status, _) = send(app(), post("/api/v1/check", &body)).await;
    assert_eq!(status, StatusCode::OK);

    let body = json!({
        "document": { "format": "bare", "content": USER },
        "options": { "pins": { "Base": "5280" } },
    });
    let (status, body) = send(app(), post("/api/v1/check", &body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "bad_request");
}

#[tokio::test]
async fn malformed_requests_are_bad_requests_with_a_json_body() {
    let request = Request::post("/api/v1/check")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let (status, body) = send(app(), request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "bad_request");

    let (status, _) = send(app(), post("/api/v1/check", &json!({ "document": {} }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, body) = send(app(), get("/api/v1/nothing")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn a_body_over_the_limit_is_rejected() {
    let small = Settings {
        max_body: 1024,
        ..settings()
    };
    let big = check_body(&"-- padding\n".repeat(500));
    let (app, _) = app_with(small.clone(), &[]);
    let (status, _) = send(app, post("/api/v1/check", &big)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    let (app, _) = app_with(small, &[]);
    let bytes = serde_json::to_vec(&big).unwrap();
    let chunked = Request::post("/api/v1/check")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from_stream(futures_once(bytes)))
        .unwrap();
    let (status, _) = send(app, chunked).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

fn futures_once(
    bytes: Vec<u8>,
) -> impl futures_core::Stream<Item = Result<Vec<u8>, std::io::Error>> {
    struct Once(Option<Vec<u8>>);
    impl futures_core::Stream for Once {
        type Item = Result<Vec<u8>, std::io::Error>;
        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            std::task::Poll::Ready(self.0.take().map(Ok))
        }
    }
    Once(Some(bytes))
}

#[tokio::test]
async fn a_check_that_takes_too_long_times_out() {
    let impatient = Settings {
        check_timeout: std::time::Duration::ZERO,
        ..settings()
    };
    let (app, _) = app_with(impatient, &[]);
    let (status, body) = send(app, post("/api/v1/check", &check_body(USER))).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(body["error"]["code"], "check_timeout");
}

#[tokio::test]
async fn a_slow_request_times_out_as_a_whole() {
    let impatient = Settings {
        request_timeout: std::time::Duration::from_millis(50),
        ..settings()
    };
    let (app, _) = app_with(impatient, &[]);
    let slow = Request::post("/api/v1/check")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from_stream(PendingStream))
        .unwrap();
    let response = app.oneshot(slow).await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
}

struct PendingStream;

impl futures_core::Stream for PendingStream {
    type Item = Result<Vec<u8>, std::io::Error>;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Pending
    }
}

#[tokio::test]
async fn a_draft_is_fetched_by_name_and_revision() {
    let (app, fetcher) = app_with(
        settings(),
        &[(
            "https://www.ietf.org/archive/id/draft-ietf-x-foo-03.txt",
            BROKEN,
        )],
    );
    let body = json!({ "draft": "draft-ietf-x-foo", "rev": "03" });
    let (status, body) = send(app, post("/api/v1/check/draft", &body)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["document"]["name"], "draft-ietf-x-foo-03");
    assert_eq!(body["document"]["content"], BROKEN);
    assert_eq!(body["modules"][0]["name"], "Broken");
    let requested = fetcher.requested.lock().unwrap().clone();
    assert_eq!(
        requested,
        [
            "https://www.ietf.org/archive/id/draft-ietf-x-foo-03.xml",
            "https://www.ietf.org/archive/id/draft-ietf-x-foo-03.txt",
        ]
    );
}

#[tokio::test]
async fn a_draft_without_a_revision_uses_the_latest() {
    let (app, _) = app_with(
        settings(),
        &[
            (
                "https://datatracker.ietf.org/api/v1/doc/document/draft-ietf-x-foo/?format=json",
                r#"{"name":"draft-ietf-x-foo","rev":"07"}"#,
            ),
            (
                "https://www.ietf.org/archive/id/draft-ietf-x-foo-07.txt",
                BROKEN,
            ),
        ],
    );
    let (status, body) = send(
        app,
        post(
            "/api/v1/check/draft",
            &json!({ "draft": "draft-ietf-x-foo" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["document"]["name"], "draft-ietf-x-foo-07");
}

#[tokio::test]
async fn a_revision_written_into_the_name_is_split_off() {
    let (app, _) = app_with(
        settings(),
        &[(
            "https://www.ietf.org/archive/id/draft-ietf-x-foo-03.txt",
            BROKEN,
        )],
    );
    let (status, body) = send(
        app,
        post(
            "/api/v1/check/draft",
            &json!({ "draft": "draft-ietf-x-foo-03" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn an_rfc_is_fetched_by_number_and_prefers_xml() {
    let xml = "<?xml version=\"1.0\"?><rfc><middle><section><artwork type=\"asn.1\">".to_owned()
        + BROKEN
        + "</artwork></section></middle></rfc>";
    let (app, _) = app_with(
        settings(),
        &[("https://www.rfc-editor.org/rfc/rfc9999.xml", xml.as_str())],
    );
    let (status, body) = send(app, post("/api/v1/check/draft", &json!({ "rfc": 9999 }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["document"]["name"], "rfc9999");
    assert_eq!(body["document"]["format"], "xml");
}

#[tokio::test]
async fn draft_requests_are_validated() {
    for body in [
        json!({}),
        json!({ "draft": "draft-a-b", "rfc": 5280 }),
        json!({ "draft": "../../etc/passwd" }),
        json!({ "draft": "draft-ietf-x-foo", "rev": "3" }),
        json!({ "draft": "Draft_x" }),
        json!({ "rfc": 0 }),
        json!({ "rfc": 100000 }),
    ] {
        let (status, response) = send(app(), post("/api/v1/check/draft", &body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
    }
}

#[tokio::test]
async fn a_missing_draft_is_not_found() {
    let (status, body) = send(
        app(),
        post(
            "/api/v1/check/draft",
            &json!({ "draft": "draft-ietf-x-foo", "rev": "01" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn the_http_fetcher_only_talks_to_allowlisted_https_hosts() {
    let fetcher = HttpFetcher::new(vec!["www.ietf.org".to_owned()], 1024).unwrap();
    for url in [
        "https://example.com/x",
        "http://www.ietf.org/x",
        "https://www.ietf.org.evil.example/x",
        "https://user@evil.example/x",
    ] {
        let result = fetcher.fetch(url).await;
        assert!(
            matches!(result, Err(FetchError::HostNotAllowed(_))),
            "{url}: {result:?}"
        );
    }
}

#[tokio::test]
async fn modules_are_searched_by_name_and_by_oid() {
    let (status, body) = send(app(), get("/api/v1/modules?q=base")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["modules"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["modules"][0],
        json!({
            "name": "Base",
            "oid": "1.3.6.1.4.1.99.1",
            "source": "rfc1000",
            "rfc": 1000,
            "section": "Appendix A",
            "status": "clean",
            "obsoleted_by": [],
        })
    );

    let (_, body) = send(app(), get("/api/v1/modules?q=99.2")).await;
    assert_eq!(body["modules"][0]["name"], "User");

    let (_, body) = send(app(), get("/api/v1/modules")).await;
    assert_eq!(body["modules"].as_array().unwrap().len(), 2);

    let (_, body) = send(app(), get("/api/v1/modules?q=nothing-like-this")).await;
    assert_eq!(body["total"], 0);
    assert!(body["modules"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_module_list_can_be_paged() {
    let (_, first) = send(app(), get("/api/v1/modules?limit=1")).await;
    assert_eq!(first["total"], 2);
    assert_eq!(first["limit"], 1);
    assert_eq!(first["offset"], 0);
    assert_eq!(first["modules"].as_array().unwrap().len(), 1);

    let (_, second) = send(app(), get("/api/v1/modules?limit=1&offset=1")).await;
    assert_eq!(second["total"], 2);
    assert_eq!(second["offset"], 1);
    assert_eq!(second["modules"].as_array().unwrap().len(), 1);
    assert_ne!(first["modules"][0]["name"], second["modules"][0]["name"]);

    let (_, past_the_end) = send(app(), get("/api/v1/modules?offset=10")).await;
    assert_eq!(past_the_end["total"], 2);
    assert!(past_the_end["modules"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_module_comes_with_its_text_and_resolved_imports() {
    let (status, body) = send(app(), get("/api/v1/modules/rfc1001/User")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "User");
    assert_eq!(body["text"], USER);
    assert_eq!(body["syntax_era"], "neutral");
    assert_eq!(
        body["imports"][0]["target"],
        json!({ "source": "rfc1000", "name": "Base" })
    );

    let (status, _) = send(app(), get("/api/v1/modules/1001/User")).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(app(), get("/api/v1/modules/rfc1000/User")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn an_oid_lists_its_assignments_and_children() {
    let (status, body) = send(app(), get("/api/v1/oids/1.3.6.1.4.1.99")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["oid"], "1.3.6.1.4.1.99");
    assert_eq!(body["parent"], "1.3.6.1.4.1");
    assert_eq!(body["assignments"][0]["name"], "id-base");
    assert_eq!(body["children"][0]["name"], "id-seven");
    assert_eq!(body["truncated"], false);

    let (_, body) = send(app(), get("/api/v1/oids/1.3.6.1.4.1.99.1")).await;
    assert_eq!(
        body["module"],
        json!({ "source": "rfc1000", "name": "Base" })
    );

    let (status, _) = send(app(), get("/api/v1/oids/not-an-oid")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_corpus_endpoint_counts_modules_by_status() {
    let (status, body) = send(app(), get("/api/v1/corpus")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["version"], "2026-01-01");
    assert_eq!(body["modules"], 2);
    assert_eq!(body["by_status"], json!({ "clean": 2 }));
    assert_eq!(body["oid_assignments"], 2);
    assert_eq!(body["notices"], json!(["test corpus"]));
}

#[tokio::test]
async fn clients_over_the_rate_limit_get_429() {
    let limited = Settings {
        rate_limit_per_minute: 2,
        trust_proxy: true,
        ..settings()
    };
    let (app, _) = app_with(limited, &[]);
    let request = |ip: &str| {
        Request::get("/api/v1/corpus")
            .header("x-forwarded-for", ip)
            .body(Body::empty())
            .unwrap()
    };
    for _ in 0..2 {
        let response = app.clone().oneshot(request("203.0.113.5")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = app.clone().oneshot(request("203.0.113.5")).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let response = app.clone().oneshot(request("203.0.113.6")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app.oneshot(get("/health")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_static_frontend_is_served_when_configured() {
    let dir = std::env::temp_dir().join(format!("glint-server-static-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<html>glint</html>").unwrap();
    let configured = Settings {
        static_dir: Some(dir),
        ..settings()
    };
    let (app, _) = app_with(configured, &[]);
    let response = app.clone().oneshot(get("/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"<html>glint</html>");
    let response = app.oneshot(get("/api/v1/health")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn responses_carry_security_headers() {
    let (app, _) = app_with(settings(), &[]);
    let response = app.oneshot(get("/api/v1/health")).await.unwrap();
    let headers = response.headers();
    let policy = headers["content-security-policy"].to_str().unwrap();
    assert!(policy.contains("default-src 'self'"), "{policy}");
    assert!(policy.contains("object-src 'none'") && policy.contains("frame-ancestors 'none'"));
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["referrer-policy"], "no-referrer");
}

#[tokio::test]
async fn large_responses_are_compressed_when_the_client_accepts_gzip() {
    let dir = std::env::temp_dir().join(format!("glint-server-gzip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<html>glint</html>".repeat(500)).unwrap();
    let configured = Settings {
        static_dir: Some(dir),
        ..settings()
    };
    let (app, _) = app_with(configured, &[]);
    let request = axum::http::Request::builder()
        .uri("/")
        .header("accept-encoding", "gzip")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.headers()["content-encoding"], "gzip");
}
