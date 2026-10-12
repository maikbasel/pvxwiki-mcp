//! One behaviour suite for every `BuildStore`. Covers what the fake can honour too; BM25
//! ranking, snippets and stemming are tested on the SQLite store only.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test support code")]

use pvxwiki_parser::parse;

use crate::{BuildStore, IndexStatus, NewPage, SearchQuery};

fn page(title: &str, revision: i64, wikitext: &str) -> NewPage {
    NewPage {
        title: title.into(),
        namespace: 100,
        revision,
        fetched_at: revision * 10,
        parsed: parse(wikitext),
    }
}

fn pages() -> Vec<NewPage> {
    vec![
        page(
            "Build:Sin",
            1,
            "{{Real-Vetting|rating=great|hero}}\n== Usage ==\nswing the dagger\n<pvxbig>[build prof=A/E][Dash][Shadow Form][/build]\n[build prof=A/E][Dash][/build]\n[build prof=W/E][Frenzy][/build]</pvxbig>",
        ),
        page(
            "Build:War",
            2,
            "{{Real-Vetting|rating=good|PvE team}}\n== Usage ==\nhit things\n<pvxbig>[build prof=W/E][Frenzy][/build]</pvxbig>",
        ),
        page(
            "Build:Old",
            3,
            "{{Real-Vetting|rating=archived|hero}}\n== Usage ==\nancient dagger",
        ),
        // Sorts before the higher-rated Sin: ordering must be by rating first, then title.
        page(
            "Build:Abc",
            4,
            "{{Real-Vetting|rating=good}}\n== Usage ==\nplain",
        ),
    ]
}

async fn titles<S: BuildStore>(store: &S, query: SearchQuery) -> Vec<String> {
    let query = SearchQuery {
        limit: if query.limit == 0 { 10 } else { query.limit },
        ..query
    };
    store
        .search(&query)
        .await
        .unwrap()
        .hits
        .into_iter()
        .map(|h| h.document.title)
        .collect()
}

pub async fn run_all<S: BuildStore>(make: impl Fn(&[NewPage]) -> S) {
    let empty = make(&[]);
    assert_eq!(
        empty.status().await.unwrap(),
        IndexStatus {
            pages: 0,
            updated_at: None
        }
    );

    let store = make(&pages());
    assert_eq!(
        store.status().await.unwrap(),
        IndexStatus {
            pages: 4,
            updated_at: Some(40)
        }
    );

    let doc = store
        .get_page("Build:War")
        .await
        .unwrap()
        .expect("stored page");
    assert_eq!((doc.title.as_str(), doc.revision), ("Build:War", 2));
    assert!(store.get_page("Build:Nope").await.unwrap().is_none());

    assert_eq!(
        titles(&store, SearchQuery::default()).await,
        ["Build:Sin", "Build:Abc", "Build:War"]
    );
    assert_eq!(
        titles(
            &store,
            SearchQuery {
                include_archived: true,
                ..Default::default()
            }
        )
        .await,
        ["Build:Sin", "Build:Abc", "Build:War", "Build:Old"]
    );
    assert_eq!(
        titles(
            &store,
            SearchQuery {
                ratings: vec!["Good".into()],
                ..Default::default()
            }
        )
        .await,
        ["Build:Abc", "Build:War"]
    );
    // Mixed-case input is normalised by `filter_key`.
    assert_eq!(
        titles(
            &store,
            SearchQuery {
                professions: vec!["Warrior".into()],
                ..Default::default()
            }
        )
        .await,
        ["Build:Sin", "Build:War"]
    );
    // Build:Sin has two Assassin-primary bars; it must still appear exactly once.
    assert_eq!(
        titles(
            &store,
            SearchQuery {
                professions: vec!["assassin".into()],
                ..Default::default()
            }
        )
        .await,
        ["Build:Sin"]
    );
    assert_eq!(
        titles(
            &store,
            SearchQuery {
                gameplay_types: vec!["pve team".into()],
                ..Default::default()
            }
        )
        .await,
        ["Build:War"]
    );
    assert_eq!(
        titles(
            &store,
            SearchQuery {
                skills: vec!["dash".into(), "shadow form".into()],
                ..Default::default()
            }
        )
        .await,
        ["Build:Sin"]
    );
    assert!(
        titles(
            &store,
            SearchQuery {
                skills: vec!["dash".into(), "frenzy".into()],
                ..Default::default()
            }
        )
        .await
        .is_empty()
    );
    assert_eq!(
        titles(
            &store,
            SearchQuery {
                text: Some("dagger".into()),
                ..Default::default()
            }
        )
        .await,
        ["Build:Sin"]
    );
    assert!(
        titles(
            &store,
            SearchQuery {
                text: Some("nothinglikethis".into()),
                ..Default::default()
            }
        )
        .await
        .is_empty()
    );

    let first = store
        .search(&SearchQuery {
            limit: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(first.has_more);
    assert_eq!(
        titles(
            &store,
            SearchQuery {
                limit: 1,
                offset: 1,
                ..Default::default()
            }
        )
        .await,
        ["Build:Abc"]
    );

    let mut replaced = pages();
    replaced.push(page(
        "Build:War",
        9,
        "{{Real-Vetting|rating=great}}\n== Usage ==\nfresh text",
    ));
    let store = make(&replaced);
    assert_eq!(
        store.get_page("Build:War").await.unwrap().unwrap().revision,
        9
    );
    assert!(
        titles(
            &store,
            SearchQuery {
                text: Some("things".into()),
                ..Default::default()
            }
        )
        .await
        .is_empty()
    );
}
