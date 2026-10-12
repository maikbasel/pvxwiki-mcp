//! Wiring for the `pvxwiki-mcp` binary: logging, store, router, serve.

pub mod config;

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::sync::Arc;

use config::{Config, LogFormat};
use pvxwiki_server::{HttpConfig, Limits};
use pvxwiki_store::SqliteStore;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

/// Our crates at the operator's level; rmcp logs several `info` lines per request.
pub const DEFAULT_LOG_FILTER: &str = "info,rmcp=warn";

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("could not open the store: {0}")]
    Store(#[from] pvxwiki_store::StoreError),
    #[error("server I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Installs the stderr subscriber. A second call in the same process is a no-op.
pub fn init_tracing(format: LogFormat) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::CLOSE)
        // Colour codes only for a person at a terminal, not in `docker logs` files.
        .with_ansi(std::io::stderr().is_terminal())
        .with_writer(std::io::stderr);
    // The two branches have different types, so each initialises itself.
    let _ = match format {
        LogFormat::Json => builder.json().flatten_event(true).try_init(),
        LogFormat::Pretty => builder.pretty().try_init(),
    };
}

/// Serves until `shutdown` completes, then stops accepting and lets in-flight requests finish.
pub async fn run(
    config: Config,
    listener: tokio::net::TcpListener,
    limits: Limits,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), RunError> {
    let store = Arc::new(SqliteStore::open(&config.data_dir)?);
    let limiter = pvxwiki_server::limiter(limits);
    pvxwiki_server::spawn_limiter_gc(limiter.clone());
    let http = HttpConfig {
        allowed_hosts: config.allowed_hosts.clone(),
        trusted_proxies: config.trusted_proxies.clone(),
    };
    let app = pvxwiki_server::router(store, &http, limiter);
    tracing::info!(
        listen = %listener.local_addr()?,
        data_dir = %config.data_dir.display(),
        allowed_hosts = ?config.allowed_hosts,
        trusted_proxies = ?config.trusted_proxies,
        log_format = ?config.log_format,
        contact = ?config.contact,
        "server started"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await?;
    tracing::info!("server stopped");
    Ok(())
}

/// Completes on SIGTERM or Ctrl-C. As PID 1 in a container nothing else handles SIGTERM, so
/// without this `docker stop` waits for its kill timeout.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = ?err, "cannot listen for Ctrl-C; relying on SIGTERM");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                term.recv().await;
            }
            Err(err) => {
                tracing::warn!(error = ?err, "cannot listen for SIGTERM; relying on Ctrl-C");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutdown signal received; finishing in-flight requests");
}
