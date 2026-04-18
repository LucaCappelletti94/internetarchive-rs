//! Upload and delete option types.

use std::path::{Path, PathBuf};

use mime::Mime;

/// Replacement policy for uploads targeting an existing file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileConflictPolicy {
    /// Return an error if the file already exists.
    Error,
    /// Skip uploads for files that already exist.
    Skip,
    /// Overwrite the existing file in place.
    Overwrite,
    /// Overwrite the existing file while preserving its old version.
    OverwriteKeepingHistory,
}

/// Per-upload options for IA S3 requests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UploadOptions {
    /// Skip the derive queue for this upload.
    pub skip_derive: bool,
    /// Keep the old file version if the key already exists.
    pub keep_old_version: bool,
    /// Request interactive priority in the ingest queue.
    pub interactive_priority: bool,
    /// Optional size hint for the full bucket.
    pub size_hint: Option<u64>,
}

/// Options for S3 deletes.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DeleteOptions {
    /// Delete related derivative files.
    pub cascade_delete: bool,
    /// Keep the old file version in history.
    pub keep_old_version: bool,
}

/// Upload source kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadSource {
    /// Upload from a local file path.
    Path(PathBuf),
    /// Upload from in-memory bytes.
    Bytes(Vec<u8>),
}

/// One file upload specification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadSpec {
    /// Final file name inside the item.
    pub filename: String,
    /// Source content.
    pub source: UploadSource,
    /// MIME type used for the request.
    pub content_type: Mime,
}

impl UploadSpec {
    /// Builds an upload spec from a local path, using the file name from the path.
    ///
    /// # Errors
    ///
    /// Returns an error if the path does not contain a final file name.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let path = path.as_ref();
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no filename")
            })?
            .to_owned();

        Ok(Self {
            filename,
            source: UploadSource::Path(path.to_path_buf()),
            content_type: mime_guess::from_path(path).first_or_octet_stream(),
        })
    }

    /// Builds an upload spec from a custom file name and bytes.
    #[must_use]
    pub fn from_bytes(filename: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        let filename = filename.into();
        Self {
            content_type: mime_guess::from_path(&filename).first_or_octet_stream(),
            filename,
            source: UploadSource::Bytes(bytes.into()),
        }
    }

    /// Overrides the upload content type.
    #[must_use]
    pub fn with_content_type(mut self, content_type: Mime) -> Self {
        self.content_type = content_type;
        self
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{FileConflictPolicy, UploadOptions, UploadSpec};

    #[test]
    fn upload_spec_from_bytes_guesses_content_type() {
        let spec = UploadSpec::from_bytes("demo.txt", b"hello");
        assert_eq!(spec.filename, "demo.txt");
        assert_eq!(spec.content_type, mime::TEXT_PLAIN);
    }

    #[test]
    fn upload_options_default_to_safe_values() {
        let options = UploadOptions::default();
        assert!(!options.skip_derive);
        assert!(!options.keep_old_version);
        assert_eq!(
            FileConflictPolicy::OverwriteKeepingHistory,
            FileConflictPolicy::OverwriteKeepingHistory
        );
    }

    #[test]
    fn upload_spec_from_path_and_content_type_override_work() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact.bin");
        std::fs::write(&path, [1_u8, 2, 3]).unwrap();

        let spec = UploadSpec::from_path(&path)
            .unwrap()
            .with_content_type(mime::APPLICATION_OCTET_STREAM);

        assert_eq!(spec.filename, "artifact.bin");
        assert_eq!(spec.content_type, mime::APPLICATION_OCTET_STREAM);
        assert!(matches!(spec.source, super::UploadSource::Path(ref source) if source == &path));
    }

    #[test]
    fn upload_spec_from_path_rejects_paths_without_a_filename() {
        let error = UploadSpec::from_path(Path::new("/")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
