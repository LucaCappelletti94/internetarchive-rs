#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(
    clippy::all,
    clippy::pedantic,
    clippy::expect_used,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unwrap_used
)]
#![allow(clippy::module_name_repetitions)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::missing_errors_doc,
        clippy::missing_panics_doc,
        clippy::panic,
        clippy::too_many_lines,
        clippy::unwrap_used
    )
)]

#[cfg(all(feature = "native-tls", feature = "rustls-ring-tls"))]
compile_error!("features `native-tls` and `rustls-ring-tls` are mutually exclusive");

#[cfg(all(feature = "native-tls", feature = "rustls-tls"))]
compile_error!("features `native-tls` and `rustls-tls` are mutually exclusive");

#[cfg(all(feature = "rustls-tls", feature = "rustls-ring-tls"))]
compile_error!("features `rustls-tls` and `rustls-ring-tls` are mutually exclusive");

#[cfg(all(feature = "native-tls", feature = "rustls-no-provider"))]
compile_error!("features `native-tls` and `rustls-no-provider` are mutually exclusive");

#[cfg(all(feature = "rustls-tls", feature = "rustls-no-provider"))]
compile_error!("features `rustls-tls` and `rustls-no-provider` are mutually exclusive");

#[cfg(all(feature = "rustls-no-provider", feature = "rustls-ring-tls"))]
compile_error!("features `rustls-no-provider` and `rustls-ring-tls` are mutually exclusive");

pub mod client;
mod client_uploader_traits_impl;
pub mod downloads;
pub mod endpoint;
pub mod error;
pub mod ids;
pub mod metadata;
pub mod model;
pub mod poll;
pub mod retry;
pub mod search;
mod serde_util;
pub mod upload;
pub mod workflow;

pub use client::{Auth, InternetArchiveClient, InternetArchiveClientBuilder};
pub use downloads::{DownloadTarget, ResolvedDownload};
pub use endpoint::Endpoint;
pub use error::InternetArchiveError;
pub use ids::{IdentifierError, ItemIdentifier, TaskId};
#[cfg(feature = "indicatif")]
pub use indicatif;
pub use metadata::{
    ItemMetadata, ItemMetadataBuilder, MediaType, MetadataChange, MetadataTarget, MetadataValue,
    PatchOperation,
};
pub use model::{
    Item, ItemFile, MetadataWriteResponse, S3LimitCheck, SearchDocument, SearchResponse,
    SearchResponseHeader, SearchResultPage, TaskSubmission,
};
pub use poll::PollOptions;
pub use retry::RetryOptions;
pub use search::{SearchQuery, SearchQueryBuilder, SearchSort, SortDirection};
pub use upload::{DeleteOptions, FileConflictPolicy, UploadOptions, UploadSource, UploadSpec};
pub use workflow::{PublishOutcome, PublishRequest};
