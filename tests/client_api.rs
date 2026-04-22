#![allow(clippy::expect_used, clippy::missing_panics_doc, clippy::unwrap_used)]

mod mock_support;

use axum::http::{Method, StatusCode};
#[cfg(feature = "indicatif")]
use internetarchive_rs::indicatif::ProgressBar;
use internetarchive_rs::{
    DeleteOptions, DownloadTarget, Endpoint, FileConflictPolicy, ItemIdentifier, ItemMetadata,
    MediaType, MetadataChange, MetadataTarget, MetadataValue, PatchOperation, SearchQuery,
    SearchSort, SortDirection, TaskId, UploadOptions, UploadSpec,
};
use mock_support::{MockInternetArchiveServer, QueuedResponse};

#[tokio::test]
async fn low_level_client_methods_cover_success_paths() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    let tempdir = tempfile::tempdir().unwrap();
    let extra_path = tempdir.path().join("extra.bin");
    tokio::fs::write(&extra_path, b"extra-body").await.unwrap();

    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [],
            "metadata": {
                "identifier": "demo-item",
                "title": "Demo item"
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/advancedsearch.php",
        StatusCode::OK,
        serde_json::json!({
            "responseHeader": {
                "status": 0,
                "QTime": 2,
                "params": {"query": "identifier:demo-item"}
            },
            "response": {
                "numFound": 1,
                "start": 0,
                "docs": [{"identifier": "demo-item", "title": "Demo item"}]
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/s3/",
        StatusCode::OK,
        serde_json::json!({
            "bucket": "demo-item",
            "accesskey": "access",
            "over_limit": 0
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 100,
            "log": "https://catalogd.archive.org/log/100"
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 101,
            "log": "https://catalogd.archive.org/log/101"
        }),
    );
    server.enqueue(
        Method::PUT,
        "/s3/demo-item/seed.txt",
        QueuedResponse::bytes(
            StatusCode::TEMPORARY_REDIRECT,
            vec![(
                String::from("location"),
                String::from("/s3-direct/demo-item/seed.txt"),
            )],
            Vec::new(),
        ),
    );
    server.enqueue(
        Method::PUT,
        "/s3-direct/demo-item/seed.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    server.enqueue(
        Method::PUT,
        "/s3/demo-item/extra.bin",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    server.enqueue(
        Method::DELETE,
        "/s3/demo-item/extra.bin",
        QueuedResponse::text(StatusCode::NO_CONTENT, ""),
    );
    server.enqueue(
        Method::GET,
        "/download/demo-item/seed.txt",
        QueuedResponse::bytes(StatusCode::OK, Vec::new(), b"seed-body".to_vec()),
    );
    server.enqueue(
        Method::GET,
        "/download/demo-item/extra.bin",
        QueuedResponse::bytes(StatusCode::OK, Vec::new(), b"extra-body".to_vec()),
    );

    let item = client.get_item_by_str("demo-item").await.unwrap();
    assert_eq!(item.metadata.title(), Some("Demo item"));

    let search = client
        .search(
            &SearchQuery::builder("identifier:demo-item")
                .field("identifier")
                .extra_param("mediatype", "texts")
                .build(),
        )
        .await
        .unwrap();
    assert_eq!(
        search.response.docs[0].identifier().unwrap().as_str(),
        "demo-item"
    );

    let limit = client.check_upload_limit(&identifier).await.unwrap();
    assert_eq!(limit.bucket, "demo-item");
    assert_eq!(limit.over_limit, 0);

    let patch_response = client
        .apply_metadata_patch(
            &identifier,
            MetadataTarget::Metadata,
            &[PatchOperation::replace("/title", "Updated title")],
        )
        .await
        .unwrap();
    assert_eq!(patch_response.task_id.unwrap().0, 100);

    let changes_response = client
        .apply_metadata_changes(
            &identifier,
            &[MetadataChange::new(
                &MetadataTarget::File(String::from("seed.txt")),
                vec![PatchOperation::add("/description", "hello")],
            )],
        )
        .await
        .unwrap();
    assert_eq!(changes_response.task_id.unwrap().0, 101);

    client
        .create_item(
            &identifier,
            &ItemMetadata::builder()
                .mediatype(MediaType::Texts)
                .title("Seed file")
                .collection("opensource")
                .creator("Codex")
                .subject("rust")
                .language("eng")
                .license_url("https://creativecommons.org/licenses/by/4.0/")
                .build(),
            &UploadSpec::from_bytes("seed.txt", b"seed-body"),
            &UploadOptions::default(),
        )
        .await
        .unwrap();

    let upload_options = UploadOptions {
        skip_derive: true,
        keep_old_version: true,
        interactive_priority: true,
        size_hint: Some(12_345),
    };

    client
        .upload_file(
            &identifier,
            &UploadSpec::from_path(&extra_path).unwrap(),
            &upload_options,
        )
        .await
        .unwrap();

    let delete_options = DeleteOptions {
        cascade_delete: true,
        keep_old_version: true,
    };
    client
        .delete_file(&identifier, "extra.bin", &delete_options)
        .await
        .unwrap();

    let resolved = client.resolve_download(&identifier, "seed.txt").unwrap();
    assert_eq!(resolved.identifier, identifier);
    assert!(resolved
        .url
        .as_str()
        .ends_with("/download/demo-item/seed.txt"));

    assert_eq!(
        client
            .download_bytes(&resolved.identifier, "seed.txt")
            .await
            .unwrap(),
        "seed-body"
    );

    let downloaded_path = tempdir.path().join("downloaded.bin");
    client
        .download_to_path(&resolved.identifier, "extra.bin", &downloaded_path)
        .await
        .unwrap();
    assert_eq!(
        tokio::fs::read(&downloaded_path).await.unwrap(),
        b"extra-body".to_vec()
    );

    let requests = server.requests();

    let search_request = requests
        .iter()
        .find(|request| request.method == Method::GET && request.path == "/advancedsearch.php")
        .unwrap();
    assert!(search_request
        .query
        .as_deref()
        .unwrap()
        .contains("mediatype=texts"));

    let limit_request = requests
        .iter()
        .find(|request| request.method == Method::GET && request.path == "/s3/")
        .unwrap();
    let limit_query = limit_request.query.as_deref().unwrap();
    assert!(limit_query.contains("check_limit=1"));
    assert!(limit_query.contains("accesskey=access"));
    assert!(limit_query.contains("bucket=demo-item"));

    let metadata_posts = requests
        .iter()
        .filter(|request| request.method == Method::POST && request.path == "/metadata/demo-item")
        .collect::<Vec<_>>();
    let patch_body = String::from_utf8(metadata_posts[0].body.clone()).unwrap();
    assert!(patch_body.contains("-target=metadata"));
    let changes_body = String::from_utf8(metadata_posts[1].body.clone()).unwrap();
    assert!(changes_body.contains("-changes="));

    let create_request = requests
        .iter()
        .find(|request| request.method == Method::PUT && request.path == "/s3/demo-item/seed.txt")
        .unwrap();
    assert_eq!(
        create_request
            .headers
            .get("x-archive-auto-make-bucket")
            .unwrap(),
        "1"
    );
    assert_eq!(
        create_request.headers.get("x-archive-meta-title").unwrap(),
        "Seed file"
    );
    assert_eq!(
        create_request
            .headers
            .get("x-archive-meta-licenseurl")
            .unwrap(),
        "https://creativecommons.org/licenses/by/4.0/"
    );

    let upload_request = requests
        .iter()
        .find(|request| request.method == Method::PUT && request.path == "/s3/demo-item/extra.bin")
        .unwrap();
    assert_eq!(
        upload_request
            .headers
            .get("x-archive-queue-derive")
            .unwrap(),
        "0"
    );
    assert_eq!(
        upload_request
            .headers
            .get("x-archive-keep-old-version")
            .unwrap(),
        "1"
    );
    assert_eq!(
        upload_request
            .headers
            .get("x-archive-interactive-priority")
            .unwrap(),
        "1"
    );
    assert_eq!(
        upload_request.headers.get("x-archive-size-hint").unwrap(),
        "12345"
    );
    assert_eq!(upload_request.body, b"extra-body".to_vec());

    let delete_request = requests
        .iter()
        .find(|request| {
            request.method == Method::DELETE && request.path == "/s3/demo-item/extra.bin"
        })
        .unwrap();
    assert_eq!(
        delete_request
            .headers
            .get("x-archive-cascade-delete")
            .unwrap(),
        "1"
    );
    assert_eq!(
        delete_request
            .headers
            .get("x-archive-keep-old-version")
            .unwrap(),
        "1"
    );
}

#[cfg(feature = "indicatif")]
#[tokio::test]
async fn progress_methods_update_indicatif_bars() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    let tempdir = tempfile::tempdir().unwrap();
    let upload_path = tempdir.path().join("progress.bin");
    let download_path = tempdir.path().join("downloaded.bin");
    let upload_body = b"hello progress".to_vec();
    let download_body = b"download progress".to_vec();
    tokio::fs::write(&upload_path, &upload_body).await.unwrap();

    server.enqueue(
        Method::PUT,
        "/s3/demo-item/progress.bin",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    server.enqueue(
        Method::GET,
        "/download/demo-item/progress.bin",
        QueuedResponse::bytes(
            StatusCode::OK,
            vec![(
                String::from("content-length"),
                download_body.len().to_string(),
            )],
            download_body.clone(),
        ),
    );
    server.enqueue(
        Method::GET,
        "/download/demo-item/progress.bin",
        QueuedResponse::bytes(
            StatusCode::OK,
            vec![(
                String::from("content-length"),
                download_body.len().to_string(),
            )],
            download_body.clone(),
        ),
    );

    let upload_bar = ProgressBar::hidden();
    client
        .upload_file_with_progress(
            &identifier,
            &UploadSpec::from_path(&upload_path).unwrap(),
            &UploadOptions::default(),
            &upload_bar,
        )
        .await
        .unwrap();
    assert_eq!(upload_bar.length(), Some(upload_body.len() as u64));
    assert_eq!(upload_bar.position(), upload_body.len() as u64);
    assert!(upload_bar.is_finished());

    let download_bar = ProgressBar::hidden();
    let bytes = client
        .download_bytes_with_progress(&identifier, "progress.bin", &download_bar)
        .await
        .unwrap();
    assert_eq!(bytes, download_body);
    assert_eq!(download_bar.length(), Some(download_body.len() as u64));
    assert_eq!(download_bar.position(), download_body.len() as u64);
    assert!(download_bar.is_finished());

    let path_bar = ProgressBar::hidden();
    client
        .download_to_path_with_progress(&identifier, "progress.bin", &download_path, &path_bar)
        .await
        .unwrap();
    assert_eq!(
        tokio::fs::read(&download_path).await.unwrap(),
        download_body
    );
    assert_eq!(path_bar.length(), Some(download_body.len() as u64));
    assert_eq!(path_bar.position(), download_body.len() as u64);
    assert!(path_bar.is_finished());
}

#[tokio::test]
async fn create_item_patches_non_header_metadata() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue(
        Method::PUT,
        "/s3/demo-item/demo.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {
                "identifier": "demo-item",
                "title": "Demo item"
            }
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {
                "identifier": "demo-item",
                "title": "Demo item"
            }
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 201,
            "log": "https://catalogd.archive.org/log/201"
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {
                "identifier": "demo-item",
                "title": "Demo item",
                "custom": {"nested": true}
            }
        }),
    );

    client
        .create_item(
            &identifier,
            &ItemMetadata::builder()
                .title("Demo item")
                .extra_json("custom", serde_json::json!({"nested": true}))
                .build(),
            &UploadSpec::from_bytes("demo.txt", b"hello"),
            &UploadOptions::default(),
        )
        .await
        .unwrap();

    let requests = server.requests();
    let patch = requests
        .iter()
        .find(|request| request.method == Method::POST && request.path == "/metadata/demo-item")
        .unwrap();
    let body = String::from_utf8(patch.body.clone()).unwrap();
    assert!(body.contains("-target=metadata"));
    assert!(body.contains("custom"));
}

