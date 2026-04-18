//! Flexible metadata wrappers and JSON Patch helpers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::serde_util::normalize_string_list;

/// Common Internet Archive media types.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MediaType {
    /// Texts and documents.
    Texts,
    /// Movies and videos.
    Movies,
    /// Audio recordings.
    Audio,
    /// Images.
    Image,
    /// Software and executables.
    Software,
    /// Datasets and miscellaneous files.
    Data,
    /// Collections of items.
    Collection,
    /// A custom mediatype string.
    Custom(String),
}

impl MediaType {
    #[must_use]
    fn as_str(&self) -> &str {
        match self {
            Self::Texts => "texts",
            Self::Movies => "movies",
            Self::Audio => "audio",
            Self::Image => "image",
            Self::Software => "software",
            Self::Data => "data",
            Self::Collection => "collection",
            Self::Custom(value) => value.as_str(),
        }
    }
}

impl Serialize for MediaType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MediaType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "texts" => Self::Texts,
            "movies" => Self::Movies,
            "audio" => Self::Audio,
            "image" => Self::Image,
            "software" => Self::Software,
            "data" => Self::Data,
            "collection" => Self::Collection,
            other => Self::Custom(other.to_owned()),
        })
    }
}

/// Flexible metadata value wrapper.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetadataValue {
    /// Single string value.
    Text(String),
    /// Multiple string values.
    TextList(Vec<String>),
    /// Raw JSON value for less common metadata shapes.
    Json(Value),
}

impl From<String> for MetadataValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for MetadataValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<String>> for MetadataValue {
    fn from(value: Vec<String>) -> Self {
        Self::TextList(value)
    }
}

impl From<Vec<&str>> for MetadataValue {
    fn from(value: Vec<&str>) -> Self {
        Self::TextList(value.into_iter().map(str::to_owned).collect())
    }
}

/// Flexible item metadata map.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemMetadata(BTreeMap<String, Value>);

impl ItemMetadata {
    /// Starts building metadata with convenient typed helpers.
    #[must_use]
    pub fn builder() -> ItemMetadataBuilder {
        ItemMetadataBuilder::default()
    }

    /// Returns the raw JSON value for a metadata key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// Returns a common metadata field as text.
    #[must_use]
    pub fn get_text(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// Returns a metadata field as one or many text values.
    #[must_use]
    pub fn get_texts(&self, key: &str) -> Option<Vec<String>> {
        self.get(key).and_then(normalize_string_list)
    }

    /// Returns the configured title, when present.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.get_text("title")
    }

    /// Returns the configured mediatype, when present.
    #[must_use]
    pub fn mediatype(&self) -> Option<MediaType> {
        self.get("mediatype")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
    }

    /// Returns the configured collections, when present.
    #[must_use]
    pub fn collections(&self) -> Option<Vec<String>> {
        self.get_texts("collection")
    }

    /// Returns the raw map view.
    #[must_use]
    pub fn as_map(&self) -> &BTreeMap<String, Value> {
        &self.0
    }

    /// Converts metadata into the raw JSON map.
    #[must_use]
    pub fn into_map(self) -> BTreeMap<String, Value> {
        self.0
    }

    pub(crate) fn as_header_encoding(&self) -> HeaderEncoding {
        let mut headers = Vec::new();
        let mut remainder = BTreeMap::new();

        for (key, value) in &self.0 {
            match value {
                Value::String(text) => headers.push((header_name(key, None), header_value(text))),
                Value::Array(values) => {
                    let strings = values.iter().map(Value::as_str).collect::<Option<Vec<_>>>();
                    if let Some(strings) = strings {
                        if strings.len() <= 1 {
                            if let Some(value) = strings.first() {
                                headers.push((header_name(key, None), header_value(value)));
                            }
                        } else {
                            for (index, value) in strings.into_iter().enumerate() {
                                headers
                                    .push((header_name(key, Some(index + 1)), header_value(value)));
                            }
                        }
                    } else {
                        remainder.insert(key.clone(), value.clone());
                    }
                }
                _ => {
                    remainder.insert(key.clone(), value.clone());
                }
            }
        }

        HeaderEncoding {
            headers,
            remainder: Self(remainder),
        }
    }
}

impl From<BTreeMap<String, Value>> for ItemMetadata {
    fn from(value: BTreeMap<String, Value>) -> Self {
        Self(value)
    }
}

impl From<Map<String, Value>> for ItemMetadata {
    fn from(value: Map<String, Value>) -> Self {
        Self(value.into_iter().collect())
    }
}

