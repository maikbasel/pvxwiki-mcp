//! Router: rmcp streamable HTTP at /mcp behind abuse limits; /healthz outside them.

use std::hash::BuildHasher;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{BoxError, Router, error_handling::HandleErrorLayer};
use governor::clock::{Clock, DefaultClock};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use pvxwiki_store::BuildStore;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tower::ServiceBuilder;
use tower::limit::ConcurrencyLimitLayer;
use tower::load_shed::LoadShedLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::{RequestBodyDeadlineLayer, TimeoutLayer};
use tower_http::trace::TraceLayer;
use tracing::Instrument;

use crate::proxy::{ProxyList, client_ip};
use crate::tools::PvxServer;

/// Tool calls are tiny JSON-RPC messages.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole request body must arrive within this, however slowly it trickles.
pub const BODY_DEADLINE: Duration = Duration::from_secs(5);
/// Requests in flight across all clients; more are refused with 503.
pub const MAX_CONCURRENT: usize = 64;
/// Sustained requests per second per client.
pub const RATE_PER_SECOND: u32 = 5;
/// Requests a client may send at once before the sustained rate applies.
pub const RATE_BURST: u32 = 20;
/// How long a `/healthz` outcome is reused before the database is asked again.
pub const HEALTH_CACHE_TTL: Duration = Duration::from_secs(2);
const LIMITER_GC_EVERY: Duration = Duration::from_secs(60);

pub struct HttpConfig {
    /// Hostnames clients may address (`Host` header); rmcp rejects others.
    pub allowed_hosts: Vec<String>,
    pub trusted_proxies: ProxyList,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub per_second: NonZeroU32,
    pub burst: NonZeroU32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_second: NonZeroU32::new(RATE_PER_SECOND).unwrap_or(NonZeroU32::MIN),
            burst: NonZeroU32::new(RATE_BURST).unwrap_or(NonZeroU32::MIN),
        }
    }
}

pub type Limiter = Arc<DefaultKeyedRateLimiter<IpAddr>>;

pub fn limiter(limits: Limits) -> Limiter {
    Arc::new(RateLimiter::keyed(
        Quota::per_second(limits.per_second).allow_burst(limits.burst),
    ))
}

/// Drops limiter state for clients whose bucket is full again. Call once at startup.
pub fn spawn_limiter_gc(limiter: Limiter) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(LIMITER_GC_EVERY);
        loop {
            tick.tick().await;
            limiter.retain_recent();
            limiter.shrink_to_fit();
        }
    })
}

#[derive(Clone)]
struct RateState {
    limiter: Limiter,
    trusted: Arc<ProxyList>,
}

/// The request's ID and `mcp_request` span, handed to the tool through the request extensions:
/// rmcp runs the handler on its own task, where the middleware's span is no longer current.
#[derive(Clone)]
pub(crate) struct RequestInfo {
    pub(crate) id: u64,
    pub(crate) span: tracing::Span,
}

/// Per-process key so logged client hashes cannot be reversed across restarts.
static CLIENT_HASH: LazyLock<std::hash::RandomState> = LazyLock::new(std::hash::RandomState::new);
/// Request IDs for log correlation; unique per process run.
static REQUEST_IDS: AtomicU64 = AtomicU64::new(1);

async fn rate_limit(
    State(state): State<RateState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut req: Request,
    next: Next,
) -> Response {
    // Repeated lines form one comma list (RFC 9110 5.3); a proxy may append its own line after
    // the client's. A non-ASCII line becomes an empty hop, which is malformed: the peer is used.
    let lines: Vec<&str> = req
        .headers()
        .get_all("x-forwarded-for")
        .iter()
        .map(|v| v.to_str().unwrap_or(""))
        .collect();
    let forwarded = (!lines.is_empty()).then(|| lines.join(","));
    let ip = client_ip(peer.ip(), forwarded.as_deref(), &state.trusted);
    let client = format!("{:016x}", CLIENT_HASH.hash_one(ip));
    let request_id = REQUEST_IDS.fetch_add(1, Ordering::Relaxed);
    let span = tracing::info_span!("mcp_request", request_id, client = %client);
    match state.limiter.check_key(&ip) {
        Ok(()) => {
            req.extensions_mut().insert(RequestInfo {
                id: request_id,
                span: span.clone(),
            });
            next.run(req).instrument(span).await
        }
        Err(not_until) => {
            let wait = not_until.wait_time_from(DefaultClock::default().now());
            let secs = (wait.as_secs() + u64::from(wait.subsec_nanos() > 0)).max(1);
            span.in_scope(|| {
                tracing::warn!(retry_after_s = secs, "client rate limited; 429 returned")
            });
            let mut res = (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs));
            res
        }
    }
}

async fn handle_overload(err: BoxError) -> (StatusCode, &'static str) {
    if err.is::<tower::load_shed::error::Overloaded>() {
        tracing::warn!("concurrency cap reached; 503 returned");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "overloaded, try again shortly",
        )
    } else {
        tracing::error!(error = ?err, "unexpected middleware error; 500 returned");
        (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

#[derive(Clone)]
struct HealthState {
    store: Arc<dyn BuildStore>,
    /// Last probe time and outcome. Held across the probe, so concurrent checks wait for it.
    last: Arc<tokio::sync::Mutex<Option<(Instant, bool)>>>,
}

/// 200 when the database answers; outside every limit so load never reads as "unhealthy". The
/// database sees at most one probe per `HEALTH_CACHE_TTL`, however often this is called.
async fn healthz(State(state): State<HealthState>) -> (StatusCode, &'static str) {
    let mut last = state.last.lock().await;
    let healthy = match *last {
        Some((at, healthy)) if at.elapsed() < HEALTH_CACHE_TTL => healthy,
        _ => {
            let healthy = match state.store.status().await {
                Ok(_) => true,
                Err(err) => {
                    tracing::warn!(error = ?err, "health check failed; 503 returned");
                    false
                }
            };
            *last = Some((Instant::now(), healthy));
            healthy
        }
    };
    if healthy {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
    }
}

pub fn router(store: Arc<dyn BuildStore>, config: &HttpConfig, limiter: Limiter) -> Router {
    let mcp_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_allowed_hosts(config.allowed_hosts.iter().cloned())
        .with_max_request_body_bytes(MAX_BODY_BYTES);
    let tool_store = store.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(PvxServer::new(tool_store.clone())),
        Arc::new(NeverSessionManager::default()),
        mcp_config,
    );
    let rate_state = RateState {
        limiter,
        trusted: Arc::new(config.trusted_proxies.clone()),
    };

    let mcp = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(handle_overload))
                .layer(LoadShedLayer::new())
                .layer(ConcurrencyLimitLayer::new(MAX_CONCURRENT))
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::REQUEST_TIMEOUT,
                    REQUEST_TIMEOUT,
                ))
                .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
                .layer(RequestBodyDeadlineLayer::new(BODY_DEADLINE)),
        )
        // Added last, so it runs first: rejected clients never reach the other layers.
        .layer(middleware::from_fn_with_state(rate_state, rate_limit));

    Router::new()
        .merge(mcp)
        .route(
            "/healthz",
            get(healthz).with_state(HealthState {
                store,
                last: Arc::default(),
            }),
        )
        .layer(TraceLayer::new_for_http())
}
