//! The operator's admin page: scraper status, run history, pages needing attention, run now.
//! Served on its own listener; its protection lives in the reverse proxy (`security.md`).

mod guard;
mod handlers;
mod views;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use pvxwiki_scraper::Runner;
use pvxwiki_store::SqliteStore;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

/// Everything the admin page serves lives under this path, so a proxy can route it from the same
/// domain as `/mcp` to this listener.
pub const BASE_PATH: &str = "/admin";

/// The admin page sends no request bodies; `POST /runs` is empty.
pub const MAX_BODY_BYTES: usize = 1024;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest prefix of a refused `Host` header that reaches the log (`logging.md`).
const MAX_LOGGED_HOST: usize = 64;
/// Vendored assets change only with a new binary.
const ASSET_CACHE: &str = "public, max-age=86400";

const HTMX_JS: &[u8] = include_bytes!("../assets/htmx.min.js");
const ADMIN_CSS: &[u8] = include_bytes!("../assets/admin.css");

/// What the admin handlers read and act on.
#[derive(Clone)]
pub struct AdminState {
    pub runner: Runner,
    pub store: Arc<SqliteStore>,
    /// `PVXWIKI_ALLOWED_HOSTS`; `localhost` and `127.0.0.1` are always allowed.
    pub allowed_hosts: Vec<String>,
}

pub fn router(state: AdminState) -> Router {
    let hosts = Arc::new(state.allowed_hosts.clone());
    let pages = Router::new()
        .route("/", get(handlers::index))
        .route("/status", get(handlers::status))
        .route("/history", get(handlers::history))
        .route("/issues", get(handlers::issues))
        .route("/runs", post(handlers::run_now))
        .route("/assets/htmx.min.js", get(htmx))
        .route("/assets/admin.css", get(css));
    Router::new()
        .nest(BASE_PATH, pages)
        // The bare port lands on the page too.
        .route("/", get(|| async { Redirect::to(BASE_PATH) }))
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        // 503, not 408: the server was slow, and 408 invites clients to resend a POST.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::SERVICE_UNAVAILABLE,
            REQUEST_TIMEOUT,
        ))
        // Basic auth makes a framed page click with the operator's credentials: forbid framing.
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("frame-ancestors 'none'"),
        ))
        // Added last, so it runs first: a foreign Host never reaches a handler.
        .layer(middleware::from_fn_with_state(hosts, check_host))
        .layer(TraceLayer::new_for_http())
}

/// Refuses a Host outside the allow-list (DNS rebinding) with 421.
async fn check_host(State(allowed): State<Arc<Vec<String>>>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if guard::host_allowed(host, &allowed) {
        next.run(req).await
    } else {
        let host: String = host.chars().take(MAX_LOGGED_HOST).collect();
        tracing::warn!(host = ?host, "admin request for an unknown host; 421 returned");
        (StatusCode::MISDIRECTED_REQUEST, "unknown host").into_response()
    }
}

async fn htmx() -> impl IntoResponse {
    asset("text/javascript", HTMX_JS)
}

async fn css() -> impl IntoResponse {
    asset("text/css", ADMIN_CSS)
}

fn asset(content_type: &'static str, body: &'static [u8]) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, ASSET_CACHE),
        ],
        body,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use pvxwiki_scraper::Runner;
    use pvxwiki_store::SqliteStore;
    use tower::ServiceExt;

    use super::*;

    fn state(dir: &tempfile::TempDir) -> AdminState {
        let store = Arc::new(SqliteStore::open(dir.path()).unwrap());
        AdminState {
            runner: Runner::new(store.clone(), None, Duration::from_secs(3600)),
            store,
            allowed_hosts: vec!["admin.example.org".into()],
        }
    }

    fn get(path: &str, host: &str) -> Request<Body> {
        Request::get(path)
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn serves_the_vendored_assets() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(state(&dir));

        let js = app
            .clone()
            .oneshot(get("/admin/assets/htmx.min.js", "localhost:8081"))
            .await
            .unwrap();
        let css = app
            .oneshot(get("/admin/assets/admin.css", "localhost:8081"))
            .await
            .unwrap();

        assert_eq!(js.status(), StatusCode::OK);
        assert_eq!(js.headers()[header::CONTENT_TYPE], "text/javascript");
        assert_eq!(css.status(), StatusCode::OK);
        assert_eq!(css.headers()[header::CONTENT_TYPE], "text/css");
    }

    #[tokio::test]
    async fn an_unknown_host_is_misdirected() {
        let dir = tempfile::tempdir().unwrap();

        let res = router(state(&dir))
            .oneshot(get("/admin/assets/htmx.min.js", "evil.example"))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::MISDIRECTED_REQUEST);
    }

    fn post_runs(site: &'static str) -> Request<Body> {
        Request::post("/admin/runs")
            .header(header::HOST, "localhost")
            .header("hx-request", "true")
            .header("sec-fetch-site", site)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn cross_site_run_now_is_forbidden() {
        let dir = tempfile::tempdir().unwrap();

        let res = router(state(&dir))
            .oneshot(post_runs("cross-site"))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn run_now_on_an_off_runner_conflicts() {
        let dir = tempfile::tempdir().unwrap();

        let res = router(state(&dir))
            .oneshot(post_runs("same-origin"))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn the_page_and_its_fragments_render() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(state(&dir));

        for (path, needle) in [
            ("/admin", "Run history"),
            ("/admin/status", r#"data-state="off""#),
            ("/admin/history", "Run history"),
            ("/admin/issues?kind=timed_out", r#"data-kind="timed_out""#),
        ] {
            let res = app.clone().oneshot(get(path, "localhost")).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            assert!(
                res.headers()[header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .starts_with("text/html"),
                "{path}"
            );
            let body = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert!(body.contains(needle), "{path}: {body}");
        }
    }

    #[tokio::test]
    async fn the_page_cannot_be_framed() {
        let dir = tempfile::tempdir().unwrap();

        let res = router(state(&dir))
            .oneshot(get("/admin", "localhost"))
            .await
            .unwrap();

        assert_eq!(res.headers()["x-frame-options"], "DENY");
        assert_eq!(
            res.headers()[header::CONTENT_SECURITY_POLICY],
            "frame-ancestors 'none'"
        );
    }

    #[tokio::test]
    async fn the_bare_port_redirects_to_the_page() {
        let dir = tempfile::tempdir().unwrap();

        let res = router(state(&dir))
            .oneshot(get("/", "localhost"))
            .await
            .unwrap();

        assert!(res.status().is_redirection(), "{}", res.status());
        assert_eq!(res.headers()[header::LOCATION], "/admin");
    }
}
