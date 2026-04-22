#![allow(
    dead_code,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::unwrap_used
)]

mod mock_support;

use std::path::Path;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use client_uploader_traits::{
    collect_upload_filenames, find_embedded_record_file_by_name, has_file_named,
    has_upload_filename, validate_upload_filenames, ClientContext, CreatePublication,
    CreatePublicationRequest, DownloadNamedPublicFile, ExistingFileConflictPolicy,
    ExistingFileConflictPolicyKind, ListResourceFiles, MaybeAuthenticatedClient,
    PublicationOutcome, ReadPublicResource, RepositoryFile, RepositoryRecord,
    SearchPublicResources, SearchResultsLike, UpdatePublication, UpdatePublicationRequest,
    UploadSourceKind, UploadSpecLike,
};
use internetarchive_rs::{
    Auth, Endpoint, FileConflictPolicy, InternetArchiveClient, InternetArchiveError, Item,
    ItemFile, ItemIdentifier, ItemMetadata, PollOptions, PublishOutcome, ResolvedDownload,
    SearchQuery, SearchResponse, UploadSpec,
};
use mock_support::{MockInternetArchiveServer, QueuedResponse};

fn assert_shared_context<C>(client: &C, timeout: Duration, connect_timeout: Duration)
where
    C: ClientContext<Endpoint = Endpoint, PollOptions = PollOptions, Error = InternetArchiveError>
        + MaybeAuthenticatedClient,
{
    assert_eq!(client.request_timeout(), Some(timeout));
    assert_eq!(client.connect_timeout(), Some(connect_timeout));
    assert_eq!(
        client.endpoint().archive_base().as_str(),
        "https://archive.org/"
    );
}

async fn read_search_list_and_download<C>(
    client: &C,
    identifier: &ItemIdentifier,
    query: &SearchQuery,
    path: &Path,
) -> Result<(Item, SearchResponse, Vec<ItemFile>, ResolvedDownload), InternetArchiveError>
where
    C: ReadPublicResource<
            ResourceId = ItemIdentifier,
            Resource = Item,
            Error = InternetArchiveError,
        > + SearchPublicResources<
            Query = SearchQuery,
            SearchResults = SearchResponse,
            Error = InternetArchiveError,
        > + ListResourceFiles<
            ResourceId = ItemIdentifier,
            File = ItemFile,
            Error = InternetArchiveError,
        > + DownloadNamedPublicFile<
            ResourceId = ItemIdentifier,
            Download = ResolvedDownload,
            Error = InternetArchiveError,
        >,
{
    let item = client.get_public_resource(identifier).await?;
    let search = client.search_public_resources(query).await?;
    let files = client.list_resource_files(identifier).await?;
    let download = client
        .download_named_public_file_to_path(identifier, "demo.txt", path)
        .await?;
    Ok((item, search, files, download))
}

#[test]
fn inspection_traits_cover_internet_archive_types() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("artifact.bin");
    std::fs::write(&path, b"bytes").unwrap();

    let bytes_upload = UploadSpec::from_bytes("artifact.txt", b"hello");
    let path_upload = UploadSpec::from_path_as(&path, "artifact.bin").unwrap();

    assert_eq!(bytes_upload.filename(), "artifact.txt");
    assert_eq!(bytes_upload.source_kind(), UploadSourceKind::Bytes);
    assert_eq!(bytes_upload.content_length(), Some(5));
    assert_eq!(bytes_upload.content_type(), Some("text/plain"));

    assert_eq!(path_upload.filename(), "artifact.bin");
    assert_eq!(path_upload.source_kind(), UploadSourceKind::Path);
    assert_eq!(path_upload.content_length(), Some(5));
    assert_eq!(path_upload.content_type(), Some("application/octet-stream"));

    validate_upload_filenames([&bytes_upload, &path_upload]).unwrap();
    assert!(has_upload_filename(
        [&bytes_upload, &path_upload],
        "artifact.txt"
    ));

    let filenames = collect_upload_filenames([&bytes_upload, &path_upload]).unwrap();
    assert!(filenames.contains("artifact.txt"));
    assert!(filenames.contains("artifact.bin"));

    let item: Item = serde_json::from_value(serde_json::json!({
        "files": [{
            "name": "demo.txt",
            "size": "5",
            "md5": "abc"
        }],
        "metadata": {
            "identifier": "demo-item",
            "title": "Demo item"
        }
    }))
    .unwrap();

    assert_eq!(item.resource_id().unwrap().as_str(), "demo-item");
    assert_eq!(item.title(), Some("Demo item"));
    assert!(has_file_named(item.files().iter(), "demo.txt"));

    let file = find_embedded_record_file_by_name(&item, "demo.txt").unwrap();
    assert_eq!(file.file_id(), None);
    assert_eq!(file.file_name(), "demo.txt");
    assert_eq!(file.size_bytes(), Some(5));
    assert_eq!(file.checksum(), Some("abc"));

    let search: SearchResponse = serde_json::from_value(serde_json::json!({
        "responseHeader": {"status": 0},
        "response": {
            "numFound": 7,
            "start": 0,
            "docs": [{
                "identifier": "demo-item",
                "title": "Demo item"
            }]
        }
    }))
    .unwrap();

    assert_eq!(search.page_len(), 1);
    assert_eq!(search.total_hits(), Some(7));
    assert_eq!(search.items()[0].title(), Some("Demo item"));

    assert_eq!(
        FileConflictPolicy::OverwriteKeepingHistory.kind(),
        ExistingFileConflictPolicyKind::OverwriteKeepingHistory
    );

    let outcome = PublishOutcome {
        item: item.clone(),
        created: true,
        uploaded_files: vec!["demo.txt".to_owned()],
        skipped_files: Vec::new(),
        metadata_changed: false,
    };
    assert_eq!(PublicationOutcome::created(&outcome), Some(true));
    assert_eq!(
        outcome.public_resource().identifier().unwrap().as_str(),
        "demo-item"
    );
}

