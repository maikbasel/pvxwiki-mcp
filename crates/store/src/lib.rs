//! Build pages in SQLite: migrations, atomic page writes, full-text search with filters.

mod document;
mod fts;
mod port;
mod search;
mod sqlite;

#[cfg(any(test, feature = "testing"))]
pub mod contract;
#[cfg(any(test, feature = "testing"))]
pub mod fake;

pub use document::{
    ARCHIVE_NAMESPACE, ArchiveDoc, AttributeDoc, BarDoc, BuildDocument, IssueDoc, NewPage,
    SectionDoc, SlotDoc, VariantDoc, VettingDoc, filter_key, profession_name, rating_name,
    status_name,
};
pub use port::{BuildStore, StoreFuture};
pub use search::{MAX_MATCHES_PER_HIT, SearchHit, SearchPage, SearchQuery, SectionMatch};
pub use sqlite::SqliteStore;

/// How full the index is. `pages == 0` means the first scrape has not finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexStatus {
    pub pages: u64,
    /// Unix seconds of the newest page write.
    pub updated_at: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("migration failed: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Every read connection stayed in use for the whole wait timeout.
    #[error("no read connection became free in time")]
    Busy,
    #[error("connection pool: {0}")]
    Pool(deadpool_sqlite::PoolError),
    #[error("connection pool setup: {0}")]
    PoolBuild(#[from] deadpool_sqlite::BuildError),
    #[error("database task failed: {0}")]
    Task(String),
    #[error("stored document is not valid: {0}")]
    Document(#[from] serde_json::Error),
    #[error("writer lock poisoned by an earlier panic")]
    Poisoned,
    #[error("query exceeded its time budget")]
    Timeout,
}

impl From<deadpool_sqlite::PoolError> for StoreError {
    // A wait timeout is load, not a fault; callers tell the client to retry.
    fn from(err: deadpool_sqlite::PoolError) -> Self {
        match err {
            deadpool_sqlite::PoolError::Timeout(_) => Self::Busy,
            other => Self::Pool(other),
        }
    }
}

impl From<deadpool_sqlite::InteractError> for StoreError {
    // InteractError holds Box<dyn Any>, which thiserror's #[from] cannot wrap.
    fn from(err: deadpool_sqlite::InteractError) -> Self {
        Self::Task(err.to_string())
    }
}
