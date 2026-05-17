#![allow(clippy::expect_used, clippy::missing_panics_doc, clippy::unwrap_used)]

mod mock_support;

use axum::http::{Method, StatusCode};
use internetarchive_rs::{
    FileConflictPolicy, InternetArchiveError, ItemIdentifier, ItemMetadata, MediaType,
    PublishRequest, UploadSpec,
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
    assert_eq!(upload.headers.get("x-amz-auto-make-bucket").unwrap(), "1");
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

#[tokio::test]
async fn upsert_item_creates_missing_items_and_backfills_stale_metadata_projection() {
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
    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "files": [{"name": "demo.txt", "size": "5"}],
            "metadata": {
                "identifier": "demo-item"
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
                "identifier": "demo-item"
            }
        }),
    );
    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 99,
            "log": "https://catalogd.archive.org/log/99"
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
                "title": "Backfilled title"
            }
        }),
    );

    let outcome = client
        .upsert_item(PublishRequest::new(
            identifier.clone(),
            ItemMetadata::builder().title("Backfilled title").build(),
            vec![UploadSpec::from_bytes("demo.txt", b"hello".to_vec())],
        ))
        .await
        .unwrap();

    assert!(outcome.created);
    assert!(outcome.metadata_changed);
    assert_eq!(outcome.uploaded_files, vec!["demo.txt".to_owned()]);
    assert_eq!(outcome.item.identifier().unwrap(), identifier);
    assert_eq!(outcome.item.metadata.title(), Some("Backfilled title"));

    let requests = server.requests();
    let patch = requests
        .iter()
        .find(|request| request.method == Method::POST && request.path == "/metadata/demo-item")
        .unwrap();
    let body = String::from_utf8(patch.body.clone()).unwrap();
    assert!(body.contains("Backfilled+title") || body.contains("Backfilled%20title"));
}

#[tokio::test]
async fn upsert_item_does_not_emit_collection_removal_when_archive_returns_superset_list() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    let superset_metadata = serde_json::json!({
        "files": [{"name": "demo.txt", "size": "5"}],
        "metadata": {
            "identifier": "demo-item",
            "mediatype": "texts",
            "title": "Demo item",
            "collection": ["test_collection", "internetarchivebooks"]
        }
    });
    for _ in 0..3 {
        server.enqueue_json(
            Method::GET,
            "/metadata/demo-item",
            StatusCode::OK,
            superset_metadata.clone(),
        );
    }

    let mut request = PublishRequest::new(
        identifier.clone(),
        ItemMetadata::builder()
            .mediatype(MediaType::Texts)
            .title("Demo item")
            .collection("test_collection")
            .build(),
        vec![UploadSpec::from_bytes("demo.txt", b"hello".to_vec())],
    );
    request.conflict_policy = FileConflictPolicy::Skip;

    let outcome = client.upsert_item(request).await.unwrap();

    assert!(!outcome.created);
    assert_eq!(outcome.skipped_files, vec!["demo.txt".to_owned()]);
    assert!(!outcome.metadata_changed);
    assert_eq!(
        outcome.item.metadata.collections().unwrap(),
        vec![
            "test_collection".to_owned(),
            "internetarchivebooks".to_owned(),
        ]
    );

    let requests = server.requests();
    assert!(
        !requests
            .iter()
            .any(|request| request.method == Method::POST),
        "metadata POST should not be issued when stored collection is a superset; saw {:?}",
        requests
            .iter()
            .filter(|request| request.method == Method::POST)
            .map(|request| (
                request.path.clone(),
                String::from_utf8_lossy(&request.body).into_owned()
            ))
            .collect::<Vec<_>>()
    );
    assert!(
        !requests.iter().any(|request| request.method == Method::PUT),
        "no upload should be attempted under Skip policy"
    );
}
#[tokio::test]
async fn make_dark_submits_make_dark_task_with_comment_and_decodes_envelope() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::POST,
        "/services/tasks.php",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "value": {
                "task_id": 5_374_921_600u64,
                "log": "https://catalogd.archive.org/log/5374921600"
            }
        }),
    );

    let submission = client
        .make_dark(&identifier, "live test cleanup")
        .await
        .unwrap();

    assert_eq!(submission.task_id.0, 5_374_921_600);
    assert_eq!(
        submission.log.as_str(),
        "https://catalogd.archive.org/log/5374921600"
    );

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let posted = &requests[0];
    assert_eq!(posted.method, Method::POST);
    assert_eq!(posted.path, "/services/tasks.php");
    assert_eq!(
        posted.headers.get("authorization").unwrap(),
        "LOW access:secret"
    );
    assert_eq!(
        posted.headers.get("content-type").unwrap(),
        "application/json"
    );
    let body: serde_json::Value = serde_json::from_slice(&posted.body).unwrap();
    assert_eq!(body["identifier"], "demo-item");
    assert_eq!(body["cmd"], "make_dark.php");
    assert_eq!(body["args"]["comment"], "live test cleanup");
}

#[tokio::test]
async fn make_dark_reports_http_failure_when_caller_lacks_permission() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("not-mine").unwrap();

    server.enqueue_json(
        Method::POST,
        "/services/tasks.php",
        StatusCode::UNAUTHORIZED,
        serde_json::json!({
            "success": false,
            "error": "Unauthorized to edit item"
        }),
    );

    let error = client
        .make_dark(&identifier, "live test cleanup")
        .await
        .unwrap_err();
    match error {
        InternetArchiveError::Http {
            status, message, ..
        } => {
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(message.as_deref(), Some("Unauthorized to edit item"));
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
}

#[tokio::test]
async fn make_dark_reports_invalid_state_when_envelope_marks_failure_with_200() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::POST,
        "/services/tasks.php",
        StatusCode::OK,
        serde_json::json!({
            "success": false,
            "error": "rate limit exceeded"
        }),
    );

    let error = client
        .make_dark(&identifier, "live test cleanup")
        .await
        .unwrap_err();
    match error {
        InternetArchiveError::InvalidState(message) => {
            assert!(
                message.contains("rate limit exceeded"),
                "expected rate-limit message, got: {message}"
            );
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
}
