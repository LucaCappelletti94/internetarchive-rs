#![allow(clippy::expect_used, clippy::missing_panics_doc, clippy::unwrap_used)]

mod cleanup_support;
mod mock_support;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use internetarchive_rs::{
    Auth, Endpoint, FileConflictPolicy, InternetArchiveClient, InternetArchiveError,
    ItemIdentifier, ItemMetadata, MediaType, MetadataChange, MetadataTarget, PatchOperation,
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
#[tokio::test]
async fn dark_with_retries_eventually_succeeds_after_transient_failures() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    for _ in 0..2 {
        server.enqueue_json(
            Method::POST,
            "/services/tasks.php",
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({"success": false, "error": "rate limited"}),
        );
    }
    server.enqueue_json(
        Method::POST,
        "/services/tasks.php",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "value": {
                "task_id": 42_u64,
                "log": "https://catalogd.archive.org/log/42"
            }
        }),
    );

    let submission = cleanup_support::dark_with_retries(
        &client,
        &identifier,
        "test",
        4,
        Duration::from_millis(1),
    )
    .await
    .unwrap();
    assert_eq!(submission.task_id.0, 42);

    let posts = server
        .requests()
        .into_iter()
        .filter(|request| request.method == Method::POST && request.path == "/services/tasks.php")
        .count();
    assert_eq!(posts, 3, "expected 3 POST attempts (2 failures + success)");
}

#[tokio::test]
async fn dark_with_retries_returns_last_error_after_exhausting_attempts() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    for _ in 0..4 {
        server.enqueue_json(
            Method::POST,
            "/services/tasks.php",
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::json!({"success": false, "error": "rate limited"}),
        );
    }

    let error = cleanup_support::dark_with_retries(
        &client,
        &identifier,
        "test",
        4,
        Duration::from_millis(1),
    )
    .await
    .unwrap_err();
    match error {
        InternetArchiveError::Http { status, .. } => {
            assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        }
        other => panic!("unexpected error variant: {other:?}"),
    }

    let posts = server
        .requests()
        .into_iter()
        .filter(|request| request.method == Method::POST && request.path == "/services/tasks.php")
        .count();
    assert_eq!(
        posts, 4,
        "expected exactly 4 POST attempts before giving up"
    );
}
#[tokio::test]
async fn apply_metadata_changes_encodes_multi_target_array_in_form_body() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 1234,
            "log": "https://catalogd.archive.org/log/1234"
        }),
    );

    client
        .apply_metadata_changes(
            &identifier,
            &[
                MetadataChange::new(
                    &MetadataTarget::Metadata,
                    vec![PatchOperation::add("/marker", "enabled")],
                ),
                MetadataChange::new(
                    &MetadataTarget::File("seed.txt".to_owned()),
                    vec![PatchOperation::add("/description", "hello")],
                ),
            ],
        )
        .await
        .unwrap();

    let requests = server.requests();
    let posted = requests
        .iter()
        .find(|request| request.method == Method::POST && request.path == "/metadata/demo-item")
        .expect("captured POST");
    let body_text = std::str::from_utf8(&posted.body).expect("utf-8 body");
    let changes_value = body_text
        .split('&')
        .find_map(|pair| pair.strip_prefix("-changes="))
        .expect("-changes field present");
    let decoded: String = url::form_urlencoded::parse(format!("v={changes_value}").as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
        .expect("url-decoded value");
    let parsed: serde_json::Value = serde_json::from_str(&decoded).expect("json array");
    let entries = parsed.as_array().expect("changes is array");
    assert_eq!(entries.len(), 2, "expected two MetadataChange entries");
    assert_eq!(entries[0]["target"], "metadata");
    assert_eq!(entries[1]["target"], "files/seed.txt");
    assert_eq!(entries[0]["patch"][0]["path"], "/marker");
    assert_eq!(entries[1]["patch"][0]["path"], "/description");
}

