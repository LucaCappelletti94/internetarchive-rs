//! Identifier newtypes used by the public API.

use std::fmt;
use std::str::FromStr;

use secrecy::SecretString;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

/// Item identifier used by Internet Archive.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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
    /// Identifier exceeds Internet Archive's documented maximum length.
    #[error("item identifier {identifier:?} is too long; maximum length is {max}")]
    TooLong {
        /// Identifier value that failed validation.
        identifier: String,
        /// Maximum general identifier length.
        max: usize,
    },
    /// Identifier starts with an unsupported character.
    #[error(
        "invalid first character {character:?} in identifier {identifier:?}; identifiers must start with an ASCII letter or digit"
    )]
    InvalidStartCharacter {
        /// Identifier value that failed validation.
        identifier: String,
        /// Unsupported first character.
        character: char,
    },
    /// Identifier is too short for Internet Archive's S3 bucket-creation layer.
    #[error(
        "item identifier {identifier:?} is too short for bucket creation; minimum length is {min}"
    )]
    TooShortForBucketCreation {
        /// Identifier value that failed validation.
        identifier: String,
        /// Minimum bucket-creation-safe length.
        min: usize,
    },
    /// Identifier is too long for Internet Archive's S3 bucket-creation layer.
    #[error(
        "item identifier {identifier:?} is too long for bucket creation; maximum length is {max}"
    )]
    TooLongForBucketCreation {
        /// Identifier value that failed validation.
        identifier: String,
        /// Maximum bucket-creation-safe length.
        max: usize,
    },
    /// Identifier contains an unsupported character.
    #[error("invalid character {character:?} in identifier {identifier:?}")]
    InvalidCharacter {
        /// Original identifier value after trimming.
        identifier: String,
        /// Unsupported character.
        character: char,
    },
    /// Identifier contains a character that the conservative IA-S3 bucket
    /// creation subset rejects.
    #[error(
        "invalid bucket-creation character {character:?} in identifier {identifier:?}; bucket-creation identifiers may contain only lowercase ASCII letters, digits, periods, and dashes"
    )]
    InvalidBucketCreationCharacter {
        /// Identifier value that failed validation.
        identifier: String,
        /// Unsupported bucket-creation character.
        character: char,
    },
    /// Identifier starts or ends with a character that the conservative IA-S3
    /// bucket-creation subset rejects.
    #[error(
        "invalid bucket-creation edge character {character:?} in identifier {identifier:?}; bucket-creation identifiers must start and end with a lowercase ASCII letter or digit"
    )]
    InvalidBucketCreationEdgeCharacter {
        /// Identifier value that failed validation.
        identifier: String,
        /// Unsupported first or last bucket-creation character.
        character: char,
    },
    /// Identifier contains adjacent periods that S3 bucket creation rejects.
    #[error(
        "item identifier {identifier:?} is invalid for bucket creation; S3 bucket names cannot contain adjacent periods"
    )]
    AdjacentBucketCreationPeriods {
        /// Identifier value that failed validation.
        identifier: String,
    },
    /// Identifier has the shape of an IPv4 address, which S3 bucket creation rejects.
    #[error(
        "item identifier {identifier:?} is invalid for bucket creation; S3 bucket names cannot be formatted as an IPv4 address"
    )]
    BucketCreationIdentifierLooksLikeIpAddress {
        /// Identifier value that failed validation.
        identifier: String,
    },
    /// Identifier contains a period adjacent to a dash, which S3-compatible bucket creation rejects.
    #[error(
        "item identifier {identifier:?} is invalid for bucket creation; S3 bucket names cannot contain periods adjacent to dashes"
    )]
    PeriodAdjacentBucketCreationDash {
        /// Identifier value that failed validation.
        identifier: String,
    },
}

impl ItemIdentifier {
    /// Longest item identifier documented by Internet Archive.
    pub const MAX_IDENTIFIER_LEN: usize = 100;
    /// Shortest identifier accepted by the conservative IA-S3 bucket-creation subset.
    pub const MIN_BUCKET_IDENTIFIER_LEN: usize = 3;
    /// Longest identifier accepted by the conservative IA-S3 bucket-creation subset.
    pub const MAX_BUCKET_IDENTIFIER_LEN: usize = 63;

