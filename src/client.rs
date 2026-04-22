//! Low-level typed Internet Archive client operations.

use std::path::Path;
#[cfg(feature = "indicatif")]
use std::pin::Pin;
#[cfg(feature = "indicatif")]
use std::task::{Context, Poll};
use std::time::Duration;

#[cfg(feature = "indicatif")]
use futures_core::Stream;
#[cfg(feature = "indicatif")]
use indicatif::ProgressBar;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE,
    LOCATION,
};
use reqwest::{Method, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use url::Url;

use crate::downloads::ResolvedDownload;
use crate::endpoint::Endpoint;
use crate::error::{decode_metadata_write_failure, InternetArchiveError};
use crate::ids::SecretPair;
use crate::metadata::{
    merge_metadata_semantically, metadata_contains_projection, HeaderEncoding, ItemMetadata,
    MetadataChange, MetadataTarget, PatchOperation,
};
use crate::model::{Item, MetadataWriteResponse, S3LimitCheck, SearchResponse};
use crate::poll::PollOptions;
use crate::search::SearchQuery;
use crate::upload::{DeleteOptions, UploadOptions, UploadSource, UploadSpec};
use crate::ItemIdentifier;

/// LOW-auth credentials for Internet Archive programmatic access.
#[derive(Clone)]
pub struct Auth {
    pub(crate) secrets: SecretPair,
}

impl Auth {
    /// Standard environment variable for the S3 access key.
    pub const ACCESS_KEY_ENV_VAR: &'static str = "INTERNET_ARCHIVE_ACCESS_KEY";
    /// Standard environment variable for the S3 secret key.
    pub const SECRET_KEY_ENV_VAR: &'static str = "INTERNET_ARCHIVE_SECRET_KEY";

    /// Creates a new auth pair.
    #[must_use]
    pub fn new(access_key: impl Into<String>, secret_key: impl Into<String>) -> Self {
        Self {
            secrets: SecretPair {
                access_key: SecretString::from(access_key.into()),
                secret_key: SecretString::from(secret_key.into()),
            },
        }
    }

    /// Reads credentials from the standard environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if either environment variable is missing.
    pub fn from_env() -> Result<Self, InternetArchiveError> {
        Self::from_env_vars(Self::ACCESS_KEY_ENV_VAR, Self::SECRET_KEY_ENV_VAR)
    }

    /// Reads credentials from custom environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if either environment variable is missing.
    pub fn from_env_vars(
        access_name: &str,
        secret_name: &str,
    ) -> Result<Self, InternetArchiveError> {
        let access_key =
            std::env::var(access_name).map_err(|source| InternetArchiveError::EnvVar {
                name: access_name.to_owned(),
                source,
            })?;
        let secret_key =
            std::env::var(secret_name).map_err(|source| InternetArchiveError::EnvVar {
                name: secret_name.to_owned(),
                source,
            })?;
        Ok(Self::new(access_key, secret_key))
    }

    #[must_use]
    pub(crate) fn authorization_header(&self) -> String {
        format!(
            "LOW {}:{}",
            self.secrets.access_key.expose_secret(),
            self.secrets.secret_key.expose_secret()
        )
    }
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("access_key", &"<redacted>")
            .field("secret_key", &"<redacted>")
            .finish()
    }
}

/// Builder for configuring an [`InternetArchiveClient`].
#[derive(Clone, Debug)]
pub struct InternetArchiveClientBuilder {
    auth: Option<Auth>,
    endpoint: Endpoint,
    poll: PollOptions,
    user_agent: Option<String>,
    request_timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
}

impl InternetArchiveClientBuilder {
    /// Sets the credentials used for authenticated operations.
    #[must_use]
    pub fn auth(mut self, auth: Auth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Overrides the endpoint roots.
    #[must_use]
    pub fn endpoint(mut self, endpoint: Endpoint) -> Self {
        self.endpoint = endpoint;
        self
    }

    /// Overrides the `User-Agent` header.
    #[must_use]
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// Sets the overall request timeout.
    #[must_use]
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = Some(timeout);
        self
    }

    /// Sets the TCP connect timeout.
    #[must_use]
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// Overrides workflow polling behavior.
    #[must_use]
    pub fn poll_options(mut self, poll: PollOptions) -> Self {
        self.poll = poll;
        self
    }

    /// Builds the client.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP clients cannot be built.
    pub fn build(self) -> Result<InternetArchiveClient, InternetArchiveError> {
        let user_agent = self
            .user_agent
            .unwrap_or_else(|| format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")));

        let build_client =
            |redirects_enabled: bool| -> Result<reqwest::Client, InternetArchiveError> {
                let mut builder = reqwest::Client::builder().user_agent(&user_agent);
                if let Some(timeout) = self.request_timeout {
                    builder = builder.timeout(timeout);
                }
                if let Some(timeout) = self.connect_timeout {
                    builder = builder.connect_timeout(timeout);
                }
                if !redirects_enabled {
                    builder = builder.redirect(reqwest::redirect::Policy::none());
                }
                builder.build().map_err(Into::into)
            };

        Ok(InternetArchiveClient {
            inner: build_client(true)?,
            s3_inner: build_client(false)?,
            auth: self.auth,
            endpoint: self.endpoint,
            poll: self.poll,
            request_timeout: self.request_timeout,
            connect_timeout: self.connect_timeout,
        })
    }
}

/// Typed async client for Internet Archive metadata, search, uploads, and downloads.
#[derive(Clone, Debug)]
pub struct InternetArchiveClient {
    pub(crate) inner: reqwest::Client,
    pub(crate) s3_inner: reqwest::Client,
    pub(crate) auth: Option<Auth>,
    pub(crate) endpoint: Endpoint,
    pub(crate) poll: PollOptions,
    pub(crate) request_timeout: Option<Duration>,
    pub(crate) connect_timeout: Option<Duration>,
}

impl InternetArchiveClient {
    /// Starts building a client.
    #[must_use]
    pub fn builder() -> InternetArchiveClientBuilder {
        InternetArchiveClientBuilder {
            auth: None,
            endpoint: Endpoint::default(),
            poll: PollOptions::default(),
            user_agent: None,
            request_timeout: None,
            connect_timeout: None,
        }
    }

    /// Builds an unauthenticated client.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP clients cannot be built.
    pub fn new() -> Result<Self, InternetArchiveError> {
        Self::builder().build()
    }

    /// Builds a client with explicit credentials.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP clients cannot be built.
    pub fn with_auth(auth: Auth) -> Result<Self, InternetArchiveError> {
        Self::builder().auth(auth).build()
    }

    /// Builds a client from the standard Internet Archive environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if the environment variables are missing or if the
    /// underlying HTTP clients cannot be built.
    pub fn from_env() -> Result<Self, InternetArchiveError> {
        Self::with_auth(Auth::from_env()?)
    }

    /// Returns the configured endpoint roots.
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Returns the configured workflow polling options.
    #[must_use]
    pub fn poll_options(&self) -> &PollOptions {
        &self.poll
    }

    /// Returns the request timeout.
    #[must_use]
    pub fn request_timeout(&self) -> Option<Duration> {
        self.request_timeout
    }

    /// Returns the connect timeout.
    #[must_use]
    pub fn connect_timeout(&self) -> Option<Duration> {
        self.connect_timeout
    }

    /// Returns whether the client currently has credentials configured.
    #[must_use]
    pub fn has_auth(&self) -> bool {
        self.auth.is_some()
    }

    /// Fetches a full item metadata record by identifier.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the item does not exist.
    pub async fn get_item(
        &self,
        identifier: &ItemIdentifier,
    ) -> Result<Item, InternetArchiveError> {
        let url = self.endpoint.metadata_url(identifier.as_str())?;
        let bytes = self
            .execute_bytes(
                self.archive_request(Method::GET, url)
                    .header(ACCEPT, "application/json"),
            )
            .await?;

        if bytes.iter().all(u8::is_ascii_whitespace) || bytes.as_ref() == b"[]" {
            return Err(InternetArchiveError::ItemNotFound {
                identifier: identifier.to_string(),
            });
        }

        let item: Item = serde_json::from_slice(&bytes)?;
        if item.identifier().as_ref() != Some(identifier) {
            return Err(InternetArchiveError::ItemNotFound {
                identifier: identifier.to_string(),
            });
        }

        Ok(item)
    }

