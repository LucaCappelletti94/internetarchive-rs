//! Error types and response decoding.

use reqwest::Response;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::ids::IdentifierError;

/// Errors produced by the Internet Archive client.
///
/// Archive-side failure modes are outside this crate's control, so new variants
/// arrive without a breaking release.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InternetArchiveError {
    /// The service returned a non-success HTTP response.
    #[error("Internet Archive returned HTTP {status}: {message:?}")]
    Http {
        /// HTTP status code.
        status: HttpStatus,
        /// Machine-friendly code when available.
        code: Option<String>,
        /// Human-readable summary when available.
        message: Option<String>,
        /// Trimmed raw response body.
        raw_body: Option<String>,
    },
    /// Metadata write returned `success: false`.
    #[error("metadata write failed: {message}")]
    MetadataWriteFailed {
        /// Error message returned by MDAPI.
        message: String,
        /// Trimmed raw response body.
        raw_body: Option<String>,
    },
    /// A public item could not be found.
    #[error("item not found: {identifier}")]
    ItemNotFound {
        /// Requested item identifier.
        identifier: String,
    },
    /// The client was used for an authenticated operation without credentials.
    #[error("this operation requires Internet Archive credentials")]
    MissingAuth,
    /// An upload policy rejected an existing file.
    #[error("item already contains file and selected policy forbids overwrite: {filename}")]
    UploadConflict {
        /// Conflicting file name.
        filename: String,
    },
    /// A requested file was not present on an item.
    #[error("item is missing file: {filename}")]
    MissingFile {
        /// Missing file name.
        filename: String,
    },
    /// A workflow invariant was violated.
    #[error("invalid Internet Archive state: {0}")]
    InvalidState(String),
    /// Polling timed out before the requested state was visible.
    #[error("timed out waiting for Internet Archive {0}")]
    Timeout(&'static str),
    /// Request transport failed.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// JSON encoding or decoding failed.
    #[error(transparent)]
    Json(#[from] JsonError),
    /// Local I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// URL construction failed.
    #[error(transparent)]
    Url(#[from] UrlError),
    /// Environment lookup failed.
    #[error("failed to read environment variable {name}: {source}")]
    EnvVar {
        /// Environment variable name.
        name: String,
        /// Underlying lookup error.
        #[source]
        source: std::env::VarError,
    },
    /// Item identifier validation failed.
    #[error(transparent)]
    Identifier(#[from] IdentifierError),
}

/// HTTP status returned by the Archive, owned so the HTTP crate's type stays private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpStatus(u16);

impl HttpStatus {
    /// Returns the status as its numeric code.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Returns whether the status is in the server-error range.
    #[must_use]
    pub const fn is_server_error(self) -> bool {
        self.0 >= 500 && self.0 < 600
    }
}

impl From<u16> for HttpStatus {
    fn from(code: u16) -> Self {
        Self(code)
    }
}

impl PartialEq<u16> for HttpStatus {
    fn eq(&self, other: &u16) -> bool {
        self.0 == *other
    }
}

impl PartialEq<HttpStatus> for u16 {
    fn eq(&self, other: &HttpStatus) -> bool {
        *self == other.0
    }
}

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Transport failure, wrapping the HTTP client's error so its type stays private.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct TransportError(reqwest::Error);

impl TransportError {
    pub(crate) fn new(source: reqwest::Error) -> Self {
        Self(source)
    }

    /// Returns whether the request timed out.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        self.0.is_timeout()
    }

    /// Returns whether the connection could not be established.
    #[must_use]
    pub fn is_connect(&self) -> bool {
        self.0.is_connect()
    }

    /// Returns whether the host could not be resolved.
    ///
    /// A resolution failure also reports as a connection failure, so this
    /// separates the two causes rather than widening what is retried.
    #[must_use]
    pub fn is_dns(&self) -> bool {
        self.0.is_dns()
    }

    /// Returns whether the failure happened while streaming a body.
    #[must_use]
    pub fn is_body(&self) -> bool {
        self.0.is_body()
    }
}

/// JSON encoding or decoding failure, wrapping the serializer's error so its type stays private.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct JsonError(serde_json::Error);

impl JsonError {
    pub(crate) fn new(source: serde_json::Error) -> Self {
        Self(source)
    }
}

/// URL construction failure, wrapping the parser's error so its type stays private.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct UrlError(url::ParseError);

impl UrlError {
    pub(crate) fn new(source: url::ParseError) -> Self {
        Self(source)
    }
}

impl InternetArchiveError {
    pub(crate) async fn from_response(response: Response) -> Self {
        let status = HttpStatus::from(response.status().as_u16());
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let body = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return Self::Transport(TransportError::new(error)),
        };

        decode_http_error(status, content_type.as_deref(), &body)
    }
}

#[derive(Debug, Deserialize)]
struct MdapiError {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct XmlError {
    #[serde(rename = "Code")]
    code: Option<String>,
    #[serde(rename = "Message")]
    message: Option<String>,
}

pub(crate) fn decode_http_error(
    status: HttpStatus,
    content_type: Option<&str>,
    body: &[u8],
) -> InternetArchiveError {
    let raw_body = trimmed_body(body);

    if looks_like_json(content_type, body) {
        if let Ok(parsed) = serde_json::from_slice::<MdapiError>(body) {
            return InternetArchiveError::Http {
                status,
                code: parsed.code,
                message: parsed.error.or(parsed.message).or(raw_body.clone()),
                raw_body,
            };
        }

        if let Ok(parsed) = serde_json::from_slice::<Value>(body) {
            return InternetArchiveError::Http {
                status,
                code: parsed
                    .get("code")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                message: parsed
                    .get("error")
                    .and_then(Value::as_str)
                    .or_else(|| parsed.get("message").and_then(Value::as_str))
                    .or_else(|| parsed.get("title").and_then(Value::as_str))
                    .map(str::to_owned)
                    .or(raw_body.clone()),
                raw_body,
            };
        }
    }

    if looks_like_xml(content_type, body) {
        if let Ok(parsed) = quick_xml::de::from_str::<XmlError>(&String::from_utf8_lossy(body)) {
            return InternetArchiveError::Http {
                status,
                code: parsed.code,
                message: parsed.message.or(raw_body.clone()),
                raw_body,
            };
        }
    }

    InternetArchiveError::Http {
        status,
        code: None,
        message: raw_body.clone(),
        raw_body,
    }
}

pub(crate) fn decode_metadata_write_failure(body: &[u8]) -> Result<(), InternetArchiveError> {
    let parsed: MdapiError = serde_json::from_slice(body).map_err(JsonError::new)?;
    match parsed.success {
        Some(true) => Ok(()),
        _ => Err(InternetArchiveError::MetadataWriteFailed {
            message: parsed
                .error
                .or(parsed.message)
                .unwrap_or_else(|| "unknown metadata write error".to_owned()),
            raw_body: trimmed_body(body),
        }),
    }
}

fn looks_like_json(content_type: Option<&str>, body: &[u8]) -> bool {
    if content_type
        .is_some_and(|value| value.starts_with("application/json") || value.ends_with("+json"))
    {
        return true;
    }

    body.iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| matches!(byte, b'{' | b'['))
}

fn looks_like_xml(content_type: Option<&str>, body: &[u8]) -> bool {
    if content_type
        .is_some_and(|value| value.starts_with("application/xml") || value.starts_with("text/xml"))
    {
        return true;
    }

    body.iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| *byte == b'<')
}

