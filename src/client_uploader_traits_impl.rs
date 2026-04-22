use std::future::Future;
use std::path::Path;

use client_uploader_traits::{
    ClientContext, CreatePublication, CreatePublicationRequest, DownloadNamedPublicFile,
    ExistingFileConflictPolicy, ExistingFileConflictPolicyKind, ListResourceFiles,
    MaybeAuthenticatedClient, PublicationOutcome, ReadPublicResource, RepositoryFile,
    RepositoryRecord, SearchPublicResources, SearchResultsLike, UpdatePublication,
    UpdatePublicationRequest, UploadSourceKind, UploadSpecLike,
};

use crate::downloads::ResolvedDownload;
use crate::endpoint::Endpoint;
use crate::error::InternetArchiveError;
use crate::metadata::ItemMetadata;
use crate::model::{Item, ItemFile, SearchDocument, SearchResponse, SearchResultPage};
use crate::poll::PollOptions;
use crate::search::SearchQuery;
use crate::upload::{FileConflictPolicy, UploadSource, UploadSpec};
use crate::workflow::{PublishOutcome, PublishRequest};
use crate::{InternetArchiveClient, ItemIdentifier};

fn into_create_publish_request(
    request: CreatePublicationRequest<ItemIdentifier, ItemMetadata, UploadSpec>,
) -> PublishRequest {
    PublishRequest::new(request.target, request.metadata, request.uploads)
}

fn into_update_publish_request(
    request: UpdatePublicationRequest<ItemIdentifier, ItemMetadata, FileConflictPolicy, UploadSpec>,
) -> PublishRequest {
    let mut publish_request =
        PublishRequest::new(request.resource_id, request.metadata, request.uploads);
    publish_request.conflict_policy = request.policy;
    publish_request
}

impl ClientContext for InternetArchiveClient {
    type Endpoint = Endpoint;
    type PollOptions = PollOptions;
    type Error = InternetArchiveError;

    fn endpoint(&self) -> &Self::Endpoint {
        self.endpoint()
    }

    fn poll_options(&self) -> &Self::PollOptions {
        self.poll_options()
    }

    fn request_timeout(&self) -> Option<std::time::Duration> {
        self.request_timeout()
    }

    fn connect_timeout(&self) -> Option<std::time::Duration> {
        self.connect_timeout()
    }
}

impl MaybeAuthenticatedClient for InternetArchiveClient {
    fn has_auth(&self) -> bool {
        self.has_auth()
    }
}

impl UploadSpecLike for UploadSpec {
    fn filename(&self) -> &str {
        &self.filename
    }

    fn source_kind(&self) -> UploadSourceKind {
        match &self.source {
            UploadSource::Path(_) => UploadSourceKind::Path,
            UploadSource::Bytes(_) => UploadSourceKind::Bytes,
        }
    }

    fn content_length(&self) -> Option<u64> {
        match &self.source {
            UploadSource::Path(path) => std::fs::metadata(path).ok().map(|metadata| metadata.len()),
            UploadSource::Bytes(bytes) => u64::try_from(bytes.len()).ok(),
        }
    }

    fn content_type(&self) -> Option<&str> {
        Some(self.content_type.as_ref())
    }
}

impl RepositoryFile for ItemFile {
    type Id = String;

    fn file_id(&self) -> Option<Self::Id> {
        None
    }

    fn file_name(&self) -> &str {
        &self.name
    }

    fn size_bytes(&self) -> Option<u64> {
        self.size
    }

    fn checksum(&self) -> Option<&str> {
        self.md5
            .as_deref()
            .or(self.sha1.as_deref())
            .or(self.crc32.as_deref())
    }
}

impl RepositoryRecord for Item {
    type Id = ItemIdentifier;
    type File = ItemFile;

    fn resource_id(&self) -> Option<Self::Id> {
        self.identifier()
    }

    fn title(&self) -> Option<&str> {
        self.metadata.title()
    }

    fn files(&self) -> &[Self::File] {
        &self.files
    }
}

impl SearchResultsLike for SearchResultPage {
    type Item = SearchDocument;

    fn items(&self) -> &[Self::Item] {
        &self.docs
    }

    fn total_hits(&self) -> Option<u64> {
        Some(self.num_found)
    }
}

impl SearchResultsLike for SearchResponse {
    type Item = SearchDocument;

    fn items(&self) -> &[Self::Item] {
        &self.response.docs
    }

    fn total_hits(&self) -> Option<u64> {
        Some(self.response.num_found)
    }
}

impl PublicationOutcome for PublishOutcome {
    type PublicResource = Item;

    fn public_resource(&self) -> &Self::PublicResource {
        &self.item
    }

    fn created(&self) -> Option<bool> {
        Some(self.created)
    }
}

impl ExistingFileConflictPolicy for FileConflictPolicy {
    fn kind(&self) -> ExistingFileConflictPolicyKind {
        match self {
            Self::Error => ExistingFileConflictPolicyKind::Error,
            Self::Skip => ExistingFileConflictPolicyKind::Skip,
            Self::Overwrite => ExistingFileConflictPolicyKind::Overwrite,
            Self::OverwriteKeepingHistory => {
                ExistingFileConflictPolicyKind::OverwriteKeepingHistory
            }
        }
    }
}

impl ReadPublicResource for InternetArchiveClient {
    type ResourceId = ItemIdentifier;
    type Resource = Item;

    fn get_public_resource(
        &self,
        id: &Self::ResourceId,
    ) -> impl Future<Output = Result<Self::Resource, Self::Error>> {
        self.get_item(id)
    }
}

impl SearchPublicResources for InternetArchiveClient {
    type Query = SearchQuery;
    type SearchResults = SearchResponse;

    fn search_public_resources(
        &self,
        query: &Self::Query,
    ) -> impl Future<Output = Result<Self::SearchResults, Self::Error>> {
        self.search(query)
    }
}

impl ListResourceFiles for InternetArchiveClient {
    type ResourceId = ItemIdentifier;
    type File = ItemFile;

    async fn list_resource_files(
        &self,
        id: &Self::ResourceId,
    ) -> Result<Vec<Self::File>, Self::Error> {
        Ok(self.get_item(id).await?.files)
    }
}

impl DownloadNamedPublicFile for InternetArchiveClient {
    type ResourceId = ItemIdentifier;
    type Download = ResolvedDownload;

    async fn download_named_public_file_to_path(
        &self,
        id: &Self::ResourceId,
        name: &str,
        path: &Path,
    ) -> Result<Self::Download, Self::Error> {
        self.download_to_path(id, name, path).await?;
        self.resolve_download(id, name)
    }
}

impl CreatePublication for InternetArchiveClient {
    type CreateTarget = ItemIdentifier;
    type Metadata = ItemMetadata;
    type Upload = UploadSpec;
    type Output = PublishOutcome;

    fn create_publication(
        &self,
        request: CreatePublicationRequest<Self::CreateTarget, Self::Metadata, Self::Upload>,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> {
        self.publish_item(into_create_publish_request(request))
    }
}

impl UpdatePublication for InternetArchiveClient {
    type ResourceId = ItemIdentifier;
    type Metadata = ItemMetadata;
    type FilePolicy = FileConflictPolicy;
    type Upload = UploadSpec;
    type Output = PublishOutcome;

    fn update_publication(
        &self,
        request: UpdatePublicationRequest<
            Self::ResourceId,
            Self::Metadata,
            Self::FilePolicy,
            Self::Upload,
        >,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> {
        self.upsert_item(into_update_publish_request(request))
    }
}
