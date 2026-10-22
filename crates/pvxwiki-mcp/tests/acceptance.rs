//! Real server on a temp database filled from the parser fixtures; tools called over
//! streamable HTTP with rmcp's client.

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]

use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::Path;
use std::time::Duration;

use pvxwiki_mcp::config::{Config, LogFormat};
use pvxwiki_scraper::{ApiClient, Pacing};
use pvxwiki_server::{Limits, ProxyList};
use pvxwiki_store::{ARCHIVE_NAMESPACE, NewPage, SqliteStore};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wiremock::matchers::{method, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../parser/tests/fixtures/pages"
);
/// MediaWiki namespace of current builds on the PvX wiki.
const BUILD_NAMESPACE: i64 = 100;

/// Every fixture listed in the fixture README: (file, title, revision).
fn fixture_index() -> Vec<(String, String, i64)> {
    let readme = std::fs::read_to_string(Path::new(FIXTURES).join("README.md")).unwrap();
    readme
        .lines()
        .filter(|l| l.starts_with("| `"))
        .map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let file = cells[1].trim_matches('`').to_string();
            let title = cells[2]
                .trim_start_matches('[')
                .split("](")
                .next()
                .unwrap()
                .to_string();
            (file, title, cells[3].parse().unwrap())
        })
        .collect()
}

fn seed(dir: &Path) {
    let store = SqliteStore::open(dir).unwrap();
    for (file, title, revision) in fixture_index() {
        let text = std::fs::read_to_string(Path::new(FIXTURES).join(&file)).unwrap();
        let namespace = if title.starts_with("Archive:") {
            ARCHIVE_NAMESPACE
        } else {
            BUILD_NAMESPACE
        };
        store
            .write_page(&NewPage {
                title,
                namespace,
                revision,
                fetched_at: 1_700_000_000,
                parsed: pvxwiki_parser::parse(&text),
            })
            .unwrap();
    }
}

fn config(addr: SocketAddr, dir: &Path) -> Config {
    Config {
        listen: addr,
        data_dir: dir.to_path_buf(),
        allowed_hosts: vec!["127.0.0.1".into()],
        trusted_proxies: ProxyList::default(),
        log_format: LogFormat::Pretty,
        contact: None,
    }
}

async fn start(seeded: bool, limits: Limits) -> (SocketAddr, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    if seeded {
        seed(dir.path());
    }
    (serve(dir.path(), limits).await, dir)
}

async fn serve(dir: &Path, limits: Limits) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = config(addr, dir);
    tokio::spawn(async move {
        pvxwiki_mcp::run(config, listener, limits, std::future::pending())
            .await
            .unwrap();
    });
    addr
}

async fn client(addr: SocketAddr) -> RunningService<rmcp::RoleClient, ()> {
    ().serve(StreamableHttpClientTransport::from_uri(format!(
        "http://{addr}/mcp"
    )))
    .await
    .unwrap()
}

async fn call(
    client: &RunningService<rmcp::RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

fn ok(result: &CallToolResult) -> &Value {
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result.structured_content.as_ref().unwrap()
}

fn error_text(result: &CallToolResult) -> String {
    assert_eq!(result.is_error, Some(true), "{result:?}");
    result.content[0].as_text().unwrap().text.clone()
}

fn titles(value: &Value) -> Vec<String> {
    value["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn shutdown_signal_stops_the_server_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(pvxwiki_mcp::run(
        config(addr, dir.path()),
        listener,
        Limits::default(),
        async {
            let _ = stopped.await;
        },
    ));
    let _ = client(addr).await.list_all_tools().await.unwrap();
    stop.send(()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .expect("server did not stop after the shutdown signal")
        .unwrap();
    assert!(result.is_ok(), "{result:?}");
}

#[tokio::test]
async fn both_tools_are_listed_with_output_schemas() {
    let (addr, _dir) = start(true, Limits::default()).await;
    let c = client(addr).await;
    let tools = c.list_tools(None).await.unwrap().tools;
    for name in ["search_builds", "get_build"] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        assert!(tool.output_schema.is_some(), "{name}");
    }
}

