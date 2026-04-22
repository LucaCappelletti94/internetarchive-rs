# internetarchive-rs

[![CI](https://github.com/LucaCappelletti94/internetarchive-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/internetarchive-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/internetarchive-rs/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/internetarchive-rs)
[![crates.io](https://img.shields.io/crates/v/internetarchive-rs.svg)](https://crates.io/crates/internetarchive-rs)
[![docs.rs](https://img.shields.io/docsrs/internetarchive-rs)](https://docs.rs/internetarchive-rs)
[![License](https://img.shields.io/crates/l/internetarchive-rs.svg)](https://github.com/LucaCappelletti94/internetarchive-rs/blob/main/LICENSE)

`internetarchive-rs` is an async Rust client for working with [Internet Archive](https://archive.org/) items. It supports public metadata reads, advanced search, authenticated uploads and deletes, metadata updates, public downloads, and higher-level create or upsert workflows.

[`InternetArchiveClient`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/client/struct.InternetArchiveClient.html) is the main entrypoint. Use [`SearchQuery`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/search/struct.SearchQuery.html) for advanced search, [`ItemMetadata`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/metadata/struct.ItemMetadata.html) and [`UploadSpec`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/upload/struct.UploadSpec.html) to describe uploads, and [`PatchOperation`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/metadata/enum.PatchOperation.html) with [`MetadataTarget`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/metadata/enum.MetadataTarget.html) for exact low-level metadata writes. If you want higher-level item creation or updates, use [`InternetArchiveClient::publish_item`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/client/struct.InternetArchiveClient.html#method.publish_item) and [`InternetArchiveClient::upsert_item`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/client/struct.InternetArchiveClient.html#method.upsert_item).

## Read Example

```rust
use internetarchive_rs::{InternetArchiveClient, ItemIdentifier};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = InternetArchiveClient::new()?;
    let identifier = ItemIdentifier::new("xfetch")?;
    let download = client.resolve_download(&identifier, "xfetch.pdf")?;
    assert!(download.url.as_str().ends_with("/download/xfetch/xfetch.pdf"));

    Ok(())
}
```

## Search Example

```rust
use internetarchive_rs::{Endpoint, SearchQuery, SortDirection};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let query = SearchQuery::builder("collection:opensource AND mediatype:texts")
        .field("identifier")
        .field("title")
        .rows(5)
        .sort("publicdate", SortDirection::Desc)
        .build();

    let url = query.into_url(Endpoint::default().search_url()?)?;
    assert!(url.as_str().contains("collection%3Aopensource"));
    assert!(url.as_str().contains("sort%5B%5D=publicdate+desc"));

    Ok(())
}
```

## Publish Example

```rust
use internetarchive_rs::{
    InternetArchiveClient, ItemIdentifier, ItemMetadata, MediaType, PublishRequest, UploadSpec,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = InternetArchiveClient::new()?;
    let upload = UploadSpec::from_path_as("/tmp/build/artifact.tmp", "artifact.txt")?;
    let request = PublishRequest::new(
        ItemIdentifier::new("my-demo-item-2026-04-18")?,
        ItemMetadata::builder()
            .mediatype(MediaType::Texts)
            .title("internetarchive-rs example")
            .description_html("<p>Created from Rust</p>")
            .date("2026-04-18")
            .collection("opensource")
            .publisher("internetarchive-rs")
            .language("eng")
            .rights("CC BY 4.0")
            .build(),
        vec![upload],
    );

    assert!(!client.has_auth());
    assert_eq!(request.identifier.as_str(), "my-demo-item-2026-04-18");
    assert_eq!(request.uploads[0].filename, "artifact.txt");

    Ok(())
}
```

## Low-Level Metadata Patch Example

```rust
use internetarchive_rs::{MetadataChange, MetadataTarget, PatchOperation};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let change = MetadataChange::new(
        &MetadataTarget::Metadata,
        vec![PatchOperation::replace("/title", "Updated title")],
    );
    let json = serde_json::to_string(&change)?;
    assert!(json.contains("\"target\":\"metadata\""));
    assert!(json.contains("\"op\":\"replace\""));

    Ok(())
}
```

## Authentication

`InternetArchiveClient::new()` is enough for public metadata reads, searches, and downloads.

Authenticated write helpers use LOW auth credentials and read these standard environment variables:
`INTERNET_ARCHIVE_ACCESS_KEY` and `INTERNET_ARCHIVE_SECRET_KEY`. You can create S3 credentials from the official Internet Archive API key page at `https://archive.org/account/s3.php`.

## Progress Bars

Enable the optional `indicatif` feature if you want upload and download helpers that update a progress bar:

```toml
internetarchive-rs = { version = "0.1.2", features = ["indicatif"] }
```

The crate re-exports `indicatif` when that feature is enabled, so you can use `internetarchive_rs::indicatif::ProgressBar` without adding a separate direct dependency.

## Operational Notes

Internet Archive's own upload-limit guidance is inconsistent, so the safest choice is to plan conservatively. The official [Uploading - Troubleshooting](https://archivesupport.zendesk.com/hc/en-us/articles/360016700691-Uploading-Troubleshooting) page, updated on August 2, 2021, says a single file should stay around 500 to 700 GB, recommends keeping an item under 10,000 files and 1 TB total, and notes that the API can technically accept up to 250,000 files. The official [Uploading - Tips](https://archivesupport.zendesk.com/hc/en-us/articles/360016475032-Uploading-Tips) page, updated on August 25, 2021, instead says there is no hard size or file-count limit, but still recommends staying under 50 GB and 1,000 files per single page. For automated ingest, it is better to treat these pages as operational guidance than as a strict contract.

Visibility is eventually consistent rather than immediate. The official [Uploading - A Basic Guide](https://archivesupport.zendesk.com/hc/en-us/articles/360002360111-Uploading-A-Basic-Guide) says item creation and follow-on tasks can take seconds, hours, or days depending on the amount and type of uploaded data, and the official [Problems or errors](https://archivesupport.zendesk.com/hc/en-us/articles/360018404871-Problems-or-errors) and [Uploading - Troubleshooting](https://archivesupport.zendesk.com/hc/en-us/articles/360016700691-Uploading-Troubleshooting) pages mention queued, running, paused, or failed tasks, `503-slowdown-spam` responses, temporary read-only item servers, and cases where users are told to wait up to 24 hours before assuming an upload is missing.

On retention, the official [Archive.org Information](https://archivesupport.zendesk.com/hc/en-us/articles/360014755952-Archive-org-Information) page says uploads are duplicated or backed up at various locations and that the Archive's intention is to store materials in perpetuity. That is a strong preservation statement, but it is not presented as a formal durability or uptime SLA. The official sources linked above do not publish an uptime guarantee; the closest operational reference they provide is [archive.org/stats](https://archive.org/stats), which is mentioned by the Help Center's [Internet Archive Statistics](https://archivesupport.zendesk.com/hc/en-us/articles/360004650632-Internet-Archive-Statistics) page.
