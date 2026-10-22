#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]
//! The runner's schedule, history rows and states against a fake `api.php`.

use std::sync::Arc;
use std::time::Duration;

use pvxwiki_scraper::{ApiClient, Pacing, Phase, Progress, RunRefused, Runner, ScraperState};
use pvxwiki_store::{Outcome, RunRecord, SqliteStore, Trigger};
use serde_json::{Value, json};
use wiremock::matchers::{method, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PACING: Pacing = Pacing {
    min_delay: Duration::from_millis(1),
    max_retries: 0,
    backoff_base: Duration::from_millis(1),
    backoff_cap: Duration::from_secs(1),
};

/// No scheduled run besides the one at startup within a test.
const LONG: Duration = Duration::from_secs(3600);

/// How long a test waits for the runner to reach a state.
const DEADLINE: Duration = Duration::from_secs(5);

/// Lists `pages` (title, revision, wikitext) in namespace 100, after `delay`, and serves them.
async fn fake_wiki(wiki: &MockServer, pages: &[(&str, i64, &str)], delay: Duration) {
    let listed: Vec<Value> = pages
        .iter()
        .map(|(t, r, _)| json!({ "ns": 100, "title": t, "lastrevid": r }))
        .collect();
    Mock::given(method("GET"))
        .and(query_param("gapnamespace", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(delay)
                .set_body_json(json!({ "batchcomplete": true, "query": { "pages": listed } })),
        )
        .mount(wiki)
        .await;
    Mock::given(method("GET"))
        .and(query_param("gapnamespace", "104"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "batchcomplete": true })))
        .mount(wiki)
        .await;
    let fetched: Vec<Value> = pages
        .iter()
        .map(|(t, r, text)| {
            json!({ "ns": 100, "title": t,
                    "revisions": [{ "revid": r, "slots": { "main": { "content": text } } }] })
        })
        .collect();
    Mock::given(method("GET"))
        .and(query_param("prop", "revisions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "batchcomplete": true, "query": { "pages": fetched } })),
        )
        .mount(wiki)
        .await;
}

fn api(wiki: &MockServer) -> ApiClient {
    ApiClient::new(
        &format!("{}/api.php", wiki.uri()),
        "ops@example.org",
        PACING,
    )
    .unwrap()
}

/// Polls `check` until it returns `Some`, or panics after `DEADLINE`.
async fn eventually<T>(mut check: impl AsyncFnMut() -> Option<T>) -> T {
    let start = tokio::time::Instant::now();
    loop {
        if let Some(found) = check().await {
            return found;
        }
        assert!(start.elapsed() < DEADLINE, "condition not met in time");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The newest run once it has `outcome`.
async fn newest_run(store: &SqliteStore, outcome: Outcome) -> RunRecord {
    eventually(async || {
        let runs = store.recent_runs(1).await.unwrap();
        runs.into_iter().next().filter(|r| r.outcome == outcome)
    })
    .await
}

#[tokio::test]
async fn startup_run_is_recorded_as_scheduled_and_finished() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path()).unwrap());
    let wiki = MockServer::start().await;
    fake_wiki(&wiki, &[("Build:A", 1, "a")], Duration::ZERO).await;
    let runner = Runner::new(store.clone(), Some(api(&wiki)), LONG);

    tokio::spawn(runner.run_forever());

    let run = newest_run(&store, Outcome::Finished).await;
    assert_eq!(run.trigger, Trigger::Scheduled);
    assert_eq!(run.counts.checked, 1);
}

#[tokio::test]
async fn a_blocked_runner_stops_running_and_refuses_manual_runs() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path()).unwrap());
    let wiki = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&wiki)
        .await;
    let runner = Runner::new(store.clone(), Some(api(&wiki)), Duration::from_millis(10));

    tokio::spawn(runner.clone().run_forever());

    newest_run(&store, Outcome::Blocked).await;
    assert!(matches!(
        runner.status().state,
        ScraperState::Blocked { .. }
    ));
    assert_eq!(runner.status().next_run_at, None, "no run is coming");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(wiki.received_requests().await.unwrap().len(), 1);
    assert_eq!(runner.run_now(), Err(RunRefused::Blocked));
}

