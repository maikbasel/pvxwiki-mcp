//! Wiring for the `pvxwiki-mcp` binary: logging, store, router, serve.

pub mod config;

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::sync::Arc;

use config::{Config, LogFormat};
use pvxwiki_admin::AdminState;
use pvxwiki_scraper::{ApiClient, Runner, ScrapeError};
use pvxwiki_server::{HttpConfig, Limits};
use pvxwiki_store::SqliteStore;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

/// Our crates at the operator's level; rmcp logs several `info` lines per request.
pub const DEFAULT_LOG_FILTER: &str = "info,rmcp=warn";

/// Where and how often the scraper copies the wiki. Off when `Config::contact` is unset.
#[derive(Debug, Clone)]
pub struct Schedule {
    pub endpoint: String,
    pub pacing: pvxwiki_scraper::Pacing,
    /// From the start of one run to the start of the next.
    pub interval: std::time::Duration,
}

/// Most build pages change rarely; a run after an idle interval costs about one request per 50
/// listed pages.
pub const SCRAPE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

impl Schedule {
    /// The production wiki at Fandom's pacing.
    pub fn fandom() -> Self {
        Self {
            endpoint: pvxwiki_scraper::FANDOM_API.into(),
            pacing: pvxwiki_scraper::Pacing::FANDOM,
            interval: SCRAPE_INTERVAL,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("could not open the store: {0}")]
    Store(#[from] pvxwiki_store::StoreError),
    #[error("could not set up the scraper: {0}")]
    Scraper(#[from] ScrapeError),
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

/// Serves the MCP and admin listeners until `shutdown` completes, then stops accepting on both
/// and lets in-flight requests finish. The scraper runs beside them when a contact is set.
pub async fn run(
    config: Config,
    listener: tokio::net::TcpListener,
    admin_listener: tokio::net::TcpListener,
    limits: Limits,
    schedule: Schedule,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), RunError> {
    let store = Arc::new(SqliteStore::open(&config.data_dir)?);
    let client = match &config.contact {
        Some(contact) => Some(ApiClient::new(
            &schedule.endpoint,
            contact,
            schedule.pacing,
        )?),
        None => {
            tracing::warn!(
                "PVXWIKI_CONTACT is not set; the scraper is off and the index is not updated"
            );
            None
        }
    };
    let runner = Runner::new(store.clone(), client, schedule.interval);
    tokio::spawn(runner.clone().run_forever());

    let limiter = pvxwiki_server::limiter(limits);
    pvxwiki_server::spawn_limiter_gc(limiter.clone());
    let http = HttpConfig {
        allowed_hosts: config.allowed_hosts.clone(),
        trusted_proxies: config.trusted_proxies.clone(),
    };
    let app = pvxwiki_server::router(store.clone(), &http, limiter);
    let admin = pvxwiki_admin::router(AdminState {
        runner,
        store,
        allowed_hosts: config.allowed_hosts.clone(),
    });
    tracing::info!(
        listen = %listener.local_addr()?,
        admin_listen = %admin_listener.local_addr()?,
        data_dir = %config.data_dir.display(),
        allowed_hosts = ?config.allowed_hosts,
        trusted_proxies = ?config.trusted_proxies,
        log_format = ?config.log_format,
        contact = ?config.contact,
        "server started"
    );
    // One signal stops both listeners.
    let (stop, stopped) = tokio::sync::watch::channel(());
    tokio::spawn(async move {
        shutdown.await;
        let _ = stop.send(());
    });
    let until_stopped = |mut rx: tokio::sync::watch::Receiver<()>| async move {
        let _ = rx.changed().await;
    };
    let mcp = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(until_stopped(stopped.clone()));
    let admin = axum::serve(admin_listener, admin).with_graceful_shutdown(until_stopped(stopped));
    tokio::try_join!(mcp.into_future(), admin.into_future())?;
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
