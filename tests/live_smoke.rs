#![allow(clippy::expect_used, clippy::missing_panics_doc, clippy::unwrap_used)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use internetarchive_rs::{
    Auth, DeleteOptions, DownloadTarget, FileConflictPolicy, InternetArchiveClient, ItemIdentifier,
    ItemMetadata, MediaType, MetadataChange, MetadataTarget, PatchOperation, PollOptions,
    PublishRequest, SearchQuery, UploadOptions, UploadSpec,
};
use tempfile::tempdir;

fn unique_identifier(label: &str) -> ItemIdentifier {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_millis();
    ItemIdentifier::new(format!(
        "internetarchive-rs-{label}-{timestamp}-{}",
        std::process::id()
    ))
    .expect("valid identifier")
}

fn live_credentials() -> Auth {
    Auth::from_env().expect("live credentials")
}

#[tokio::test]
#[ignore = "requires live Internet Archive credentials"]
async fn live_low_level_client_api_round_trip() {
    let auth = live_credentials();
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

    let item = client.get_item(&identifier).await.expect("get item");
    assert_eq!(item.identifier().expect("identifier"), identifier);
    assert!(client
        .get_item_by_str(identifier.as_str())
        .await
        .expect("get item by str")
        .file("seed.txt")
        .is_some());

    let search = client
        .search(
            &SearchQuery::builder(format!("identifier:{}", identifier.as_str()))
                .field("identifier")
                .field("title")
                .rows(5)
                .sort("publicdate", internetarchive_rs::SortDirection::Desc)
                .build(),
        )
        .await
        .expect("search item");
    assert_eq!(
        search.response.docs[0]
            .identifier()
            .expect("search identifier"),
        identifier
    );

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
#[ignore = "requires live Internet Archive credentials"]
async fn live_workflow_helpers_round_trip() {
    let client = InternetArchiveClient::from_env().expect("live credentials");
    let identifier = unique_identifier("live-workflow");
    let tempdir = tempdir().expect("tempdir");
    let artifact = tempdir.path().join("artifact.txt");
    tokio::fs::write(&artifact, "workflow artifact")
        .await
        .expect("write artifact");

    let mut publish_request = PublishRequest::new(
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
        vec![UploadSpec::from_bytes("artifact.txt", b"workflow artifact")],
    );
    publish_request.upload_options.skip_derive = true;

    let published = client
        .publish_item(publish_request)
        .await
        .expect("publish item");
    assert!(published.created);
    assert!(published.item.file("artifact.txt").is_some());

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
    assert_eq!(
        updated.item.identifier().expect("updated identifier"),
        identifier
    );
}
