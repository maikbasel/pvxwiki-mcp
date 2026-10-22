#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]

use pvxwiki_parser::parse;
use pvxwiki_store::{NewPage, ParseFailure, SqliteStore};

fn page(title: &str, namespace: i64, revision: i64, wikitext: &str) -> NewPage {
    NewPage {
        title: title.into(),
        namespace,
        revision,
        fetched_at: 1_700_000_000,
        parsed: parse(wikitext),
    }
}

fn open() -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    (dir, store)
}

#[tokio::test]
async fn recorded_failure_lists_stored_and_failed_revision() {
    let (_dir, store) = open();
    store.write_page(&page("A", 100, 1, "x")).unwrap();
    store.record_parse_failure("A", 2, 50).unwrap();
    assert_eq!(
        store.parse_failures().await.unwrap(),
        [ParseFailure {
            title: "A".into(),
            stored_revision: 1,
            failed_revision: 2,
            first_seen: 50
        }]
    );
}

#[tokio::test]
async fn writing_or_deleting_the_page_clears_its_failure() {
    let (_dir, store) = open();
    store.write_page(&page("A", 100, 1, "x")).unwrap();
    store.write_page(&page("B", 100, 1, "x")).unwrap();
    store.record_parse_failure("A", 2, 50).unwrap();
    store.record_parse_failure("B", 2, 50).unwrap();
    store.write_page(&page("A", 100, 3, "x")).unwrap();
    store.delete_page("B").unwrap();
    assert!(store.parse_failures().await.unwrap().is_empty());
    let n: i64 = store
        .writer_for_tests()
        .query_row("SELECT COUNT(*) FROM parse_failures", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn namespace_counts_split_build_and_archive() {
    let (_dir, store) = open();
    store.write_page(&page("A", 100, 1, "x")).unwrap();
    store.write_page(&page("B", 100, 1, "x")).unwrap();
    store.write_page(&page("C", 104, 1, "x")).unwrap();
    assert_eq!(
        store.namespace_counts().await.unwrap(),
        [(100, 2), (104, 1)]
    );
}

#[tokio::test]
async fn issue_counts_and_capped_issue_list() {
    let (_dir, store) = open();
    let deep = "{{".repeat(300);
    for n in 0..51 {
        store
            .write_page(&page(&format!("Deep {n:02}"), 100, n, &deep))
            .unwrap();
    }
    let unknown = "== A ==\n<pvxbig>[build prof=Q/E][Dash][/build]</pvxbig>";
    store.write_page(&page("Odd", 100, 7, unknown)).unwrap();

    assert_eq!(
        store.issue_counts().await.unwrap(),
        [
            ("too_deeply_nested".to_string(), 51),
            ("unknown_profession".to_string(), 1)
        ]
    );
    let (pages, total) = store.pages_with_issue("too_deeply_nested").await.unwrap();
    assert_eq!((pages.len(), total), (50, 51));
    assert_eq!(pages[0].title, "Deep 00");
    assert_eq!(pages[49].title, "Deep 49");
}
