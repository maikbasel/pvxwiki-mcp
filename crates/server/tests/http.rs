#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]

use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode, header};
use pvxwiki_parser::parse;
use pvxwiki_server::{HttpConfig, Limits, MAX_CONCURRENT, ProxyList, limiter, router};
use pvxwiki_store::fake::FakeStore;
use pvxwiki_store::{
    BuildDocument, BuildStore, IndexStatus, NewPage, SearchPage, SearchQuery, StoreError,
    StoreFuture,
};
use tower::ServiceExt as _;

fn app_with(store: Arc<dyn BuildStore>, peer: [u8; 4], trusted: &str, burst: u32) -> axum::Router {
    let config = HttpConfig {
        allowed_hosts: vec!["127.0.0.1".into()],
        trusted_proxies: ProxyList::parse(trusted).unwrap(),
    };
    let one = NonZeroU32::new(1).unwrap();
    router(
        store,
        &config,
        limiter(Limits {
            per_second: one,
            burst: NonZeroU32::new(burst).unwrap(),
        }),
    )
    .layer(MockConnectInfo(SocketAddr::from((peer, 4000))))
}

fn app(peer: [u8; 4], trusted: &str, burst: u32) -> axum::Router {
    let store = Arc::new(FakeStore::new(&[NewPage {
        title: "Build:X".into(),
        namespace: 100,
        revision: 1,
        fetched_at: 1,
        parsed: parse("x"),
    }]));
    app_with(store, peer, trusted, burst)
}

fn post(host: &str, forwarded_for: Option<&str>, body: &'static str) -> Request<Body> {
    let mut req = Request::post("/mcp")
        .header(header::HOST, host)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(ff) = forwarded_for {
        req = req.header("x-forwarded-for", ff);
    }
    req.body(Body::from(body)).unwrap()
}

fn ping(host: &str, forwarded_for: Option<&str>) -> Request<Body> {
    post(
        host,
        forwarded_for,
        r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
    )
}

fn healthz() -> Request<Body> {
    Request::get("/healthz").body(Body::empty()).unwrap()
}

#[derive(PartialEq)]
enum Status {
    Fails,
    Hangs,
    Answers,
}

/// `status()` fails, never answers, or answers; counts calls and dropped (cancelled) calls.
struct StuckStore {
    status: Status,
    entered: AtomicUsize,
    dropped: AtomicUsize,
}

fn stuck(status: Status) -> Arc<StuckStore> {
    Arc::new(StuckStore {
        status,
        entered: AtomicUsize::new(0),
        dropped: AtomicUsize::new(0),
    })
}

struct CountDrop<'a>(&'a AtomicUsize);

impl Drop for CountDrop<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl BuildStore for StuckStore {
    fn search<'a>(&'a self, _: &'a SearchQuery) -> StoreFuture<'a, SearchPage> {
        Box::pin(std::future::pending())
    }
    fn get_page<'a>(&'a self, _: &'a str) -> StoreFuture<'a, Option<BuildDocument>> {
        Box::pin(std::future::pending())
    }
    fn status(&self) -> StoreFuture<'_, IndexStatus> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        match self.status {
            Status::Fails => Box::pin(async { Err(StoreError::Poisoned) }),
            Status::Answers => Box::pin(async {
                Ok(IndexStatus {
                    pages: 1,
                    updated_at: None,
                })
            }),
            Status::Hangs => Box::pin(async move {
                let _guard = CountDrop(&self.dropped);
                std::future::pending().await
            }),
        }
    }
}

async fn wait_until(what: impl Fn() -> bool) {
    let poll = async {
        while !what() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5), poll)
        .await
        .unwrap();
}

const SEARCH_CALL: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_builds","arguments":{}}}"#;

#[tokio::test]
async fn second_request_over_burst_is_429_with_retry_after() {
    let app = app([203, 0, 113, 7], "", 1);
    assert_ne!(
        app.clone()
            .oneshot(ping("127.0.0.1", None))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let limited = app.clone().oneshot(ping("127.0.0.1", None)).await.unwrap();
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.headers()[header::RETRY_AFTER], "1");
}

