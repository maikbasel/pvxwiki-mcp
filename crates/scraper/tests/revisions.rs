#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]
//! Listing a namespace's revisions against a fake `api.php`. No test reaches the live wiki.

use std::collections::HashMap;
use std::time::Duration;

use pvxwiki_scraper::{ApiClient, FetchedPage, Pacing, ScrapeError};
use serde_json::json;
use wiremock::matchers::{method, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PACING: Pacing = Pacing {
    min_delay: Duration::from_millis(1),
    max_retries: 0,
    backoff_base: Duration::from_millis(1),
    backoff_cap: Duration::from_secs(1),
};

async fn client(server: &MockServer) -> ApiClient {
    ApiClient::new(
        &format!("{}/api.php", server.uri()),
        "ops@example.org",
        PACING,
    )
    .unwrap()
}

fn listing() -> wiremock::MockBuilder {
    Mock::given(method("GET"))
        .and(query_param("generator", "allpages"))
        .and(query_param("gapnamespace", "100"))
        .and(query_param("gaplimit", "50"))
        .and(query_param("gapfilterredir", "nonredirects"))
        .and(query_param("prop", "info"))
}

#[tokio::test]
async fn follows_continue_until_the_namespace_is_listed() {
    let server = MockServer::start().await;
    listing()
        .and(query_param_is_missing("gapcontinue"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "continue": { "gapcontinue": "Build:M", "continue": "gapcontinue||" },
            "query": { "pages": [
                { "pageid": 1, "ns": 100, "title": "Build:A", "lastrevid": 11 },
                { "pageid": 2, "ns": 100, "title": "Build:B", "lastrevid": 12 }
            ] }
        })))
        .mount(&server)
        .await;
    listing()
        .and(query_param("gapcontinue", "Build:M"))
        .and(query_param("continue", "gapcontinue||"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "batchcomplete": true,
            "query": { "pages": [
                { "pageid": 3, "ns": 100, "title": "Build:M", "lastrevid": 13 }
            ] }
        })))
        .mount(&server)
        .await;

    let revisions = client(&server).await.revisions(100).await.unwrap();

    assert_eq!(
        revisions,
        HashMap::from([
            ("Build:A".into(), 11),
            ("Build:B".into(), 12),
            ("Build:M".into(), 13)
        ])
    );
}

#[tokio::test]
async fn a_page_without_a_revision_fails_the_listing() {
    let server = MockServer::start().await;
    listing()
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "batchcomplete": true,
            "query": { "pages": [ { "pageid": 1, "ns": 100, "title": "Build:A" } ] }
        })))
        .mount(&server)
        .await;

    let result = client(&server).await.revisions(100).await;

    assert!(
        matches!(result, Err(ScrapeError::Malformed { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn an_answer_without_pages_fails_the_listing() {
    let server = MockServer::start().await;
    listing()
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let result = client(&server).await.revisions(100).await;

    assert!(
        matches!(result, Err(ScrapeError::Malformed { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn an_empty_namespace_lists_nothing() {
    let server = MockServer::start().await;
    listing()
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "batchcomplete": true })))
        .mount(&server)
        .await;

    let revisions = client(&server).await.revisions(100).await.unwrap();

    assert!(revisions.is_empty());
}

#[tokio::test]
async fn contents_returns_the_wikitext_of_each_existing_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("prop", "revisions"))
        .and(query_param("rvprop", "ids|content"))
        .and(query_param("rvslots", "main"))
        .and(query_param("titles", "Build:A|Build:Gone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "batchcomplete": true,
            "query": { "pages": [
                { "pageid": 1, "ns": 104, "title": "Build:A",
                  "revisions": [{ "revid": 7, "slots": { "main": { "content": "== Usage ==" } } }] },
                { "ns": 100, "title": "Build:Gone", "missing": true }
            ] }
        })))
        .mount(&server)
        .await;

    let pages = client(&server)
        .await
        .contents(&["Build:A".into(), "Build:Gone".into()])
        .await
        .unwrap();

    assert_eq!(
        pages,
        [FetchedPage {
            title: "Build:A".into(),
            namespace: 104,
            revision: 7,
            wikitext: "== Usage ==".into(),
        }]
    );
}
