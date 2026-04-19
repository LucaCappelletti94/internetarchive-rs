#![allow(clippy::expect_used, clippy::missing_panics_doc, clippy::unwrap_used)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use internetarchive_rs::{
    Auth, DeleteOptions, DownloadTarget, FileConflictPolicy, InternetArchiveClient,
    InternetArchiveError, ItemIdentifier, ItemMetadata, MediaType, MetadataChange, MetadataTarget,
    PatchOperation, PollOptions, PublishRequest, SearchQuery, UploadOptions, UploadSpec,
};
use tempfile::tempdir;

static UNIQUE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_identifier(label: &str) -> ItemIdentifier {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    let counter = UNIQUE_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| String::from("local"));
    let run_attempt = std::env::var("GITHUB_RUN_ATTEMPT").unwrap_or_else(|_| String::from("0"));
    ItemIdentifier::new(format!(
        "internetarchive-rs-{label}-{run_id}-{run_attempt}-{timestamp}-{counter}-{}",
        std::process::id(),
    ))
    .expect("valid identifier")
}

fn live_credentials() -> Option<Auth> {
    Auth::from_env().ok()
}

async fn wait_for_item_file(
    client: &InternetArchiveClient,
    identifier: &ItemIdentifier,
    filename: &str,
    max_wait: Duration,
) {
    let started = tokio::time::Instant::now();
    let mut delay = Duration::from_secs(1);

    loop {
        match client.get_item(identifier).await {
            Ok(item) if item.file(filename).is_some() => return,
            Ok(_) | Err(InternetArchiveError::ItemNotFound { .. }) => {}
            Err(error) => panic!("failed while waiting for file visibility: {error}"),
        }

        assert!(
            started.elapsed() < max_wait,
            "timed out waiting for {filename} to become visible on {identifier}"
        );
        tokio::time::sleep(delay).await;
        delay = std::cmp::min(delay.saturating_mul(2), Duration::from_secs(10));
    }
}

async fn wait_for_search_hit(
    client: &InternetArchiveClient,
    identifier: &ItemIdentifier,
    max_wait: Duration,
) {
    let started = tokio::time::Instant::now();
    let mut delay = Duration::from_secs(1);

    loop {
        let query = SearchQuery::builder(format!("identifier:{}", identifier.as_str()))
            .field("identifier")
            .field("title")
            .rows(5)
            .sort("publicdate", internetarchive_rs::SortDirection::Desc)
            .build();

        match client.search(&query).await {
            Ok(search)
                if search
                    .response
                    .docs
                    .iter()
                    .any(|document| document.identifier().as_ref() == Some(identifier)) =>
            {
                return;
            }
            Ok(_) => {}
            Err(InternetArchiveError::Http { status, .. }) if status.is_server_error() => {}
            Err(error) => panic!("failed while waiting for search visibility: {error}"),
        }

        assert!(
            started.elapsed() < max_wait,
            "timed out waiting for {identifier} to appear in search"
        );
        tokio::time::sleep(delay).await;
        delay = std::cmp::min(delay.saturating_mul(2), Duration::from_secs(10));
    }
}

async fn publish_with_fresh_identifier(
    client: &InternetArchiveClient,
    artifact_path: &std::path::Path,
) -> (ItemIdentifier, internetarchive_rs::PublishOutcome) {
    let max_attempts = 3;

    for attempt in 0..max_attempts {
        let identifier = unique_identifier("live-workflow");
        let mut request = PublishRequest::new(
            identifier.clone(),
            ItemMetadata::builder()
                .mediatype(MediaType::Texts)
                .title(format!(
                    "internetarchive-rs workflow {}",
                    identifier.as_str()
                ))
                .description_html("<p>internetarchive-rs workflow helper test</p>")
                .collection("opensource")
                .language("eng")
                .build(),
            vec![UploadSpec::from_path(artifact_path).expect("publish artifact")],
        );
        request.upload_options.skip_derive = true;

        match client.publish_item(request).await {
            Ok(outcome) => {
                assert!(outcome.created);
                return (identifier, outcome);
            }
            Err(InternetArchiveError::InvalidState(message))
                if message.contains("already exists") && attempt + 1 < max_attempts => {}
            Err(error) => panic!("publish item: {error}"),
        }
    }

    panic!("publish item: exhausted identifier retries");
}