    /// Fetches a full item metadata record from a string identifier.
    ///
    /// # Errors
    ///
    /// Returns an error if the identifier is invalid, the request fails, or the
    /// item does not exist.
    pub async fn get_item_by_str(
        &self,
        identifier: impl AsRef<str>,
    ) -> Result<Item, InternetArchiveError> {
        let identifier = ItemIdentifier::new(identifier.as_ref())?;
        self.get_item(&identifier).await
    }

    /// Runs an advanced search query.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response is invalid.
    pub async fn search(
        &self,
        query: &SearchQuery,
    ) -> Result<SearchResponse, InternetArchiveError> {
        let url = query.into_url(self.endpoint.search_url()?)?;
        let response = self
            .archive_request(Method::GET, url)
            .header(ACCEPT, "application/json")
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(InternetArchiveError::from_response(response).await);
        }

        let bytes = response.bytes().await?;
        decode_search_response(&bytes)
    }

    /// Checks whether the S3 queue is currently over its documented upload limit.
    ///
    /// # Errors
    ///
    /// Returns an error if the client has no credentials or if the request
    /// fails.
    pub async fn check_upload_limit(
        &self,
        identifier: &ItemIdentifier,
    ) -> Result<S3LimitCheck, InternetArchiveError> {
        let auth = self
            .auth
            .as_ref()
            .ok_or(InternetArchiveError::MissingAuth)?;
        let url = self
            .endpoint
            .s3_limit_check_url(auth.secrets.access_key.expose_secret(), identifier.as_str())?;
        self.execute_json(self.s3_request(Method::GET, url, HeaderMap::new())?)
            .await
    }

    /// Applies a single-target metadata patch document.
    ///
    /// # Errors
    ///
    /// Returns an error if the client has no credentials, the request fails, or
    /// MDAPI rejects the patch.
    pub async fn apply_metadata_patch(
        &self,
        identifier: &ItemIdentifier,
        target: MetadataTarget,
        patch: &[PatchOperation],
    ) -> Result<MetadataWriteResponse, InternetArchiveError> {
        if self.auth.is_none() {
            return Err(InternetArchiveError::MissingAuth);
        }
        let url = self.endpoint.metadata_url(identifier.as_str())?;
        let patch = serde_json::to_string(patch)?;
        self.execute_metadata_write(
            self.archive_request(Method::POST, url)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .form(&[("-target", target.as_str()), ("-patch", patch)]),
        )
        .await
    }

    /// Applies an atomic multi-target metadata write.
    ///
    /// # Errors
    ///
    /// Returns an error if the client has no credentials, the request fails, or
    /// MDAPI rejects the patch document.
    pub async fn apply_metadata_changes(
        &self,
        identifier: &ItemIdentifier,
        changes: &[MetadataChange],
    ) -> Result<MetadataWriteResponse, InternetArchiveError> {
        if self.auth.is_none() {
            return Err(InternetArchiveError::MissingAuth);
        }
        let url = self.endpoint.metadata_url(identifier.as_str())?;
        let payload = serde_json::to_string(changes)?;
        self.execute_metadata_write(
            self.archive_request(Method::POST, url)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .form(&[("-changes", payload)]),
        )
        .await
    }

    /// Updates the item-level metadata document by overlaying the provided keys
    /// onto the current metadata and diffing the result.
    ///
    /// Missing keys in `metadata` are left untouched. Use
    /// [`Self::apply_metadata_patch`] when you want exact JSON Patch behavior,
    /// including removals.
    ///
    /// If there are no effective changes, the method returns a synthetic
    /// successful response with no task id and does not require authentication.
    ///
    /// # Errors
    ///
    /// Returns an error if the client has no credentials, the item cannot be
    /// fetched, or the patch cannot be applied.
    pub async fn update_item_metadata(
        &self,
        identifier: &ItemIdentifier,
        metadata: &ItemMetadata,
    ) -> Result<MetadataWriteResponse, InternetArchiveError> {
        let current = self.get_item(identifier).await?;
        let current_value = serde_json::to_value(&current.metadata)?;
        let desired_value =
            serde_json::to_value(merge_metadata_semantically(&current.metadata, metadata))?;
        let patch_value = json_patch::diff(&current_value, &desired_value);
        let patch: Vec<PatchOperation> =
            serde_json::from_value(serde_json::to_value(patch_value)?)?;

        if patch.is_empty() {
            return Ok(MetadataWriteResponse {
                success: true,
                task_id: None,
                log: None,
                error: None,
            });
        }

        if self.auth.is_none() {
            return Err(InternetArchiveError::MissingAuth);
        }

        self.apply_metadata_patch(identifier, MetadataTarget::Metadata, &patch)
            .await
    }

    /// Uploads a file to an existing item.
    ///
    /// # Errors
    ///
    /// Returns an error if the client has no credentials, the request fails, or
    /// IA rejects the upload.
    pub async fn upload_file(
        &self,
        identifier: &ItemIdentifier,
        spec: &UploadSpec,
        options: &UploadOptions,
    ) -> Result<(), InternetArchiveError> {
        self.put_object(identifier, spec, options, None, false)
            .await
    }

    /// Uploads a file to an existing item while updating an `indicatif`
    /// progress bar.
    ///
    /// Available when the `indicatif` feature is enabled.
    ///
    /// # Errors
    ///
    /// Returns an error if the client has no credentials, the request fails, or
    /// IA rejects the upload.
    #[cfg(feature = "indicatif")]
    pub async fn upload_file_with_progress(
        &self,
        identifier: &ItemIdentifier,
        spec: &UploadSpec,
        options: &UploadOptions,
        progress: &ProgressBar,
    ) -> Result<(), InternetArchiveError> {
        self.put_object_with_progress(identifier, spec, options, None, false, progress)
            .await
    }

    /// Creates a new item by uploading the first file with automatic bucket
    /// creation headers and initial metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the client has no credentials, the request fails, or
    /// IA rejects the upload.
    pub async fn create_item(
        &self,
        identifier: &ItemIdentifier,
        metadata: &ItemMetadata,
        spec: &UploadSpec,
        options: &UploadOptions,
    ) -> Result<(), InternetArchiveError> {
        let header_encoding = metadata.as_header_encoding();
        let remainder = header_encoding.remainder.clone();
        self.put_object(identifier, spec, options, Some(header_encoding), true)
            .await?;

        if !remainder.as_map().is_empty() {
            self.wait_for_item(identifier).await?;
            self.update_item_metadata(identifier, &remainder).await?;
            self.wait_for_item_projection(identifier, &[], &remainder)
                .await?;
        }

        Ok(())
    }

    /// Creates a new item while updating an `indicatif` progress bar for the
    /// initial upload step.
    ///
    /// Available when the `indicatif` feature is enabled.
    ///
    /// # Errors
    ///
    /// Returns an error if the client has no credentials, the request fails, or
    /// IA rejects the upload.
    #[cfg(feature = "indicatif")]
    pub async fn create_item_with_progress(
        &self,
        identifier: &ItemIdentifier,
        metadata: &ItemMetadata,
        spec: &UploadSpec,
        options: &UploadOptions,
        progress: &ProgressBar,
    ) -> Result<(), InternetArchiveError> {
        let header_encoding = metadata.as_header_encoding();
        let remainder = header_encoding.remainder.clone();
        self.put_object_with_progress(
            identifier,
            spec,
            options,
            Some(header_encoding),
            true,
            progress,
        )
        .await?;

        if !remainder.as_map().is_empty() {
            self.wait_for_item(identifier).await?;
            self.update_item_metadata(identifier, &remainder).await?;
            self.wait_for_item_projection(identifier, &[], &remainder)
                .await?;
        }

        Ok(())
    }

