#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]
//! The API client against a fake `api.php`. No test reaches the live wiki (`testing.md`).

use std::time::{Duration, Instant};

use pvxwiki_scraper::{ApiClient, Pacing, ScrapeError, Transient};
use serde_json::json;
use wiremock::matchers::{header_regex, method, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CONTACT: &str = "ops@example.org";

/// Fast enough for tests, slow enough that a missing wait shows up in elapsed time.
const TEST_PACING: Pacing = Pacing {
    min_delay: Duration::from_millis(150),
    max_retries: 2,
    backoff_base: Duration::from_millis(10),
    backoff_cap: Duration::from_secs(2),
};

fn ok_body() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "query": { "pages": [] } }))
}

async fn client(server: &MockServer) -> ApiClient {
    ApiClient::new(&format!("{}/api.php", server.uri()), CONTACT, TEST_PACING).unwrap()
}

async fn requests(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

#[tokio::test]
async fn sends_the_fixed_parameters_and_an_identifying_user_agent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("action", "query"))
        .and(query_param("format", "json"))
        .and(query_param("formatversion", "2"))
        .and(query_param("maxlag", "5"))
        .and(query_param("list", "categorymembers"))
        .and(header_regex(
            "user-agent",
            r"^pvxwiki-mcp/\d+\.\d+\.\d+ \(https://github\.com/maikbasel/pvxwiki-mcp; ops@example\.org\)$",
        ))
        .respond_with(ok_body())
        .mount(&server)
        .await;

    let value = client(&server)
        .await
        .query(&[("list", "categorymembers")])
        .await
        .unwrap();

    assert_eq!(value, json!({ "query": { "pages": [] } }));
}

#[tokio::test]
async fn spaces_consecutive_requests_by_the_minimum_delay() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok_body())
        .mount(&server)
        .await;
    let mut client = client(&server).await;

    let start = Instant::now();
    for _ in 0..3 {
        client.query(&[]).await.unwrap();
    }

    assert!(
        start.elapsed() >= TEST_PACING.min_delay * 2,
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn retries_a_maxlag_error_then_returns_the_answer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).insert_header("Retry-After", "0").set_body_json(
            json!({ "error": { "code": "maxlag", "info": "Waiting for a replica: 6 seconds lagged" } }),
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ok_body())
        .mount(&server)
        .await;

    let value = client(&server).await.query(&[]).await.unwrap();

    assert_eq!(value["query"]["pages"], json!([]));
    assert_eq!(requests(&server).await, 2);
}

#[tokio::test]
async fn retries_rate_limits_and_server_errors() {
    for status in [429, 500, 502, 503, 504] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(status))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ok_body())
            .mount(&server)
            .await;

        let result = client(&server).await.query(&[]).await;

        assert!(result.is_ok(), "HTTP {status}: {result:?}");
        assert_eq!(requests(&server).await, 2, "HTTP {status}");
    }
}

#[tokio::test]
async fn honours_retry_after_when_longer_than_the_backoff() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ok_body())
        .mount(&server)
        .await;

    let start = Instant::now();
    client(&server).await.query(&[]).await.unwrap();

    assert!(
        start.elapsed() >= Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn a_retry_after_beyond_the_cap_ends_the_run() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "3600"))
        .mount(&server)
        .await;

    let result = tokio::time::timeout(Duration::from_secs(5), client(&server).await.query(&[]))
        .await
        .expect("must not sleep for the hour the server asked for");

    assert!(
        matches!(
            result,
            Err(ScrapeError::RetryAfterTooLong { seconds: 3600 })
        ),
        "{result:?}"
    );
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn gives_up_after_the_retry_budget() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let result = client(&server).await.query(&[]).await;

    assert!(
        matches!(
            result,
            Err(ScrapeError::RetriesExhausted {
                attempts: 3,
                last: Transient::Status(503)
            })
        ),
        "{result:?}"
    );
    assert_eq!(requests(&server).await, 3);
}

#[tokio::test]
async fn a_retry_waits_the_minimum_delay_too() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ok_body())
        .mount(&server)
        .await;

    let start = Instant::now();
    client(&server).await.query(&[]).await.unwrap();

    assert!(
        start.elapsed() >= TEST_PACING.min_delay,
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn forbidden_stops_without_retrying() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let result = client(&server).await.query(&[]).await;

    assert!(
        matches!(result, Err(ScrapeError::Blocked { status: 403 })),
        "{result:?}"
    );
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn a_challenge_page_instead_of_json_stops_without_retrying() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("<html><title>Just a moment...</title></html>", "text/html"),
        )
        .mount(&server)
        .await;

    let result = client(&server).await.query(&[]).await;

    assert!(
        matches!(result, Err(ScrapeError::NotJson { status: 200, .. })),
        "{result:?}"
    );
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn an_api_error_is_returned_without_retrying() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "error": { "code": "badvalue", "info": "Unrecognized value" } }),
        ))
        .mount(&server)
        .await;

    let result = client(&server).await.query(&[]).await;

    assert!(
        matches!(&result, Err(ScrapeError::Api { code, .. }) if code == "badvalue"),
        "{result:?}"
    );
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn an_unexpected_status_is_returned_without_retrying() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({})))
        .mount(&server)
        .await;

    let result = client(&server).await.query(&[]).await;

    assert!(
        matches!(result, Err(ScrapeError::Status { status: 404 })),
        "{result:?}"
    );
    assert_eq!(requests(&server).await, 1);
}

#[test]
fn a_missing_or_unsafe_contact_is_rejected() {
    for contact in ["", "  ", "ops@example.org\r\nX-Injected: 1"] {
        let result = ApiClient::new("http://127.0.0.1:1/api.php", contact, TEST_PACING);
        assert!(
            matches!(result, Err(ScrapeError::InvalidContact)),
            "{contact:?}"
        );
    }
}

#[tokio::test]
async fn a_connection_failure_is_retried_within_the_budget() {
    // Port 1 on loopback: nothing listens, so every connect is refused.
    let mut client = ApiClient::new("http://127.0.0.1:1/api.php", CONTACT, TEST_PACING).unwrap();

    let result = client.query(&[]).await;

    assert!(
        matches!(
            result,
            Err(ScrapeError::RetriesExhausted {
                attempts: 3,
                last: Transient::Network(_)
            })
        ),
        "{result:?}"
    );
}
