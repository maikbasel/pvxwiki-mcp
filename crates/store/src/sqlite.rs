//! The SQLite implementation: one writer connection, a read-only pool for queries.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use deadpool_sqlite::{Config, Hook, HookError, Pool, Runtime};
use include_dir::{Dir, include_dir};
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use rusqlite_migration::Migrations;

use crate::{BuildDocument, IndexStatus, NewPage, SlotDoc, StoreError, filter_key};

static MIGRATIONS_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/migrations");

/// File name inside the data directory.
const DB_FILE: &str = "pvxwiki.sqlite3";
/// Read connections; requests beyond this wait for a free one.
const READ_POOL_SIZE: usize = 8;
/// Longest a request waits for a free read connection before it gets `StoreError::Busy`.
const READ_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
/// Longest a single read query may run before it is interrupted.
const QUERY_BUDGET: Duration = Duration::from_millis(500);
/// SQLite VM steps between deadline checks.
const PROGRESS_STEPS: i32 = 1_000;

pub struct SqliteStore {
    pub(crate) writer: Mutex<Connection>,
    pub(crate) readers: Pool,
}

impl SqliteStore {
    /// Opens (creating if needed) the database in `dir` and applies pending migrations.
    #[tracing::instrument(skip_all, fields(dir = %dir.display()))]
    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        let path = dir.join(DB_FILE);
        let mut writer = Connection::open(&path)?;
        // journal_mode answers with a row, so the _and_check variant is required.
        let _mode: String =
            writer.pragma_update_and_check(None, "journal_mode", "wal", |r| r.get(0))?;
        writer.pragma_update(None, "foreign_keys", true)?;
        Migrations::from_directory(&MIGRATIONS_DIR)?.to_latest(&mut writer)?;
        let readers = read_pool(&path)?;
        tracing::info!("store opened");
        Ok(Self {
            writer: Mutex::new(writer),
            readers,
        })
    }

    /// Replaces every row of `page.title` with the new revision in one transaction. Readers see
    /// the old copy or the new one, never a mix.
    #[tracing::instrument(skip_all, fields(title = ?page.title, revision = page.revision))]
    pub fn write_page(&self, page: &NewPage) -> Result<(), StoreError> {
        let doc =
            BuildDocument::from_parsed(&page.title, page.namespace, page.revision, &page.parsed);
        let json = serde_json::to_string(&doc)?;
        let mut writer = self.writer.lock().map_err(|_| StoreError::Poisoned)?;
        let tx = writer.transaction()?;
        remove(&tx, &page.title)?;
        let id: i64 = tx.query_row(
            "INSERT INTO pages (title, namespace, revision, fetched_at, vetting, rating, status, archived, document)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id",
            params![
                page.title,
                page.namespace,
                page.revision,
                page.fetched_at,
                doc.vetting.kind,
                doc.vetting.rating,
                doc.vetting.status,
                doc.is_archived(),
                json,
            ],
            |r| r.get(0),
        )?;
        for gameplay_type in &doc.vetting.gameplay_types {
            tx.execute(
                "INSERT INTO page_types (page_id, gameplay_type) VALUES (?1, ?2)",
                params![id, filter_key(gameplay_type)],
            )?;
        }
        for (bar_index, bar) in doc.skill_bars.iter().enumerate() {
            if let Some(primary) = &bar.primary {
                tx.execute(
                    "INSERT INTO page_profs (page_id, profession) VALUES (?1, ?2)",
                    params![id, filter_key(primary)],
                )?;
            }
            for slot in &bar.slots {
                if let SlotDoc::Skill { name, .. } = slot {
                    tx.execute(
                        "INSERT INTO page_skills (page_id, bar, skill) VALUES (?1, ?2, ?3)",
                        params![
                            id,
                            i64::try_from(bar_index).unwrap_or(i64::MAX),
                            filter_key(name)
                        ],
                    )?;
                }
            }
        }
        for (index, section) in page.parsed.sections.iter().enumerate() {
            tx.execute(
                "INSERT INTO sections_fts (title, heading, body, page_id, section) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    page.title,
                    section.heading.as_deref().unwrap_or(""),
                    section.text,
                    id,
                    i64::try_from(index).unwrap_or(i64::MAX),
                ],
            )?;
        }
        tx.commit()?;
        tracing::debug!("page written");
        Ok(())
    }

    /// Removes `title` and its search rows in one transaction. Unknown titles are a no-op.
    #[tracing::instrument(skip_all, fields(title = ?title))]
    pub fn delete_page(&self, title: &str) -> Result<(), StoreError> {
        let mut writer = self.writer.lock().map_err(|_| StoreError::Poisoned)?;
        let tx = writer.transaction()?;
        remove(&tx, title)?;
        tx.commit()?;
        Ok(())
    }

    /// Title to stored revision for every page, so the scraper refetches only what changed.
    pub fn revisions(&self) -> Result<HashMap<String, i64>, StoreError> {
        let writer = self.writer.lock().map_err(|_| StoreError::Poisoned)?;
        let mut stmt = writer.prepare("SELECT title, revision FROM pages")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The writer connection, for tests that inspect raw rows.
    #[doc(hidden)]
    pub fn writer_for_tests(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A poisoned lock in a test is itself a test failure.
        self.writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Runs `f` on a pooled read-only connection under `QUERY_BUDGET`.
    pub(crate) async fn read<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let conn = self.readers.get().await?;
        conn.interact(move |c| with_budget(c, f)).await?
    }
}