    /// Deletes a file from an item through the S3-like API.
    ///
    /// # Errors
    ///
    /// Returns an error if the client has no credentials, the request fails, or
    /// IA rejects the delete.
    pub async fn delete_file(
        &self,
        identifier: &ItemIdentifier,
        filename: &str,
        options: &DeleteOptions,
    ) -> Result<(), InternetArchiveError> {
        let mut headers = HeaderMap::new();
        if options.cascade_delete {
            headers.insert("x-archive-cascade-delete", HeaderValue::from_static("1"));
        }
        if options.keep_old_version {
            headers.insert("x-archive-keep-old-version", HeaderValue::from_static("1"));
        }

        let url = self.endpoint.s3_object_url(identifier.as_str(), filename)?;
        self.execute_s3(Method::DELETE, url, headers, None).await?;
        Ok(())
    }

    /// Resolves the public download URL for a file.
    ///
    /// # Errors
    ///
    /// Returns an error if URL construction fails.
    pub fn resolve_download(
        &self,
        identifier: &ItemIdentifier,
        filename: &str,
    ) -> Result<ResolvedDownload, InternetArchiveError> {
        Ok(ResolvedDownload {
            identifier: identifier.clone(),
            filename: filename.to_owned(),
            url: self.endpoint.download_url(identifier.as_str(), filename)?,
        })
    }