#[tokio::test]
async fn empty_index_answers_with_a_tool_error() {
    let (addr, _dir) = start(false, Limits::default()).await;
    let c = client(addr).await;
    assert!(
        error_text(&call(&c, "search_builds", json!({"query": "dagger"})).await)
            .contains("index not built yet")
    );
}

#[tokio::test]
async fn hero_builds_that_work_with_daggers() {
    let (addr, _dir) = start(true, Limits::default()).await;
    let c = client(addr).await;
    let out = call(
        &c,
        "search_builds",
        json!({"query": "dagger", "gameplay_types": ["hero"]}),
    )
    .await;
    let results = ok(&out)["results"].as_array().unwrap().clone();
    assert!(!results.is_empty());
    for r in &results {
        assert!(
            r["gameplay_types"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t.as_str().unwrap().eq_ignore_ascii_case("hero")),
            "{r}"
        );
        assert!(
            r["matches"].as_array().unwrap().iter().any(|m| m["snippet"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("dagger")),
            "{r}"
        );
        assert_eq!(r["attribution"]["licence"], "CC BY-NC-SA 2.5");
    }
}

#[tokio::test]
async fn death_blossom_with_a_zealous_dagger() {
    let (addr, _dir) = start(true, Limits::default()).await;
    let c = client(addr).await;
    let current = call(
        &c,
        "search_builds",
        json!({"skills": ["Death Blossom"], "query": "zealous dagger"}),
    )
    .await;
    let mut found = titles(ok(&current));
    found.sort();
    assert_eq!(
        found,
        [
            "Build:A/Me HazeDemic",
            "Build:A/Mo Imperial Sanctum Runner",
            "Build:A/W Skull Crack Hero",
        ]
    );
    let with_archive = call(
        &c,
        "search_builds",
        json!({"skills": ["Death Blossom"], "query": "zealous dagger", "include_archived": true}),
    )
    .await;
    let found = titles(ok(&with_archive));
    assert!(
        found.contains(&"Archive:A/D Death Blossom Farmer".to_string()),
        "{found:?}"
    );
    assert!(
        found.contains(&"Build:A/Me HazeDemic".to_string()),
        "{found:?}"
    );
}

#[tokio::test]
async fn weapon_mod_questions_return_matching_snippets() {
    let (addr, _dir) = start(true, Limits::default()).await;
    let c = client(addr).await;
    // Words, not phrases: "of the Profession" itself is a link target the parser drops, so the
    // index answers the concrete mod name instead.
    let necro = call(&c, "search_builds", json!({"query": "of the necromancer"})).await;
    let found = titles(ok(&necro));
    assert!(
        found.contains(&"Build:A/Me Shadow Theft Deadly Arts Caster".to_string()),
        "{found:?}"
    );
    let some = call(&c, "search_builds", json!({"query": "of Enchanting"})).await;
    let results = ok(&some)["results"].as_array().unwrap().clone();
    assert!(!results.is_empty());
    // The stemmer folds enchanting/enchantment/enchants into one term; the highlight shows it.
    for r in &results {
        let snippet = r["matches"][0]["snippet"].as_str().unwrap().to_lowercase();
        assert!(snippet.contains("«enchant"), "{r}");
    }
}

#[tokio::test]
async fn get_build_returns_the_full_page() {
    let (addr, _dir) = start(true, Limits::default()).await;
    let c = client(addr).await;
    let out = call(
        &c,
        "get_build",
        json!({"title": "https://gwpvx.fandom.com/wiki/Build:A/Me_Vaettir_Farm"}),
    )
    .await;
    let body = ok(&out);
    assert_eq!(
        body["url"],
        "https://gwpvx.fandom.com/wiki/Build:A/Me_Vaettir_Farm"
    );
    assert_eq!(body["build"]["revision"], 1306202);
    assert_eq!(body["build"]["sections"].as_array().unwrap().len(), 11);
    assert_eq!(body["build"]["skill_bars"][0]["primary"], "Assassin");
    assert_eq!(body["attribution"]["credit"], "PvX wiki contributors");
    assert_eq!(body["attribution"]["licence"], "CC BY-NC-SA 2.5");
}

#[tokio::test]
async fn out_of_bound_inputs_and_unknown_titles_are_tool_errors() {
    let (addr, _dir) = start(true, Limits::default()).await;
    let c = client(addr).await;
    let cases: [(&str, Value, &str); 8] = [
        ("search_builds", json!({"query": "x".repeat(201)}), "query"),
        ("search_builds", json!({"skills": vec!["a"; 9]}), "skills"),
        (
            "search_builds",
            json!({"skills": ["x".repeat(65)]}),
            "skills",
        ),
        ("search_builds", json!({"cursor": "abc"}), "cursor"),
        ("search_builds", json!({"limit": 26}), "limit"),
        (
            "search_builds",
            json!({"gameplay_types": ["nonsense"]}),
            "gameplay_types",
        ),
        ("get_build", json!({"title": "x".repeat(301)}), "title"),
        ("get_build", json!({"title": "Build:Nope"}), "Build:Nope"),
    ];
    for (tool, args, expected) in cases {
        let text = error_text(&call(&c, tool, args.clone()).await);
        assert!(text.contains(expected), "{tool} {args}: {text}");
    }
}

/// Raw HTTP so the rate limiter is observed directly, not through the client's retries.
async fn raw_ping(addr: SocketAddr) -> u16 {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[tokio::test]
async fn bursts_over_the_limit_get_429() {
    let one = NonZeroU32::new(1).unwrap();
    let (addr, _dir) = start(
        true,
        Limits {
            per_second: one,
            burst: NonZeroU32::new(2).unwrap(),
        },
    )
    .await;
    let statuses = [
        raw_ping(addr).await,
        raw_ping(addr).await,
        raw_ping(addr).await,
    ];
    assert_eq!(statuses, [200, 200, 429]);
}

#[tokio::test]
async fn a_scrape_run_fills_the_index_from_the_wiki() {
    let wiki = MockServer::start().await;
    let title = "Build:A/Me Vaettir Farm";
    Mock::given(method("GET"))
        .and(query_param("generator", "allpages"))
        .and(query_param("gapnamespace", BUILD_NAMESPACE.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "batchcomplete": true,
            "query": { "pages": [{ "pageid": 1, "ns": 100, "title": title, "lastrevid": 1306202 }] }
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
        .and(query_param("titles", title))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "batchcomplete": true,
            "query": { "pages": [{
                "pageid": 1, "ns": 100, "title": title,
                "revisions": [{ "revid": 1306202, "slots": { "main": { "content": text } } }]
            }] }
        })))
        .mount(&wiki)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut api = ApiClient::new(
        &format!("{}/api.php", wiki.uri()),
        "ops@example.org",
        Pacing {
            min_delay: Duration::from_millis(1),
            max_retries: 0,
            backoff_base: Duration::from_millis(1),
            backoff_cap: Duration::from_secs(1),
        },
    )
    .unwrap();

    pvxwiki_scraper::run(&mut api, &SqliteStore::open(dir.path()).unwrap())
        .await
        .unwrap();

    let c = client(serve(dir.path(), Limits::default()).await).await;
    let out = call(&c, "get_build", json!({ "title": title })).await;
    let body = ok(&out);
    assert_eq!(body["build"]["revision"], 1306202);
    assert_eq!(body["build"]["skill_bars"][0]["primary"], "Assassin");
    assert_eq!(body["attribution"]["credit"], "PvX wiki contributors");
}
