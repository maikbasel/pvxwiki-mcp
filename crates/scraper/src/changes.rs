//! Which pages a run must fetch, and which stored pages left the wiki.

use std::collections::HashMap;

use serde_json::Value;

/// Titles per call: the API's limit for clients without the bot right.
pub(crate) const BATCH_SIZE: usize = 50;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// New on the wiki, or at a revision other than the stored one. Sorted.
    pub fetch: Vec<String>,
    /// Stored, but no longer listed: deleted, or renamed to a redirect. Sorted.
    pub gone: Vec<String>,
}

/// One page's current revision, as fetched for parsing.
#[derive(Debug, PartialEq, Eq)]
pub struct FetchedPage {
    pub title: String,
    pub namespace: i64,
    pub revision: i64,
    pub wikitext: String,
}

impl crate::ApiClient {
    /// `title -> current revision` of every non-redirect page in `namespace`.
    pub async fn revisions(
        &mut self,
        namespace: i64,
    ) -> Result<HashMap<String, i64>, crate::ScrapeError> {
        let namespace = namespace.to_string();
        let batch = BATCH_SIZE.to_string();
        let params = [
            ("generator", "allpages"),
            ("gapnamespace", namespace.as_str()),
            ("gaplimit", batch.as_str()),
            ("gapfilterredir", "nonredirects"),
            ("prop", "info"),
        ];
        let mut revisions = HashMap::new();
        self.each_page(&params, |page| {
            // A skipped entry would later read as a page gone from the wiki.
            let (Some(title), Some(revision)) =
                (page["title"].as_str(), page["lastrevid"].as_i64())
            else {
                return Err(crate::ScrapeError::Malformed {
                    what: "a listed page's title or lastrevid",
                });
            };
            revisions.insert(title.to_string(), revision);
            Ok(())
        })
        .await?;
        Ok(revisions)
    }

    /// Current wikitext of `titles` (at most `BATCH_SIZE`). Pages deleted since the listing are
    /// left out; the next run sees them gone.
    pub async fn contents(
        &mut self,
        titles: &[String],
    ) -> Result<Vec<FetchedPage>, crate::ScrapeError> {
        let titles = titles.join("|");
        let params = [
            ("prop", "revisions"),
            ("rvprop", "ids|content"),
            ("rvslots", "main"),
            ("titles", titles.as_str()),
        ];
        let mut fetched = Vec::new();
        self.each_page(&params, |page| {
            // Missing pages were deleted after the listing. A page without `revisions` gets
            // them in a later `continue` batch.
            let Some(revision) = page["revisions"].get(0) else {
                return Ok(());
            };
            let (Some(title), Some(namespace), Some(revision), Some(wikitext)) = (
                page["title"].as_str(),
                page["ns"].as_i64(),
                revision["revid"].as_i64(),
                revision["slots"]["main"]["content"].as_str(),
            ) else {
                return Err(crate::ScrapeError::Malformed {
                    what: "a fetched page's title, ns, revid or content",
                });
            };
            fetched.push(FetchedPage {
                title: title.to_string(),
                namespace,
                revision,
                wikitext: wikitext.to_string(),
            });
            Ok(())
        })
        .await?;
        Ok(fetched)
    }

    /// Calls `f` on every entry of `query.pages`, following `continue` to the last batch.
    async fn each_page(
        &mut self,
        params: &[(&str, &str)],
        mut f: impl FnMut(&Value) -> Result<(), crate::ScrapeError>,
    ) -> Result<(), crate::ScrapeError> {
        let mut continuation: Vec<(String, String)> = Vec::new();
        loop {
            let mut all = params.to_vec();
            all.extend(continuation.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            let value = self.query(&all).await?;
            // An empty result answers with `batchcomplete` and no `query`.
            let pages = match value["query"]["pages"].as_array() {
                Some(pages) => pages.as_slice(),
                None if value["batchcomplete"] == true && value.get("query").is_none() => &[],
                None => {
                    return Err(crate::ScrapeError::Malformed {
                        what: "query.pages",
                    });
                }
            };
            pages.iter().try_for_each(&mut f)?;
            let Some(next) = value["continue"].as_object() else {
                return Ok(());
            };
            continuation = next
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect();
        }
    }
}

/// Compares `title -> revision` as listed by the wiki with what the store holds.
pub fn diff(wiki: &HashMap<String, i64>, stored: &HashMap<String, i64>) -> Changes {
    let mut fetch: Vec<String> = wiki
        .iter()
        .filter(|(title, revision)| stored.get(*title) != Some(revision))
        .map(|(title, _)| title.clone())
        .collect();
    let mut gone: Vec<String> = stored
        .keys()
        .filter(|title| !wiki.contains_key(*title))
        .cloned()
        .collect();
    fetch.sort();
    gone.sort();
    Changes { fetch, gone }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn revs(pairs: &[(&str, i64)]) -> HashMap<String, i64> {
        pairs.iter().map(|(t, r)| ((*t).to_string(), *r)).collect()
    }

    #[test]
    fn fetches_new_and_changed_pages_and_reports_pages_gone_from_the_wiki() {
        let wiki = revs(&[("Build:New", 5), ("Build:Changed", 9), ("Build:Same", 3)]);
        let stored = revs(&[
            ("Build:Changed", 8),
            ("Build:Same", 3),
            ("Build:Deleted", 2),
        ]);

        assert_eq!(
            diff(&wiki, &stored),
            Changes {
                fetch: vec!["Build:Changed".into(), "Build:New".into()],
                gone: vec!["Build:Deleted".into()],
            }
        );
    }
}