fn trimmed_body(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    for line in text.lines().map(str::trim) {
        if !line.is_empty() {
            return Some(line.chars().take(512).collect());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        decode_http_error, decode_metadata_write_failure, HttpStatus, InternetArchiveError,
    };
    use axum::http::StatusCode as AxumStatusCode;
    use axum::routing::get;
    use axum::{Json, Router};
    use reqwest::StatusCode;
    use serde_json::json;
    use tokio::net::TcpListener;

    #[test]
    fn decodes_json_http_errors() {
        let error = decode_http_error(
            HttpStatus::from(StatusCode::BAD_REQUEST.as_u16()),
            Some("application/json"),
            br#"{"error":"no changes made"}"#,
        );

        match error {
            InternetArchiveError::Http { message, .. } => {
                assert_eq!(message.as_deref(), Some("no changes made"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn decodes_xml_http_errors() {
        let error = decode_http_error(
            HttpStatus::from(StatusCode::SERVICE_UNAVAILABLE.as_u16()),
            Some("application/xml"),
            br"<Error><Code>SlowDown</Code><Message>Too many requests</Message></Error>",
        );

        match error {
            InternetArchiveError::Http { code, message, .. } => {
                assert_eq!(code.as_deref(), Some("SlowDown"));
                assert_eq!(message.as_deref(), Some("Too many requests"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn decodes_plain_text_http_errors() {
        let error = decode_http_error(
            HttpStatus::from(StatusCode::BAD_GATEWAY.as_u16()),
            Some("text/plain"),
            b"gateway down",
        );
        match error {
            InternetArchiveError::Http { message, .. } => {
                assert_eq!(message.as_deref(), Some("gateway down"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn metadata_write_failure_detection_treats_success_false_as_error() {
        let error = decode_metadata_write_failure(
            br#"{"success":false,"error":"No changes made to _meta.xml"}"#,
        )
        .unwrap_err();
        match error {
            InternetArchiveError::MetadataWriteFailed { message, .. } => {
                assert!(message.contains("No changes made"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        assert!(decode_metadata_write_failure(br#"{"success":true,"task_id":1}"#).is_ok());
    }

    #[test]
    fn decodes_json_fallback_value_errors_and_body_heuristics() {
        let error = decode_http_error(
            HttpStatus::from(StatusCode::BAD_REQUEST.as_u16()),
            None,
            br#"  {"error":{"nested":true},"title":"fallback title","code":"bad_request"}"#,
        );

        match error {
            InternetArchiveError::Http {
                code,
                message,
                raw_body,
                ..
            } => {
                assert_eq!(code.as_deref(), Some("bad_request"));
                assert_eq!(message.as_deref(), Some("fallback title"));
                assert!(raw_body.unwrap().contains("fallback title"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn decodes_xml_without_content_type_and_trims_text_bodies() {
        let error = decode_http_error(
            HttpStatus::from(StatusCode::BAD_GATEWAY.as_u16()),
            None,
            b"\n   <Error><Message>temporary outage</Message></Error>",
        );

        match error {
            InternetArchiveError::Http { message, .. } => {
                assert_eq!(message.as_deref(), Some("temporary outage"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let long_text = format!("\n\n{}", "x".repeat(600));
        let trimmed = decode_http_error(
            HttpStatus::from(StatusCode::BAD_GATEWAY.as_u16()),
            Some("text/plain"),
            long_text.as_bytes(),
        );
        match trimmed {
            InternetArchiveError::Http { message, .. } => {
                assert_eq!(message.unwrap().len(), 512);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn metadata_write_failure_without_message_uses_default_error() {
        let error = decode_metadata_write_failure(br#"{"success":false}"#).unwrap_err();
        match error {
            InternetArchiveError::MetadataWriteFailed { message, raw_body } => {
                assert_eq!(message, "unknown metadata write error");
                assert_eq!(raw_body.as_deref(), Some(r#"{"success":false}"#));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn empty_plaintext_body_produces_no_message() {
        let error = decode_http_error(
            HttpStatus::from(StatusCode::BAD_GATEWAY.as_u16()),
            Some("text/plain"),
            b"\n \n\t",
        );
        match error {
            InternetArchiveError::Http {
                message, raw_body, ..
            } => {
                assert_eq!(message, None);
                assert_eq!(raw_body, None);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn from_response_decodes_http_failures() {
        async fn handler() -> (AxumStatusCode, Json<serde_json::Value>) {
            (
                AxumStatusCode::BAD_REQUEST,
                Json(json!({"error":"request failed","code":"bad_request"})),
            )
        }

        let app = Router::new().route("/", get(handler));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        crate::client::ensure_rustls_provider();
        let response = reqwest::get(format!("http://{addr}/")).await.unwrap();
        let error = InternetArchiveError::from_response(response).await;
        match error {
            InternetArchiveError::Http { code, message, .. } => {
                assert_eq!(code.as_deref(), Some("bad_request"));
                assert_eq!(message.as_deref(), Some("request failed"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        server.abort();
    }
}
