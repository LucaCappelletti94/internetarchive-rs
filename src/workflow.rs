//! Higher-level item publication and update workflows.

use crate::client::InternetArchiveClient;
use crate::error::InternetArchiveError;
use crate::metadata::ItemMetadata;
use crate::model::Item;
use crate::upload::{FileConflictPolicy, UploadOptions, UploadSpec};
use crate::ItemIdentifier;

/// Request used by high-level publish and upsert helpers.
#[derive(Clone, Debug, PartialEq)]
pub struct PublishRequest {
    /// Item identifier to create or update.
    pub identifier: ItemIdentifier,
    /// Desired metadata document.
    pub metadata: ItemMetadata,
    /// Files to upload.
    pub uploads: Vec<UploadSpec>,
    /// Conflict policy for uploads targeting existing names.
    pub conflict_policy: FileConflictPolicy,
    /// Per-upload options.
    pub upload_options: UploadOptions,
}

impl PublishRequest {
    /// Creates a new publish request with default overwrite behavior.
    #[must_use]
    pub fn new(
        identifier: ItemIdentifier,
        metadata: ItemMetadata,
        uploads: Vec<UploadSpec>,
    ) -> Self {
        Self {
            identifier,
            metadata,
            uploads,
            conflict_policy: FileConflictPolicy::Overwrite,
            upload_options: UploadOptions::default(),
        }
    }
}

/// Result returned by high-level publish or upsert helpers.
#[derive(Clone, Debug, PartialEq)]
pub struct PublishOutcome {
    /// Final item state after the workflow.
    pub item: Item,
    /// Whether the item was created during this workflow.
    pub created: bool,
    /// File names uploaded during this workflow.
    pub uploaded_files: Vec<String>,
    /// File names skipped because of the selected policy.
    pub skipped_files: Vec<String>,
    /// Whether metadata was updated through MDAPI after the upload step.
    pub metadata_changed: bool,
}

impl InternetArchiveClient {
    /// Creates a brand-new item and uploads all requested files.
    ///
    /// # Errors
    ///
    /// Returns an error if the item already exists, the request has no files, or
    /// any network step fails.
    pub async fn publish_item(
        &self,
        request: PublishRequest,
    ) -> Result<PublishOutcome, InternetArchiveError> {
        match self.get_item(&request.identifier).await {
            Ok(_) => Err(InternetArchiveError::InvalidState(format!(
                "item {} already exists",
                request.identifier
            ))),
            Err(InternetArchiveError::ItemNotFound { .. }) => {
                self.create_or_update_item(request, None, true).await
            }
            Err(error) => Err(error),
        }
    }

    /// Creates or updates an item using the provided upload conflict policy.
    ///
    /// # Errors
    ///
    /// Returns an error if any required network step fails.
    pub async fn upsert_item(
        &self,
        request: PublishRequest,
    ) -> Result<PublishOutcome, InternetArchiveError> {
        let existing = match self.get_item(&request.identifier).await {
            Ok(item) => Some(item),
            Err(InternetArchiveError::ItemNotFound { .. }) => None,
            Err(error) => return Err(error),
        };
        self.create_or_update_item(request, existing, false).await
    }

