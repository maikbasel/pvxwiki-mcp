//! One scrape run: list, diff, fetch what changed, store it, drop what left the wiki.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use pvxwiki_parser::{IssueKind, ParsedPage};
use pvxwiki_store::{ARCHIVE_NAMESPACE, NewPage, SqliteStore};

use crate::changes::BATCH_SIZE;
use crate::{ApiClient, ScrapeError, diff};

/// Namespaces holding build pages: current builds (100) and archived ones.
const NAMESPACES: [i64; 2] = [100, ARCHIVE_NAMESPACE];

/// What a finished run did, for the run's log line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RunReport {
    /// Pages the wiki listed.
    pub checked: usize,
    /// Pages written at a new revision.
    pub updated: usize,
    /// Pages whose new revision failed to parse; the stored copy stays.
    pub kept: usize,
    /// Pages removed because the wiki no longer lists them.
    pub removed: usize,
}

/// Removals happen only after every namespace was listed in full, so an aborted run never
/// deletes a page.
#[tracing::instrument(skip_all, fields(run = unix_now()))]
pub async fn run(client: &mut ApiClient, store: &SqliteStore) -> Result<RunReport, ScrapeError> {
    tracing::info!("scrape run started");
    let mut wiki = HashMap::new();
    for namespace in NAMESPACES {
        wiki.extend(client.revisions(namespace).await?);
    }
    let stored = store.revisions()?;
    let changes = diff(&wiki, &stored);
    let mut report = RunReport {
        checked: wiki.len(),
        ..RunReport::default()
    };
    for titles in changes.fetch.chunks(BATCH_SIZE) {
        for page in client.contents(titles).await? {
            let parsed = pvxwiki_parser::parse(&page.wikitext);
            if failed_entirely(&parsed) && stored.contains_key(&page.title) {
                tracing::warn!(
                    title = ?page.title,
                    revision = page.revision,
                    "new revision failed to parse; keeping the stored copy"
                );
                report.kept += 1;
                continue;
            }
            store.write_page(&NewPage {
                parsed,
                title: page.title,
                namespace: page.namespace,
                revision: page.revision,
                fetched_at: unix_now(),
            })?;
            report.updated += 1;
        }
    }
    for title in &changes.gone {
        store.delete_page(title)?;
        report.removed += 1;
    }
    tracing::info!(
        checked = report.checked,
        updated = report.updated,
        kept = report.kept,
        removed = report.removed,
        "scrape run finished"
    );
    Ok(report)
}

/// The parser gave up on the page as a whole, so a stored copy is better than this one.
fn failed_entirely(page: &ParsedPage) -> bool {
    page.issues
        .iter()
        .any(|i| matches!(i.kind, IssueKind::TooDeeplyNested | IssueKind::TimedOut))
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