#[tokio::test]
async fn workflow_policies_cover_error_and_history_paths() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    let publish_error = client
        .publish_item(internetarchive_rs::PublishRequest::new(
            identifier.clone(),
            ItemMetadata::builder().title("Old title").build(),
            vec![UploadSpec::from_bytes("demo.txt", b"hello")],
        ))
        .await
        .unwrap_err();
    assert!(format!("{publish_error}").contains("already exists"));

    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    let mut conflict_request = internetarchive_rs::PublishRequest::new(
        identifier.clone(),
        ItemMetadata::builder().title("New title").build(),
        vec![UploadSpec::from_bytes("demo.txt", b"hello")],
    );
    conflict_request.conflict_policy = FileConflictPolicy::Error;
    let conflict_error = client.upsert_item(conflict_request).await.unwrap_err();
    assert!(format!("{conflict_error}").contains("selected policy forbids overwrite"));

    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    server.enqueue(
        Method::PUT,
        "/s3/demo-item/demo.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 202,
            "log": "https://catalogd.archive.org/log/202"
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "New title"}
        }),
    );

    let mut history_request = internetarchive_rs::PublishRequest::new(
        identifier.clone(),
        ItemMetadata::builder().title("New title").build(),
        vec![UploadSpec::from_bytes("demo.txt", b"hello")],
    );
    history_request.conflict_policy = FileConflictPolicy::OverwriteKeepingHistory;
    let outcome = client.upsert_item(history_request).await.unwrap();
    assert!(!outcome.created);
    assert_eq!(outcome.uploaded_files, vec![String::from("demo.txt")]);
    assert!(outcome.metadata_changed);

    let overwrite_request = server
        .requests()
        .into_iter()
        .find(|request| request.method == Method::PUT && request.path == "/s3/demo-item/demo.txt")
        .unwrap();
    assert_eq!(
        overwrite_request
            .headers
            .get("x-archive-keep-old-version")
            .unwrap(),
        "1"
    );
}

