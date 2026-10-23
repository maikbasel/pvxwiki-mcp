#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]

use pvxwiki_store::{Outcome, RUN_HISTORY_LIMIT, RunCounts, SqliteStore, Trigger};

fn store() -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    (dir, store)
}

#[tokio::test]
async fn started_run_reads_back_as_running() {
    let (_dir, store) = store();
    store.start_run(Trigger::Manual, 10).unwrap();
    let runs = store.recent_runs(10).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].trigger, Trigger::Manual);
    assert_eq!(runs[0].outcome, Outcome::Running);
    assert_eq!(runs[0].started_at, 10);
    assert_eq!(runs[0].finished_at, None);
}

#[tokio::test]
async fn finished_run_keeps_counts_outcome_and_error() {
    let (_dir, store) = store();
    let id = store.start_run(Trigger::Scheduled, 10).unwrap();
    let counts = RunCounts {
        checked: 3,
        updated: 2,
        kept: 1,
        removed: 4,
    };
    store
        .finish_run(id, Outcome::Failed, 20, counts, Some("boom"))
        .unwrap();
    let run = store.recent_runs(1).await.unwrap().remove(0);
    assert_eq!(run.id, id);
    assert_eq!(run.outcome, Outcome::Failed);
    assert_eq!(run.finished_at, Some(20));
    assert_eq!(run.counts, counts);
    assert_eq!(run.error.as_deref(), Some("boom"));
}

#[tokio::test]
async fn interrupt_marks_only_running_rows() {
    let (_dir, store) = store();
    let done = store.start_run(Trigger::Scheduled, 10).unwrap();
    store
        .finish_run(done, Outcome::Finished, 11, RunCounts::default(), None)
        .unwrap();
    store.start_run(Trigger::Manual, 12).unwrap();
    assert_eq!(store.interrupt_running_runs().unwrap(), 1);
    let runs = store.recent_runs(10).await.unwrap();
    assert_eq!(runs[0].outcome, Outcome::Interrupted);
    assert_eq!(runs[1].outcome, Outcome::Finished);
}

#[tokio::test]
async fn history_keeps_the_newest_runs_and_never_the_running_one() {
    let (_dir, store) = store();
    for n in 0..=RUN_HISTORY_LIMIT {
        let id = store
            .start_run(Trigger::Scheduled, i64::try_from(n).unwrap())
            .unwrap();
        store
            .finish_run(id, Outcome::Finished, 1, RunCounts::default(), None)
            .unwrap();
    }
    store.start_run(Trigger::Manual, 500).unwrap();
    let runs = store.recent_runs(200).await.unwrap();
    assert_eq!(runs.len(), RUN_HISTORY_LIMIT);
    assert_eq!(runs[0].outcome, Outcome::Running);
    assert!(runs.iter().all(|r| r.id != 1));
}

#[tokio::test]
async fn pruning_spares_an_old_run_that_is_still_running() {
    let (_dir, store) = store();
    let stuck = store.start_run(Trigger::Manual, 0).unwrap();
    for n in 1..=RUN_HISTORY_LIMIT {
        let id = store
            .start_run(Trigger::Scheduled, i64::try_from(n).unwrap())
            .unwrap();
        store
            .finish_run(id, Outcome::Finished, 1, RunCounts::default(), None)
            .unwrap();
    }
    let runs = store.recent_runs(200).await.unwrap();
    assert!(runs.iter().any(|r| r.id == stuck));
}
