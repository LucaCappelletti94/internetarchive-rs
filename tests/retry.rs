#![allow(clippy::expect_used, clippy::missing_panics_doc, clippy::unwrap_used)]

mod mock_support;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use internetarchive_rs::{
    InternetArchiveClient, InternetArchiveError, ItemIdentifier, RetryOptions, UploadOptions,
    UploadSpec,
};
use mock_support::{MockInternetArchiveServer, QueuedResponse};

fn fast_retry(server: &MockInternetArchiveServer, max_retries: u32) -> InternetArchiveClient {
    server
        .client_builder()
        .retry_options(RetryOptions {
            max_retries,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
        })
        .build()
        .expect("build retrying client")
}

fn count(server: &MockInternetArchiveServer, method: &Method, path: &str) -> usize {
    server
        .requests()
        .iter()
        .filter(|request| request.method == *method && request.path == path)
        .count()
}

#[tokio::test]
async fn upload_retries_on_service_unavailable_then_succeeds() {
    let server = MockInternetArchiveServer::start().await;
    let client = fast_retry(&server, 3);
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue(
        Method::PUT,
        "/s3/demo-item/demo.txt",
        QueuedResponse::text(StatusCode::SERVICE_UNAVAILABLE, "SlowDown"),
    );
    server.enqueue(
        Method::PUT,
        "/s3/demo-item/demo.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );

    client
        .upload_file(
            &identifier,
            &UploadSpec::from_bytes("demo.txt", b"hello"),
            &UploadOptions::default(),
        )
        .await
        .unwrap();

    assert_eq!(count(&server, &Method::PUT, "/s3/demo-item/demo.txt"), 2);
}

#[tokio::test]
async fn upload_gives_up_after_exhausting_retries() {
    let server = MockInternetArchiveServer::start().await;
    let client = fast_retry(&server, 2);
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    for _ in 0..3 {
        server.enqueue(
            Method::PUT,
            "/s3/demo-item/demo.txt",
            QueuedResponse::text(StatusCode::SERVICE_UNAVAILABLE, "SlowDown"),
        );
    }

    let error = client
        .upload_file(
            &identifier,
            &UploadSpec::from_bytes("demo.txt", b"hello"),
            &UploadOptions::default(),
        )
        .await
        .unwrap_err();

    match error {
        InternetArchiveError::Http { status, .. } => {
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(count(&server, &Method::PUT, "/s3/demo-item/demo.txt"), 3);
}

#[tokio::test]
async fn download_retries_on_server_error_then_succeeds() {
    let server = MockInternetArchiveServer::start().await;
    let client = fast_retry(&server, 3);
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue(
        Method::GET,
        "/download/demo-item/seed.txt",
        QueuedResponse::text(StatusCode::INTERNAL_SERVER_ERROR, "boom"),
    );
    server.enqueue(
        Method::GET,
        "/download/demo-item/seed.txt",
        QueuedResponse::text(StatusCode::OK, "seed-body"),
    );

    let bytes = client
        .download_bytes(&identifier, "seed.txt")
        .await
        .unwrap();

    assert_eq!(bytes, "seed-body");
    assert_eq!(
        count(&server, &Method::GET, "/download/demo-item/seed.txt"),
        2
    );
}

#[tokio::test]
async fn download_does_not_retry_client_errors() {
    let server = MockInternetArchiveServer::start().await;
    let client = fast_retry(&server, 3);
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue(
        Method::GET,
        "/download/demo-item/missing.txt",
        QueuedResponse::text(StatusCode::NOT_FOUND, "nope"),
    );

    let error = client
        .download_bytes(&identifier, "missing.txt")
        .await
        .unwrap_err();

    match error {
        InternetArchiveError::Http { status, .. } => assert_eq!(status, StatusCode::NOT_FOUND),
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(
        count(&server, &Method::GET, "/download/demo-item/missing.txt"),
        1
    );
}

#[tokio::test]
async fn download_retries_on_request_timeout() {
    let server = MockInternetArchiveServer::start().await;
    let client = server
        .client_builder()
        .request_timeout(Duration::from_millis(100))
        .retry_options(RetryOptions {
            max_retries: 3,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
        })
        .build()
        .expect("build retrying client");
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    // The first response is delayed well past the request timeout so the first
    // attempt times out, then the second response returns immediately.
    server.enqueue(
        Method::GET,
        "/download/demo-item/slow.txt",
        QueuedResponse::text(StatusCode::OK, "late").with_delay(Duration::from_secs(1)),
    );
    server.enqueue(
        Method::GET,
        "/download/demo-item/slow.txt",
        QueuedResponse::text(StatusCode::OK, "on-time"),
    );

    let bytes = client
        .download_bytes(&identifier, "slow.txt")
        .await
        .unwrap();

    assert_eq!(bytes, "on-time");
    assert_eq!(
        count(&server, &Method::GET, "/download/demo-item/slow.txt"),
        2
    );
}

#[cfg(feature = "indicatif")]
#[tokio::test]
async fn download_with_progress_retries_on_server_error() {
    use internetarchive_rs::indicatif::ProgressBar;

    let server = MockInternetArchiveServer::start().await;
    let client = fast_retry(&server, 3);
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue(
        Method::GET,
        "/download/demo-item/progress.bin",
        QueuedResponse::text(StatusCode::BAD_GATEWAY, "boom"),
    );
    server.enqueue(
        Method::GET,
        "/download/demo-item/progress.bin",
        QueuedResponse::text(StatusCode::OK, "progress-body"),
    );

    let bytes = client
        .download_bytes_with_progress(&identifier, "progress.bin", &ProgressBar::hidden())
        .await
        .unwrap();

    assert_eq!(bytes, "progress-body");
    assert_eq!(
        count(&server, &Method::GET, "/download/demo-item/progress.bin"),
        2
    );
}

#[cfg(feature = "indicatif")]
#[tokio::test]
async fn upload_with_progress_retries_on_service_unavailable() {
    use internetarchive_rs::indicatif::ProgressBar;

    let server = MockInternetArchiveServer::start().await;
    let client = fast_retry(&server, 3);
    let identifier = ItemIdentifier::new("demo-item").unwrap();

    server.enqueue(
        Method::PUT,
        "/s3/demo-item/demo.txt",
        QueuedResponse::text(StatusCode::SERVICE_UNAVAILABLE, "SlowDown"),
    );
    server.enqueue(
        Method::PUT,
        "/s3/demo-item/demo.txt",
        QueuedResponse::text(StatusCode::OK, ""),
    );

    client
        .upload_file_with_progress(
            &identifier,
            &UploadSpec::from_bytes("demo.txt", b"hello"),
            &UploadOptions::default(),
            &ProgressBar::hidden(),
        )
        .await
        .unwrap();

    assert_eq!(count(&server, &Method::PUT, "/s3/demo-item/demo.txt"), 2);
}
