#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]
//! A whole scrape run against a fake `api.php` and a real temp-dir store.

use std::collections::HashMap;
use std::time::Duration;

use pvxwiki_scraper::{ApiClient, Pacing, RunReport, run};
use pvxwiki_store::{NewPage, SqliteStore};
use serde_json::{Value, json};
use wiremock::matchers::{method, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PACING: Pacing = Pacing {
    min_delay: Duration::from_millis(1),
    max_retries: 0,
    backoff_base: Duration::from_millis(1),
    backoff_cap: Duration::from_secs(1),
};

fn stored(store: &SqliteStore, title: &str, revision: i64, wikitext: &str) {
    store
        .write_page(&NewPage {
            title: title.into(),
            namespace: 100,
            revision,
            fetched_at: 1,
            parsed: pvxwiki_parser::parse(wikitext),
        })
        .unwrap();
}

/// Lists `pages` (title, revision) in namespace 100; namespace 104 is empty.
async fn listing(wiki: &MockServer, pages: &[(&str, i64)]) {
    let listed: Vec<Value> = pages
        .iter()
        .map(|(t, r)| json!({ "ns": 100, "title": t, "lastrevid": r }))
        .collect();
    Mock::given(method("GET"))
        .and(query_param("gapnamespace", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "batchcomplete": true, "query": { "pages": listed } })),
        )
        .mount(wiki)
        .await;
    Mock::given(method("GET"))
        .and(query_param("gapnamespace", "104"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "batchcomplete": true })))
        .mount(wiki)
        .await;
}

/// Answers one `prop=revisions` call for exactly `pages` (title, revision, wikitext).
async fn contents(wiki: &MockServer, pages: &[(&str, i64, &str)]) {
    let titles: Vec<&str> = pages.iter().map(|p| p.0).collect();
    let fetched: Vec<Value> = pages
        .iter()
        .map(|(t, r, text)| {
            json!({ "ns": 100, "title": t,
                    "revisions": [{ "revid": r, "slots": { "main": { "content": text } } }] })
        })
        .collect();
    Mock::given(method("GET"))
        .and(query_param("prop", "revisions"))
        .and(query_param("titles", titles.join("|")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "batchcomplete": true, "query": { "pages": fetched } })),
        )
        .mount(wiki)
        .await;
}

async fn api(wiki: &MockServer) -> ApiClient {
    ApiClient::new(
        &format!("{}/api.php", wiki.uri()),
        "ops@example.org",
        PACING,
    )
    .unwrap()
}

#[tokio::test]
async fn writes_changed_pages_and_removes_pages_gone_from_the_wiki() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    stored(&store, "Build:Same", 3, "same");
    stored(&store, "Build:Changed", 8, "old");
    stored(&store, "Build:Deleted", 2, "deleted");
    let wiki = MockServer::start().await;
    listing(
        &wiki,
        &[("Build:Same", 3), ("Build:Changed", 9), ("Build:New", 5)],
    )
    .await;
    contents(
        &wiki,
        &[("Build:Changed", 9, "new"), ("Build:New", 5, "new")],
    )
    .await;

    let report = run(&mut api(&wiki).await, &store).await.unwrap();

    assert_eq!(
        report,
        RunReport {
            checked: 3,
            updated: 2,
            kept: 0,
            removed: 1
        }
    );
    assert_eq!(
        store.revisions().unwrap(),
        HashMap::from([
            ("Build:Same".into(), 3),
            ("Build:Changed".into(), 9),
            ("Build:New".into(), 5)
        ])
    );
}

#[tokio::test]
async fn a_revision_that_fails_to_parse_keeps_the_stored_copy() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    stored(&store, "Build:A", 1, "== Usage ==\ngood");
    let wiki = MockServer::start().await;
    listing(&wiki, &[("Build:A", 2)]).await;
    contents(&wiki, &[("Build:A", 2, &"{{".repeat(300))]).await;

    let report = run(&mut api(&wiki).await, &store).await.unwrap();

    assert_eq!(
        report,
        RunReport {
            checked: 1,
            updated: 0,
            kept: 1,
            removed: 0
        }
    );
    assert_eq!(
        store.revisions().unwrap(),
        HashMap::from([("Build:A".into(), 1)])
    );
}

#[tokio::test]
async fn a_new_page_that_fails_to_parse_is_stored_as_raw_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    let wiki = MockServer::start().await;
    listing(&wiki, &[("Build:A", 2)]).await;
    contents(&wiki, &[("Build:A", 2, &"{{".repeat(300))]).await;

    let report = run(&mut api(&wiki).await, &store).await.unwrap();

    assert_eq!(report.updated, 1);
    assert_eq!(
        store.revisions().unwrap(),
        HashMap::from([("Build:A".into(), 2)])
    );
}

#[tokio::test]
async fn a_failed_listing_removes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    stored(&store, "Build:A", 1, "a");
    let wiki = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("gapnamespace", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "batchcomplete": true })))
        .mount(&wiki)
        .await;
    Mock::given(method("GET"))
        .and(query_param("gapnamespace", "104"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&wiki)
        .await;

    let result = run(&mut api(&wiki).await, &store).await;

    assert!(result.is_err(), "{result:?}");
    assert_eq!(
        store.revisions().unwrap(),
        HashMap::from([("Build:A".into(), 1)])
    );
}