#[tokio::test]
async fn trusted_proxy_rate_limits_per_forwarded_client() {
    let app = app([10, 0, 0, 5], "10.0.0.5", 1);
    assert_ne!(
        app.clone()
            .oneshot(ping("127.0.0.1", Some("198.51.100.1")))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_ne!(
        app.clone()
            .oneshot(ping("127.0.0.1", Some("198.51.100.2")))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        app.clone()
            .oneshot(ping("127.0.0.1", Some("198.51.100.1")))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn spoofed_header_from_untrusted_peer_shares_the_peer_bucket() {
    let app = app([203, 0, 113, 7], "", 1);
    assert_ne!(
        app.clone()
            .oneshot(ping("127.0.0.1", Some("1.1.1.1")))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        app.clone()
            .oneshot(ping("127.0.0.1", Some("2.2.2.2")))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn healthz_is_outside_every_limit() {
    let app = app([203, 0, 113, 7], "", 1);
    let _ = app.clone().oneshot(ping("127.0.0.1", None)).await.unwrap();
    for _ in 0..5 {
        let res = app.clone().oneshot(healthz()).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn healthz_is_503_when_the_store_fails() {
    let app = app_with(stuck(Status::Fails), [203, 0, 113, 7], "", 1);
    let res = app.oneshot(healthz()).await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn requests_over_the_concurrency_cap_are_shed_with_503() {
    let store = stuck(Status::Hangs);
    let app = app_with(store.clone(), [203, 0, 113, 7], "", 1_000);
    let stuck: Vec<_> = (0..MAX_CONCURRENT)
        .map(|_| tokio::spawn(app.clone().oneshot(post("127.0.0.1", None, SEARCH_CALL))))
        .collect();
    wait_until(|| store.entered.load(Ordering::SeqCst) == MAX_CONCURRENT).await;
    let res = app.clone().oneshot(ping("127.0.0.1", None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    for task in stuck {
        task.abort();
    }
}

#[tokio::test]
async fn oversized_body_is_413() {
    let app = app([203, 0, 113, 7], "", 10);
    let big = "x".repeat(pvxwiki_server::MAX_BODY_BYTES + 1);
    let req = Request::post("/mcp")
        .header(header::HOST, "127.0.0.1")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, big.len())
        .body(Body::from(big))
        .unwrap();
    assert_eq!(
        app.oneshot(req).await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn unknown_host_is_rejected() {
    let app = app([203, 0, 113, 7], "", 10);
    let status = app
        .oneshot(ping("evil.example", None))
        .await
        .unwrap()
        .status();
    assert!(status.is_client_error(), "{status}");
}

#[tokio::test]
async fn a_spoofed_first_forwarded_for_line_does_not_pick_the_bucket() {
    // The client sends its own line; the trusted proxy appends the real address as a second line.
    let app = app([10, 0, 0, 5], "10.0.0.5", 1);
    let two_lines = |spoofed: &str| {
        let mut req = ping("127.0.0.1", Some(spoofed));
        req.headers_mut()
            .append("x-forwarded-for", "198.51.100.9".parse().unwrap());
        req
    };
    assert_ne!(
        app.clone()
            .oneshot(two_lines("1.1.1.1"))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        app.clone()
            .oneshot(two_lines("2.2.2.2"))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn a_non_ascii_forwarded_for_line_falls_back_to_the_peer() {
    let app = app([10, 0, 0, 5], "10.0.0.5", 1);
    let with_bad_line = |real: &str| {
        let mut req = ping("127.0.0.1", Some(real));
        req.headers_mut().append(
            "x-forwarded-for",
            axum::http::HeaderValue::from_bytes(b"\xff").unwrap(),
        );
        req
    };
    assert_ne!(
        app.clone()
            .oneshot(with_bad_line("198.51.100.1"))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // A different forwarded address, but both requests are keyed on the peer.
    assert_eq!(
        app.clone()
            .oneshot(with_bad_line("198.51.100.2"))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn a_dropped_request_cancels_its_store_call() {
    let store = stuck(Status::Hangs);
    let app = app_with(store.clone(), [203, 0, 113, 7], "", 10);
    let call = tokio::spawn(app.oneshot(post("127.0.0.1", None, SEARCH_CALL)));
    wait_until(|| store.entered.load(Ordering::SeqCst) == 1).await;
    // What a client disconnect or REQUEST_TIMEOUT does to the HTTP future.
    call.abort();
    wait_until(|| store.dropped.load(Ordering::SeqCst) == 1).await;
}

#[tokio::test]
async fn healthz_probes_the_store_at_most_once_per_ttl() {
    let store = stuck(Status::Answers);
    let app = app_with(store.clone(), [203, 0, 113, 7], "", 1);
    let calls: Vec<_> = (0..20)
        .map(|_| tokio::spawn(app.clone().oneshot(healthz())))
        .collect();
    for call in calls {
        assert_eq!(call.await.unwrap().unwrap().status(), StatusCode::OK);
    }
    assert_eq!(
        app.clone().oneshot(healthz()).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(store.entered.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn internal_tool_error_carries_the_http_request_id() {
    let app = app_with(stuck(Status::Fails), [203, 0, 113, 7], "", 10);
    let res = app
        .oneshot(post("127.0.0.1", None, SEARCH_CALL))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("request ID "), "{body}");
}
