#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]

use pvxwiki_parser::parse;
use pvxwiki_store::BuildStore;
use pvxwiki_store::{NewPage, SlotDoc, SqliteStore};

fn page(title: &str, revision: i64, wikitext: &str) -> NewPage {
    NewPage {
        title: title.into(),
        namespace: 100,
        revision,
        fetched_at: 1_700_000_000 + revision,
        parsed: parse(wikitext),
    }
}

const VAETTIR: &str =
    include_str!("../../parser/tests/fixtures/pages/Build-A-Me_Vaettir_Farm__1306202.wikitext");

#[tokio::test]
async fn written_page_reads_back_as_a_document() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    store
        .write_page(&page("Build:A/Me Vaettir Farm", 1306202, VAETTIR))
        .unwrap();

    let doc = store
        .get_page("Build:A/Me Vaettir Farm")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(doc.revision, 1306202);
    assert_eq!(doc.vetting.kind, "vetted");
    assert_eq!(doc.vetting.rating.as_deref(), Some("great"));
    assert_eq!(doc.sections.len(), 11);
    assert_eq!(
        doc.sections[2].heading.as_deref(),
        Some("Attributes and Skills")
    );
    assert!(
        doc.sections[2]
            .wikitext
            .contains("[build prof=assas/mesmer")
    );
    let bar = &doc.skill_bars[0];
    assert_eq!(bar.primary.as_deref(), Some("Assassin"));
    assert_eq!(bar.attributes[0].name.as_deref(), Some("Shadow Arts"));
    assert_eq!(bar.slots[3], SlotDoc::Optional);
    let status = store.status().await.unwrap();
    assert_eq!(
        (status.pages, status.updated_at),
        (1, Some(1_700_000_000 + 1306202))
    );
}

#[tokio::test]
async fn unknown_title_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    assert!(store.get_page("Build:Nope").await.unwrap().is_none());
}

#[tokio::test]
async fn rewrite_replaces_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    let old = "{{Real-Vetting|rating=good|hero}}\n== Usage ==\noldword\n<pvxbig>[build prof=A/E][Dash][/build]</pvxbig>";
    let new = "{{Real-Vetting|rating=great|farming}}\n== Usage ==\nnewword\n<pvxbig>[build prof=W/E][Frenzy][/build]</pvxbig>";
    store.write_page(&page("Build:X", 1, old)).unwrap();
    store.write_page(&page("Build:X", 2, new)).unwrap();

    let doc = store.get_page("Build:X").await.unwrap().unwrap();
    assert_eq!(
        (doc.revision, doc.vetting.rating.as_deref()),
        (2, Some("great"))
    );
    let writer = store.writer_for_tests();
    let count = |sql: &str| -> i64 { writer.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM pages"), 1);
    assert_eq!(
        count("SELECT COUNT(*) FROM page_types WHERE gameplay_type = 'hero'"),
        0
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM page_skills WHERE skill = 'dash'"),
        0
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM page_profs WHERE profession = 'assassin'"),
        0
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM page_profs WHERE profession = 'warrior'"),
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM sections_fts WHERE sections_fts MATCH '\"oldword\"'"),
        0
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM sections_fts WHERE sections_fts MATCH '\"newword\"'"),
        1
    );
}

#[tokio::test]
async fn archive_namespace_and_archived_rating_are_archived() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    let mut in_archive = page("Archive:A/D X", 1, "text");
    in_archive.namespace = pvxwiki_store::ARCHIVE_NAMESPACE;
    store.write_page(&in_archive).unwrap();
    store
        .write_page(&page("Build:Y", 1, "{{Real-Vetting|rating=archived}}"))
        .unwrap();
    assert!(
        store
            .get_page("Archive:A/D X")
            .await
            .unwrap()
            .unwrap()
            .is_archived()
    );
    assert!(
        store
            .get_page("Build:Y")
            .await
            .unwrap()
            .unwrap()
            .is_archived()
    );
}

#[tokio::test]
async fn raw_slot_text_bar_source_and_parse_issues_survive() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    let wikitext = "== Bar ==\n<pvxbig>[build prof=W/Zz Strength=12][Flail@5][/build]</pvxbig>";
    store.write_page(&page("Build:Z", 1, wikitext)).unwrap();

    let doc = store.get_page("Build:Z").await.unwrap().unwrap();
    let bar = &doc.skill_bars[0];
    assert!(bar.raw.starts_with("[build prof=W/Zz"));
    assert!(bar.raw.ends_with("[/build]"));
    assert_eq!(
        bar.slots[0],
        SlotDoc::Skill {
            name: "Flail".into(),
            raw: "[Flail@5]".into()
        }
    );
    assert_eq!(doc.issues.len(), 1);
    assert_eq!(doc.issues[0].kind, "unknown_profession");
    assert_eq!(doc.issues[0].detail.as_deref(), Some("Zz"));
    assert_eq!(doc.issues[0].section, Some(1));
}
