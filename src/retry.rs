//! Retry configuration for transient upload and download failures.

use std::time::Duration;

use reqwest::StatusCode;

use crate::error::InternetArchiveError;

/// Exponential-backoff retry settings for transient upload and download
/// transfers.
///
/// Uploads and downloads are retried when the transfer fails with a transient
/// transport error (connection or timeout) or when Internet Archive returns a
/// rate-limit (`429`) or a transient server-error (`500`, `502`, `503`, `504`,
/// including the `503 SlowDown` response IA uses for throttling) status. Reads,
/// searches, and metadata writes are not retried by this mechanism. Set
/// [`RetryOptions::max_retries`] to zero to disable retrying.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryOptions {
    /// Maximum number of retries attempted after the initial try. Zero disables
    /// retrying.
    pub max_retries: u32,
    /// Delay before the first retry.
    pub initial_backoff: Duration,
    /// Upper bound on the delay between retries. The delay doubles after each
    /// retry up to this value.
    pub max_backoff: Duration,
}

impl Default for RetryOptions {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(8),
        }
    }
}

/// Returns whether an HTTP status is worth retrying.
///
/// Covers rate limiting (`429`) and the transient server errors `500`, `502`,
/// `503`, and `504`. This includes the `503 SlowDown` response Internet Archive
/// returns when throttling. Permanent server errors such as `501 Not
/// Implemented` are not retried, since retrying them only wastes the backoff
/// budget before surfacing the same failure.
pub(crate) fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

/// Returns whether a transfer error is transient and worth retrying.
pub(crate) fn is_retryable_transfer_error(error: &InternetArchiveError) -> bool {
    match error {
        InternetArchiveError::Transport(source) => {
            source.is_timeout() || source.is_connect() || source.is_body()
        }
        InternetArchiveError::Http { status, .. } => is_retryable_status(*status),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_retryable_status, is_retryable_transfer_error, RetryOptions};
    use crate::error::InternetArchiveError;
    use reqwest::StatusCode;
    use std::time::Duration;

    #[test]
    fn defaults_are_conservative() {
        let retry = RetryOptions::default();
        assert_eq!(retry.max_retries, 3);
        assert_eq!(retry.initial_backoff, Duration::from_millis(500));
        assert_eq!(retry.max_backoff, Duration::from_secs(8));
    }

    #[test]
    fn retryable_statuses_cover_throttling_and_server_errors() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::GATEWAY_TIMEOUT));
        assert!(!is_retryable_status(StatusCode::NOT_IMPLEMENTED));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
        assert!(!is_retryable_status(StatusCode::OK));
    }

    #[test]
    fn transfer_error_classification_matches_http_status() {
        let throttled = InternetArchiveError::Http {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: Some("SlowDown".to_owned()),
            message: None,
            raw_body: None,
        };
        assert!(is_retryable_transfer_error(&throttled));

        let missing = InternetArchiveError::Http {
            status: StatusCode::NOT_FOUND,
            code: None,
            message: None,
            raw_body: None,
        };
        assert!(!is_retryable_transfer_error(&missing));

        assert!(!is_retryable_transfer_error(
            &InternetArchiveError::MissingAuth
        ));
        assert!(!is_retryable_transfer_error(
            &InternetArchiveError::Timeout("demo")
        ));
    }
}
