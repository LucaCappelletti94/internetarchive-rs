//! Advanced-search query builder.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

/// Sort direction used by advanced search.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    /// Ascending sort.
    Asc,
    /// Descending sort.
    Desc,
}

impl SortDirection {
    #[must_use]
    fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

/// One sort clause for advanced search.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchSort {
    /// Field name to sort by.
    pub field: String,
    /// Sort direction.
    pub direction: SortDirection,
}

impl SearchSort {
    /// Creates a new sort clause.
    #[must_use]
    pub fn new(field: impl Into<String>, direction: SortDirection) -> Self {
        Self {
            field: field.into(),
            direction,
        }
    }

    #[must_use]
    pub(crate) fn as_param(&self) -> String {
        format!("{} {}", self.field, self.direction.as_str())
    }
}

/// Query object for `advancedsearch.php`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    query: String,
    fields: Vec<String>,
    rows: Option<u32>,
    page: Option<u32>,
    sorts: Vec<SearchSort>,
    extra_params: BTreeMap<String, String>,
}

impl SearchQuery {
    /// Creates a raw search query string.
    #[must_use]
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            fields: Vec::new(),
            rows: None,
            page: None,
            sorts: Vec::new(),
            extra_params: BTreeMap::new(),
        }
    }

    /// Creates an identifier-only query.
    #[must_use]
    pub fn identifier(identifier: impl AsRef<str>) -> Self {
        Self::new(format!("identifier:{}", identifier.as_ref()))
    }

    /// Starts building a search query.
    #[must_use]
    pub fn builder(query: impl Into<String>) -> SearchQueryBuilder {
        SearchQueryBuilder {
            inner: Self::new(query),
        }
    }

    /// Returns the raw query string.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Returns the requested fields.
    #[must_use]
    pub fn fields(&self) -> &[String] {
        &self.fields
    }

    /// Encodes the query onto a search endpoint URL.
    ///
    /// # Errors
    ///
    /// Returns an error if query parameters cannot be appended.
    pub fn into_url(&self, mut url: Url) -> Result<Url, url::ParseError> {
        {
            let mut query_pairs = url.query_pairs_mut();
            query_pairs
                .append_pair("q", &self.query)
                .append_pair("output", "json");

            if !self.fields.is_empty() {
                for field in &self.fields {
                    query_pairs.append_pair("fl[]", field);
                }
            }

            if let Some(rows) = self.rows {
                query_pairs.append_pair("rows", &rows.to_string());
            }

            if let Some(page) = self.page {
                query_pairs.append_pair("page", &page.to_string());
            }

            for sort in &self.sorts {
                query_pairs.append_pair("sort[]", &sort.as_param());
            }

            for (key, value) in &self.extra_params {
                query_pairs.append_pair(key, value);
            }
        }

        Ok(url)
    }
}

/// Builder for [`SearchQuery`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQueryBuilder {
    inner: SearchQuery,
}

impl SearchQueryBuilder {
    /// Adds a field to the response.
    #[must_use]
    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.inner.fields.push(field.into());
        self
    }

    /// Sets the page size.
    #[must_use]
    pub fn rows(mut self, rows: u32) -> Self {
        self.inner.rows = Some(rows);
        self
    }

    /// Sets the page number.
    #[must_use]
    pub fn page(mut self, page: u32) -> Self {
        self.inner.page = Some(page);
        self
    }

    /// Adds a sort clause.
    #[must_use]
    pub fn sort(mut self, field: impl Into<String>, direction: SortDirection) -> Self {
        self.inner.sorts.push(SearchSort::new(field, direction));
        self
    }

    /// Appends a raw extra query parameter.
    #[must_use]
    pub fn extra_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.inner.extra_params.insert(key.into(), value.into());
        self
    }

    /// Builds the query.
    #[must_use]
    pub fn build(self) -> SearchQuery {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::{SearchQuery, SearchSort, SortDirection};
    use url::Url;

    #[test]
    fn search_query_encodes_common_parameters() {
        let query = SearchQuery::builder("identifier:xfetch")
            .field("identifier")
            .field("title")
            .rows(10)
            .page(2)
            .sort("publicdate", SortDirection::Desc)
            .build();

        let url = query
            .into_url(Url::parse("https://archive.org/advancedsearch.php").unwrap())
            .unwrap();

        assert!(url.as_str().contains("q=identifier%3Axfetch"));
        assert!(url.as_str().contains("fl%5B%5D=identifier"));
        assert!(url.as_str().contains("rows=10"));
        assert!(url.as_str().contains("page=2"));
        assert!(url.as_str().contains("sort%5B%5D=publicdate+desc"));
        assert!(url.as_str().contains("output=json"));
    }

    #[test]
    fn identifier_query_accessors_and_extra_params_work() {
        let query = SearchQuery::builder(SearchQuery::identifier("demo-item").query())
            .field("identifier")
            .extra_param("mediatype", "texts")
            .sort("title", SortDirection::Asc)
            .build();

        assert_eq!(query.query(), "identifier:demo-item");
        assert_eq!(query.fields(), &["identifier".to_owned()]);

        let url = query
            .into_url(Url::parse("https://archive.org/advancedsearch.php").unwrap())
            .unwrap();
        assert!(url.as_str().contains("mediatype=texts"));
        assert!(url.as_str().contains("sort%5B%5D=title+asc"));
    }

    #[test]
    fn search_sort_new_preserves_field_and_direction() {
        let sort = SearchSort::new("publicdate", SortDirection::Asc);
        assert_eq!(sort.field, "publicdate");
        assert_eq!(sort.direction, SortDirection::Asc);
        assert_eq!(sort.as_param(), "publicdate asc");
    }
}
