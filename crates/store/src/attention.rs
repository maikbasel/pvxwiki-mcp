use crate::{SqliteStore, StoreError};

/// Most pages listed for one issue kind.
pub const ISSUE_LIST_LIMIT: usize = 50;

/// A page whose newest revision failed to parse while an older copy stays served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFailure {
    pub title: String,
    pub stored_revision: i64,
    pub failed_revision: i64,
    pub first_seen: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuePage {
    pub title: String,
    pub revision: i64,
}

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

impl SqliteStore {
    /// Upsert: keeps `first_seen`, updates `failed_revision`.
    pub fn record_parse_failure(
        &self,
        title: &str,
        failed_revision: i64,
        seen_at: i64,
    ) -> Result<(), StoreError> {
        let writer = self.writer.lock().map_err(|_| StoreError::Poisoned)?;
        writer.execute(
            "INSERT INTO parse_failures (title, failed_revision, first_seen) VALUES (?1, ?2, ?3)
             ON CONFLICT(title) DO UPDATE SET failed_revision = excluded.failed_revision",
            rusqlite::params![title, failed_revision, seen_at],
        )?;
        Ok(())
    }

    /// Ordered by title, with the revision still being served.
    pub async fn parse_failures(&self) -> Result<Vec<ParseFailure>, StoreError> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT f.title, p.revision, f.failed_revision, f.first_seen
                 FROM parse_failures f JOIN pages p ON p.title = f.title ORDER BY f.title",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(ParseFailure {
                    title: r.get(0)?,
                    stored_revision: r.get(1)?,
                    failed_revision: r.get(2)?,
                    first_seen: r.get(3)?,
                })
            })?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
        .await
    }

    /// Pages per namespace, ordered by namespace.
    pub async fn namespace_counts(&self) -> Result<Vec<(i64, u64)>, StoreError> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT namespace, COUNT(*) FROM pages GROUP BY namespace ORDER BY namespace",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, count(r.get(1)?))))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
        .await
    }

    /// Pages per issue kind (a page with several issues of one kind counts once), by kind.
    pub async fn issue_counts(&self) -> Result<Vec<(String, u64)>, StoreError> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT j.value ->> 'kind', COUNT(DISTINCT p.id)
                 FROM pages p, json_each(p.document, '$.issues') j
                 GROUP BY 1 ORDER BY 1",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, count(r.get(1)?))))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
        .await
    }

    /// At most `ISSUE_LIST_LIMIT` pages with an issue of `kind`, by title, and the total.
    pub async fn pages_with_issue(&self, kind: &str) -> Result<(Vec<IssuePage>, u64), StoreError> {
        let kind = kind.to_string();
        let limit = i64::try_from(ISSUE_LIST_LIMIT).unwrap_or(i64::MAX);
        self.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT p.title, p.revision FROM pages p
                 WHERE EXISTS (SELECT 1 FROM json_each(p.document, '$.issues') j
                               WHERE j.value ->> 'kind' = ?1)
                 ORDER BY p.title LIMIT ?2",
            )?;
            let pages = stmt
                .query_map(rusqlite::params![kind, limit], |r| {
                    Ok(IssuePage {
                        title: r.get(0)?,
                        revision: r.get(1)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            let total: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pages p
                 WHERE EXISTS (SELECT 1 FROM json_each(p.document, '$.issues') j
                               WHERE j.value ->> 'kind' = ?1)",
                [&kind],
                |r| r.get(0),
            )?;
            Ok((pages, count(total)))
        })
        .await
    }
}
