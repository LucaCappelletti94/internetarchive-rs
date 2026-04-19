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
            max_wait: Duration::from_secs(120),
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(5),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PollOptions;
    use std::time::Duration;

    #[test]
    fn defaults_allow_for_archive_eventual_consistency() {
        let poll = PollOptions::default();
        assert_eq!(poll.max_wait, Duration::from_secs(120));
        assert_eq!(poll.initial_delay, Duration::from_millis(500));
        assert_eq!(poll.max_delay, Duration::from_secs(5));
    }
}