#[test]
fn client_context_traits_match_existing_client_configuration() {
    let poll = PollOptions {
        max_wait: Duration::from_secs(7),
        initial_delay: Duration::from_millis(20),
        max_delay: Duration::from_millis(200),
    };
    let timeout = Duration::from_secs(30);
    let connect_timeout = Duration::from_secs(5);

    let client = InternetArchiveClient::builder()
        .poll_options(poll.clone())
        .request_timeout(timeout)
        .connect_timeout(connect_timeout)
        .build()
        .unwrap();
    assert_shared_context(&client, timeout, connect_timeout);
    assert_eq!(client.poll_options(), &poll);
    assert!(!client.has_auth());
    assert!(!MaybeAuthenticatedClient::has_auth(&client));

    let auth_client = InternetArchiveClient::builder()
        .auth(Auth::new("access", "secret"))
        .build()
        .unwrap();
    assert!(MaybeAuthenticatedClient::has_auth(&auth_client));
}

#[tokio::test]
async fn resource_capability_traits_route_to_existing_client_methods() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5", "md5": "abc"}],
            "metadata": {"identifier": "demo-item", "title": "Demo item"}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/advancedsearch.php",
        StatusCode::OK,
        serde_json::json!({
            "responseHeader": {"status": 0},
            "response": {
                "numFound": 1,
                "start": 0,
                "docs": [{"identifier": "demo-item", "title": "Demo item"}]
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5", "md5": "abc"}],
            "metadata": {"identifier": "demo-item", "title": "Demo item"}
        }),
    );
    server.enqueue(
        Method::GET,
        "/download/demo-item/demo.txt",
        QueuedResponse::bytes(StatusCode::OK, Vec::new(), b"hello".to_vec()),
    );

    let tempdir = tempfile::tempdir().unwrap();
    let download_path = tempdir.path().join("download.txt");
    let (item, search, files, download) = read_search_list_and_download(
        &client,
        &identifier,
        &SearchQuery::identifier("demo-item"),
        &download_path,
    )
    .await
    .unwrap();

    assert_eq!(item.identifier().unwrap(), identifier);
    assert_eq!(search.total_hits(), Some(1));
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_name(), "demo.txt");
    assert_eq!(download.filename, "demo.txt");
    assert!(download
        .url
        .as_str()
        .ends_with("/download/demo-item/demo.txt"));
    assert_eq!(tokio::fs::read(&download_path).await.unwrap(), b"hello");
}

#[tokio::test]
async fn publication_capability_traits_route_to_existing_workflows() {
    let create_server = MockInternetArchiveServer::start().await;
    let create_client = create_server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    create_server.enqueue(
        Method::GET,
        "/metadata/demo-item",
        QueuedResponse::bytes(
            StatusCode::OK,
            vec![("content-type".into(), "application/json".into())],
            b"[]".to_vec(),
        ),
    );
    create_server.enqueue(
        Method::PUT,
        "/s3/demo-item/demo.txt",
        QueuedResponse::bytes(
            StatusCode::TEMPORARY_REDIRECT,
            vec![("location".into(), "/s3-direct/demo-item/demo.txt".into())],
            Vec::new(),
        ),
    );
    create_server.enqueue(
        Method::PUT,
        "/s3-direct/demo-item/demo.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    for _ in 0..2 {
        create_server.enqueue_json(
            Method::GET,
            "/metadata/demo-item",
            StatusCode::OK,
            serde_json::json!({
                "files": [{"name": "demo.txt", "size": "5"}],
                "metadata": {"identifier": "demo-item", "title": "Demo item"}
            }),
        );
    }

    let created = CreatePublication::create_publication(
        &create_client,
        CreatePublicationRequest::new(
            identifier.clone(),
            ItemMetadata::builder().title("Demo item").build(),
            vec![UploadSpec::from_bytes("demo.txt", b"hello")],
        ),
    )
    .await
    .unwrap();
    assert_eq!(PublicationOutcome::created(&created), Some(true));
    assert_eq!(
        created.public_resource().identifier().unwrap().as_str(),
        "demo-item"
    );

    let create_requests = create_server.requests();
    assert!(create_requests
        .iter()
        .any(|request| request.method == Method::PUT && request.path == "/s3/demo-item/demo.txt"));

    let update_server = MockInternetArchiveServer::start().await;
    let update_client = update_server.client();

    update_server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    update_server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    update_server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 88,
            "log": "https://catalogd.archive.org/log/88"
        }),
    );
    update_server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "New title"}
        }),
    );

    let updated = UpdatePublication::update_publication(
        &update_client,
        UpdatePublicationRequest::new(
            identifier.clone(),
            ItemMetadata::builder().title("New title").build(),
            FileConflictPolicy::Skip,
            vec![UploadSpec::from_bytes("demo.txt", b"hello")],
        ),
    )
    .await
    .unwrap();

    assert_eq!(PublicationOutcome::created(&updated), Some(false));
    assert_eq!(updated.skipped_files, vec!["demo.txt".to_owned()]);

    let update_requests = update_server.requests();
    assert!(!update_requests
        .iter()
        .any(|request| request.method == Method::PUT));
    assert!(update_requests
        .iter()
        .any(|request| request.method == Method::POST && request.path == "/metadata/demo-item"));
}
