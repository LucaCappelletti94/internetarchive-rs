# internetarchive-rs

[![CI](https://github.com/LucaCappelletti94/internetarchive-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/internetarchive-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/internetarchive-rs/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/internetarchive-rs)
[![crates.io](https://img.shields.io/crates/v/internetarchive-rs.svg)](https://crates.io/crates/internetarchive-rs)
[![docs.rs](https://img.shields.io/docsrs/internetarchive-rs)](https://docs.rs/internetarchive-rs)
[![License](https://img.shields.io/crates/l/internetarchive-rs.svg)](https://github.com/LucaCappelletti94/internetarchive-rs/blob/main/LICENSE)

`internetarchive-rs` is an async Rust client for working with [Internet Archive](https://archive.org/) items. It supports public metadata reads, advanced search, authenticated uploads and deletes, metadata updates, public downloads, and higher-level create or upsert workflows, while staying close to the real Internet Archive APIs instead of hiding them behind a made-up abstraction.

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
    let request = PublishRequest::new(
        ItemIdentifier::new("my-demo-item-2026-04-18")?,
        ItemMetadata::builder()
            .mediatype(MediaType::Texts)
            .title("internetarchive-rs example")
            .description_html("<p>Created from Rust</p>")
            .collection("opensource")
            .language("eng")
            .build(),
        vec![UploadSpec::from_bytes("artifact.txt", b"Created from Rust")],
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