    /// Downloads a file into memory.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn download_bytes(
        &self,
        identifier: &ItemIdentifier,
        filename: &str,
    ) -> Result<bytes::Bytes, InternetArchiveError> {
        let resolved = self.resolve_download(identifier, filename)?;
        self.execute_bytes(self.inner.get(resolved.url)).await
    }

    /// Downloads a file into memory while updating an `indicatif` progress bar.
    ///
    /// Available when the `indicatif` feature is enabled.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    #[cfg(feature = "indicatif")]
    pub async fn download_bytes_with_progress(
        &self,
        identifier: &ItemIdentifier,
        filename: &str,
        progress: &ProgressBar,
    ) -> Result<bytes::Bytes, InternetArchiveError> {
        let resolved = self.resolve_download(identifier, filename)?;
        self.execute_bytes_with_progress(self.inner.get(resolved.url), progress)
            .await
    }

    /// Downloads a file to a local path.
    ///
    /// # Errors
    ///
    /// Returns an error if the request or local file write fails.
    pub async fn download_to_path(
        &self,
        identifier: &ItemIdentifier,
        filename: &str,
        path: impl AsRef<Path>,
    ) -> Result<(), InternetArchiveError> {
        let bytes = self.download_bytes(identifier, filename).await?;
        tokio::fs::write(path, &bytes).await?;
        Ok(())
    }

    /// Downloads a file to a local path while updating an `indicatif`
    /// progress bar.
    ///
    /// Available when the `indicatif` feature is enabled.
    ///
    /// # Errors
    ///
    /// Returns an error if the request or local file write fails.
    #[cfg(feature = "indicatif")]
    pub async fn download_to_path_with_progress(
        &self,
        identifier: &ItemIdentifier,
        filename: &str,
        path: impl AsRef<Path>,
        progress: &ProgressBar,
    ) -> Result<(), InternetArchiveError> {
        let bytes = self
            .download_bytes_with_progress(identifier, filename, progress)
            .await?;
        tokio::fs::write(path, &bytes).await?;
        Ok(())
    }

    pub(crate) async fn wait_for_item(
        &self,
        identifier: &ItemIdentifier,
    ) -> Result<Item, InternetArchiveError> {
        self.wait_until("item visibility", || async {
            self.get_item(identifier).await
        })
        .await
    }

    pub(crate) async fn wait_for_item_projection(
        &self,
        identifier: &ItemIdentifier,
        expected_files: &[String],
        expected_metadata: &ItemMetadata,
    ) -> Result<Item, InternetArchiveError> {
        self.wait_until("item projection visibility", || async {
            let item = self.get_item(identifier).await?;
            if expected_files
                .iter()
                .all(|filename| item.file(filename).is_some())
                && metadata_contains_projection(&item.metadata, expected_metadata)
            {
                Ok(item)
            } else {
                Err(InternetArchiveError::ItemNotFound {
                    identifier: identifier.to_string(),
                })
            }
        })
        .await
    }

    async fn put_object(
        &self,
        identifier: &ItemIdentifier,
        spec: &UploadSpec,
        options: &UploadOptions,
        metadata: Option<HeaderEncoding>,
        auto_make_bucket: bool,
    ) -> Result<(), InternetArchiveError> {
        let (url, headers, body) = self
            .prepare_put_object(identifier, spec, options, metadata, auto_make_bucket)
            .await?;
        self.execute_s3(Method::PUT, url, headers, Some(body))
            .await?;
        Ok(())
    }

    #[cfg(feature = "indicatif")]
    async fn put_object_with_progress(
        &self,
        identifier: &ItemIdentifier,
        spec: &UploadSpec,
        options: &UploadOptions,
        metadata: Option<HeaderEncoding>,
        auto_make_bucket: bool,
        progress: &ProgressBar,
    ) -> Result<(), InternetArchiveError> {
        let (url, headers, body) = self
            .prepare_put_object(identifier, spec, options, metadata, auto_make_bucket)
            .await?;
        self.execute_s3_with_progress(Method::PUT, url, headers, Some(body), progress)
            .await?;
        Ok(())
    }

    async fn prepare_put_object(
        &self,
        identifier: &ItemIdentifier,
        spec: &UploadSpec,
        options: &UploadOptions,
        metadata: Option<HeaderEncoding>,
        auto_make_bucket: bool,
    ) -> Result<(Url, HeaderMap, ReplayableBody), InternetArchiveError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_str(spec.content_type.as_ref()).map_err(|_| {
                InternetArchiveError::InvalidState("invalid content type".to_owned())
            })?,
        );

        if auto_make_bucket {
            headers.insert("x-archive-auto-make-bucket", HeaderValue::from_static("1"));
        }
        if options.skip_derive {
            headers.insert("x-archive-queue-derive", HeaderValue::from_static("0"));
        }
        if options.keep_old_version {
            headers.insert("x-archive-keep-old-version", HeaderValue::from_static("1"));
        }
        if options.interactive_priority {
            headers.insert(
                "x-archive-interactive-priority",
                HeaderValue::from_static("1"),
            );
        }
        if let Some(size_hint) = options.size_hint {
            headers.insert(
                "x-archive-size-hint",
                HeaderValue::from_str(&size_hint.to_string()).map_err(|_| {
                    InternetArchiveError::InvalidState("invalid size hint".to_owned())
                })?,
            );
        }
        if let Some(metadata) = metadata {
            for (name, value) in metadata.headers {
                let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                    InternetArchiveError::InvalidState("invalid metadata header name".to_owned())
                })?;
                headers.insert(
                    name,
                    HeaderValue::from_str(&value).map_err(|_| {
                        InternetArchiveError::InvalidState(
                            "invalid metadata header value".to_owned(),
                        )
                    })?,
                );
            }
        }

        let body = match &spec.source {
            UploadSource::Path(path) => {
                let length = tokio::fs::metadata(path).await?.len();
                ReplayableBody::Path {
                    path: path.clone(),
                    length,
                }
            }
            UploadSource::Bytes(bytes) => ReplayableBody::Bytes(bytes.clone()),
        };

        let url = self
            .endpoint
            .s3_object_url(identifier.as_str(), &spec.filename)?;
        Ok((url, headers, body))
    }

    fn archive_request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        let mut request = self.inner.request(method, url);
        if let Some(auth) = &self.auth {
            request = request.header(AUTHORIZATION, auth.authorization_header());
        }
        request
    }

    fn s3_request(
        &self,
        method: Method,
        url: Url,
        headers: HeaderMap,
    ) -> Result<reqwest::RequestBuilder, InternetArchiveError> {
        let auth = self
            .auth
            .as_ref()
            .ok_or(InternetArchiveError::MissingAuth)?;
        Ok(self
            .s3_inner
            .request(method, url)
            .headers(headers)
            .header(AUTHORIZATION, auth.authorization_header()))
    }

    async fn execute_json<T>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, InternetArchiveError>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(InternetArchiveError::from_response(response).await);
        }
        let bytes = response.bytes().await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn execute_bytes(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<bytes::Bytes, InternetArchiveError> {
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(InternetArchiveError::from_response(response).await);
        }
        response.bytes().await.map_err(Into::into)
    }

    #[cfg(feature = "indicatif")]
    async fn execute_bytes_with_progress(
        &self,
        request: reqwest::RequestBuilder,
        progress: &ProgressBar,
    ) -> Result<bytes::Bytes, InternetArchiveError> {
        progress.set_position(0);

        let mut response = request.send().await?;
        if !response.status().is_success() {
            return Err(InternetArchiveError::from_response(response).await);
        }

        if let Some(length) = response.content_length() {
            progress.set_length(length);
        }

        let mut bytes = response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .map_or_else(Vec::new, Vec::with_capacity);

        while let Some(chunk) = response.chunk().await? {
            progress.inc(chunk.len() as u64);
            bytes.extend_from_slice(&chunk);
        }

        progress.finish();
        Ok(bytes::Bytes::from(bytes))
    }

    async fn execute_metadata_write(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<MetadataWriteResponse, InternetArchiveError> {
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(InternetArchiveError::from_response(response).await);
        }

        let bytes = response.bytes().await?;
        decode_metadata_write_failure(&bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn execute_s3(
        &self,
        method: Method,
        url: Url,
        headers: HeaderMap,
        body: Option<ReplayableBody>,
    ) -> Result<reqwest::Response, InternetArchiveError> {
        let mut current_url = url;
        let mut remaining_redirects = 8_u8;

        loop {
            let mut request =
                self.s3_request(method.clone(), current_url.clone(), headers.clone())?;
            if let Some(body) = &body {
                request = body.apply(request).await?;
            }

            let response = request.send().await?;
            if is_redirect(response.status()) {
                let Some(location) = response.headers().get(LOCATION).cloned() else {
                    return Err(InternetArchiveError::InvalidState(
                        "redirect response missing location header".to_owned(),
                    ));
                };

                if remaining_redirects == 0 {
                    return Err(InternetArchiveError::InvalidState(
                        "too many redirects during S3 request".to_owned(),
                    ));
                }

                let location = location.to_str().map_err(|_| {
                    InternetArchiveError::InvalidState(
                        "redirect location is not valid UTF-8".to_owned(),
                    )
                })?;
                let redirected_url = current_url.join(location)?;
                if redirected_url.origin() != self.endpoint.s3_base().origin() {
                    return Err(InternetArchiveError::InvalidState(
                        "refusing to forward credentials to redirected S3 host".to_owned(),
                    ));
                }
                current_url = redirected_url;
                remaining_redirects -= 1;
                continue;
            }

            if !response.status().is_success() {
                return Err(InternetArchiveError::from_response(response).await);
            }

            return Ok(response);
        }
    }

    #[cfg(feature = "indicatif")]
    async fn execute_s3_with_progress(
        &self,
        method: Method,
        url: Url,
        headers: HeaderMap,
        body: Option<ReplayableBody>,
        progress: &ProgressBar,
    ) -> Result<reqwest::Response, InternetArchiveError> {
        let mut current_url = url;
        let mut remaining_redirects = 8_u8;

        loop {
            let mut request =
                self.s3_request(method.clone(), current_url.clone(), headers.clone())?;
            if let Some(body) = &body {
                request = body.apply_with_progress(request, progress).await?;
            }

            let response = request.send().await?;
            if is_redirect(response.status()) {
                let Some(location) = response.headers().get(LOCATION).cloned() else {
                    return Err(InternetArchiveError::InvalidState(
                        "redirect response missing location header".to_owned(),
                    ));
                };

                if remaining_redirects == 0 {
                    return Err(InternetArchiveError::InvalidState(
                        "too many redirects during S3 request".to_owned(),
                    ));
                }

                let location = location.to_str().map_err(|_| {
                    InternetArchiveError::InvalidState(
                        "redirect location is not valid UTF-8".to_owned(),
                    )
                })?;
                let redirected_url = current_url.join(location)?;
                if redirected_url.origin() != self.endpoint.s3_base().origin() {
                    return Err(InternetArchiveError::InvalidState(
                        "refusing to forward credentials to redirected S3 host".to_owned(),
                    ));
                }
                current_url = redirected_url;
                remaining_redirects -= 1;
                continue;
            }

            if !response.status().is_success() {
                return Err(InternetArchiveError::from_response(response).await);
            }

            progress.finish();
            return Ok(response);
        }
    }

    pub(crate) async fn wait_until<T, F, Fut>(
        &self,
        label: &'static str,
        mut action: F,
    ) -> Result<T, InternetArchiveError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, InternetArchiveError>>,
    {
        let started = tokio::time::Instant::now();
        let mut delay = self.poll.initial_delay;

        loop {
            match action().await {
                Ok(value) => return Ok(value),
                Err(error)
                    if started.elapsed() < self.poll.max_wait
                        && is_retryable_wait_error(&error) =>
                {
                    tokio::time::sleep(delay).await;
                    delay = std::cmp::min(delay.saturating_mul(2), self.poll.max_delay);
                }
                Err(error) => return Err(error),
            }

            if started.elapsed() >= self.poll.max_wait {
                return Err(InternetArchiveError::Timeout(label));
            }
        }
    }
}

#[derive(Clone, Debug)]
enum ReplayableBody {
    Path {
        path: std::path::PathBuf,
        length: u64,
    },
    Bytes(Vec<u8>),
}

impl ReplayableBody {
    async fn apply(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, InternetArchiveError> {
        match self {
            Self::Path { path, length } => {
                let file = File::open(path).await?;
                Ok(request
                    .header(CONTENT_LENGTH, *length)
                    .body(reqwest::Body::wrap_stream(ReaderStream::new(file))))
            }
            Self::Bytes(bytes) => Ok(request
                .header(CONTENT_LENGTH, bytes.len())
                .body(bytes.clone())),
        }
    }

    #[cfg(feature = "indicatif")]
    async fn apply_with_progress(
        &self,
        request: reqwest::RequestBuilder,
        progress: &ProgressBar,
    ) -> Result<reqwest::RequestBuilder, InternetArchiveError> {
        progress.set_position(0);

        match self {
            Self::Path { path, length } => {
                progress.set_length(*length);
                let file = File::open(path).await?;
                Ok(request
                    .header(CONTENT_LENGTH, *length)
                    .body(reqwest::Body::wrap_stream(ProgressStream::new(
                        ReaderStream::new(file),
                        progress.clone(),
                    ))))
            }
            Self::Bytes(bytes) => {
                let length = bytes.len() as u64;
                progress.set_length(length);
                Ok(request
                    .header(CONTENT_LENGTH, length)
                    .body(reqwest::Body::wrap_stream(ProgressStream::new(
                        ChunkedBytesStream::new(bytes::Bytes::from(bytes.clone())),
                        progress.clone(),
                    ))))
            }
        }
    }
}

#[cfg(feature = "indicatif")]
struct ProgressStream<S> {
    inner: S,
    progress: ProgressBar,
}

#[cfg(feature = "indicatif")]
impl<S> ProgressStream<S> {
    fn new(inner: S, progress: ProgressBar) -> Self {
        Self { inner, progress }
    }
}

#[cfg(feature = "indicatif")]
impl<S> Stream for ProgressStream<S>
where
    S: Stream<Item = Result<bytes::Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<bytes::Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                this.progress.inc(chunk.len() as u64);
                Poll::Ready(Some(Ok(chunk)))
            }
            other => other,
        }
    }
}

#[cfg(feature = "indicatif")]
struct ChunkedBytesStream {
    bytes: bytes::Bytes,
    offset: usize,
}

#[cfg(feature = "indicatif")]
impl ChunkedBytesStream {
    const CHUNK_SIZE: usize = 16 * 1024;

    fn new(bytes: bytes::Bytes) -> Self {
        Self { bytes, offset: 0 }
    }
}

#[cfg(feature = "indicatif")]
impl Stream for ChunkedBytesStream {
    type Item = Result<bytes::Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.offset >= this.bytes.len() {
            return Poll::Ready(None);
        }

