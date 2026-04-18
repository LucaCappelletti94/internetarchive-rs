//! Polling configuration used by high-level workflow helpers.

use std::time::Duration;

/// Exponential-backoff settings for polling Internet Archive state changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollOptions {
    /// Maximum total time spent polling.
    pub max_wait: Duration,
    /// First delay between attempts.
    pub initial_delay: Duration,
    /// Maximum delay between attempts.
    pub max_delay: Duration,
}

impl Default for PollOptions {
    fn default() -> Self {
        Self {
            max_wait: Duration::from_secs(30),
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(3),
        }
    }
}