#[tokio::test]
async fn apply_metadata_changes_encodes_user_json_and_root_targets_in_form_body() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 7777,
            "log": "https://catalogd.archive.org/log/7777"
        }),
    );

    client
        .apply_metadata_changes(
            &identifier,
            &[
                MetadataChange::new(
                    &MetadataTarget::UserJson("workflow".to_owned()),
                    vec![PatchOperation::add("/state", "running")],
                ),
                MetadataChange::new(
                    &MetadataTarget::RootUserJson(identifier.clone()),
                    vec![PatchOperation::add("/alive", true)],
                ),
            ],
        )
        .await
        .unwrap();

    let requests = server.requests();
    let posted = requests
        .iter()
        .find(|request| request.method == Method::POST && request.path == "/metadata/demo-item")
        .expect("captured POST");
    let body_text = std::str::from_utf8(&posted.body).expect("utf-8 body");
    let changes_value = body_text
        .split('&')
        .find_map(|pair| pair.strip_prefix("-changes="))
        .expect("-changes field present");
    let decoded: String = url::form_urlencoded::parse(format!("v={changes_value}").as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
        .expect("url-decoded value");
    let parsed: serde_json::Value = serde_json::from_str(&decoded).expect("json array");
    let entries = parsed.as_array().expect("changes is array");
    assert_eq!(entries.len(), 2, "expected two MetadataChange entries");
    assert_eq!(entries[0]["target"], "workflow");
    assert_eq!(
        entries[1]["target"], "demo-item",
        "RootUserJson must encode as the item identifier (IA rejects empty target)"
    );
    assert_eq!(entries[0]["patch"][0]["path"], "/state");
    assert_eq!(entries[1]["patch"][0]["path"], "/alive");
}
#[tokio::test]
async fn apply_metadata_patch_encodes_test_remove_and_ia_extension_ops_in_form_body() {
    let server = MockInternetArchiveServer::start().await;
    let client = server.client();
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue_json(
        Method::POST,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "task_id": 2024,
            "log": "https://catalogd.archive.org/log/2024"
        }),
    );

    client
        .apply_metadata_patch(
            &identifier,
            MetadataTarget::Metadata,
            &[
                PatchOperation::test("/title", "expected"),
                PatchOperation::Remove {
                    path: "/obsolete".to_owned(),
                },
                PatchOperation::RemoveFirst {
                    path: "/tags".to_owned(),
                    value: serde_json::json!("legacy"),
                },
                PatchOperation::RemoveAll {
                    path: "/tags".to_owned(),
                    value: serde_json::json!("dup"),
                },
            ],
        )
        .await
        .unwrap();

    let requests = server.requests();
    let posted = requests
        .iter()
        .find(|request| request.method == Method::POST && request.path == "/metadata/demo-item")
        .expect("captured POST");
    let body_text = std::str::from_utf8(&posted.body).expect("utf-8 body");
    let patch_value = body_text
        .split('&')
        .find_map(|pair| pair.strip_prefix("-patch="))
        .expect("-patch field present");
    let decoded: String = url::form_urlencoded::parse(format!("v={patch_value}").as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
        .expect("url-decoded value");
    let parsed: serde_json::Value = serde_json::from_str(&decoded).expect("json array");
    let entries = parsed.as_array().expect("patch is array");
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0]["op"], "test");
    assert_eq!(entries[1]["op"], "remove");
    assert_eq!(entries[2]["op"], "remove-first");
    assert_eq!(entries[2]["value"], "legacy");
    assert_eq!(entries[3]["op"], "remove-all");
    assert_eq!(entries[3]["value"], "dup");
}

#[tokio::test]
async fn client_builder_user_agent_override_reaches_wire() {
    let server = MockInternetArchiveServer::start().await;
    let client = InternetArchiveClient::builder()
        .auth(Auth::new("access", "secret"))
        .endpoint(Endpoint::custom(
            server.archive_base.clone(),
            server.s3_base.clone(),
        ))
        .user_agent("internetarchive-rs-test/9.9.9")
        .build()
        .unwrap();

    server.enqueue_json(
        Method::GET,
        "/metadata/demo-item",
        StatusCode::OK,
        serde_json::json!({
            "metadata": {"identifier": "demo-item"},
            "files": []
        }),
    );

    let _item = client.get_item_by_str("demo-item").await.unwrap();

    let captured = server.requests().into_iter().next().expect("one request");
    assert_eq!(
        captured.headers.get("user-agent").map(String::as_str),
        Some("internetarchive-rs-test/9.9.9"),
    );
}
