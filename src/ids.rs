//! Identifier newtypes used by the public API.

use std::fmt;
use std::str::FromStr;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Item identifier used by Internet Archive.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemIdentifier(String);

/// Task identifier returned by Metadata Write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(
    /// Raw numeric task identifier.
    pub u64,
);

/// Validation errors for [`ItemIdentifier`].
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum IdentifierError {
    /// Identifier was empty after trimming.
    #[error("item identifier cannot be empty")]
    Empty,
    /// Identifier contains an unsupported character.
    #[error("invalid character {character:?} in identifier {identifier:?}")]
    InvalidCharacter {
        /// Original identifier value after trimming.
        identifier: String,
        /// Unsupported character.
        character: char,
    },
}

impl ItemIdentifier {
    /// Creates a validated item identifier.
    ///
    /// # Errors
    ///
    /// Returns an error if the identifier is empty or contains characters
    /// outside of `[A-Za-z0-9_-]`.
    pub fn new(value: impl AsRef<str>) -> Result<Self, IdentifierError> {
        let trimmed = value.as_ref().trim();
        if trimmed.is_empty() {
            return Err(IdentifierError::Empty);
        }

        if let Some(character) = trimmed.chars().find(|character| {
            !character.is_ascii_alphanumeric() && *character != '_' && *character != '-'
        }) {
            return Err(IdentifierError::InvalidCharacter {
                identifier: trimmed.to_owned(),
                character,
            });
        }

        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the raw identifier string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ItemIdentifier {
    type Err = IdentifierError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for ItemIdentifier {
    type Error = IdentifierError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for ItemIdentifier {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ItemIdentifier> for String {
    fn from(value: ItemIdentifier) -> Self {
        value.0
    }
}

impl From<u64> for TaskId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Pair of LOW-auth secrets used for authenticated Internet Archive requests.
#[derive(Clone)]
pub(crate) struct SecretPair {
    pub(crate) access_key: SecretString,
    pub(crate) secret_key: SecretString,
}

impl std::fmt::Debug for SecretPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretPair")
            .field("access_key", &"<redacted>")
            .field("secret_key", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::{IdentifierError, ItemIdentifier, SecretPair, TaskId};

    #[test]
    fn item_identifier_accepts_documented_shapes() {
        assert_eq!(
            ItemIdentifier::new("xfetch-2026_demo").unwrap().as_str(),
            "xfetch-2026_demo"
        );
        assert_eq!(
            "demo_item".parse::<ItemIdentifier>().unwrap().as_str(),
            "demo_item"
        );
    }

    #[test]
    fn item_identifier_rejects_empty_and_invalid_values() {
        assert_eq!(
            ItemIdentifier::new("   ").unwrap_err(),
            IdentifierError::Empty
        );
        assert!(matches!(
            ItemIdentifier::new("bad item").unwrap_err(),
            IdentifierError::InvalidCharacter { character: ' ', .. }
        ));
        assert!(matches!(
            ItemIdentifier::new("bad/item").unwrap_err(),
            IdentifierError::InvalidCharacter { character: '/', .. }
        ));
    }

    #[test]
    fn task_ids_round_trip() {
        let task = TaskId::from(42_u64);
        assert_eq!(task.0, 42);
        assert_eq!(task.to_string(), "42");
    }

    #[test]
    fn identifier_try_from_and_string_round_trip_work() {
        let identifier = ItemIdentifier::try_from(String::from("demo-item")).unwrap();
        assert_eq!(identifier.as_str(), "demo-item");
        assert_eq!(identifier.to_string(), "demo-item");
        assert_eq!(String::from(identifier.clone()), "demo-item");
        assert_eq!(ItemIdentifier::try_from("demo-item").unwrap(), identifier);
    }

    #[test]
    fn secret_pair_debug_is_redacted() {
        let secrets = SecretPair {
            access_key: SecretString::from(String::from("actual-access-secret")),
            secret_key: SecretString::from(String::from("actual-secret-key")),
        };

        let debug = format!("{secrets:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("actual-access-secret"));
        assert!(!debug.contains("actual-secret-key"));
    }
}
