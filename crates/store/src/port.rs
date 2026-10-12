//! The read side the MCP server depends on.

use std::future::Future;
use std::pin::Pin;

use crate::{BuildDocument, IndexStatus, SearchPage, SearchQuery, StoreError};

/// Boxed so `BuildStore` stays object safe (`Arc<dyn BuildStore>` in the server).
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

/// Implemented by `SqliteStore` and, for tests, `fake::FakeStore`. Both pass `contract::run_all`.
pub trait BuildStore: Send + Sync {
    fn search<'a>(&'a self, query: &'a SearchQuery) -> StoreFuture<'a, SearchPage>;
    fn get_page<'a>(&'a self, title: &'a str) -> StoreFuture<'a, Option<BuildDocument>>;
    fn status(&self) -> StoreFuture<'_, IndexStatus>;
}