impl crate::BuildStore for SqliteStore {
    fn search<'a>(
        &'a self,
        query: &'a crate::SearchQuery,
    ) -> crate::StoreFuture<'a, crate::SearchPage> {
        let query = query.clone();
        Box::pin(self.read(move |conn| crate::search::search(conn, &query)))
    }

    fn get_page<'a>(&'a self, title: &'a str) -> crate::StoreFuture<'a, Option<BuildDocument>> {
        let title = title.to_string();
        Box::pin(self.read(move |conn| {
            let json: Option<String> = conn
                .query_row(
                    "SELECT document FROM pages WHERE title = ?1",
                    [&title],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
        }))
    }

    fn status(&self) -> crate::StoreFuture<'_, IndexStatus> {
        Box::pin(self.read(|conn| {
            let (pages, updated_at): (i64, Option<i64>) =
                conn.query_row("SELECT COUNT(*), MAX(fetched_at) FROM pages", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            Ok(IndexStatus {
                pages: u64::try_from(pages).unwrap_or(0),
                updated_at,
            })
        }))
    }
}

/// Deletes `title`'s page row (child rows cascade) and its FTS rows, which cannot cascade.
fn remove(tx: &rusqlite::Transaction<'_>, title: &str) -> Result<(), StoreError> {
    let old: Option<i64> = tx
        .query_row("SELECT id FROM pages WHERE title = ?1", [title], |r| {
            r.get(0)
        })
        .optional()?;
    if let Some(old) = old {
        tx.execute("DELETE FROM sections_fts WHERE page_id = ?1", [old])?;
        tx.execute("DELETE FROM pages WHERE id = ?1", [old])?;
    }
    Ok(())
}

pub(crate) fn load_by_id(conn: &Connection, id: i64) -> Result<BuildDocument, StoreError> {
    let json: String = conn.query_row("SELECT document FROM pages WHERE id = ?1", [id], |r| {
        r.get(0)
    })?;
    Ok(serde_json::from_str(&json)?)
}

/// Percent-encodes the characters that would end or alter the path part of a SQLite URI.
fn uri_path(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('%', "%25")
        .replace('?', "%3f")
        .replace('#', "%23")
}

/// deadpool-sqlite opens with rusqlite's default flags, which include SQLITE_OPEN_URI, so a
/// `mode=ro` URI gives a read-only handle; `query_only` is a second guard.
fn read_pool(path: &Path) -> Result<Pool, StoreError> {
    let uri = format!("file:{}?mode=ro", uri_path(path));
    // ConfigError is Infallible, so this pattern cannot fail.
    let Ok(builder) = Config::new(uri).builder(Runtime::Tokio1);
    let pool = builder
        .max_size(READ_POOL_SIZE)
        .wait_timeout(Some(READ_WAIT_TIMEOUT))
        .post_create(Hook::async_fn(|conn, _metrics| {
            Box::pin(async move {
                conn.interact(|c| c.pragma_update(None, "query_only", true))
                    .await
                    .map_err(|e| HookError::message(e.to_string()))?
                    .map_err(HookError::Backend)
            })
        }))
        .build()?;
    Ok(pool)
}