    /// Creates a validated item identifier.
    ///
    /// # Errors
    ///
    /// Returns an error if the identifier is empty, longer than the documented
    /// maximum, does not start with an ASCII letter or digit, or contains
    /// characters outside of `[A-Za-z0-9_.-]`.
    pub fn new(value: impl AsRef<str>) -> Result<Self, IdentifierError> {
        let trimmed = value.as_ref().trim();
        if trimmed.is_empty() {
            return Err(IdentifierError::Empty);
        }

        if trimmed.len() > Self::MAX_IDENTIFIER_LEN {
            return Err(IdentifierError::TooLong {
                identifier: trimmed.to_owned(),
                max: Self::MAX_IDENTIFIER_LEN,
            });
        }

        let Some(first) = trimmed.chars().next() else {
            return Err(IdentifierError::Empty);
        };
        if !first.is_ascii_alphanumeric() {
            return Err(IdentifierError::InvalidStartCharacter {
                identifier: trimmed.to_owned(),
                character: first,
            });
        }

        if let Some(character) = trimmed.chars().find(|character| {
            !character.is_ascii_alphanumeric()
                && *character != '_'
                && *character != '-'
                && *character != '.'
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

    /// Validates that this identifier can safely create an IA-S3 bucket.
    ///
    /// Internet Archive's general item identifiers can include uppercase
    /// letters, underscores, and periods, but its upload path maps item
    /// identifiers to S3 bucket names when creating new items. Bucket creation
    /// therefore uses a conservative DNS-compatible subset.
    ///
    /// This is intentionally narrower than Internet Archive's general
    /// identifier rules and the Python client's optional S3 identifier
    /// validator. Use it only before requests that ask IA-S3 to create a
    /// bucket, not before existing-item upload, delete, or queue-limit checks.
    ///
    /// # Errors
    ///
    /// Returns an error if the identifier is outside the bucket-creation-safe
    /// length range, contains a bucket-unsafe character, starts or ends with a
    /// character rejected by IA-S3 bucket creation, contains adjacent periods,
    /// contains a period next to a dash, or looks like an IPv4 address.
    pub fn validate_for_bucket_creation(&self) -> Result<(), IdentifierError> {
        let identifier = self.as_str();
        let length = identifier.len();

        if length < Self::MIN_BUCKET_IDENTIFIER_LEN {
            return Err(IdentifierError::TooShortForBucketCreation {
                identifier: identifier.to_owned(),
                min: Self::MIN_BUCKET_IDENTIFIER_LEN,
            });
        }

        if length > Self::MAX_BUCKET_IDENTIFIER_LEN {
            return Err(IdentifierError::TooLongForBucketCreation {
                identifier: identifier.to_owned(),
                max: Self::MAX_BUCKET_IDENTIFIER_LEN,
            });
        }

        if let Some(character) = identifier
            .chars()
            .find(|character| !is_bucket_creation_safe_character(*character))
        {
            return Err(IdentifierError::InvalidBucketCreationCharacter {
                identifier: identifier.to_owned(),
                character,
            });
        }

        for character in [identifier.chars().next(), identifier.chars().next_back()]
            .into_iter()
            .flatten()
        {
            if !is_bucket_creation_safe_edge_character(character) {
                return Err(IdentifierError::InvalidBucketCreationEdgeCharacter {
                    identifier: identifier.to_owned(),
                    character,
                });
            }
        }

        if identifier.contains("..") {
            return Err(IdentifierError::AdjacentBucketCreationPeriods {
                identifier: identifier.to_owned(),
            });
        }

        if looks_like_ipv4_address(identifier) {
            return Err(
                IdentifierError::BucketCreationIdentifierLooksLikeIpAddress {
                    identifier: identifier.to_owned(),
                },
            );
        }

        if identifier.contains("-.") || identifier.contains(".-") {
            return Err(IdentifierError::PeriodAdjacentBucketCreationDash {
                identifier: identifier.to_owned(),
            });
        }

        Ok(())
    }
}

fn is_bucket_creation_safe_character(character: char) -> bool {
    character.is_ascii_lowercase()
        || character.is_ascii_digit()
        || character == '-'
        || character == '.'
}

fn is_bucket_creation_safe_edge_character(character: char) -> bool {
    character.is_ascii_lowercase() || character.is_ascii_digit()
}

fn looks_like_ipv4_address(identifier: &str) -> bool {
    let mut parts = identifier.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(second) = parts.next() else {
        return false;
    };
    let Some(third) = parts.next() else {
        return false;
    };
    let Some(fourth) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }

    [first, second, third, fourth]
        .into_iter()
        .all(|part| part.parse::<u8>().is_ok())
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

impl<'de> Deserialize<'de> for ItemIdentifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
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
        assert_eq!(
            ItemIdentifier::new("Demo_Item").unwrap().as_str(),
            "Demo_Item"
        );
        assert_eq!(
            ItemIdentifier::new("Demo.Item_2026").unwrap().as_str(),
            "Demo.Item_2026"
        );
    }

    #[test]
    fn item_identifier_rejects_empty_and_invalid_values() {
        assert_eq!(
            ItemIdentifier::new("   ").unwrap_err(),
            IdentifierError::Empty
        );
        let long_identifier = "a".repeat(ItemIdentifier::MAX_IDENTIFIER_LEN + 1);
        assert_eq!(
            ItemIdentifier::new(&long_identifier).unwrap_err(),
            IdentifierError::TooLong {
                identifier: long_identifier,
                max: ItemIdentifier::MAX_IDENTIFIER_LEN,
            }
        );
        assert_eq!(
            ItemIdentifier::new("-bad").unwrap_err(),
            IdentifierError::InvalidStartCharacter {
                identifier: String::from("-bad"),
                character: '-',
            }
        );
        assert_eq!(
            ItemIdentifier::new("_bad").unwrap_err(),
            IdentifierError::InvalidStartCharacter {
                identifier: String::from("_bad"),
                character: '_',
            }
        );
        assert_eq!(
            ItemIdentifier::new(".bad").unwrap_err(),
            IdentifierError::InvalidStartCharacter {
                identifier: String::from(".bad"),
                character: '.',
            }
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
    fn item_identifier_validates_bucket_creation_safe_subset() {
        ItemIdentifier::new("demo-item.2026")
            .unwrap()
            .validate_for_bucket_creation()
            .unwrap();

        assert_eq!(
            ItemIdentifier::new("ab")
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap_err(),
            IdentifierError::TooShortForBucketCreation {
                identifier: String::from("ab"),
                min: ItemIdentifier::MIN_BUCKET_IDENTIFIER_LEN,
            }
        );

        let long_identifier = "a".repeat(ItemIdentifier::MAX_BUCKET_IDENTIFIER_LEN + 1);
        assert_eq!(
            ItemIdentifier::new(&long_identifier)
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap_err(),
            IdentifierError::TooLongForBucketCreation {
                identifier: long_identifier,
                max: ItemIdentifier::MAX_BUCKET_IDENTIFIER_LEN,
            }
        );

        assert_eq!(
            ItemIdentifier::new("Demo-item")
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap_err(),
            IdentifierError::InvalidBucketCreationCharacter {
                identifier: String::from("Demo-item"),
                character: 'D',
            }
        );
        assert_eq!(
            ItemIdentifier::new("demo_item")
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap_err(),
            IdentifierError::InvalidBucketCreationCharacter {
                identifier: String::from("demo_item"),
                character: '_',
            }
        );
        assert_eq!(
            ItemIdentifier::new("demo-")
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap_err(),
            IdentifierError::InvalidBucketCreationEdgeCharacter {
                identifier: String::from("demo-"),
                character: '-',
            }
        );
        assert_eq!(
            ItemIdentifier::new("demo.")
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap_err(),
            IdentifierError::InvalidBucketCreationEdgeCharacter {
                identifier: String::from("demo."),
                character: '.',
            }
        );
        assert_eq!(
            ItemIdentifier::new("demo..item")
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap_err(),
            IdentifierError::AdjacentBucketCreationPeriods {
                identifier: String::from("demo..item"),
            }
        );
        assert_eq!(
            ItemIdentifier::new("192.168.5.4")
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap_err(),
            IdentifierError::BucketCreationIdentifierLooksLikeIpAddress {
                identifier: String::from("192.168.5.4"),
            }
        );
        assert_eq!(
            ItemIdentifier::new("demo-.item")
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap_err(),
            IdentifierError::PeriodAdjacentBucketCreationDash {
                identifier: String::from("demo-.item"),
            }
        );
        assert_eq!(
            ItemIdentifier::new("demo.-item")
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap_err(),
            IdentifierError::PeriodAdjacentBucketCreationDash {
                identifier: String::from("demo.-item"),
            }
        );
        for identifier in [
            "xn--demo",
            "sthree-demo",
            "amzn-s3-demo-item",
            "demo-s3alias",
            "demo--ol-s3",
            "demo.mrap",
            "demo--x-s3",
            "demo--table-s3",
        ] {
            ItemIdentifier::new(identifier)
                .unwrap()
                .validate_for_bucket_creation()
                .unwrap();
        }
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
    fn identifier_serde_round_trip_validates_values() {
        let identifier: ItemIdentifier = serde_json::from_str("\"Demo.Item_2026\"").unwrap();
        assert_eq!(identifier.as_str(), "Demo.Item_2026");
        assert_eq!(
            serde_json::to_string(&identifier).unwrap(),
            "\"Demo.Item_2026\""
        );
        assert!(serde_json::from_str::<ItemIdentifier>("\"bad item\"").is_err());
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
