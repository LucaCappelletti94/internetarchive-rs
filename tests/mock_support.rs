#![allow(
    missing_docs,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::unwrap_used
)]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;
use internetarchive_rs::{Auth, Endpoint, InternetArchiveClient, PollOptions};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;

#[derive(Clone, Debug)]
pub struct CapturedRequest {
    pub method: Method,
    pub path: String,
    pub query: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct QueuedResponse {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl QueuedResponse {
    pub fn json(status: StatusCode, body: serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(&body).expect("json serialization"),
        }
    }

    pub fn text(status: StatusCode, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: body.into().into_bytes(),
        }
    }

    pub fn bytes(status: StatusCode, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }
}

#[derive(Default)]
struct MockState {
    responses: Mutex<HashMap<(Method, String), VecDeque<QueuedResponse>>>,
    requests: Mutex<Vec<CapturedRequest>>,
}

pub struct MockInternetArchiveServer {
    pub archive_base: Url,
    pub s3_base: Url,
    state: Arc<MockState>,
    handle: JoinHandle<()>,
}

impl MockInternetArchiveServer {
    pub async fn start() -> Self {
        let state = Arc::new(MockState::default());
        let app = Router::new()
            .fallback(any(handle_request))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let addr = listener.local_addr().expect("mock address");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock server");
        });

        Self {
            archive_base: Url::parse(&format!("http://{addr}/")).expect("archive url"),
            s3_base: Url::parse(&format!("http://{addr}/s3/")).expect("s3 url"),
            state,
            handle,
        }
    }

    pub fn client(&self) -> InternetArchiveClient {
        InternetArchiveClient::builder()
            .auth(Auth::new("access", "secret"))
            .endpoint(Endpoint::custom(
                self.archive_base.clone(),
                self.s3_base.clone(),
            ))
            .poll_options(PollOptions {
                max_wait: Duration::from_millis(200),
                initial_delay: Duration::from_millis(5),
                max_delay: Duration::from_millis(10),
            })
            .build()
            .expect("build mock client")
    }

    pub fn enqueue(&self, method: Method, path: &str, response: QueuedResponse) {
        let mut responses = self.state.responses.lock().expect("lock responses");
        responses
            .entry((method, path.to_owned()))
            .or_default()
            .push_back(response);
    }

    pub fn enqueue_json(
        &self,
        method: Method,
        path: &str,
        status: StatusCode,
        body: serde_json::Value,
    ) {
        self.enqueue(method, path, QueuedResponse::json(status, body));
    }

    pub fn requests(&self) -> Vec<CapturedRequest> {
        self.state.requests.lock().expect("lock requests").clone()
    }
}

impl Drop for MockInternetArchiveServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn handle_request(
    State(state): State<Arc<MockState>>,
    request: Request<Body>,
) -> impl IntoResponse {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, usize::MAX).await.expect("read request body");

    let captured = CapturedRequest {
        method: parts.method.clone(),
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().map(str::to_owned),
        headers: normalize_headers(&parts.headers),
        body: body.to_vec(),
    };
    state
        .requests
        .lock()
        .expect("lock requests")
        .push(captured.clone());

    let response = state
        .responses
        .lock()
        .expect("lock responses")
        .get_mut(&(parts.method, parts.uri.path().to_owned()))
        .and_then(VecDeque::pop_front);

    match response {
        Some(response) => build_response(response),
        None => build_response(QueuedResponse::text(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "no queued response for {} {}{}",
                captured.method,
                captured.path,
                captured
                    .query
                    .as_deref()
                    .map(|value| format!("?{value}"))
                    .unwrap_or_default()
            ),
        )),
    }
}

fn normalize_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_ascii_lowercase(), value.to_owned()))
        })
        .collect()
}

fn build_response(response: QueuedResponse) -> Response<Body> {
    let mut builder = Response::builder().status(response.status);
    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }

    builder
        .body(Body::from(response.body))
        .expect("build response")
}
