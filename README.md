# internetarchive-rs

[![CI](https://github.com/LucaCappelletti94/internetarchive-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/internetarchive-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/internetarchive-rs/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/internetarchive-rs)
[![crates.io](https://img.shields.io/crates/v/internetarchive-rs.svg)](https://crates.io/crates/internetarchive-rs)
[![docs.rs](https://img.shields.io/docsrs/internetarchive-rs)](https://docs.rs/internetarchive-rs)
[![License](https://img.shields.io/crates/l/internetarchive-rs.svg)](https://github.com/LucaCappelletti94/internetarchive-rs/blob/main/LICENSE)

`internetarchive-rs` is an async Rust client for working with [Internet Archive](https://archive.org/) items. It supports public metadata reads, advanced search, authenticated uploads and deletes, metadata updates, public downloads, and higher-level create or upsert workflows, while staying close to the real Internet Archive APIs instead of hiding them behind a made-up abstraction.

[`InternetArchiveClient`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/struct.InternetArchiveClient.html) is the main entrypoint. Use [`SearchQuery`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/struct.SearchQuery.html) for advanced search, [`ItemMetadata`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/struct.ItemMetadata.html) and [`UploadSpec`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/struct.UploadSpec.html) to describe uploads, and [`PatchOperation`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/enum.PatchOperation.html) with [`MetadataTarget`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/enum.MetadataTarget.html) for exact low-level metadata writes. If you want higher-level item creation or updates, use [`InternetArchiveClient::publish_item`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/struct.InternetArchiveClient.html#method.publish_item) and [`InternetArchiveClient::upsert_item`](https://docs.rs/internetarchive-rs/latest/internetarchive_rs/struct.InternetArchiveClient.html#method.upsert_item).

## Read Example

```rust,no_run
use internetarchive_rs::InternetArchiveClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = InternetArchiveClient::new()?;
    let item = client.get_item_by_str("xfetch").await?;
    let pdf = item.file("xfetch.pdf").expect("file exists");
    assert_eq!(pdf.name, "xfetch.pdf");

    Ok(())
}
```

## Search Example

```rust,no_run
use internetarchive_rs::{InternetArchiveClient, SearchQuery, SortDirection};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = InternetArchiveClient::new()?;
    let results = client
        .search(
            &SearchQuery::builder("collection:opensource AND mediatype:texts")
                .field("identifier")
                .field("title")
                .rows(5)
                .sort("publicdate", SortDirection::Desc)
                .build(),
        )
        .await?;

    for doc in results.response.docs {
        let identifier = doc.identifier().expect("identifier field requested");
        let title = doc.title().unwrap_or("<untitled>");
        println!("{identifier}: {title}");
    }

    Ok(())
}
```

## Publish Example

```rust,no_run
use internetarchive_rs::{
    InternetArchiveClient, ItemIdentifier, ItemMetadata, MediaType, PublishRequest, UploadSpec,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = InternetArchiveClient::from_env()?;
    let request = PublishRequest::new(
        ItemIdentifier::new("my-demo-item-2026-04-18")?,
        ItemMetadata::builder()
            .mediatype(MediaType::Texts)
            .title("internetarchive-rs example")
            .description_html("<p>Created from Rust</p>")
            .collection("opensource")
            .language("eng")
            .build(),
        vec![UploadSpec::from_path("artifact.txt")?],
    );

    let outcome = client.upsert_item(request).await?;
    println!(
        "created={}, uploaded={:?}",
        outcome.created, outcome.uploaded_files
    );

    Ok(())
}
```

## Low-Level Metadata Patch Example

```rust,no_run
use internetarchive_rs::{
    InternetArchiveClient, ItemIdentifier, MetadataTarget, PatchOperation,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = InternetArchiveClient::from_env()?;
    let identifier = ItemIdentifier::new("my-demo-item-2026-04-18")?;

    client
        .apply_metadata_patch(
            &identifier,
            MetadataTarget::Metadata,
            &[PatchOperation::replace("/title", "Updated title")],
        )
        .await?;

    Ok(())
}
```

## Authentication

`InternetArchiveClient::new()` is enough for public metadata reads, searches, and downloads.

Authenticated write helpers use LOW auth credentials and read these standard environment variables:
`INTERNET_ARCHIVE_ACCESS_KEY` and `INTERNET_ARCHIVE_SECRET_KEY`. You can create S3 credentials from the official Internet Archive API key page at `https://archive.org/account/s3.php`.
