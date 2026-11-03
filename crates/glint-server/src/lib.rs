//! The Glint HTTP API (`/api/v1`) and static frontend hosting.

mod api;
mod error;
mod fetch;
mod settings;
mod state;

pub use api::router;
pub use error::ApiError;
pub use fetch::{BoxFuture, FetchError, Fetcher, HttpFetcher};
pub use settings::{Settings, SettingsError};
pub use state::AppState;
