//! Higher-level item publication and update workflows.

use std::time::Duration;

use crate::client::InternetArchiveClient;
use crate::error::InternetArchiveError;
use crate::metadata::{metadata_contains_projection, ItemMetadata};
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
    /// Optional bounded wait for catalog visibility after every file uploads.
    ///
    /// `None` (the default) returns as soon as all uploads complete, without
    /// requiring the item to be queryable in the public catalog first. `Some(d)`
    /// polls up to `d` for the item to project the uploaded files and metadata,
    /// and still returns `Ok` if that wait times out. A successful upload is
    /// never turned into an error by a visibility timeout.
    pub wait_for_visibility: Option<Duration>,
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
            wait_for_visibility: None,
        }
    }

    /// Sets a bounded wait for catalog visibility after uploading.
    ///
    /// When set, the workflow polls up to `max_wait` for the item to project
    /// into the public catalog and populates [`PublishOutcome::item`] on
    /// success. A timeout still returns `Ok`, leaving `item` as `None`.
    #[must_use]
    pub fn wait_for_visibility(mut self, max_wait: Duration) -> Self {
        self.wait_for_visibility = Some(max_wait);
        self
    }
}

/// Result returned by high-level publish or upsert helpers.
#[derive(Clone, Debug, PartialEq)]
pub struct PublishOutcome {
    /// Final item state after the workflow, when catalog projection was
    /// confirmed.
    ///
    /// `None` means every file uploaded successfully but the item was not
    /// confirmed visible in the public catalog before returning. This is the
    /// default (no visibility wait) and also the result of an opt-in
    /// [`PublishRequest::wait_for_visibility`] wait that timed out. A `None`
    /// here is not a failure: the bytes landed.
    pub item: Option<Item>,
    /// Whether the item was created during this workflow.
    pub created: bool,
    /// File names uploaded during this workflow.
    pub uploaded_files: Vec<String>,
    /// File names skipped because of the selected policy.
    pub skipped_files: Vec<String>,
    /// Whether metadata was updated through MDAPI after the upload step.
    pub metadata_changed: bool,
}

impl PublishOutcome {
    /// Returns whether the item was confirmed projected into the public catalog.
    ///
    /// Equivalent to [`PublishOutcome::item`] being `Some`. A `false` result
    /// does not mean the upload failed, only that catalog visibility was not
    /// confirmed before returning.
    #[must_use]
    pub fn projection_confirmed(&self) -> bool {
        self.item.is_some()
    }
}

impl InternetArchiveClient {
    /// Creates a brand-new item and uploads all requested files.
    ///
    /// # Errors
    ///
    /// Returns an error if the identifier is not valid for IA-S3 bucket
    /// creation, the item already exists, the request has no files, or any
    /// network step fails.
    pub async fn publish_item(
        &self,
        request: PublishRequest,
    ) -> Result<PublishOutcome, InternetArchiveError> {
        request.identifier.validate_for_bucket_creation()?;

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
    /// Returns an error if the identifier is not valid for IA-S3 bucket
    /// creation when a new item must be created, or if any required network
    /// step fails.
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
        let metadata_changed;

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
            let (created_files, created_changed) = self.create_item_and_reconcile(&request).await?;
            uploaded_files = created_files;
            metadata_changed = created_changed;
        }

        // Catalog projection is eventually consistent and can take many minutes
        // on a fresh item. A successful upload returns immediately with `item`
        // as `None` unless the caller opted into a bounded visibility wait, which
        // still returns `Ok` (with `item` as `None`) on timeout.
        let item = match request.wait_for_visibility {
            Some(max_wait) => {
                self.try_wait_for_item_projection(
                    &request.identifier,
                    &uploaded_files,
                    &request.metadata,
                    max_wait,
                )
                .await?
            }
            None => None,
        };

        Ok(PublishOutcome {
            item,
            created,
            uploaded_files,
            skipped_files,
            metadata_changed,
        })
    }

    /// Creates a brand-new item, uploads the remaining files, and reconciles
    /// non-header metadata when the caller opted into a visibility wait.
    ///
    /// Returns the uploaded file names and whether metadata was written through
    /// MDAPI after the create step.
    async fn create_item_and_reconcile(
        &self,
        request: &PublishRequest,
    ) -> Result<(Vec<String>, bool), InternetArchiveError> {
        let first = &request.uploads[0];
        // Create the item with header metadata only, without blocking on catalog
        // visibility. Non-header (nested or complex) metadata is reconciled below
        // only when the caller opted into a wait, so a brand-new item never hangs
        // by default.
        self.create_item_object(
            &request.identifier,
            &request.metadata,
            first,
            &request.upload_options,
        )
        .await?;
        let mut uploaded_files = vec![first.filename.clone()];

        for spec in request.uploads.iter().skip(1) {
            self.upload_file(&request.identifier, spec, &request.upload_options)
                .await?;
            uploaded_files.push(spec.filename.clone());
        }

        let mut metadata_changed = false;
        // Reconciling non-header metadata needs the new item to be catalogued,
        // which is eventually consistent, so only do it when the caller opted
        // into a visibility wait. The wait is non-fatal.
        if let Some(max_wait) = request.wait_for_visibility {
            if let Some(current) = self
                .try_wait_for_item(&request.identifier, max_wait)
                .await?
            {
                if !metadata_contains_projection(&current.metadata, &request.metadata) {
                    let response = self
                        .update_item_metadata(&request.identifier, &request.metadata)
                        .await?;
                    metadata_changed = response.task_id.is_some();
                }
            }
        }

        Ok((uploaded_files, metadata_changed))
    }
}

#[cfg(test)]
mod tests {
    use super::PublishRequest;
    use crate::client::InternetArchiveClient;
    use crate::error::InternetArchiveError;
    use crate::metadata::{ItemMetadata, MediaType};
    use crate::upload::UploadSpec;
    use crate::{IdentifierError, ItemIdentifier};

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
    async fn publish_rejects_bucket_unsafe_identifiers_before_lookup() {
        let client = InternetArchiveClient::new().unwrap();
        let request = PublishRequest::new(
            ItemIdentifier::new("Demo-item").unwrap(),
            ItemMetadata::builder().title("Demo").build(),
            vec![UploadSpec::from_bytes("demo.txt", b"hello")],
        );

        assert!(matches!(
            client.publish_item(request).await.unwrap_err(),
            InternetArchiveError::Identifier(IdentifierError::InvalidBucketCreationCharacter {
                character: 'D',
                ..
            })
        ));
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