        let end = this
            .offset
            .saturating_add(Self::CHUNK_SIZE)
            .min(this.bytes.len());
        let chunk = this.bytes.slice(this.offset..end);
        this.offset = end;
        Poll::Ready(Some(Ok(chunk)))
    }
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn is_retryable_wait_error(error: &InternetArchiveError) -> bool {
    match error {
        InternetArchiveError::ItemNotFound { .. } => true,
        InternetArchiveError::Http { status, .. } if status.is_server_error() => true,
        _ => false,
    }
}

fn decode_search_response(bytes: &[u8]) -> Result<SearchResponse, InternetArchiveError> {
    let value: Value = serde_json::from_slice(bytes)?;

    if value.get("response").is_some() {
        return Ok(serde_json::from_value(value)?);
    }

    let message = value
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| value.get("message").and_then(Value::as_str))
        .or_else(|| value.get("title").and_then(Value::as_str))
        .map_or_else(
            || {
                String::from_utf8_lossy(bytes)
                    .trim()
                    .chars()
                    .take(512)
                    .collect()
            },
            str::to_owned,
        );

    Err(InternetArchiveError::InvalidState(format!(
        "unexpected search response: {message}"
    )))
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "indicatif")]
    use std::pin::Pin;
    use std::sync::OnceLock;
    #[cfg(feature = "indicatif")]
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;

    use axum::extract::State;
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use axum::routing::{get, put};
    use axum::{Json, Router};
    #[cfg(feature = "indicatif")]
    use bytes::Bytes;
    #[cfg(feature = "indicatif")]
    use futures_core::Stream;
    #[cfg(feature = "indicatif")]
    use indicatif::ProgressBar;
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use url::Url;

    use super::{Auth, InternetArchiveClient};
    #[cfg(feature = "indicatif")]
    use super::{ChunkedBytesStream, ProgressStream, ReplayableBody};
    use crate::error::InternetArchiveError;
    use crate::metadata::{ItemMetadata, MetadataChange, MetadataTarget, PatchOperation};
    use crate::search::{SearchQuery, SortDirection};
    use crate::upload::{DeleteOptions, UploadOptions, UploadSpec};
    use crate::{Endpoint, ItemIdentifier, PollOptions};
    use reqwest::header::LOCATION;

    #[derive(Default)]
    struct StateData {
        seen_upload_auth: Mutex<Vec<String>>,
        seen_delete_auth: Mutex<Vec<String>>,
        captured_mdapi_body: Mutex<Vec<String>>,
        metadata_reads: Mutex<u8>,
    }

    fn test_client(addr: std::net::SocketAddr) -> InternetArchiveClient {
        InternetArchiveClient::builder()
            .auth(Auth::new("access", "secret"))
            .endpoint(Endpoint::custom(
                Url::parse(&format!("http://{addr}/")).unwrap(),
                Url::parse(&format!("http://{addr}/s3/")).unwrap(),
            ))
            .poll_options(PollOptions {
                max_wait: Duration::from_millis(50),
                initial_delay: Duration::from_millis(5),
                max_delay: Duration::from_millis(10),
            })
            .build()
            .unwrap()
    }

    fn unauthenticated_test_client(addr: std::net::SocketAddr) -> InternetArchiveClient {
        InternetArchiveClient::builder()
            .endpoint(Endpoint::custom(
                Url::parse(&format!("http://{addr}/")).unwrap(),
                Url::parse(&format!("http://{addr}/s3/")).unwrap(),
            ))
            .poll_options(PollOptions {
                max_wait: Duration::from_millis(50),
                initial_delay: Duration::from_millis(5),
                max_delay: Duration::from_millis(10),
            })
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn search_get_item_metadata_write_download_and_redirected_s3_calls_work() {
        async fn metadata() -> Json<Value> {
            Json(json!({
                "created": 1,
                "files": [{"name": "demo.txt", "size": "5"}],
                "metadata": {
                    "identifier": "demo-item",
                    "title": "Demo item",
                    "collection": ["opensource"]
                }
            }))
        }

        async fn advanced_search() -> Json<Value> {
            Json(json!({
                "responseHeader": {
                    "status": 0,
                    "QTime": 1,
                    "params": {"query": "identifier:demo-item"}
                },
                "response": {
                    "numFound": 1,
                    "start": 0,
                    "docs": [{"identifier": "demo-item", "title": "Demo item"}]
                }
            }))
        }

        async fn metadata_write(
            State(state): State<std::sync::Arc<StateData>>,
            headers: HeaderMap,
            body: String,
        ) -> (StatusCode, Json<Value>) {
            state.captured_mdapi_body.lock().await.push(body);
            assert_eq!(headers.get("authorization").unwrap(), "LOW access:secret");
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "task_id": 42,
                    "log": "https://catalogd.archive.org/log/42"
                })),
            )
        }

        async fn download() -> &'static str {
            "hello"
        }

        async fn first_upload() -> (StatusCode, HeaderMap) {
            let mut headers = HeaderMap::new();
            headers.insert(
                LOCATION,
                HeaderValue::from_static("/s3-redirected/demo-item/demo.txt"),
            );
            (StatusCode::TEMPORARY_REDIRECT, headers)
        }

        async fn redirected_upload(
            State(state): State<std::sync::Arc<StateData>>,
            headers: HeaderMap,
            body: String,
        ) -> StatusCode {
            state.seen_upload_auth.lock().await.push(
                headers
                    .get("authorization")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
            );
            assert_eq!(body, "hello");
            StatusCode::OK
        }

        async fn first_delete() -> (StatusCode, HeaderMap) {
            let mut headers = HeaderMap::new();
            headers.insert(
                LOCATION,
                HeaderValue::from_static("/s3-redirected/demo-item/demo.txt"),
            );
            (StatusCode::TEMPORARY_REDIRECT, headers)
        }

        async fn redirected_delete(
            State(state): State<std::sync::Arc<StateData>>,
            headers: HeaderMap,
        ) -> StatusCode {
            state.seen_delete_auth.lock().await.push(
                headers
                    .get("authorization")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
            );
            StatusCode::NO_CONTENT
        }

        let state = std::sync::Arc::new(StateData::default());
        let app = Router::new()
            .route("/metadata/demo-item", get(metadata).post(metadata_write))
            .route("/advancedsearch.php", get(advanced_search))
            .route("/download/demo-item/demo.txt", get(download))
            .route(
                "/s3/demo-item/demo.txt",
                put(first_upload).delete(first_delete),
            )
            .route(
                "/s3-redirected/demo-item/demo.txt",
                put(redirected_upload).delete(redirected_delete),
            )
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = test_client(addr);
        let identifier = ItemIdentifier::new("demo-item").unwrap();

        let item = client.get_item(&identifier).await.unwrap();
        assert_eq!(item.metadata.title(), Some("Demo item"));

        let search = client
            .search(
                &SearchQuery::builder("identifier:demo-item")
                    .field("identifier")
                    .field("title")
                    .sort("publicdate", SortDirection::Desc)
                    .build(),
            )
            .await
            .unwrap();
        assert_eq!(
            search.response.docs[0].identifier().unwrap().as_str(),
            "demo-item"
        );

        let write = client
            .apply_metadata_patch(
                &identifier,
                MetadataTarget::Metadata,
                &[PatchOperation::replace("/title", "Updated title")],
            )
            .await
            .unwrap();
        assert_eq!(write.task_id, Some(crate::TaskId(42)));

        let spec = UploadSpec::from_bytes("demo.txt", b"hello".to_vec());
        client
            .upload_file(&identifier, &spec, &UploadOptions::default())
            .await
            .unwrap();
        client
            .delete_file(&identifier, "demo.txt", &DeleteOptions::default())
            .await
            .unwrap();
        assert_eq!(
            client
                .download_bytes(&identifier, "demo.txt")
                .await
                .unwrap(),
            "hello"
        );

        assert_eq!(state.seen_upload_auth.lock().await[0], "LOW access:secret");
        assert_eq!(state.seen_delete_auth.lock().await[0], "LOW access:secret");
        assert!(state.captured_mdapi_body.lock().await[0].contains("-target=metadata"));

        server.abort();
    }

    #[tokio::test]
    async fn search_reports_non_search_json_payloads_as_invalid_state() {
        async fn advanced_search_error() -> Json<Value> {
            Json(json!({
                "message": "search backend warming up"
            }))
        }

        let app = Router::new().route("/advancedsearch.php", get(advanced_search_error));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = unauthenticated_test_client(addr);

        let error = client
            .search(&SearchQuery::builder("identifier:demo-item").build())
            .await
            .unwrap_err();

        assert!(
            matches!(error, InternetArchiveError::InvalidState(message) if message.contains("search backend warming up"))
        );

        server.abort();
    }

    #[tokio::test]
    async fn search_uses_trimmed_raw_json_when_no_message_fields_are_present() {
        async fn advanced_search_error() -> Json<Value> {
            Json(json!({
                "foo": "bar"
            }))
        }

        let app = Router::new().route("/advancedsearch.php", get(advanced_search_error));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = unauthenticated_test_client(addr);

        let error = client
            .search(&SearchQuery::builder("identifier:demo-item").build())
            .await
            .unwrap_err();

        assert!(
            matches!(error, InternetArchiveError::InvalidState(message) if message.contains("\"foo\":\"bar\""))
        );

        server.abort();
    }

    #[tokio::test]
    async fn s3_redirects_do_not_forward_credentials_to_foreign_hosts() {
        async fn initial_upload_redirect() -> (StatusCode, HeaderMap) {
            let trap = TRAP_BASE_URL.get().expect("trap base url");
            let mut headers = HeaderMap::new();
            headers.insert(
                LOCATION,
                HeaderValue::from_str(&format!("{trap}stolen/demo-item/demo.txt")).unwrap(),
            );
            (StatusCode::TEMPORARY_REDIRECT, headers)
        }

        async fn trap_handler(
            State(state): State<std::sync::Arc<StateData>>,
            headers: HeaderMap,
        ) -> StatusCode {
            state.seen_upload_auth.lock().await.push(
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_owned(),
            );
            StatusCode::OK
        }

        static TRAP_BASE_URL: OnceLock<String> = OnceLock::new();

        let trap_state = std::sync::Arc::new(StateData::default());
        let trap_app = Router::new()
            .route("/stolen/demo-item/demo.txt", put(trap_handler))
            .with_state(trap_state.clone());
        let trap_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let trap_addr = trap_listener.local_addr().unwrap();
        let trap_server =
            tokio::spawn(async move { axum::serve(trap_listener, trap_app).await.unwrap() });
        TRAP_BASE_URL
            .set(format!("http://{trap_addr}/"))
            .expect("set trap base url once");

        let origin_state = std::sync::Arc::new(StateData::default());
        let origin_app = Router::new()
            .route("/s3/demo-item/demo.txt", put(initial_upload_redirect))
            .with_state(origin_state);
        let origin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin_listener.local_addr().unwrap();
        let origin_server =
            tokio::spawn(async move { axum::serve(origin_listener, origin_app).await.unwrap() });

        let client = test_client(origin_addr);
        let error = client
            .upload_file(
                &ItemIdentifier::new("demo-item").unwrap(),
                &UploadSpec::from_bytes("demo.txt", b"hello"),
                &UploadOptions::default(),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            InternetArchiveError::InvalidState(message)
                if message.contains("redirected S3 host")
        ));
        assert!(trap_state.seen_upload_auth.lock().await.is_empty());

        origin_server.abort();
        trap_server.abort();
    }

    #[test]
    fn auth_debug_is_redacted_and_env_helpers_work() {
        let auth = Auth::new("access", "secret");
        assert!(format!("{auth:?}").contains("<redacted>"));
    }

    #[tokio::test]
    async fn update_item_metadata_returns_synthetic_success_for_noop_diff() {
        async fn metadata() -> Json<Value> {
            Json(json!({
                "files": [],
                "metadata": {
                    "identifier": "demo-item",
                    "title": "Demo item",
                    "collection": ["opensource"]
                }
            }))
        }

        let app = Router::new().route("/metadata/demo-item", get(metadata));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = InternetArchiveClient::builder()
            .endpoint(Endpoint::custom(
                Url::parse(&format!("http://{addr}/")).unwrap(),
                Url::parse(&format!("http://{addr}/s3/")).unwrap(),
            ))
            .build()
            .unwrap();

        let response = client
            .update_item_metadata(
                &ItemIdentifier::new("demo-item").unwrap(),
                &ItemMetadata::builder()
                    .title("Demo item")
                    .collection("opensource")
                    .build(),
            )
            .await
            .unwrap();
        assert!(response.success);
        assert_eq!(response.task_id, None);

        server.abort();
    }

    #[test]
    fn builder_accessors_env_helpers_and_wait_until_paths_work() {
        static ENV_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

        let _guard = ENV_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap();

        let custom_access = "IA_TEST_ACCESS_KEY";
        let custom_secret = "IA_TEST_SECRET_KEY";
        std::env::set_var(custom_access, "custom-access");
        std::env::set_var(custom_secret, "custom-secret");
        std::env::set_var(Auth::ACCESS_KEY_ENV_VAR, "default-access");
        std::env::set_var(Auth::SECRET_KEY_ENV_VAR, "default-secret");

        let auth = Auth::from_env_vars(custom_access, custom_secret).unwrap();
        assert_eq!(
            auth.authorization_header(),
            "LOW custom-access:custom-secret"
        );
        assert_eq!(
            Auth::from_env().unwrap().authorization_header(),
            "LOW default-access:default-secret"
        );
        assert!(matches!(
            Auth::from_env_vars("MISSING_ACCESS", custom_secret).unwrap_err(),
            InternetArchiveError::EnvVar { name, .. } if name == "MISSING_ACCESS"
        ));

        let poll = PollOptions {
            max_wait: Duration::from_millis(100),
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
        };
        let endpoint = Endpoint::custom(
            Url::parse("http://localhost:3000/archive").unwrap(),
            Url::parse("http://localhost:3000/s3").unwrap(),
        );
        let client = InternetArchiveClient::builder()
            .auth(auth.clone())
            .endpoint(endpoint.clone())
            .user_agent("internetarchive-rs-tests")
            .request_timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(1))
            .poll_options(poll.clone())
            .build()
            .unwrap();

        assert!(client.has_auth());
        assert_eq!(client.endpoint(), &endpoint);
        assert_eq!(client.poll_options(), &poll);
        assert_eq!(client.request_timeout(), Some(Duration::from_secs(5)));
        assert_eq!(client.connect_timeout(), Some(Duration::from_secs(1)));
        assert!(!InternetArchiveClient::new().unwrap().has_auth());
        assert!(InternetArchiveClient::with_auth(auth).unwrap().has_auth());
        assert!(InternetArchiveClient::from_env().unwrap().has_auth());

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut attempts = 0_u8;
        runtime.block_on(async {
            let value = client
                .wait_until("demo wait", || {
                    attempts += 1;
                    async move {
                        if attempts < 3 {
                            Err(InternetArchiveError::ItemNotFound {
                                identifier: "demo-item".to_owned(),
                            })
                        } else {
                            Ok("ready")
                        }
                    }
                })
                .await
                .unwrap();
            assert_eq!(value, "ready");

            let mut transient_attempts = 0_u8;
            let recovered = client
                .wait_until("demo transient", || {
                    transient_attempts += 1;
                    async move {
                        if transient_attempts < 3 {
                            Err(InternetArchiveError::Http {
                                status: StatusCode::BAD_GATEWAY,
                                code: None,
                                message: Some("temporary outage".to_owned()),
                                raw_body: None,
                            })
                        } else {
                            Ok("recovered")
                        }
                    }
                })
                .await
                .unwrap();
            assert_eq!(recovered, "recovered");

            let error = client
                .wait_until("demo error", || async {
                    Err::<(), _>(InternetArchiveError::InvalidState("boom".to_owned()))
                })
                .await
                .unwrap_err();
            assert!(
                matches!(error, InternetArchiveError::InvalidState(message) if message == "boom")
            );

            let timeout = client
                .wait_until("demo timeout", || async {
                    Err::<(), _>(InternetArchiveError::ItemNotFound {
                        identifier: "demo-item".to_owned(),
                    })
                })
                .await
                .unwrap_err();
            assert!(matches!(
                timeout,
                InternetArchiveError::Timeout("demo timeout")
            ));
        });

        std::env::remove_var(custom_access);
        std::env::remove_var(custom_secret);
        std::env::remove_var(Auth::ACCESS_KEY_ENV_VAR);
        std::env::remove_var(Auth::SECRET_KEY_ENV_VAR);
    }

    #[tokio::test]
    async fn missing_auth_and_http_error_paths_are_reported() {
        async fn metadata() -> Json<Value> {
            Json(json!({
                "files": [],
                "metadata": {
                    "identifier": "demo-item",
                    "title": "Old title"
                }
            }))
        }

        async fn blank_metadata() -> &'static str {
            "   "
        }

        async fn non_item_metadata() -> Json<Value> {
            Json(json!({
                "error": "identifier not found",
                "success": false
            }))
        }

        async fn mismatched_metadata() -> Json<Value> {
            Json(json!({
                "files": [],
                "metadata": {
                    "identifier": "other-item",
                    "title": "Wrong item"
                }
            }))
        }

        async fn search_error() -> (StatusCode, Json<Value>) {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error":"search failed","code":"bad_gateway"})),
            )
        }

        async fn metadata_error() -> (StatusCode, Json<Value>) {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"metadata failed","code":"bad_request"})),
            )
        }

        async fn download_error() -> (StatusCode, &'static str) {
            (StatusCode::BAD_GATEWAY, "download failed")
        }

        async fn missing_location() -> StatusCode {
            StatusCode::TEMPORARY_REDIRECT
        }

        async fn failing_upload() -> (StatusCode, &'static str) {
            (StatusCode::INTERNAL_SERVER_ERROR, "upload failed")
        }

        let app = Router::new()
            .route("/metadata/demo-item", get(metadata).post(metadata_error))
            .route("/metadata/blank-item", get(blank_metadata))
            .route("/metadata/non-item", get(non_item_metadata))
            .route("/metadata/mismatched-item", get(mismatched_metadata))
            .route("/advancedsearch.php", get(search_error))
            .route("/download/demo-item/missing.txt", get(download_error))
            .route("/s3/demo-item/missing-location.bin", put(missing_location))
            .route("/s3/demo-item/failing.bin", put(failing_upload));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let unauth = unauthenticated_test_client(addr);
        let auth = test_client(addr);
        let identifier = ItemIdentifier::new("demo-item").unwrap();

        assert!(matches!(
            unauth.get_item_by_str("bad item").await.unwrap_err(),
            InternetArchiveError::Identifier(_)
        ));
        assert!(matches!(
            unauth.check_upload_limit(&identifier).await.unwrap_err(),
            InternetArchiveError::MissingAuth
        ));
        assert!(matches!(
            unauth
                .apply_metadata_patch(
                    &identifier,
                    MetadataTarget::Metadata,
                    &[PatchOperation::replace("/title", "New title")],
                )
                .await
                .unwrap_err(),
            InternetArchiveError::MissingAuth
        ));
        assert!(matches!(
            unauth
                .apply_metadata_changes(
                    &identifier,
                    &[MetadataChange {
                        target: "metadata".to_owned(),
                        patch: vec![PatchOperation::add("/subject/-", "rust")],
                    }],
                )
                .await
                .unwrap_err(),
            InternetArchiveError::MissingAuth
        ));
        assert!(matches!(
            unauth
                .upload_file(
                    &identifier,
                    &UploadSpec::from_bytes("demo.txt", b"hello"),
                    &UploadOptions::default(),
                )
                .await
                .unwrap_err(),
            InternetArchiveError::MissingAuth
        ));
        assert!(matches!(
            unauth
                .create_item(
                    &identifier,
                    &ItemMetadata::builder().title("Demo").build(),
                    &UploadSpec::from_bytes("demo.txt", b"hello"),
                    &UploadOptions::default(),
                )
                .await
                .unwrap_err(),
            InternetArchiveError::MissingAuth
        ));
        assert!(matches!(
            unauth
                .delete_file(&identifier, "demo.txt", &DeleteOptions::default())
                .await
                .unwrap_err(),
            InternetArchiveError::MissingAuth
        ));
        assert!(matches!(
            unauth
                .update_item_metadata(
                    &identifier,
                    &ItemMetadata::builder().title("New title").build(),
                )
                .await
                .unwrap_err(),
            InternetArchiveError::MissingAuth
        ));

        assert!(matches!(
            auth.get_item(&ItemIdentifier::new("blank-item").unwrap())
                .await
                .unwrap_err(),
            InternetArchiveError::ItemNotFound { .. }
        ));
        assert!(matches!(
            auth.get_item(&ItemIdentifier::new("non-item").unwrap())
                .await
                .unwrap_err(),
            InternetArchiveError::ItemNotFound { .. }
        ));
        assert!(matches!(
            auth.get_item(&ItemIdentifier::new("mismatched-item").unwrap())
                .await
                .unwrap_err(),
            InternetArchiveError::ItemNotFound { .. }
        ));
        assert!(matches!(
            auth.search(&SearchQuery::identifier("demo-item"))
                .await
                .unwrap_err(),
            InternetArchiveError::Http { status, .. } if status == StatusCode::BAD_GATEWAY
        ));
        assert!(matches!(
            auth.download_bytes(&identifier, "missing.txt")
                .await
                .unwrap_err(),
            InternetArchiveError::Http { status, .. } if status == StatusCode::BAD_GATEWAY
        ));
        assert!(matches!(
            auth.apply_metadata_patch(
                &identifier,
                MetadataTarget::Metadata,
                &[PatchOperation::replace("/title", "New title")],
            )
            .await
            .unwrap_err(),
            InternetArchiveError::Http { status, .. } if status == StatusCode::BAD_REQUEST
        ));
        assert!(matches!(
            auth.upload_file(
                &identifier,
                &UploadSpec::from_bytes("missing-location.bin", b"hello"),
                &UploadOptions::default(),
            )
            .await
            .unwrap_err(),
            InternetArchiveError::InvalidState(message) if message.contains("missing location")
        ));
        assert!(matches!(
            auth.upload_file(
                &identifier,
                &UploadSpec::from_bytes("failing.bin", b"hello"),
                &UploadOptions::default(),
            )
            .await
            .unwrap_err(),
            InternetArchiveError::Http { status, .. } if status == StatusCode::INTERNAL_SERVER_ERROR
        ));

        server.abort();
    }

    #[cfg(feature = "indicatif")]
    #[tokio::test]
    async fn create_item_with_progress_handles_redirects_and_metadata_remainders() {
        async fn metadata(State(state): State<std::sync::Arc<StateData>>) -> Json<Value> {
            let mut reads = state.metadata_reads.lock().await;
            let payload = if *reads < 2 {
                json!({
                    "files": [{"name": "demo.txt", "size": "5"}],
                    "metadata": {
                        "identifier": "demo-item",
                        "title": "Demo item"
                    }
                })
            } else {
                json!({
                    "files": [{"name": "demo.txt", "size": "5"}],
                    "metadata": {
                        "identifier": "demo-item",
                        "title": "Demo item",
                        "custom": {"nested": true}
                    }
                })
            };
            *reads += 1;
            Json(payload)
        }

        async fn metadata_write(
            State(state): State<std::sync::Arc<StateData>>,
            body: String,
        ) -> (StatusCode, Json<Value>) {
            state.captured_mdapi_body.lock().await.push(body);
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "task_id": 200,
                    "log": "https://catalogd.archive.org/log/200"
                })),
            )
        }

        async fn first_upload() -> (StatusCode, HeaderMap) {
            let mut headers = HeaderMap::new();
            headers.insert(
                LOCATION,
                HeaderValue::from_static("/s3-redirected/demo-item/demo.txt"),
            );
            (StatusCode::TEMPORARY_REDIRECT, headers)
        }

        async fn redirected_upload(body: String) -> StatusCode {
            assert_eq!(body, "hello");
            StatusCode::OK
        }

        let state = std::sync::Arc::new(StateData::default());
        let app = Router::new()
            .route("/metadata/demo-item", get(metadata).post(metadata_write))
            .route("/s3/demo-item/demo.txt", put(first_upload))
            .route("/s3-redirected/demo-item/demo.txt", put(redirected_upload))
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = test_client(addr);
        let identifier = ItemIdentifier::new("demo-item").unwrap();
        let progress = ProgressBar::hidden();

        client
            .create_item_with_progress(
                &identifier,
                &ItemMetadata::builder()
                    .title("Demo item")
                    .extra_json("custom", json!({"nested": true}))
                    .build(),
                &UploadSpec::from_bytes("demo.txt", b"hello"),
                &UploadOptions::default(),
                &progress,
            )
            .await
            .unwrap();

        assert_eq!(progress.length(), Some(5));
        assert_eq!(progress.position(), 5);
        assert!(progress.is_finished());
        assert_eq!(*state.metadata_reads.lock().await, 3);
        assert!(state.captured_mdapi_body.lock().await[0].contains("custom"));

        server.abort();
    }

    #[cfg(feature = "indicatif")]
    #[tokio::test]
    async fn progress_upload_reports_missing_location_foreign_redirect_and_http_errors() {
        async fn missing_location() -> StatusCode {
            StatusCode::TEMPORARY_REDIRECT
        }

        async fn foreign_redirect() -> (StatusCode, HeaderMap) {
            let mut headers = HeaderMap::new();
            headers.insert(
                LOCATION,
                HeaderValue::from_static("http://example.com/stolen/demo-item/foreign.bin"),
            );
            (StatusCode::TEMPORARY_REDIRECT, headers)
        }

        async fn failing_upload() -> (StatusCode, &'static str) {
            (StatusCode::INTERNAL_SERVER_ERROR, "upload failed")
        }

        let app = Router::new()
            .route("/s3/demo-item/missing-location.bin", put(missing_location))
            .route("/s3/demo-item/foreign.bin", put(foreign_redirect))
            .route("/s3/demo-item/failing.bin", put(failing_upload));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = test_client(addr);
        let identifier = ItemIdentifier::new("demo-item").unwrap();

        let missing_progress = ProgressBar::hidden();
        assert!(matches!(
            client
                .upload_file_with_progress(
                    &identifier,
                    &UploadSpec::from_bytes("missing-location.bin", b"hello"),
                    &UploadOptions::default(),
                    &missing_progress,
                )
                .await
                .unwrap_err(),
            InternetArchiveError::InvalidState(message) if message.contains("missing location")
        ));

        let foreign_progress = ProgressBar::hidden();
        assert!(matches!(
            client
                .upload_file_with_progress(
                    &identifier,
                    &UploadSpec::from_bytes("foreign.bin", b"hello"),
                    &UploadOptions::default(),
                    &foreign_progress,
                )
                .await
                .unwrap_err(),
            InternetArchiveError::InvalidState(message)
                if message.contains("redirected S3 host")
        ));

        let failing_progress = ProgressBar::hidden();
        assert!(matches!(
            client
                .upload_file_with_progress(
                    &identifier,
                    &UploadSpec::from_bytes("failing.bin", b"hello"),
                    &UploadOptions::default(),
                    &failing_progress,
                )
                .await
                .unwrap_err(),
            InternetArchiveError::Http { status, .. } if status == StatusCode::INTERNAL_SERVER_ERROR
        ));

        server.abort();
    }

    #[cfg(feature = "indicatif")]
    #[tokio::test]
    async fn redirect_edge_cases_are_reported_for_plain_and_progress_uploads() {
        async fn endless_redirect() -> (StatusCode, HeaderMap) {
            let mut headers = HeaderMap::new();
            headers.insert(LOCATION, HeaderValue::from_static("/s3/demo-item/spin.bin"));
            (StatusCode::TEMPORARY_REDIRECT, headers)
        }

        async fn invalid_location() -> (StatusCode, HeaderMap) {
            let mut headers = HeaderMap::new();
            headers.insert(
                LOCATION,
                HeaderValue::from_bytes(b"/s3/demo-item/\xff.bin").unwrap(),
            );
            (StatusCode::TEMPORARY_REDIRECT, headers)
        }

        let app = Router::new()
            .route("/s3/demo-item/spin.bin", put(endless_redirect))
            .route("/s3/demo-item/bad-location.bin", put(invalid_location));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = test_client(addr);
        let identifier = ItemIdentifier::new("demo-item").unwrap();

        assert!(matches!(
            client
                .upload_file(
                    &identifier,
                    &UploadSpec::from_bytes("spin.bin", b"hello"),
                    &UploadOptions::default(),
                )
                .await
                .unwrap_err(),
            InternetArchiveError::InvalidState(message) if message.contains("too many redirects")
        ));
        assert!(matches!(
            client
                .upload_file(
                    &identifier,
                    &UploadSpec::from_bytes("bad-location.bin", b"hello"),
                    &UploadOptions::default(),
                )
                .await
                .unwrap_err(),
            InternetArchiveError::InvalidState(message)
                if message.contains("not valid UTF-8")
        ));

        let spin_progress = ProgressBar::hidden();
        assert!(matches!(
            client
                .upload_file_with_progress(
                    &identifier,
                    &UploadSpec::from_bytes("spin.bin", b"hello"),
                    &UploadOptions::default(),
                    &spin_progress,
                )
                .await
                .unwrap_err(),
            InternetArchiveError::InvalidState(message) if message.contains("too many redirects")
        ));

        let bad_progress = ProgressBar::hidden();
        assert!(matches!(
            client
                .upload_file_with_progress(
                    &identifier,
                    &UploadSpec::from_bytes("bad-location.bin", b"hello"),
                    &UploadOptions::default(),
                    &bad_progress,
                )
                .await
                .unwrap_err(),
            InternetArchiveError::InvalidState(message)
                if message.contains("not valid UTF-8")
        ));

        server.abort();
    }

    #[cfg(feature = "indicatif")]
    #[tokio::test]
    async fn download_bytes_with_progress_reports_http_errors() {
        async fn download_error() -> (StatusCode, &'static str) {
            (StatusCode::BAD_GATEWAY, "download failed")
        }

        let app = Router::new().route("/download/demo-item/missing.txt", get(download_error));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = test_client(addr);
        let progress = ProgressBar::hidden();

        assert!(matches!(
            client
                .download_bytes_with_progress(&ItemIdentifier::new("demo-item").unwrap(), "missing.txt", &progress)
                .await
                .unwrap_err(),
            InternetArchiveError::Http { status, .. } if status == StatusCode::BAD_GATEWAY
        ));

        server.abort();
    }

    #[cfg(feature = "indicatif")]
    #[tokio::test]
    async fn replayable_body_apply_with_progress_sets_lengths_for_paths_and_bytes() {
        let client = reqwest::Client::new();

        let bytes_progress = ProgressBar::hidden();
        let _bytes_request = ReplayableBody::Bytes(b"hello".to_vec())
            .apply_with_progress(client.put("http://example.com/bytes"), &bytes_progress)
            .await
            .unwrap();
        assert_eq!(bytes_progress.length(), Some(5));
        assert_eq!(bytes_progress.position(), 0);

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("demo.txt");
        tokio::fs::write(&path, b"hello").await.unwrap();

        let path_progress = ProgressBar::hidden();
        let _path_request = ReplayableBody::Path { path, length: 5 }
            .apply_with_progress(client.put("http://example.com/path"), &path_progress)
            .await
            .unwrap();
        assert_eq!(path_progress.length(), Some(5));
        assert_eq!(path_progress.position(), 0);
    }

    #[cfg(feature = "indicatif")]
    #[test]
    fn progress_stream_and_chunked_bytes_stream_cover_poll_paths() {
        let payload = Bytes::from_static(b"hello world");
        let progress = ProgressBar::hidden();
        let mut stream =
            ProgressStream::new(ChunkedBytesStream::new(payload.clone()), progress.clone());
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);

        let first = Pin::new(&mut stream).poll_next(&mut context);
        assert!(matches!(first, Poll::Ready(Some(Ok(ref chunk))) if chunk == &payload));
        assert_eq!(progress.position(), payload.len() as u64);

        let second = Pin::new(&mut stream).poll_next(&mut context);
        assert!(matches!(second, Poll::Ready(None)));
    }
}
