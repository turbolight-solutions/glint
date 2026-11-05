use std::net::SocketAddr;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use glint_corpus::{Corpus, CorpusData};
use glint_server::{AppState, HttpFetcher, Settings, router};

fn load_corpus(explicit: Option<&Path>) -> Result<Corpus, String> {
    let Some(path) = explicit else {
        return Ok(Corpus::default());
    };
    let json =
        std::fs::read(path).map_err(|e| format!("cannot open corpus {}: {e}", path.display()))?;
    let data = CorpusData::from_json(&json).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Corpus::from_data(data))
}

async fn serve() -> Result<(), String> {
    let settings = Settings::from_env().map_err(|e| e.to_string())?;
    let corpus = load_corpus(settings.corpus.as_deref())?;
    let fetcher = HttpFetcher::new(settings.fetch_allowlist.clone(), settings.max_fetch_bytes)
        .map_err(|e| e.to_string())?;
    let listener = tokio::net::TcpListener::bind(settings.bind)
        .await
        .map_err(|e| format!("cannot bind {}: {e}", settings.bind))?;
    tracing::info!(bind = %settings.bind, corpus = %corpus_version(&corpus), "listening");
    let app = router(AppState::new(corpus, Arc::new(fetcher), settings));
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await
    .map_err(|e| e.to_string())
}

fn corpus_version(corpus: &Corpus) -> String {
    use glint_corpus::CorpusProvider;
    corpus.info().build.clone()
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match serve().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            tracing::error!(%message, "glint-server failed");
            ExitCode::FAILURE
        }
    }
}