fn with_budget<T>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let deadline = Instant::now() + QUERY_BUDGET;
    conn.progress_handler(PROGRESS_STEPS, Some(move || Instant::now() > deadline))?;
    let result = f(conn);
    conn.progress_handler(0, None::<fn() -> bool>)?;
    result.map_err(|err| match err {
        StoreError::Sqlite(ref e)
            if e.sqlite_error_code() == Some(ErrorCode::OperationInterrupted) =>
        {
            StoreError::Timeout
        }
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BuildStore;

    #[tokio::test]
    async fn fresh_store_is_empty_and_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path()).unwrap();
        assert_eq!(
            store.status().await.unwrap(),
            IndexStatus {
                pages: 0,
                updated_at: None
            }
        );
        let writer = store.writer.lock().unwrap();
        let version: i64 = writer
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
        let mode: String = writer
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[tokio::test]
    async fn reopening_keeps_the_schema() {
        let dir = tempfile::tempdir().unwrap();
        drop(SqliteStore::open(dir.path()).unwrap());
        let store = SqliteStore::open(dir.path()).unwrap();
        assert_eq!(store.status().await.unwrap().pages, 0);
    }

    #[tokio::test]
    async fn readers_cannot_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path()).unwrap();
        let conn = store.readers.get().await.unwrap();
        let err = conn
            .interact(|c| c.execute("DELETE FROM pages", []))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(err.sqlite_error_code(), Some(rusqlite::ErrorCode::ReadOnly));
    }

    #[test]
    fn migrations_are_valid() {
        Migrations::from_directory(&MIGRATIONS_DIR)
            .unwrap()
            .validate()
            .unwrap();
    }

    #[test]
    fn uri_path_escapes_uri_metacharacters() {
        assert_eq!(uri_path(Path::new("/d/a?b#c%d/x")), "/d/a%3fb%23c%25d/x");
    }

    #[tokio::test]
    async fn data_dir_with_uri_metacharacters_opens() {
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("we?ird#100%");
        std::fs::create_dir(&dir).unwrap();
        let store = SqliteStore::open(&dir).unwrap();
        assert_eq!(store.status().await.unwrap().pages, 0);
    }

    #[tokio::test]
    async fn runaway_query_times_out_and_the_connection_stays_usable() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path()).unwrap();
        let runaway = store
            .read(|c| {
                Ok(c.query_row(
                    "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) \
                     SELECT count(*) FROM c",
                    [],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .await;
        assert!(
            matches!(runaway, Err(StoreError::Timeout)),
            "got {runaway:?}"
        );
        // read() installs a fresh handler on every call, which would mask a stale one, so use the
        // pooled connection directly. Only one connection exists, so it is the one that timed out.
        // This query runs far more VM steps than PROGRESS_STEPS: a handler left installed (deadline
        // long past) would interrupt it.
        let conn = store.readers.get().await.unwrap();
        let after = conn
            .interact(|c| {
                c.query_row(
                    "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c LIMIT 5000) \
                     SELECT count(*) FROM c",
                    [],
                    |r| r.get::<_, i64>(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(after.unwrap(), 5000);
    }

    #[tokio::test]
    async fn exhausted_read_pool_is_busy() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path()).unwrap();
        let mut held = Vec::new();
        for _ in 0..READ_POOL_SIZE {
            held.push(store.readers.get().await.unwrap());
        }
        let err = store.status().await.unwrap_err();
        assert!(matches!(err, StoreError::Busy), "got {err:?}");
        drop(held);
        assert_eq!(store.status().await.unwrap().pages, 0);
    }

    #[test]
    fn only_a_pool_wait_timeout_is_busy() {
        use deadpool_sqlite::{PoolError, TimeoutType};
        assert!(matches!(
            StoreError::from(PoolError::Timeout(TimeoutType::Wait)),
            StoreError::Busy
        ));
        assert!(matches!(
            StoreError::from(PoolError::Closed),
            StoreError::Pool(PoolError::Closed)
        ));
    }

    struct TempStore {
        _dir: tempfile::TempDir,
        store: SqliteStore,
    }

    impl crate::BuildStore for TempStore {
        fn search<'a>(
            &'a self,
            q: &'a crate::SearchQuery,
        ) -> crate::StoreFuture<'a, crate::SearchPage> {
            self.store.search(q)
        }
        fn get_page<'a>(
            &'a self,
            title: &'a str,
        ) -> crate::StoreFuture<'a, Option<crate::BuildDocument>> {
            self.store.get_page(title)
        }
        fn status(&self) -> crate::StoreFuture<'_, IndexStatus> {
            self.store.status()
        }
    }

    #[tokio::test]
    async fn sqlite_store_honours_the_contract() {
        crate::contract::run_all(|pages| {
            let dir = tempfile::tempdir().unwrap();
            let store = SqliteStore::open(dir.path()).unwrap();
            for page in pages {
                store.write_page(page).unwrap();
            }
            TempStore { _dir: dir, store }
        })
        .await;
    }
}