#[tokio::test]
async fn workflow_lookup_errors_are_propagated() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue(
        Method::GET,
        "/metadata/demo-item",
        QueuedResponse::text(StatusCode::BAD_GATEWAY, "lookup failed"),
    );
    let publish_error = client
        .publish_item(internetarchive_rs::PublishRequest::new(
            identifier.clone(),
            ItemMetadata::builder().title("Demo").build(),
            vec![UploadSpec::from_bytes("demo.txt", b"hello")],
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        publish_error,
        internetarchive_rs::InternetArchiveError::Http { status, .. }
            if status == StatusCode::BAD_GATEWAY
    ));

    server.enqueue(
        Method::GET,
        "/metadata/demo-item",
        QueuedResponse::text(StatusCode::BAD_GATEWAY, "lookup failed"),
    );
    let upsert_error = client
        .upsert_item(internetarchive_rs::PublishRequest::new(
            identifier,
            ItemMetadata::builder().title("Demo").build(),
            vec![UploadSpec::from_bytes("demo.txt", b"hello")],
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        upsert_error,
        internetarchive_rs::InternetArchiveError::Http { status, .. }
            if status == StatusCode::BAD_GATEWAY
    ));
}

#[tokio::test]
async fn workflow_outcome_waits_for_uploaded_file_visibility() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue(
        Method::GET,
        "/metadata/demo-item",
        QueuedResponse::bytes(
            StatusCode::OK,
            vec![(
                String::from("content-type"),
                String::from("application/json"),
            )],
            b"[]".to_vec(),
        ),
    );
    server.enqueue(
        Method::PUT,
        "/s3/demo-item/demo.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [],
            "metadata": {"identifier": "demo-item", "title": "Demo item"}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [],
            "metadata": {"identifier": "demo-item", "title": "Demo item"}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Demo item"}
        }),
    );

    let outcome = client
        .publish_item(internetarchive_rs::PublishRequest::new(
            identifier,
            ItemMetadata::builder().title("Demo item").build(),
            vec![UploadSpec::from_bytes("demo.txt", b"hello")],
        ))
        .await
        .unwrap();

    assert!(outcome.item.file("demo.txt").is_some());
}

