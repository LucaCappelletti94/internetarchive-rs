//! Typed API response models.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::metadata::ItemMetadata;
use crate::serde_util::deserialize_option_u64ish;
use crate::{ItemIdentifier, TaskId};

/// Full response returned by `GET /metadata/{identifier}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Item creation timestamp, when present.
    #[serde(default, deserialize_with = "deserialize_option_u64ish")]
    pub created: Option<u64>,
    /// Primary data node hostname.
    #[serde(default)]
    pub d1: Option<String>,
    /// Secondary data node hostname.
    #[serde(default)]
    pub d2: Option<String>,
    /// Item directory path inside IA storage.
    #[serde(default)]
    pub dir: Option<String>,
    /// Files contained in the item.
    #[serde(default)]
    pub files: Vec<ItemFile>,
    /// Reported file count.
    #[serde(default, deserialize_with = "deserialize_option_u64ish")]
    pub files_count: Option<u64>,
    /// Last updated timestamp.
    #[serde(default, deserialize_with = "deserialize_option_u64ish")]
    pub item_last_updated: Option<u64>,
    /// Reported item size in bytes.
    #[serde(default, deserialize_with = "deserialize_option_u64ish")]
    pub item_size: Option<u64>,
    /// Flexible metadata map.
    #[serde(default)]
    pub metadata: ItemMetadata,
    /// Host currently serving the metadata read.
    #[serde(default)]
    pub server: Option<String>,
    /// Unique record hash or sequence number.
    #[serde(default, deserialize_with = "deserialize_option_u64ish")]
    pub uniq: Option<u64>,
    /// Alternate workable hosts.
    #[serde(default)]
    pub workable_servers: Vec<String>,
    /// Any unmodeled top-level fields.
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Item {
    /// Returns the validated item identifier from metadata.
    #[must_use]
    pub fn identifier(&self) -> Option<ItemIdentifier> {
        self.metadata
            .get_text("identifier")
            .and_then(|value| ItemIdentifier::new(value).ok())
    }

    /// Finds a file by exact name.
    #[must_use]
    pub fn file(&self, name: &str) -> Option<&ItemFile> {
        self.files.iter().find(|file| file.name == name)
    }
}

/// File entry returned by metadata reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemFile {
    /// File name relative to the item root.
    pub name: String,
    /// Origin of the file, such as `original` or `derivative`.
    #[serde(default)]
    pub source: Option<String>,
    /// IA format label.
    #[serde(default)]
    pub format: Option<String>,
    /// Last modified timestamp.
    #[serde(default, deserialize_with = "deserialize_option_u64ish")]
    pub mtime: Option<u64>,
    /// Size in bytes.
    #[serde(default, deserialize_with = "deserialize_option_u64ish")]
    pub size: Option<u64>,
    /// MD5 hash when available.
    #[serde(default)]
    pub md5: Option<String>,
    /// CRC32 hash when available.
    #[serde(default)]
    pub crc32: Option<String>,
    /// SHA1 hash when available.
    #[serde(default)]
    pub sha1: Option<String>,
    /// Original file name for derivative files.
    #[serde(default)]
    pub original: Option<String>,
    /// Any additional file metadata.
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Response returned by MDAPI metadata writes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetadataWriteResponse {
    /// Whether the request was accepted.
    pub success: bool,
    /// Queued task identifier.
    #[serde(default)]
    pub task_id: Option<TaskId>,
    /// Log URL for the queued task.
    #[serde(default)]
    pub log: Option<Url>,
    /// Error message when `success` is false.
    #[serde(default)]
    pub error: Option<String>,
}

/// Echo block returned by `advancedsearch.php`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchResponseHeader {
    /// Status code reported by the search service.
    #[serde(default)]
    pub status: i64,
    /// Query time in milliseconds, when present.
    #[serde(default)]
    #[serde(rename = "QTime")]
    pub q_time: Option<i64>,
    /// Echoed request parameters.
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

/// Document list returned by search.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResultPage {
    /// Number of matching documents.
    #[serde(rename = "numFound")]
    pub num_found: u64,
    /// Start offset of this page.
    pub start: u64,
    /// Returned documents.
    #[serde(default)]
    pub docs: Vec<SearchDocument>,
}

/// Search response wrapper.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    /// Header and echoed parameters.
    #[serde(default)]
    #[serde(rename = "responseHeader")]
    pub response_header: SearchResponseHeader,
    /// Main result page.
    pub response: SearchResultPage,
}

/// Flexible document returned by advanced search.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SearchDocument(BTreeMap<String, Value>);

impl SearchDocument {
    /// Returns the raw field value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// Returns a string field.
    #[must_use]
    pub fn get_text(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// Returns the validated item identifier from a search document.
    #[must_use]
    pub fn identifier(&self) -> Option<ItemIdentifier> {
        self.get_text("identifier")
            .and_then(|value| ItemIdentifier::new(value).ok())
    }

    /// Returns the title field when present.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.get_text("title")
    }

    /// Returns the raw field map.
    #[must_use]
    pub fn as_map(&self) -> &BTreeMap<String, Value> {
        &self.0
    }
}

/// Response returned by the S3 limit-check endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct S3LimitCheck {
    /// Bucket name echoed by the service.
    pub bucket: String,
    /// Access key echoed by the service.
    pub accesskey: String,
    /// Whether the queue is over limit.
    pub over_limit: i64,
    /// Backend-specific detail string.
    #[serde(default)]
    pub detail: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::{Item, SearchResponse};

    #[test]
    fn item_deserializes_realistic_metadata_payloads() {
        let item: Item = serde_json::from_value(serde_json::json!({
            "created": 1_776_513_537,
            "files": [
                {
                    "name": "xfetch.pdf",
                    "size": 419_170,
                    "md5": "abc"
                }
            ],
            "metadata": {
                "identifier": "xfetch",
                "title": "XFETCH"
            }
        }))
        .unwrap();

        assert_eq!(item.file("xfetch.pdf").unwrap().size, Some(419_170));
        assert_eq!(item.identifier().unwrap().as_str(), "xfetch");
    }

    #[test]
    fn search_response_deserializes_advancedsearch_shape() {
        let response: SearchResponse = serde_json::from_value(serde_json::json!({
            "responseHeader": {
                "status": 0,
                "QTime": 12,
                "params": { "query": "identifier:xfetch" }
            },
            "response": {
                "numFound": 1,
                "start": 0,
                "docs": [
                    {
                        "identifier": "xfetch",
                        "title": "XFETCH"
                    }
                ]
            }
        }))
        .unwrap();

        assert_eq!(
            response.response.docs[0].identifier().unwrap().as_str(),
            "xfetch"
        );
        assert_eq!(response.response.docs[0].title(), Some("XFETCH"));
        assert_eq!(
            response.response.docs[0].as_map()["title"],
            serde_json::Value::String("XFETCH".to_owned())
        );
    }

    #[test]
    fn search_response_deserializes_without_response_header() {
        let response: SearchResponse = serde_json::from_value(serde_json::json!({
            "response": {
                "numFound": 1,
                "start": 0,
                "docs": [
                    {
                        "identifier": "xfetch",
                        "title": "XFETCH"
                    }
                ]
            }
        }))
        .unwrap();

        assert_eq!(response.response_header.status, 0);
        assert!(response.response_header.params.is_empty());
        assert_eq!(
            response.response.docs[0].identifier().unwrap().as_str(),
            "xfetch"
        );
    }
}
