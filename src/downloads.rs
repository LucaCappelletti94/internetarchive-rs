//! Download helper types.

use std::path::PathBuf;

use url::Url;

use crate::ItemIdentifier;

/// Resolved file download descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDownload {
    /// Item identifier being downloaded.
    pub identifier: ItemIdentifier,
    /// Requested file name.
    pub filename: String,
    /// Final resolved download URL.
    pub url: Url,
}

/// Download target used by helper methods.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadTarget {
    /// Return bytes in memory.
    Bytes,
    /// Write directly to a local path.
    Path(PathBuf),
}
