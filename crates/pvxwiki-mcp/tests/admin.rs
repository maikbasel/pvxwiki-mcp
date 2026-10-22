//! Real server with the admin listener on its own port; the wiki is faked with wiremock.

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use pvxwiki_mcp::Schedule;
use pvxwiki_mcp::config::{Config, LogFormat};
use pvxwiki_scraper::Pacing;
use pvxwiki_server::{Limits, ProxyList};
use pvxwiki_store::ARCHIVE_NAMESPACE;
use serde_json::json;
use wiremock::matchers::{method, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../parser/tests/fixtures/pages"
);
/// MediaWiki namespace of current builds on the PvX wiki.
const BUILD_NAMESPACE: i64 = 100;
const VAETTIR: &str = "Build:A/Me Vaettir Farm";
const CONTACT: &str = "ops@example.org";
const WAIT: Duration = Duration::from_secs(5);

const FAST: Pacing = Pacing {
    min_delay: Duration::from_millis(1),
    max_retries: 0,
    backoff_base: Duration::from_millis(1),
    backoff_cap: Duration::from_secs(1),
};

/// A wiki with one build page, the Vaettir fixture at revision 1306202.
async fn fake_wiki() -> MockServer {
    let wiki = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("generator", "allpages"))
        .and(query_param("gapnamespace", BUILD_NAMESPACE.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "batchcomplete": true,
            "query": { "pages": [{ "pageid": 1, "ns": 100, "title": VAETTIR, "lastrevid": 1306202 }] }
        })))
        .mount(&wiki)
        .await;
    Mock::given(method("GET"))
        .and(query_param("generator", "allpages"))
        .and(query_param("gapnamespace", ARCHIVE_NAMESPACE.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "batchcomplete": true })))
        .mount(&wiki)
        .await;
    let text = std::fs::read_to_string(
        Path::new(FIXTURES).join("Build-A-Me_Vaettir_Farm__1306202.wikitext"),
    )
    .unwrap();
    Mock::given(method("GET"))
        .and(query_param("prop", "revisions"))
        .and(query_param("titles", VAETTIR))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "batchcomplete": true,
            "query": { "pages": [{
                "pageid": 1, "ns": 100, "title": VAETTIR,
                "revisions": [{ "revid": 1306202, "slots": { "main": { "content": text } } }]
            }] }
        })))
        .mount(&wiki)
        .await;
    wiki
}

/// Both listeners on free ports; returns the admin address and the data directory guard.
async fn start(contact: Option<&str>, wiki: &MockServer) -> (SocketAddr, tempfile::TempDir) {
    let (admin, _mcp, dir) = start_both(contact, wiki).await;
    (admin, dir)
}

/// Like `start`, but also returns the MCP address.
async fn start_both(
    contact: Option<&str>,
    wiki: &MockServer,
) -> (SocketAddr, SocketAddr, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let admin_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let admin = admin_listener.local_addr().unwrap();
    let config = Config {
        listen: addr,
        admin_listen: admin,
        data_dir: dir.path().to_path_buf(),
        allowed_hosts: vec!["127.0.0.1".into()],
        trusted_proxies: ProxyList::default(),
        log_format: LogFormat::Pretty,
        contact: contact.map(Into::into),
    };
    let schedule = Schedule {
        endpoint: format!("{}/api.php", wiki.uri()),
        pacing: FAST,
        interval: Duration::from_secs(3600),
    };
    tokio::spawn(pvxwiki_mcp::run(
        config,
        listener,
        admin_listener,
        Limits::default(),
        schedule,
        std::future::pending(),
    ));
    (admin, addr, dir)
}

async fn post_run(addr: SocketAddr, headers: &[(&str, &str)]) -> reqwest::Response {
    let mut request = reqwest::Client::new().post(format!("http://{addr}/runs"));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.send().await.unwrap()
}

const SAME_ORIGIN_HTMX: [(&str, &str); 2] =
    [("HX-Request", "true"), ("Sec-Fetch-Site", "same-origin")];