/// Helper produced when turning metadata into S3 creation headers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HeaderEncoding {
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) remainder: ItemMetadata,
}

/// Builder for [`ItemMetadata`].
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ItemMetadataBuilder {
    inner: BTreeMap<String, Value>,
}

impl ItemMetadataBuilder {
    /// Sets the item mediatype.
    #[must_use]
    pub fn mediatype(mut self, mediatype: MediaType) -> Self {
        let mediatype = match mediatype {
            MediaType::Texts => "texts".to_owned(),
            MediaType::Movies => "movies".to_owned(),
            MediaType::Audio => "audio".to_owned(),
            MediaType::Image => "image".to_owned(),
            MediaType::Software => "software".to_owned(),
            MediaType::Data => "data".to_owned(),
            MediaType::Collection => "collection".to_owned(),
            MediaType::Custom(value) => value,
        };
        self.inner
            .insert("mediatype".to_owned(), Value::String(mediatype));
        self
    }

    /// Sets the item title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.inner
            .insert("title".to_owned(), Value::String(title.into()));
        self
    }

    /// Sets the item description.
    #[must_use]
    pub fn description_html(mut self, description: impl Into<String>) -> Self {
        self.inner
            .insert("description".to_owned(), Value::String(description.into()));
        self
    }

    /// Appends a collection membership.
    #[must_use]
    pub fn collection(mut self, collection: impl Into<String>) -> Self {
        append_text_value(&mut self.inner, "collection", collection.into());
        self
    }

    /// Appends a creator value.
    #[must_use]
    pub fn creator(mut self, creator: impl Into<String>) -> Self {
        append_text_value(&mut self.inner, "creator", creator.into());
        self
    }

    /// Appends a subject value.
    #[must_use]
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        append_text_value(&mut self.inner, "subject", subject.into());
        self
    }

    /// Appends a language value.
    #[must_use]
    pub fn language(mut self, language: impl Into<String>) -> Self {
        append_text_value(&mut self.inner, "language", language.into());
        self
    }

    /// Sets the metadata license URL.
    #[must_use]
    pub fn license_url(mut self, license_url: impl Into<String>) -> Self {
        self.inner
            .insert("licenseurl".to_owned(), Value::String(license_url.into()));
        self
    }

    /// Sets any extra metadata key to a string value.
    #[must_use]
    pub fn extra_text(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.inner.insert(key.into(), Value::String(value.into()));
        self
    }

    /// Sets any extra metadata key to a multi-value string list.
    #[must_use]
    pub fn extra_texts(
        mut self,
        key: impl Into<String>,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.inner.insert(
            key.into(),
            Value::Array(
                values
                    .into_iter()
                    .map(|value| Value::String(value.into()))
                    .collect(),
            ),
        );
        self
    }

    /// Sets any extra metadata key to an arbitrary JSON-compatible value.
    #[must_use]
    pub fn extra_json(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.inner.insert(key.into(), value.into());
        self
    }

    /// Builds the metadata value.
    #[must_use]
    pub fn build(self) -> ItemMetadata {
        ItemMetadata(self.inner)
    }
}

/// Metadata write target for the MDAPI write endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetadataTarget {
    /// Update the item-level `metadata` object.
    Metadata,
    /// Update metadata for a specific file entry.
    File(String),
    /// Update a named user JSON document.
    UserJson(String),
    /// Update the unnamed root user JSON document.
    RootUserJson,
}

impl MetadataTarget {
    #[must_use]
    pub(crate) fn as_str(&self) -> String {
        match self {
            Self::Metadata => "metadata".to_owned(),
            Self::File(name) => format!("files/{name}"),
            Self::UserJson(name) => name.clone(),
            Self::RootUserJson => String::new(),
        }
    }
}

/// One patch operation accepted by MDAPI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum PatchOperation {
    /// Adds a new value at a path.
    Add {
        /// JSON Pointer path to modify.
        path: String,
        /// JSON value to insert.
        value: Value,
    },
    /// Removes a value at a path.
    Remove {
        /// JSON Pointer path to modify.
        path: String,
    },
    /// Replaces a value at a path.
    Replace {
        /// JSON Pointer path to modify.
        path: String,
        /// Replacement JSON value.
        value: Value,
    },
    /// Asserts the current value at a path.
    Test {
        /// JSON Pointer path to compare.
        path: String,
        /// Expected JSON value.
        value: Value,
    },
    /// Internet Archive extension: removes the first matching value.
    #[serde(rename = "remove-first")]
    RemoveFirst {
        /// JSON Pointer path to the target array, ending in `/-`.
        path: String,
        /// JSON value to match and remove.
        value: Value,
    },
    /// Internet Archive extension: removes all matching values.
    #[serde(rename = "remove-all")]
    RemoveAll {
        /// JSON Pointer path to the target array or object, ending in `/-`.
        path: String,
        /// JSON value to match and remove.
        value: Value,
    },
}

