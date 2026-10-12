//! MCP tools (`search_builds`, `get_build`) and the HTTP router with abuse limits.

mod http;
mod proxy;
mod tools;

pub use http::{
    BODY_DEADLINE, HEALTH_CACHE_TTL, HttpConfig, Limiter, Limits, MAX_BODY_BYTES, MAX_CONCURRENT,
    RATE_BURST, RATE_PER_SECOND, REQUEST_TIMEOUT, limiter, router, spawn_limiter_gc,
};
pub use proxy::{ProxyList, client_ip};
pub use tools::{CREDIT, LICENCE_NAME, LICENCE_URL, PvxServer};