    async fn create_or_update_item(
        &self,
        request: PublishRequest,
        existing: Option<Item>,
        must_create: bool,
    ) -> Result<PublishOutcome, InternetArchiveError> {
        if request.uploads.is_empty() {
            return Err(InternetArchiveError::InvalidState(
                "Internet Archive item workflows require at least one upload".to_owned(),
            ));
        }

        let created = existing.is_none();

        if must_create && existing.is_some() {
            return Err(InternetArchiveError::InvalidState(format!(
                "item {} already exists",
                request.identifier
            )));
        }

        let mut uploaded_files = Vec::new();
        let mut skipped_files = Vec::new();
        let mut metadata_changed = false;

        if let Some(existing) = existing.as_ref() {
            for spec in &request.uploads {
                let already_present = existing.file(&spec.filename).is_some();
                match (already_present, request.conflict_policy) {
                    (true, FileConflictPolicy::Error) => {
                        return Err(InternetArchiveError::UploadConflict {
                            filename: spec.filename.clone(),
                        });
                    }
                    (true, FileConflictPolicy::Skip) => {
                        skipped_files.push(spec.filename.clone());
                    }
                    (true, FileConflictPolicy::OverwriteKeepingHistory) => {
                        let mut options = request.upload_options.clone();
                        options.keep_old_version = true;
                        self.upload_file(&request.identifier, spec, &options)
                            .await?;
                        uploaded_files.push(spec.filename.clone());
                    }
                    _ => {
                        self.upload_file(&request.identifier, spec, &request.upload_options)
                            .await?;
                        uploaded_files.push(spec.filename.clone());
                    }
                }
            }

            let response = self
                .update_item_metadata(&request.identifier, &request.metadata)
                .await?;
            metadata_changed = response.task_id.is_some();
        } else {
            let first = &request.uploads[0];
            self.create_item(
                &request.identifier,
                &request.metadata,
                first,
                &request.upload_options,
            )
            .await?;
            uploaded_files.push(first.filename.clone());

            for spec in request.uploads.iter().skip(1) {
                self.upload_file(&request.identifier, spec, &request.upload_options)
                    .await?;
                uploaded_files.push(spec.filename.clone());
            }

            let current = self.wait_for_item(&request.identifier).await?;
            if current.metadata != request.metadata {
                let response = self
                    .update_item_metadata(&request.identifier, &request.metadata)
                    .await?;
                metadata_changed = response.task_id.is_some();
            }
        }

        let item = self.wait_for_item(&request.identifier).await?;
        Ok(PublishOutcome {
            item,
            created,
            uploaded_files,
            skipped_files,
            metadata_changed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PublishRequest;
    use crate::client::InternetArchiveClient;
    use crate::error::InternetArchiveError;
    use crate::metadata::{ItemMetadata, MediaType};
    use crate::upload::UploadSpec;
    use crate::ItemIdentifier;

    #[test]
    fn publish_request_defaults_are_sensible() {
        let request = PublishRequest::new(
            ItemIdentifier::new("demo-item").unwrap(),
            ItemMetadata::builder()
                .mediatype(MediaType::Texts)
                .title("Demo")
                .build(),
            vec![UploadSpec::from_bytes("demo.txt", b"hello")],
        );

        assert_eq!(request.uploads.len(), 1);
        assert_eq!(request.identifier.as_str(), "demo-item");
    }

    #[tokio::test]
    async fn create_or_update_item_rejects_empty_upload_lists_before_network_access() {
        let client = InternetArchiveClient::new().unwrap();
        let request = PublishRequest::new(
            ItemIdentifier::new("demo-item").unwrap(),
            ItemMetadata::builder().title("Demo").build(),
            Vec::new(),
        );

        let error = client
            .create_or_update_item(request, None, false)
            .await
            .unwrap_err();
        assert!(
            matches!(error, InternetArchiveError::InvalidState(message) if message.contains("at least one upload"))
        );
    }

    #[tokio::test]
    async fn create_or_update_item_rejects_existing_items_when_creation_is_forced() {
        let client = InternetArchiveClient::new().unwrap();
        let request = PublishRequest::new(
            ItemIdentifier::new("demo-item").unwrap(),
            ItemMetadata::builder().title("Demo").build(),
            vec![UploadSpec::from_bytes("demo.txt", b"hello")],
        );
        let existing = serde_json::from_value(serde_json::json!({
            "files": [],
            "metadata": {"identifier": "demo-item"}
        }))
        .unwrap();

        let error = client
            .create_or_update_item(request, Some(existing), true)
            .await
            .unwrap_err();
        assert!(
            matches!(error, InternetArchiveError::InvalidState(message) if message.contains("already exists"))
        );
    }
}