#[tokio::test]
async fn live_low_level_client_api_round_trip() {
    let Some(auth) = live_credentials() else {
        eprintln!(
            "Skipping live_low_level_client_api_round_trip: missing Internet Archive credentials"
        );
        return;
    };
    let access_key = std::env::var(Auth::ACCESS_KEY_ENV_VAR).unwrap();
    let secret_key = std::env::var(Auth::SECRET_KEY_ENV_VAR).unwrap();
    std::env::set_var("IA_LIVE_ACCESS_COPY", &access_key);
    std::env::set_var("IA_LIVE_SECRET_COPY", &secret_key);
    let copied_auth = Auth::from_env_vars("IA_LIVE_ACCESS_COPY", "IA_LIVE_SECRET_COPY")
        .expect("copied live credentials");
    assert!(format!("{copied_auth:?}").contains("<redacted>"));
    std::env::remove_var("IA_LIVE_ACCESS_COPY");
    std::env::remove_var("IA_LIVE_SECRET_COPY");

    let poll = PollOptions {
        max_wait: Duration::from_secs(90),
        initial_delay: Duration::from_millis(500),
        max_delay: Duration::from_secs(5),
    };
    let client = InternetArchiveClient::builder()
        .auth(auth.clone())
        .user_agent("internetarchive-rs/live-daily")
        .request_timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(30))
        .poll_options(poll.clone())
        .build()
        .expect("builder client");
    let env_client = InternetArchiveClient::from_env().expect("env client");
    let with_auth_client = InternetArchiveClient::with_auth(auth).expect("with auth client");
    let unauthenticated_client = InternetArchiveClient::new().expect("unauthenticated client");

    assert!(client.has_auth());
    assert!(env_client.has_auth());
    assert!(with_auth_client.has_auth());
    assert!(!unauthenticated_client.has_auth());
    assert_eq!(client.poll_options(), &poll);
    assert_eq!(client.request_timeout(), Some(Duration::from_secs(120)));
    assert_eq!(client.connect_timeout(), Some(Duration::from_secs(30)));
    assert!(client
        .endpoint()
        .details_url("xfetch")
        .expect("details url")
        .as_str()
        .ends_with("/details/xfetch"));

    let identifier = unique_identifier("live-api");
    let tempdir = tempdir().expect("tempdir");
    let seed_path = tempdir.path().join("seed.txt");
    let extra_path = tempdir.path().join("extra.txt");
    tokio::fs::write(&seed_path, "hello from internetarchive-rs")
        .await
        .expect("write seed");
    tokio::fs::write(&extra_path, "secondary artifact")
        .await
        .expect("write extra");

    let metadata = ItemMetadata::builder()
        .mediatype(MediaType::Texts)
        .title(format!(
            "internetarchive-rs low-level {}",
            identifier.as_str()
        ))
        .description_html("<p>internetarchive-rs live full API test</p>")
        .collection("opensource")
        .creator("internetarchive-rs")
        .subject("live-api")
        .language("eng")
        .license_url("https://creativecommons.org/licenses/by/4.0/")
        .build();

    let create_options = UploadOptions {
        skip_derive: true,
        ..UploadOptions::default()
    };

    let _limit = client
        .check_upload_limit(&identifier)
        .await
        .expect("check upload limit");
    client
        .create_item(
            &identifier,
            &metadata,
            &UploadSpec::from_path(&seed_path).expect("seed upload spec"),
            &create_options,
        )
        .await
        .expect("create item");

    wait_for_item_file(&client, &identifier, "seed.txt", Duration::from_secs(120)).await;
    assert!(client
        .get_item_by_str(identifier.as_str())
        .await
        .expect("get item by str")
        .file("seed.txt")
        .is_some());

    wait_for_search_hit(&client, &identifier, Duration::from_secs(180)).await;

    client
        .apply_metadata_patch(
            &identifier,
            MetadataTarget::Metadata,
            &[PatchOperation::replace(
                "/title",
                format!("internetarchive-rs patched {}", identifier.as_str()),
            )],
        )
        .await
        .expect("apply metadata patch");
    client
        .apply_metadata_changes(
            &identifier,
            &[MetadataChange::new(
                &MetadataTarget::Metadata,
                vec![PatchOperation::add("/live_api_marker", "enabled")],
            )],
        )
        .await
        .expect("apply metadata changes");
    client
        .update_item_metadata(
            &identifier,
            &ItemMetadata::builder()
                .subject("internetarchive-rs live daily")
                .build(),
        )
        .await
        .expect("update item metadata");

    let upload_options = UploadOptions {
        skip_derive: true,
        keep_old_version: true,
        ..UploadOptions::default()
    };
    client
        .upload_file(
            &identifier,
            &UploadSpec::from_path(&extra_path)
                .expect("extra upload spec")
                .with_content_type(mime::TEXT_PLAIN),
            &upload_options,
        )
        .await
        .expect("upload extra file");
    wait_for_item_file(&client, &identifier, "extra.txt", Duration::from_secs(120)).await;

    let resolved = client
        .resolve_download(&identifier, "seed.txt")
        .expect("resolved download");
    assert_eq!(resolved.identifier, identifier);
    let _memory_target = DownloadTarget::Bytes;
    let downloaded_path = tempdir.path().join("downloaded.txt");
    let _path_target = DownloadTarget::Path(downloaded_path.clone());

    let seed_bytes = client
        .download_bytes(&identifier, "seed.txt")
        .await
        .expect("download seed bytes");
    assert_eq!(seed_bytes, "hello from internetarchive-rs");
    client
        .download_to_path(&identifier, "extra.txt", &downloaded_path)
        .await
        .expect("download extra file");
    assert_eq!(
        tokio::fs::read_to_string(&downloaded_path)
            .await
            .expect("read downloaded extra"),
        "secondary artifact"
    );

    client
        .delete_file(&identifier, "extra.txt", &DeleteOptions::default())
        .await
        .expect("delete extra file");
}

