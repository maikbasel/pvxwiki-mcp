//! In-memory `BuildStore` for server unit tests. Text search is a case-insensitive word match
//! within one section (the page title is not searched); it has no ranking or snippets beyond the section text.

use crate::search::rating_rank;
use crate::{
    BuildDocument, BuildStore, IndexStatus, NewPage, SearchHit, SearchPage, SearchQuery,
    SectionMatch, SlotDoc, StoreFuture, filter_key,
};

struct Entry {
    document: BuildDocument,
    sections: Vec<(Option<String>, String)>,
    fetched_at: i64,
}

pub struct FakeStore {
    entries: Vec<Entry>,
}

/// True when no filter is set or one of `have` is among the wanted keys.
fn any_of(wanted: &[String], have: impl IntoIterator<Item = String>) -> bool {
    wanted.is_empty()
        || have
            .into_iter()
            .any(|h| wanted.iter().any(|w| filter_key(w) == h))
}

impl FakeStore {
    pub fn new(pages: &[NewPage]) -> Self {
        let mut entries: Vec<Entry> = Vec::new();
        for page in pages {
            entries.retain(|e| e.document.title != page.title);
            entries.push(Entry {
                document: BuildDocument::from_parsed(
                    &page.title,
                    page.namespace,
                    page.revision,
                    &page.parsed,
                ),
                sections: page
                    .parsed
                    .sections
                    .iter()
                    .map(|s| (s.heading.clone(), s.text.to_lowercase()))
                    .collect(),
                fetched_at: page.fetched_at,
            });
        }
        Self { entries }
    }

    fn matches(entry: &Entry, query: &SearchQuery) -> Option<Vec<SectionMatch>> {
        let doc = &entry.document;
        let keep = (query.include_archived || !doc.is_archived())
            && any_of(&query.ratings, doc.vetting.rating.clone())
            && any_of(
                &query.professions,
                doc.skill_bars
                    .iter()
                    .filter_map(|b| b.primary.as_deref().map(filter_key)),
            )
            && any_of(
                &query.gameplay_types,
                doc.vetting.gameplay_types.iter().map(|t| filter_key(t)),
            )
            && (query.skills.is_empty()
                || doc.skill_bars.iter().any(|bar| {
                    query.skills.iter().all(|wanted| {
                        bar.slots.iter().any(|s| {
                            matches!(s, SlotDoc::Skill { name, .. } if filter_key(name) == filter_key(wanted))
                        })
                    })
                }));
        if !keep {
            return None;
        }
        let Some(text) = query.text.as_deref().filter(|t| !t.trim().is_empty()) else {
            return Some(Vec::new());
        };
        let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
        let found: Vec<SectionMatch> = entry
            .sections
            .iter()
            .filter(|(_, body)| {
                words
                    .iter()
                    .all(|w| body.split_whitespace().any(|b| b == w))
            })
            .map(|(heading, body)| SectionMatch {
                heading: heading.clone(),
                snippet: body.clone(),
            })
            .collect();
        (!found.is_empty()).then_some(found)
    }
}

impl BuildStore for FakeStore {
    fn search<'a>(&'a self, query: &'a SearchQuery) -> StoreFuture<'a, SearchPage> {
        Box::pin(async move {
            let mut hits: Vec<SearchHit> = self
                .entries
                .iter()
                .filter_map(|e| {
                    Self::matches(e, query).map(|matches| SearchHit {
                        document: e.document.clone(),
                        matches,
                    })
                })
                .collect();
            hits.sort_by(|a, b| {
                rating_rank(a.document.vetting.rating.as_deref())
                    .cmp(&rating_rank(b.document.vetting.rating.as_deref()))
                    .then(a.document.title.cmp(&b.document.title))
            });
            let has_more = hits.len() > query.offset.saturating_add(query.limit);
            let hits = hits
                .into_iter()
                .skip(query.offset)
                .take(query.limit)
                .collect();
            Ok(SearchPage { hits, has_more })
        })
    }

    fn get_page<'a>(&'a self, title: &'a str) -> StoreFuture<'a, Option<BuildDocument>> {
        Box::pin(async move {
            Ok(self
                .entries
                .iter()
                .find(|e| e.document.title == title)
                .map(|e| e.document.clone()))
        })
    }

    fn status(&self) -> StoreFuture<'_, IndexStatus> {
        Box::pin(async move {
            Ok(IndexStatus {
                pages: u64::try_from(self.entries.len()).unwrap_or(0),
                updated_at: self.entries.iter().map(|e| e.fetched_at).max(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn fake_store_honours_the_contract() {
        crate::contract::run_all(super::FakeStore::new).await;
    }
}
