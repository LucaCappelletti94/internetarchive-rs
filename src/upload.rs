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
        let filename = path_filename(path)?;

        Ok(Self {
            filename,
            source: UploadSource::Path(path.to_path_buf()),
            content_type: guess_content_type(path, None, None),
        })
    }

    /// Builds an upload spec from a local path and an explicit archive file name.
    ///
    /// The content type is guessed from the archive file name first and falls
    /// back to the local path when the new name has no recognizable extension.
    ///
    /// # Errors
    ///
    /// Returns an error if the archive file name is empty.
    pub fn from_path_as(
        path: impl AsRef<Path>,
        filename: impl Into<String>,
    ) -> Result<Self, std::io::Error> {
        let path = path.as_ref();
        let filename = validate_archive_filename(filename.into())?;

        Ok(Self {
            content_type: guess_content_type(path, Some(&filename), None),
            filename,
            source: UploadSource::Path(path.to_path_buf()),
        })
    }

    /// Builds upload specs from `(archive_filename, local_path)` manifest pairs.
    ///
    /// # Errors
    ///
    /// Returns an error if any archive file name is empty.
    pub fn from_manifest<I, F, P>(entries: I) -> Result<Vec<Self>, std::io::Error>
    where
        I: IntoIterator<Item = (F, P)>,
        F: Into<String>,
        P: AsRef<Path>,
    {
        entries
            .into_iter()
            .map(|(filename, path)| Self::from_path_as(path, filename))
            .collect()
    }

    /// Builds an upload spec from a custom file name and bytes.
    #[must_use]
    pub fn from_bytes(filename: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        let filename = filename.into();
        Self {
            content_type: guess_content_type(Path::new(&filename), Some(&filename), None),
            filename,
            source: UploadSource::Bytes(bytes.into()),
        }
    }

    /// Overrides the archive file name.
    ///
    /// The content type is re-guessed from the new file name and falls back to
    /// the previous content type when the name has no recognizable extension.
    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        let filename = filename.into();
        self.content_type = match &self.source {
            UploadSource::Path(path) => {
                guess_content_type(path, Some(&filename), Some(self.content_type.clone()))
            }
            UploadSource::Bytes(_) => guess_content_type(
                Path::new(&filename),
                Some(&filename),
                Some(self.content_type.clone()),
            ),
        };
        self.filename = filename;
        self
    }

    /// Overrides the upload content type.
    #[must_use]
    pub fn with_content_type(mut self, content_type: Mime) -> Self {
        self.content_type = content_type;
        self
    }
}

fn path_filename(path: &Path) -> Result<String, std::io::Error> {
    path.file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no filename")
        })
        .map(str::to_owned)
}

fn validate_archive_filename(filename: String) -> Result<String, std::io::Error> {
    if filename.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "archive filename cannot be empty",
        ));
    }
    Ok(filename)
}

fn guess_content_type(path: &Path, archive_filename: Option<&str>, fallback: Option<Mime>) -> Mime {
    archive_filename
        .and_then(|filename| mime_guess::from_path(filename).first())
        .or_else(|| mime_guess::from_path(path).first())
        .or(fallback)
        .unwrap_or(mime::APPLICATION_OCTET_STREAM)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{FileConflictPolicy, UploadOptions, UploadSource, UploadSpec};

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
        assert!(matches!(spec.source, UploadSource::Path(ref source) if source == &path));
    }

    #[test]
    fn upload_spec_from_path_as_uses_archive_filename_for_name_and_content_type() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact.bin");
        std::fs::write(&path, [1_u8, 2, 3]).unwrap();

        let spec = UploadSpec::from_path_as(&path, "artifact.txt").unwrap();

        assert_eq!(spec.filename, "artifact.txt");
        assert_eq!(spec.content_type, mime::TEXT_PLAIN);
        assert!(matches!(spec.source, UploadSource::Path(ref source) if source == &path));
    }

    #[test]
    fn upload_spec_with_filename_refreshes_content_type() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact.bin");
        std::fs::write(&path, [1_u8, 2, 3]).unwrap();

        let spec = UploadSpec::from_path(&path)
            .unwrap()
            .with_filename("artifact.txt");

        assert_eq!(spec.filename, "artifact.txt");
        assert_eq!(spec.content_type, mime::TEXT_PLAIN);
        assert!(matches!(spec.source, UploadSource::Path(ref source) if source == &path));
    }

    #[test]
    fn upload_spec_from_manifest_builds_renamed_specs_in_order() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.bin");
        let second = directory.path().join("second.bin");
        std::fs::write(&first, [1_u8]).unwrap();
        std::fs::write(&second, [2_u8]).unwrap();

        let specs = UploadSpec::from_manifest([
            ("release/first.txt", first.as_path()),
            ("release/second.bin", second.as_path()),
        ])
        .unwrap();

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].filename, "release/first.txt");
        assert_eq!(specs[0].content_type, mime::TEXT_PLAIN);
        assert_eq!(specs[1].filename, "release/second.bin");
        assert_eq!(specs[1].content_type, mime::APPLICATION_OCTET_STREAM);
        assert!(matches!(specs[0].source, UploadSource::Path(ref source) if source == &first));
        assert!(matches!(specs[1].source, UploadSource::Path(ref source) if source == &second));
    }

    #[test]
    fn upload_spec_from_path_rejects_paths_without_a_filename() {
        let error = UploadSpec::from_path(Path::new("/")).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn upload_spec_from_path_as_rejects_empty_archive_filename() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact.bin");
        std::fs::write(&path, [1_u8, 2, 3]).unwrap();

        let error = UploadSpec::from_path_as(&path, "").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