#[tokio::test]
async fn live_workflow_helpers_round_trip() {
    let Some(_) = live_credentials() else {
        eprintln!(
            "Skipping live_workflow_helpers_round_trip: missing Internet Archive credentials"
        );
        return;
    };
    let client = InternetArchiveClient::from_env().expect("live credentials");
    let tempdir = tempdir().expect("tempdir");
    let artifact = tempdir.path().join("artifact.txt");
    tokio::fs::write(&artifact, "workflow artifact")
        .await
        .expect("write artifact");

    let (identifier, _) = publish_with_fresh_identifier(&client, &artifact).await;
    wait_for_item_file(
        &client,
        &identifier,
        "artifact.txt",
        Duration::from_secs(120),
    )
    .await;

    let mut upsert_request = PublishRequest::new(
        identifier.clone(),
        ItemMetadata::builder()
            .mediatype(MediaType::Texts)
            .title(format!(
                "internetarchive-rs workflow {}",
                identifier.as_str()
            ))
            .subject("workflow-upsert")
            .build(),
        vec![UploadSpec::from_path(&artifact).expect("upsert artifact")],
    );
    upsert_request.conflict_policy = FileConflictPolicy::Skip;
    upsert_request.upload_options.skip_derive = true;

    let updated = client
        .upsert_item(upsert_request)
        .await
        .expect("upsert item");
    assert!(!updated.created);
    assert_eq!(updated.skipped_files, vec![String::from("artifact.txt")]);
    wait_for_item_file(
        &client,
        &identifier,
        "artifact.txt",
        Duration::from_secs(120),
    )
    .await;
}
