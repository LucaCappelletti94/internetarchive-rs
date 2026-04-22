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

pub mod client;
mod client_uploader_traits_impl;
pub mod downloads;
pub mod endpoint;
pub mod error;
pub mod ids;
pub mod metadata;
pub mod model;
pub mod poll;
pub mod search;
mod serde_util;
pub mod upload;
pub mod workflow;

pub use client::{Auth, InternetArchiveClient, InternetArchiveClientBuilder};
pub use downloads::{DownloadTarget, ResolvedDownload};
pub use endpoint::Endpoint;
pub use error::InternetArchiveError;
pub use ids::{IdentifierError, ItemIdentifier, TaskId};
pub use metadata::{
    ItemMetadata, ItemMetadataBuilder, MediaType, MetadataChange, MetadataTarget, MetadataValue,
    PatchOperation,
};
pub use model::{
    Item, ItemFile, MetadataWriteResponse, S3LimitCheck, SearchDocument, SearchResponse,
    SearchResponseHeader, SearchResultPage,
};
pub use poll::PollOptions;
pub use search::{SearchQuery, SearchQueryBuilder, SearchSort, SortDirection};
pub use upload::{DeleteOptions, FileConflictPolicy, UploadOptions, UploadSource, UploadSpec};
pub use workflow::{PublishOutcome, PublishRequest};
