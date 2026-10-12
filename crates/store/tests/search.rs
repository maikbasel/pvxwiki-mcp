#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]

use pvxwiki_parser::parse;
use pvxwiki_store::BuildStore;
use pvxwiki_store::{NewPage, SearchQuery, SqliteStore};

fn store(pages: &[(&str, &str)]) -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path()).unwrap();
    for (i, (title, text)) in pages.iter().enumerate() {
        let revision = i64::try_from(i).unwrap() + 1;
        store
            .write_page(&NewPage {
                title: (*title).into(),
                namespace: 100,
                revision,
                fetched_at: revision,
                parsed: parse(text),
            })
            .unwrap();
    }
    (dir, store)
}

fn titles(page: &pvxwiki_store::SearchPage) -> Vec<&str> {
    page.hits
        .iter()
        .map(|h| h.document.title.as_str())
        .collect()
}

fn text(q: &str) -> SearchQuery {
    SearchQuery {
        text: Some(q.into()),
        limit: 10,
        ..Default::default()
    }
}

#[tokio::test]
async fn heading_match_outranks_body_match_and_snippets_mark_the_word() {
    let (_d, s) = store(&[
        (
            "Build:Body",
            "{{Real-Vetting|rating=good}}\n== Usage ==\nswing your dagger",
        ),
        (
            "Build:Heading",
            "{{Real-Vetting|rating=good}}\n== Dagger tips ==\nunrelated",
        ),
    ]);
    let page = s.search(&text("daggers")).await.unwrap();
    assert_eq!(titles(&page), ["Build:Heading", "Build:Body"]);
    let body = &page.hits[1].matches[0];
    assert_eq!(body.heading.as_deref(), Some("Usage"));
    assert!(body.snippet.contains("«dagger»"), "{}", body.snippet);
}

#[tokio::test]
async fn operator_input_matches_literally() {
    let (_d, s) = store(&[
        ("Build:Lit", "== Usage ==\ndo NOT dagger* stop"),
        ("Build:Other", "== Usage ==\ndagger"),
    ]);
    assert_eq!(
        titles(&s.search(&text("NOT dagger*")).await.unwrap()),
        ["Build:Lit"]
    );
    assert!(
        s.search(&text("\" NEAR( col:x ^"))
            .await
            .unwrap()
            .hits
            .is_empty()
    );
}

#[tokio::test]
async fn filters_only_orders_by_rating_then_title() {
    let (_d, s) = store(&[
        ("Build:B good", "{{Real-Vetting|rating=good}}"),
        ("Build:A testing", "{{Real-Vetting|rating=testing}}"),
        ("Build:C great", "{{Real-Vetting|rating=great}}"),
        ("Build:A great", "{{Real-Vetting|rating=great}}"),
    ]);
    let page = s
        .search(&SearchQuery {
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        titles(&page),
        [
            "Build:A great",
            "Build:C great",
            "Build:B good",
            "Build:A testing"
        ]
    );
    assert!(page.hits.iter().all(|h| h.matches.is_empty()));
}

#[tokio::test]
async fn skills_must_share_one_bar() {
    let (_d, s) = store(&[(
        "Build:Two bars",
        "<pvxbig>[build prof=A/E][Dash][Shadow Form][/build]\n[build prof=W/E][Frenzy][/build]</pvxbig>",
    )]);
    let q = |skills: &[&str]| SearchQuery {
        skills: skills.iter().map(|s| s.to_string()).collect(),
        limit: 10,
        ..Default::default()
    };
    assert_eq!(
        s.search(&q(&["dash", "shadow form"]))
            .await
            .unwrap()
            .hits
            .len(),
        1
    );
    assert!(
        s.search(&q(&["dash", "frenzy"]))
            .await
            .unwrap()
            .hits
            .is_empty()
    );
    assert_eq!(s.search(&q(&["dash", "dash"])).await.unwrap().hits.len(), 1);
}

#[tokio::test]
async fn profession_type_rating_and_archive_filters() {
    let (_d, s) = store(&[
        (
            "Build:Sin",
            "{{Real-Vetting|rating=great|hero}}\n<pvxbig>[build prof=A/E][Dash][/build]</pvxbig>",
        ),
        (
            "Build:War",
            "{{Real-Vetting|rating=good|PvE team}}\n<pvxbig>[build prof=W/E][Frenzy][/build]</pvxbig>",
        ),
        ("Build:Old", "{{Real-Vetting|rating=archived|hero}}"),
    ]);
    let base = SearchQuery {
        limit: 10,
        ..Default::default()
    };
    let with = |f: fn(&mut SearchQuery)| {
        let mut q = base.clone();
        f(&mut q);
        q
    };
    assert_eq!(
        titles(
            &s.search(&with(|q| q.professions = vec!["assassin".into()]))
                .await
                .unwrap()
        ),
        ["Build:Sin"]
    );
    assert_eq!(
        titles(
            &s.search(&with(|q| q.gameplay_types = vec!["pve team".into()]))
                .await
                .unwrap()
        ),
        ["Build:War"]
    );
    assert_eq!(
        titles(
            &s.search(&with(|q| q.ratings = vec!["good".into()]))
                .await
                .unwrap()
        ),
        ["Build:War"]
    );
    assert_eq!(
        titles(
            &s.search(&with(|q| q.gameplay_types = vec!["hero".into()]))
                .await
                .unwrap()
        ),
        ["Build:Sin"]
    );
    let archived = s
        .search(&with(|q| {
            q.gameplay_types = vec!["hero".into()];
            q.include_archived = true;
        }))
        .await
        .unwrap();
    assert_eq!(titles(&archived), ["Build:Sin", "Build:Old"]);
}

#[tokio::test]
async fn pagination_reports_more() {
    let (_d, s) = store(&[("Build:A", "x"), ("Build:B", "x"), ("Build:C", "x")]);
    let first = s
        .search(&SearchQuery {
            limit: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        (titles(&first), first.has_more),
        (vec!["Build:A", "Build:B"], true)
    );
    let second = s
        .search(&SearchQuery {
            limit: 2,
            offset: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!((titles(&second), second.has_more), (vec!["Build:C"], false));
}

#[tokio::test]
async fn at_most_three_matches_per_hit() {
    let (_d, s) = store(&[(
        "Build:Many",
        "== A ==\ndagger\n== B ==\ndagger\n== C ==\ndagger\n== D ==\ndagger",
    )]);
    assert_eq!(
        s.search(&text("dagger")).await.unwrap().hits[0]
            .matches
            .len(),
        pvxwiki_store::MAX_MATCHES_PER_HIT
    );
}

#[tokio::test]
async fn duplicate_profession_rows_never_duplicate_a_hit() {
    let (_d, s) = store(&[(
        "Build:Twice",
        "<pvxbig>[build prof=A/E][Dash][/build]\n[build prof=A/W][Dash][/build]</pvxbig>",
    )]);
    let q = SearchQuery {
        professions: vec!["Assassin".into()],
        limit: 10,
        ..Default::default()
    };
    assert_eq!(titles(&s.search(&q).await.unwrap()), ["Build:Twice"]);
}
