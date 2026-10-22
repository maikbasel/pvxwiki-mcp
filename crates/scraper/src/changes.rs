//! Which pages a run must fetch, and which stored pages left the wiki.

use std::collections::HashMap;

/// Titles per call: the API's limit for clients without the bot right.
const BATCH_SIZE: &str = "50";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// New on the wiki, or at a revision other than the stored one. Sorted.
    pub fetch: Vec<String>,
    /// Stored, but no longer listed: deleted, or renamed to a redirect. Sorted.
    pub gone: Vec<String>,
}

impl crate::ApiClient {
    /// `title -> current revision` of every non-redirect page in `namespace`.
    pub async fn revisions(
        &mut self,
        namespace: i64,
    ) -> Result<HashMap<String, i64>, crate::ScrapeError> {
        let namespace = namespace.to_string();
        let mut revisions = HashMap::new();
        let mut continuation: Vec<(String, String)> = Vec::new();
        loop {
            let mut params = vec![
                ("generator", "allpages"),
                ("gapnamespace", namespace.as_str()),
                ("gaplimit", BATCH_SIZE),
                ("gapfilterredir", "nonredirects"),
                ("prop", "info"),
            ];
            params.extend(continuation.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            let value = self.query(&params).await?;
            // An empty namespace answers with `batchcomplete` and no `query`.
            let pages = match value["query"]["pages"].as_array() {
                Some(pages) => pages.as_slice(),
                None if value["batchcomplete"] == true && value.get("query").is_none() => &[],
                None => {
                    return Err(crate::ScrapeError::Malformed {
                        what: "query.pages",
                    });
                }
            };
            for page in pages {
                // A skipped entry would later read as a page gone from the wiki.
                let (Some(title), Some(revision)) =
                    (page["title"].as_str(), page["lastrevid"].as_i64())
                else {
                    return Err(crate::ScrapeError::Malformed {
                        what: "a listed page's title or lastrevid",
                    });
                };
                revisions.insert(title.to_string(), revision);
            }
            let Some(next) = value["continue"].as_object() else {
                return Ok(revisions);
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
