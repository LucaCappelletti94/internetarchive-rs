//! Endpoint configuration for archive.org and custom test deployments.

use url::Url;

/// Endpoint roots used by the client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    archive_base: Url,
    s3_base: Url,
}

impl Endpoint {
    /// Creates a custom endpoint from archive and S3 roots.
    ///
    /// The archive root is used for metadata, search, details, and downloads.
    /// The S3 root is used for authenticated upload and delete operations.
    #[must_use]
    pub fn custom(archive_base: Url, s3_base: Url) -> Self {
        Self {
            archive_base: normalize_root(archive_base),
            s3_base: normalize_root(s3_base),
        }
    }

    /// Returns the archive root URL.
    #[must_use]
    pub fn archive_base(&self) -> &Url {
        &self.archive_base
    }

    /// Returns the S3 root URL.
    #[must_use]
    pub fn s3_base(&self) -> &Url {
        &self.s3_base
    }

    /// Returns the metadata endpoint for a given item.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured base URL cannot accept the path
    /// segments.
    pub fn metadata_url(&self, identifier: &str) -> Result<Url, url::ParseError> {
        Ok(join_segments(
            self.archive_base.clone(),
            &["metadata", identifier],
        ))
    }

    /// Returns the advanced search endpoint URL.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured base URL cannot accept the path.
    pub fn search_url(&self) -> Result<Url, url::ParseError> {
        self.archive_base.join("advancedsearch.php")
    }

    /// Returns the details page URL for an item.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured base URL cannot accept the path
    /// segments.
    pub fn details_url(&self, identifier: &str) -> Result<Url, url::ParseError> {
        Ok(join_segments(
            self.archive_base.clone(),
            &["details", identifier],
        ))
    }

    /// Returns the download URL for a specific file.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured base URL cannot accept the path
    /// segments.
    pub fn download_url(&self, identifier: &str, filename: &str) -> Result<Url, url::ParseError> {
        Ok(join_segments(
            self.archive_base.clone(),
            &["download", identifier, filename],
        ))
    }

    /// Returns the S3 item URL for bucket-level operations.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured base URL cannot accept the path
    /// segments.
    pub fn s3_item_url(&self, identifier: &str) -> Result<Url, url::ParseError> {
        Ok(join_segments(self.s3_base.clone(), &[identifier]))
    }

    /// Returns the S3 object URL for upload or delete operations.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured base URL cannot accept the path
    /// segments.
    pub fn s3_object_url(&self, identifier: &str, filename: &str) -> Result<Url, url::ParseError> {
        Ok(join_segments(self.s3_base.clone(), &[identifier, filename]))
    }

    /// Returns the S3 limit-check URL for an item identifier and access key.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured base URL cannot accept query
    /// parameters.
    pub fn s3_limit_check_url(
        &self,
        access_key: &str,
        identifier: &str,
    ) -> Result<Url, url::ParseError> {
        let mut url = self.s3_base.clone();
        url.query_pairs_mut()
            .append_pair("check_limit", "1")
            .append_pair("accesskey", access_key)
            .append_pair("bucket", identifier);
        Ok(url)
    }
}

impl Default for Endpoint {
    fn default() -> Self {
        Self::custom(
            match Url::parse("https://archive.org/") {
                Ok(url) => url,
                Err(_) => unreachable!("archive.org root is a valid URL"),
            },
            match Url::parse("https://s3.us.archive.org/") {
                Ok(url) => url,
                Err(_) => unreachable!("archive.org s3 root is a valid URL"),
            },
        )
    }
}

fn normalize_root(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let mut path = url.path().to_owned();
        path.push('/');
        url.set_path(&path);
    }
    url
}

fn join_segments(mut base: Url, segments: &[&str]) -> Url {
    let mut path = base.path().trim_end_matches('/').to_owned();
    for segment in segments {
        path.push('/');
        path.push_str(segment);
    }
    base.set_path(&path);
    base
}

#[cfg(test)]
mod tests {
    use super::Endpoint;
    use url::Url;

    #[test]
    fn default_endpoints_match_archive_org() {
        let endpoint = Endpoint::default();
        assert_eq!(endpoint.archive_base().as_str(), "https://archive.org/");
        assert_eq!(endpoint.s3_base().as_str(), "https://s3.us.archive.org/");
        assert_eq!(
            endpoint.metadata_url("xfetch").unwrap().as_str(),
            "https://archive.org/metadata/xfetch"
        );
        assert_eq!(
            endpoint
                .download_url("xfetch", "xfetch.pdf")
                .unwrap()
                .as_str(),
            "https://archive.org/download/xfetch/xfetch.pdf"
        );
        assert_eq!(
            endpoint.details_url("xfetch").unwrap().as_str(),
            "https://archive.org/details/xfetch"
        );
        assert_eq!(
            endpoint.s3_item_url("xfetch").unwrap().as_str(),
            "https://s3.us.archive.org/xfetch"
        );
    }

    #[test]
    fn custom_endpoints_are_normalized() {
        let endpoint = Endpoint::custom(
            Url::parse("http://localhost:3000/root").unwrap(),
            Url::parse("http://localhost:3000/s3").unwrap(),
        );

        assert_eq!(
            endpoint.archive_base().as_str(),
            "http://localhost:3000/root/"
        );
        assert_eq!(endpoint.s3_base().as_str(), "http://localhost:3000/s3/");
        assert_eq!(
            endpoint.search_url().unwrap().as_str(),
            "http://localhost:3000/root/advancedsearch.php"
        );
        assert_eq!(
            endpoint.s3_object_url("demo", "file.txt").unwrap().as_str(),
            "http://localhost:3000/s3/demo/file.txt"
        );
    }

    #[test]
    fn limit_check_urls_include_expected_query_pairs() {
        let endpoint = Endpoint::default();
        let url = endpoint.s3_limit_check_url("abc", "demo").unwrap();
        assert!(url.as_str().contains("check_limit=1"));
        assert!(url.as_str().contains("accesskey=abc"));
        assert!(url.as_str().contains("bucket=demo"));
    }
}