impl PatchOperation {
    /// Creates a `test` operation from a JSON-compatible value.
    #[must_use]
    pub fn test(path: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Test {
            path: path.into(),
            value: value.into(),
        }
    }

    /// Creates a `replace` operation from a JSON-compatible value.
    #[must_use]
    pub fn replace(path: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Replace {
            path: path.into(),
            value: value.into(),
        }
    }

    /// Creates an `add` operation from a JSON-compatible value.
    #[must_use]
    pub fn add(path: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Add {
            path: path.into(),
            value: value.into(),
        }
    }
}

/// Multi-target metadata write entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetadataChange {
    /// The target being updated.
    pub target: String,
    /// The patch document to apply.
    pub patch: Vec<PatchOperation>,
}

impl MetadataChange {
    /// Creates a new metadata change.
    #[must_use]
    pub fn new(target: &MetadataTarget, patch: Vec<PatchOperation>) -> Self {
        Self {
            target: target.as_str(),
            patch,
        }
    }
}

fn append_text_value(map: &mut BTreeMap<String, Value>, key: &str, value: String) {
    match map.get_mut(key) {
        Some(Value::Array(values)) => values.push(Value::String(value)),
        Some(existing) => {
            let previous = existing.take();
            *existing = Value::Array(vec![previous, Value::String(value)]);
        }
        None => {
            map.insert(key.to_owned(), Value::String(value));
        }
    }
}

fn header_name(key: &str, position: Option<usize>) -> String {
    let normalized = key.replace('_', "--");
    if let Some(position) = position {
        format!("x-archive-meta{position:02}-{normalized}")
    } else {
        format!("x-archive-meta-{normalized}")
    }
}

