use std::sync::Arc;

use glint_corpus::Corpus;
use tokio::sync::Semaphore;

use crate::error::ApiError;
use crate::fetch::Fetcher;
use crate::settings::Settings;

/// What every handler shares. The corpus is loaded once and never changes.
#[derive(Clone)]
pub struct AppState {
    pub(crate) corpus: Arc<Corpus>,
    pub(crate) fetcher: Arc<dyn Fetcher>,
    pub(crate) settings: Arc<Settings>,
    permits: Arc<Semaphore>,
}

impl AppState {
    pub fn new(corpus: Corpus, fetcher: Arc<dyn Fetcher>, settings: Settings) -> Self {
        let permits = Arc::new(Semaphore::new(settings.max_concurrent_checks.max(1)));
        Self {
            corpus: Arc::new(corpus),
            fetcher,
            settings: Arc::new(settings),
            permits,
        }
    }

    /// Runs `job` on the blocking pool, at most `max_concurrent_checks` at a time, and gives up waiting after
    /// `check_timeout`. A job that has started cannot be stopped; it keeps its slot until it ends.
    pub(crate) async fn run_blocking<T, F>(&self, job: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&Corpus) -> T + Send + 'static,
    {
        let corpus = Arc::clone(&self.corpus);
        let permits = Arc::clone(&self.permits);
        let work = async move {
            let permit = permits
                .acquire_owned()
                .await
                .map_err(|_| ApiError::internal())?;
            tokio::task::spawn_blocking(move || {
                let output = job(&corpus);
                drop(permit);
                output
            })
            .await
            .map_err(|_| ApiError::internal())
        };
        match tokio::time::timeout(self.settings.check_timeout, work).await {
            Ok(result) => result,
            Err(_) => Err(ApiError::timeout()),
        }
    }
}