#[tokio::test]
async fn manual_run_while_running_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path()).unwrap());
    let wiki = MockServer::start().await;
    fake_wiki(&wiki, &[("Build:A", 1, "a")], Duration::from_millis(500)).await;
    let runner = Runner::new(store.clone(), Some(api(&wiki)), LONG);

    tokio::spawn(runner.clone().run_forever());

    eventually(async || {
        matches!(runner.status().state, ScraperState::Running { .. }).then_some(())
    })
    .await;
    assert_eq!(runner.run_now(), Err(RunRefused::Running));
    newest_run(&store, Outcome::Finished).await;
    assert_eq!(runner.run_now(), Ok(()));
    let manual = eventually(async || {
        let runs = store.recent_runs(1).await.unwrap();
        runs.into_iter()
            .next()
            .filter(|r| r.trigger == Trigger::Manual && r.outcome == Outcome::Finished)
    })
    .await;
    assert_eq!(manual.counts.checked, 1);
}

#[tokio::test]
async fn a_failed_run_is_recorded_and_the_runner_returns_to_idle() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path()).unwrap());
    let wiki = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&wiki)
        .await;
    let runner = Runner::new(store.clone(), Some(api(&wiki)), LONG);

    tokio::spawn(runner.clone().run_forever());

    let run = newest_run(&store, Outcome::Failed).await;
    assert!(run.error.is_some(), "{run:?}");
    assert_eq!(runner.status().state, ScraperState::Idle);
    assert_eq!(runner.run_now(), Ok(()));
}

#[tokio::test]
async fn without_a_client_the_runner_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path()).unwrap());
    let runner = Runner::new(store, None, LONG);

    runner.clone().run_forever().await;

    assert_eq!(runner.status().state, ScraperState::Off);
    assert_eq!(runner.status().next_run_at, None);
    assert_eq!(runner.run_now(), Err(RunRefused::Off));
}

#[tokio::test]
async fn interrupted_rows_are_closed_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path()).unwrap());
    let killed = store.start_run(Trigger::Manual, 1).unwrap();
    let wiki = MockServer::start().await;
    fake_wiki(&wiki, &[("Build:A", 1, "a")], Duration::ZERO).await;
    let runner = Runner::new(store.clone(), Some(api(&wiki)), LONG);

    tokio::spawn(runner.run_forever());

    newest_run(&store, Outcome::Finished).await;
    let runs = store.recent_runs(10).await.unwrap();
    let old = runs.iter().find(|r| r.id == killed).unwrap();
    assert_eq!(old.outcome, Outcome::Interrupted);
}

#[tokio::test]
async fn status_shows_the_progress_of_a_running_run() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path()).unwrap());
    let wiki = MockServer::start().await;
    fake_wiki(&wiki, &[("Build:A", 1, "a")], Duration::from_millis(500)).await;
    let runner = Runner::new(store, Some(api(&wiki)), LONG);

    tokio::spawn(runner.clone().run_forever());

    let progress = eventually(async || match runner.status().state {
        ScraperState::Running { progress, .. } => progress,
        _ => None,
    })
    .await;
    assert_eq!(
        progress,
        Progress {
            phase: Phase::Listing,
            done: 0,
            total: 2
        }
    );
}

#[tokio::test]
async fn a_running_scheduled_run_shows_the_next_run_in_the_future() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path()).unwrap());
    let wiki = MockServer::start().await;
    fake_wiki(&wiki, &[("Build:A", 1, "a")], Duration::from_secs(2)).await;
    let runner = Runner::new(store, Some(api(&wiki)), LONG);

    tokio::spawn(runner.clone().run_forever());

    let next = eventually(async || {
        let status = runner.status();
        matches!(status.state, ScraperState::Running { .. }).then_some(status.next_run_at)
    })
    .await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(next.unwrap() > i64::try_from(now).unwrap(), "{next:?}");
}
