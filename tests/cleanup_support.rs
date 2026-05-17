#![allow(
    dead_code,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::unwrap_used
)]

use std::time::Duration;

use internetarchive_rs::{
    InternetArchiveClient, InternetArchiveError, ItemIdentifier, TaskSubmission,
};

/// Submits a `make_dark.php` task with an exponential-backoff retry loop, so a
/// transient `429`/`503` from the IA catalog tasks API does not leak a test
/// item. Returns the last error if every attempt fails.
pub async fn dark_with_retries(
    client: &InternetArchiveClient,
    identifier: &ItemIdentifier,
    comment: &str,
    max_attempts: u32,
    initial_delay: Duration,
) -> Result<TaskSubmission, InternetArchiveError> {
    let mut delay = initial_delay;
    let mut last_error: Option<InternetArchiveError> = None;
    for _ in 0..max_attempts {
        match client.make_dark(identifier, comment).await {
            Ok(submission) => return Ok(submission),
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        InternetArchiveError::InvalidState(
            "cleanup exited retry loop without an attempt".to_owned(),
        )
    }))
}