#[tokio::test]
async fn workflow_outcome_waits_for_metadata_visibility() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 301,
            "log": "https://catalogd.archive.org/log/301"
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "New title"}
        }),
    );

    let mut request = internetarchive_rs::PublishRequest::new(
        identifier,
        ItemMetadata::builder().title("New title").build(),
        vec![UploadSpec::from_bytes("demo.txt", b"hello")],
    );
    request.conflict_policy = FileConflictPolicy::Skip;

    let outcome = client.upsert_item(request).await.unwrap();
    assert_eq!(outcome.item.metadata.title(), Some("New title"));
}

#[tokio::test]
async fn workflow_projection_wait_retries_transient_server_errors() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Old title"}
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 302,
            "log": "https://catalogd.archive.org/log/302"
        }),
    );
    server.enqueue(
        Method::GET,
        "/metadata/demo-item",
        QueuedResponse::text(StatusCode::BAD_GATEWAY, "temporary outage"),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "New title"}
        }),
    );

    let mut request = internetarchive_rs::PublishRequest::new(
        identifier,
        ItemMetadata::builder().title("New title").build(),
        vec![UploadSpec::from_bytes("demo.txt", b"hello")],
    );
    request.conflict_policy = FileConflictPolicy::Skip;

    let outcome = client.upsert_item(request).await.unwrap();
    assert_eq!(outcome.item.metadata.title(), Some("New title"));
}

