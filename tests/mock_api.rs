#![allow(clippy::expect_used, clippy::missing_panics_doc, clippy::unwrap_used)]

mod mock_support;

use axum::http::{Method, StatusCode};
use internetarchive_rs::{
    FileConflictPolicy, ItemIdentifier, ItemMetadata, MediaType, PublishRequest, UploadSpec,
};
use mock_support::{MockInternetArchiveServer, QueuedResponse};

#[tokio::test]
async fn publish_item_creates_item_and_patches_non_header_metadata() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue(
        Method::GET,
        "/metadata/demo-item",
        QueuedResponse::bytes(
            StatusCode::OK,
            vec![("content-type".into(), "application/json".into())],
            b"[]".to_vec(),
        ),
    );
    server.enqueue(
        Method::PUT,
        "/s3/demo-item/demo.txt",
        QueuedResponse::bytes(
            StatusCode::TEMPORARY_REDIRECT,
            vec![("location".into(), "/s3-direct/demo-item/demo.txt".into())],
            Vec::new(),
        ),
    );
    server.enqueue(
        Method::PUT,
        "/s3-direct/demo-item/demo.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );
    for _ in 0..2 {
        server.enqueue_json(
            Method::GET,
            "/metadata/demo-item",
            StatusCode::OK,
            serde_json::json!({
                "files": [{"name": "demo.txt", "size": "5"}],
                "metadata": {
                    "identifier": "demo-item",
                    "mediatype": "texts",
                    "title": "Demo item",
                    "collection": ["opensource"]
                }
            }),
        );
    }
    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 77,
            "log": "https://catalogd.archive.org/log/77"
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
                "mediatype": "texts",
                "title": "Demo item",
                "collection": ["opensource"],
                "custom": {"nested": true}
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
                "mediatype": "texts",
                "title": "Demo item",
                "collection": ["opensource"],
                "custom": {"nested": true}
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
                "mediatype": "texts",
                "title": "Demo item",
                "collection": ["opensource"],
                "custom": {"nested": true}
            }
        }),
    );

    let request = PublishRequest::new(
        identifier.clone(),
        ItemMetadata::builder()
            .mediatype(MediaType::Texts)
            .title("Demo item")
            .collection("opensource")
            .extra_json("custom", serde_json::json!({"nested": true}))
            .build(),
        vec![UploadSpec::from_bytes("demo.txt", b"hello".to_vec())],
    );
    let outcome = client.publish_item(request).await.unwrap();

    assert!(outcome.created);
    assert_eq!(outcome.uploaded_files, vec!["demo.txt".to_owned()]);
    assert!(outcome.metadata_changed);
    assert_eq!(outcome.item.identifier().unwrap(), identifier);

    let requests = server.requests();
    let upload = requests
        .iter()
        .find(|request| request.method == Method::PUT && request.path == "/s3/demo-item/demo.txt")
        .unwrap();
    assert_eq!(
        upload.headers.get("authorization").unwrap(),
        "LOW access:secret"
    );
    assert_eq!(
        upload.headers.get("x-archive-auto-make-bucket").unwrap(),
        "1"
    );
    assert_eq!(
        upload.headers.get("x-archive-meta-title").unwrap(),
        "Demo item"
    );

    let patch = requests
        .iter()
        .find(|request| request.method == Method::POST && request.path == "/metadata/demo-item")
        .unwrap();
    let body = String::from_utf8(patch.body.clone()).unwrap();
    assert!(body.contains("-target=metadata"));
    assert!(body.contains("custom"));
}

#[tokio::test]
async fn upsert_item_skip_policy_skips_existing_uploads_and_updates_metadata() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {
                "identifier": "demo-item",
                "title": "Old title"
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
                "title": "Old title"
            }
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 88,
            "log": "https://catalogd.archive.org/log/88"
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
                "title": "New title"
            }
        }),
    );

    let mut request = PublishRequest::new(
        identifier.clone(),
        ItemMetadata::builder().title("New title").build(),
        vec![UploadSpec::from_bytes("demo.txt", b"hello".to_vec())],
    );
    request.conflict_policy = FileConflictPolicy::Skip;

    let outcome = client.upsert_item(request).await.unwrap();
    assert!(!outcome.created);
    assert_eq!(outcome.skipped_files, vec!["demo.txt".to_owned()]);
    assert!(outcome.metadata_changed);

    let requests = server.requests();
    assert!(!requests.iter().any(|request| request.method == Method::PUT));
    let patch = requests
        .iter()
        .find(|request| request.method == Method::POST && request.path == "/metadata/demo-item")
        .unwrap();
    let body = String::from_utf8(patch.body.clone()).unwrap();
    assert!(body.contains("New+title") || body.contains("New%20title"));
}