async fn page(addr: SocketAddr) -> String {
    reqwest::get(format!("http://{addr}/"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

/// Polls the page until it contains `needle`; panics with the last body after `WAIT`.
async fn wait_for(addr: SocketAddr, needle: &str) -> String {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let body = page(addr).await;
        if body.contains(needle) {
            return body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no {needle:?} on the page:\n{body}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn history_shows_the_startup_run_once_it_finishes() {
    let wiki = fake_wiki().await;
    let (admin, _dir) = start(Some(CONTACT), &wiki).await;
    let body = wait_for(admin, "data-outcome=\"finished\"").await;
    assert!(body.contains("data-trigger=\"scheduled\""), "{body}");
    assert!(body.contains("data-checked=\"1\""), "{body}");
}

#[tokio::test]
async fn run_now_starts_a_manual_run() {
    let wiki = fake_wiki().await;
    let (admin, _dir) = start(Some(CONTACT), &wiki).await;
    wait_for(admin, "data-outcome=\"finished\"").await;
    let response = post_run(admin, &SAME_ORIGIN_HTMX).await;
    assert_eq!(response.status(), 202);
    wait_for(admin, "data-trigger=\"manual\"").await;
}

#[tokio::test]
async fn run_now_while_running_is_refused() {
    let wiki = MockServer::start().await;
    // The startup run is still listing namespace 100 when the request arrives.
    Mock::given(method("GET"))
        .and(query_param("generator", "allpages"))
        .and(query_param("gapnamespace", BUILD_NAMESPACE.to_string()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "batchcomplete": true }))
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&wiki)
        .await;
    let (admin, _dir) = start(Some(CONTACT), &wiki).await;
    wait_for(admin, "data-state=\"running\"").await;
    let response = post_run(admin, &SAME_ORIGIN_HTMX).await;
    assert_eq!(response.status(), 409);
}

#[tokio::test]
async fn cross_site_run_now_is_refused() {
    let wiki = fake_wiki().await;
    let (admin, _dir) = start(Some(CONTACT), &wiki).await;
    wait_for(admin, "data-outcome=\"finished\"").await;
    let cross_site = post_run(
        admin,
        &[("HX-Request", "true"), ("Sec-Fetch-Site", "cross-site")],
    )
    .await;
    assert_eq!(cross_site.status(), 403);
    let no_htmx = post_run(admin, &[("Sec-Fetch-Site", "same-origin")]).await;
    assert_eq!(no_htmx.status(), 403);
}

#[tokio::test]
async fn run_now_without_a_contact_is_refused() {
    let wiki = fake_wiki().await;
    let (admin, _dir) = start(None, &wiki).await;
    let response = post_run(admin, &SAME_ORIGIN_HTMX).await;
    assert_eq!(response.status(), 409);
    assert!(page(admin).await.contains("PVXWIKI_CONTACT"));
}

#[tokio::test]
async fn run_now_after_a_block_is_refused() {
    let wiki = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&wiki)
        .await;
    let (admin, _dir) = start(Some(CONTACT), &wiki).await;
    wait_for(admin, "data-state=\"blocked\"").await;
    let response = post_run(admin, &SAME_ORIGIN_HTMX).await;
    assert_eq!(response.status(), 409);
}

#[tokio::test]
async fn unknown_host_is_refused() {
    let wiki = fake_wiki().await;
    let (admin, _dir) = start(Some(CONTACT), &wiki).await;
    let response = reqwest::Client::new()
        .get(format!("http://{admin}/"))
        .header("Host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 421);
}

#[tokio::test]
async fn admin_is_not_served_on_the_mcp_port() {
    let wiki = fake_wiki().await;
    let (_admin, mcp, _dir) = start_both(Some(CONTACT), &wiki).await;
    assert_eq!(page_status(mcp).await, 404);
    let response = post_run(mcp, &SAME_ORIGIN_HTMX).await;
    assert_eq!(response.status(), 404);
}

async fn page_status(addr: SocketAddr) -> u16 {
    reqwest::get(format!("http://{addr}/"))
        .await
        .unwrap()
        .status()
        .as_u16()
}
