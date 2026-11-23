//! Error type for the Jira client.

use reqwest::StatusCode;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    /// Jira returned a non-2xx response with a decoded error body.
    #[error("jira api error {status}: {message}")]
    Api {
        status: StatusCode,
        /// Human-readable message assembled from errorMessages + errors.
        message: String,
    },

    #[error("invalid response: {0}")]
    Parse(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),
}