#[tokio::test]
async fn workflow_default_overwrite_and_multi_upload_creation_paths_are_covered() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Demo"}
        }),
    );
    server.enqueue(
        Method::PUT,
        "/s3/demo-item/demo.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Demo"}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {"identifier": "demo-item", "title": "Demo"}
        }),
    );

    let overwrite_outcome = client
        .upsert_item(internetarchive_rs::PublishRequest::new(
            identifier.clone(),
            ItemMetadata::builder().title("Demo").build(),
            vec![UploadSpec::from_bytes("demo.txt", b"hello")],
        ))
        .await
        .unwrap();
    assert_eq!(
        overwrite_outcome.uploaded_files,
        vec![String::from("demo.txt")]
    );
    assert!(!overwrite_outcome.metadata_changed);

    let created_identifier = ItemIdentifier::new("demo-multi").unwrap();
    server.enqueue(
        Method::GET,
        "/metadata/demo-multi",
        QueuedResponse::bytes(
            StatusCode::OK,
            vec![(
                String::from("content-type"),
                String::from("application/json"),
            )],
            b"[]".to_vec(),
        ),
    );
    server.enqueue(
        Method::PUT,
        "/s3/demo-multi/first.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    server.enqueue(
        Method::PUT,
        "/s3/demo-multi/second.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-multi",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "first.txt", "size": "5"}, {"name": "second.txt", "size": "5"}],
            "metadata": {"identifier": "demo-multi", "title": "Demo multi"}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-multi",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "first.txt", "size": "5"}, {"name": "second.txt", "size": "5"}],
            "metadata": {"identifier": "demo-multi", "title": "Demo multi"}
        }),
    );
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-multi",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "first.txt", "size": "5"}, {"name": "second.txt", "size": "5"}],
            "metadata": {"identifier": "demo-multi", "title": "Demo multi"}
        }),
    );

    let created_outcome = client
        .publish_item(internetarchive_rs::PublishRequest::new(
            created_identifier,
            ItemMetadata::builder().title("Demo multi").build(),
            vec![
                UploadSpec::from_bytes("first.txt", b"hello"),
                UploadSpec::from_bytes("second.txt", b"world"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(
        created_outcome.uploaded_files,
        vec![String::from("first.txt"), String::from("second.txt")]
    );
}

#[test]
fn public_helper_apis_cover_remaining_daily_surface() {
    let default_endpoint = Endpoint::default();
    assert_eq!(
        default_endpoint.archive_base().as_str(),
        "https://archive.org/"
    );
    assert_eq!(
        default_endpoint.s3_base().as_str(),
        "https://s3.us.archive.org/"
    );

    let custom_endpoint = Endpoint::custom(
        url::Url::parse("https://archive.org/root").unwrap(),
        url::Url::parse("https://s3.us.archive.org/custom").unwrap(),
    );
    assert_eq!(
        custom_endpoint.metadata_url("demo-item").unwrap().as_str(),
        "https://archive.org/root/metadata/demo-item"
    );
    assert_eq!(
        custom_endpoint.search_url().unwrap().as_str(),
        "https://archive.org/root/advancedsearch.php"
    );
    assert_eq!(
        custom_endpoint.details_url("demo-item").unwrap().as_str(),
        "https://archive.org/root/details/demo-item"
    );
    assert_eq!(
        custom_endpoint
            .download_url("demo-item", "demo.txt")
            .unwrap()
            .as_str(),
        "https://archive.org/root/download/demo-item/demo.txt"
    );
    assert_eq!(
        custom_endpoint.s3_item_url("demo-item").unwrap().as_str(),
        "https://s3.us.archive.org/custom/demo-item"
    );
    assert_eq!(
        custom_endpoint
            .s3_object_url("demo-item", "demo.txt")
            .unwrap()
            .as_str(),
        "https://s3.us.archive.org/custom/demo-item/demo.txt"
    );
    let limit_url = custom_endpoint
        .s3_limit_check_url("demo-access", "demo-item")
        .unwrap();
    assert!(limit_url.as_str().contains("check_limit=1"));
    assert!(limit_url.as_str().contains("accesskey=demo-access"));
    assert!(limit_url.as_str().contains("bucket=demo-item"));

    let client = internetarchive_rs::InternetArchiveClient::builder()
        .endpoint(custom_endpoint.clone())
        .build()
        .unwrap();
    assert_eq!(client.endpoint(), &custom_endpoint);

    let identifier = ItemIdentifier::new("demo-item").unwrap();
    let resolved = client.resolve_download(&identifier, "demo.txt").unwrap();
    assert_eq!(resolved.identifier, identifier);
    assert_eq!(resolved.filename, "demo.txt");
    assert_eq!(
        resolved.url.as_str(),
        "https://archive.org/root/download/demo-item/demo.txt"
    );

    let raw_query = SearchQuery::new("identifier:demo-item");
    assert_eq!(raw_query.query(), "identifier:demo-item");
    assert!(raw_query.fields().is_empty());

    let identifier_query = SearchQuery::identifier("demo-item");
    assert_eq!(identifier_query.query(), "identifier:demo-item");
    let sort = SearchSort::new("publicdate", SortDirection::Desc);
    assert_eq!(sort.field, "publicdate");
    assert_eq!(sort.direction, SortDirection::Desc);
    let query = SearchQuery::builder(identifier_query.query())
        .field("identifier")
        .field("title")
        .rows(5)
        .page(2)
        .sort(sort.field.clone(), sort.direction)
        .extra_param("mediatype", "texts")
        .build();
    assert_eq!(
        query.fields(),
        &["identifier".to_owned(), "title".to_owned()]
    );
    let search_url = query
        .into_url(default_endpoint.search_url().unwrap())
        .unwrap();
    assert!(search_url.as_str().contains("q=identifier%3Ademo-item"));
    assert!(search_url.as_str().contains("rows=5"));
    assert!(search_url.as_str().contains("page=2"));
    assert!(search_url.as_str().contains("sort%5B%5D=publicdate+desc"));
    assert!(search_url.as_str().contains("mediatype=texts"));

    let file_change = MetadataChange::new(
        &MetadataTarget::File(String::from("demo.txt")),
        vec![PatchOperation::replace("/description", "Demo file")],
    );
    assert_eq!(file_change.target, "files/demo.txt");
    let user_json_change = MetadataChange::new(
        &MetadataTarget::UserJson(String::from("extra.json")),
        vec![
            PatchOperation::test("/version", 1),
            PatchOperation::add("/tags/-", "rust"),
            PatchOperation::Remove {
                path: String::from("/obsolete"),
            },
            PatchOperation::RemoveFirst {
                path: String::from("/tags/-"),
                value: serde_json::json!("legacy"),
            },
            PatchOperation::RemoveAll {
                path: String::from("/tags/-"),
                value: serde_json::json!("dup"),
            },
        ],
    );
    let root_change = MetadataChange::new(
        &MetadataTarget::RootUserJson,
        vec![PatchOperation::add("/alive", true)],
    );
    assert_eq!(root_change.target, "");
    let serialized_change = serde_json::to_string(&user_json_change).unwrap();
    assert!(serialized_change.contains("extra.json"));
    assert!(serialized_change.contains("\"remove-first\""));
    assert!(serialized_change.contains("\"remove-all\""));

    assert_eq!(
        MetadataValue::from(String::from("demo")),
        MetadataValue::Text(String::from("demo"))
    );
    assert_eq!(
        MetadataValue::from(vec!["a", "b"]),
        MetadataValue::TextList(vec![String::from("a"), String::from("b")])
    );

    let bytes_target = DownloadTarget::Bytes;
    let path_target = DownloadTarget::Path(std::path::PathBuf::from("/tmp/demo.txt"));
    assert!(matches!(bytes_target, DownloadTarget::Bytes));
    assert!(matches!(path_target, DownloadTarget::Path(_)));

    let task_id = TaskId::from(7_u64);
    assert_eq!(task_id.to_string(), "7");
}