fn header_value(value: &str) -> String {
    if value.is_ascii() {
        value.to_owned()
    } else {
        let encoded =
            percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC);
        format!("uri({encoded})")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Map, Value};

    use super::{
        ItemMetadata, MediaType, MetadataChange, MetadataTarget, MetadataValue, PatchOperation,
    };

    #[test]
    fn builder_handles_common_fields_and_lists() {
        let metadata = ItemMetadata::builder()
            .mediatype(MediaType::Texts)
            .title("Demo")
            .collection("opensource")
            .collection("community")
            .creator("Doe, Jane")
            .language("eng")
            .build();

        assert_eq!(metadata.title(), Some("Demo"));
        assert_eq!(metadata.mediatype(), Some(MediaType::Texts));
        assert_eq!(
            metadata.collections().unwrap(),
            vec!["opensource".to_owned(), "community".to_owned()]
        );
    }

    #[test]
    fn media_types_and_metadata_value_conversions_cover_all_variants() {
        let variants = [
            (MediaType::Texts, "texts"),
            (MediaType::Movies, "movies"),
            (MediaType::Audio, "audio"),
            (MediaType::Image, "image"),
            (MediaType::Software, "software"),
            (MediaType::Data, "data"),
            (MediaType::Collection, "collection"),
        ];

        for (variant, expected) in variants {
            assert_eq!(
                serde_json::to_value(&variant).unwrap(),
                Value::String(expected.to_owned())
            );
            assert_eq!(
                serde_json::from_value::<MediaType>(Value::String(expected.to_owned())).unwrap(),
                variant
            );
        }
        assert_eq!(
            serde_json::from_value::<MediaType>(Value::String("custom".to_owned())).unwrap(),
            MediaType::Custom("custom".to_owned())
        );

        for variant in [
            MediaType::Movies,
            MediaType::Audio,
            MediaType::Image,
            MediaType::Software,
            MediaType::Data,
            MediaType::Collection,
        ] {
            let metadata = ItemMetadata::builder().mediatype(variant.clone()).build();
            assert_eq!(metadata.mediatype(), Some(variant));
        }

        assert_eq!(
            MetadataValue::from(String::from("demo")),
            MetadataValue::Text(String::from("demo"))
        );
        assert_eq!(
            MetadataValue::from("demo"),
            MetadataValue::Text(String::from("demo"))
        );
        assert_eq!(
            MetadataValue::from(vec![String::from("a"), String::from("b")]),
            MetadataValue::TextList(vec![String::from("a"), String::from("b")])
        );
        assert_eq!(
            MetadataValue::from(vec!["a", "b"]),
            MetadataValue::TextList(vec![String::from("a"), String::from("b")])
        );
    }

    #[test]
    fn builder_and_accessors_cover_all_common_metadata_helpers() {
        let metadata = ItemMetadata::builder()
            .mediatype(MediaType::Custom("zines".to_owned()))
            .title("Demo")
            .description_html("<p>Description</p>")
            .collection("opensource")
            .creator("Jane Doe")
            .subject("rust")
            .language("eng")
            .license_url("https://creativecommons.org/licenses/by/4.0/")
            .extra_text("identifier", "demo-item")
            .extra_texts("collection", ["opensource", "community"])
            .extra_json("custom", json!({"nested": true}))
            .build();

        assert_eq!(metadata.get("custom").unwrap(), &json!({"nested": true}));
        assert_eq!(metadata.get_text("title"), Some("Demo"));
        assert_eq!(
            metadata.get_texts("collection").unwrap(),
            vec!["opensource".to_owned(), "community".to_owned()]
        );
        assert_eq!(metadata.title(), Some("Demo"));
        assert_eq!(
            metadata.mediatype(),
            Some(MediaType::Custom("zines".to_owned()))
        );
        assert_eq!(
            metadata.collections().unwrap(),
            vec!["opensource".to_owned(), "community".to_owned()]
        );
        assert!(metadata.as_map().contains_key("licenseurl"));

        let raw = metadata.clone().into_map();
        assert_eq!(raw["identifier"], Value::String("demo-item".to_owned()));
    }

    #[test]
    fn header_encoding_supports_ascii_lists_and_leaves_complex_values_for_patching() {
        let metadata = ItemMetadata::builder()
            .mediatype(MediaType::Texts)
            .title("Demo")
            .collection("opensource")
            .collection("community")
            .extra_json("custom", serde_json::json!({"nested": true}))
            .build();

        let encoding = metadata.as_header_encoding();
        assert_eq!(encoding.headers.len(), 4);
        assert_eq!(
            encoding.remainder.get("custom").unwrap(),
            &serde_json::json!({"nested": true})
        );
    }

    #[test]
    fn header_encoding_uri_encodes_unicode_values() {
        let metadata = ItemMetadata::builder().title("Snowman ☃").build();
        let encoding = metadata.as_header_encoding();
        assert!(encoding.headers[0].1.starts_with("uri("));
    }

    #[test]
    fn header_encoding_handles_single_value_arrays_and_map_conversions() {
        let mut map = Map::new();
        map.insert("single".to_owned(), json!(["only"]));
        map.insert("mixed".to_owned(), json!([1, 2, 3]));

        let metadata = ItemMetadata::from(map);
        let encoding = metadata.as_header_encoding();

        assert!(encoding
            .headers
            .iter()
            .any(|(name, value)| name == "x-archive-meta-single" && value == "only"));
        assert_eq!(encoding.remainder.get("mixed").unwrap(), &json!([1, 2, 3]));
    }

    #[test]
    fn metadata_targets_and_patch_helpers_serialize_as_expected() {
        let change = MetadataChange::new(
            &MetadataTarget::Metadata,
            vec![
                PatchOperation::test("/version", 1),
                PatchOperation::replace("/title", "Updated"),
                PatchOperation::add("/subjects/-", "rust"),
                PatchOperation::Remove {
                    path: "/deprecated".to_owned(),
                },
                PatchOperation::RemoveFirst {
                    path: "/subjects/-".to_owned(),
                    value: Value::String("old".to_owned()),
                },
                PatchOperation::RemoveAll {
                    path: "/subjects/-".to_owned(),
                    value: Value::String("older".to_owned()),
                },
            ],
        );
        let json = serde_json::to_value(change).unwrap();
        assert_eq!(json["target"], "metadata");
        assert_eq!(json["patch"][0]["op"], "test");
        assert_eq!(
            MetadataTarget::File("demo.txt".into()).as_str(),
            "files/demo.txt"
        );
        assert_eq!(
            MetadataTarget::UserJson("extra.json".into()).as_str(),
            "extra.json"
        );
        assert_eq!(MetadataTarget::RootUserJson.as_str(), "");
    }
}
