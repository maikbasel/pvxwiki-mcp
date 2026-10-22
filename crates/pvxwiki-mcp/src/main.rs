use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

use pvxwiki_mcp::config::{self, Config, LogFormat};
use pvxwiki_server::Limits;

/// How long `healthcheck` waits for the server.
const HEALTHCHECK_TIMEOUT: Duration = Duration::from_secs(3);
/// Exit status for a command line the binary does not understand.
const USAGE_EXIT: u8 = 2;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => serve(),
        Some("healthcheck") => healthcheck(),
        Some(other) => {
            pvxwiki_mcp::init_tracing(env_log_format().unwrap_or(LogFormat::Json));
            tracing::error!(
                subcommand = ?other,
                accepted = "serve, healthcheck",
                "unknown subcommand; exiting"
            );
            ExitCode::from(USAGE_EXIT)
        }
    }
}

fn env_log_format() -> Result<LogFormat, config::ConfigError> {
    config::log_format(std::env::var("PVXWIKI_LOG_FORMAT").ok().as_deref())
}

fn serve() -> ExitCode {
    let format = match env_log_format() {
        Ok(format) => format,
        Err(err) => {
            pvxwiki_mcp::init_tracing(LogFormat::Json);
            tracing::error!(error = %err, "invalid configuration; exiting");
            return ExitCode::FAILURE;
        }
    };
    pvxwiki_mcp::init_tracing(format);
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => {
            tracing::error!(error = %err, "invalid configuration; exiting");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(err) => {
            tracing::error!(error = ?err, "could not start the async runtime; exiting");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(config.listen).await?;
        pvxwiki_mcp::run(
            config,
            listener,
            Limits::default(),
            pvxwiki_mcp::Schedule::fandom(),
            pvxwiki_mcp::shutdown_signal(),
        )
        .await
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = ?err, "server stopped; exiting");
            ExitCode::FAILURE
        }
    }
}

/// GET /healthz on the configured listen address. The distroless image has no curl or shell.
/// Silent: the container runtime records the exit status.
fn healthcheck() -> ExitCode {
    let Ok(listen) = config::listen(std::env::var("PVXWIKI_LISTEN").ok().as_deref()) else {
        return ExitCode::FAILURE;
    };
    let target = config::healthcheck_target(listen);
    let ok = (|| -> std::io::Result<bool> {
        let mut stream = TcpStream::connect_timeout(&target, HEALTHCHECK_TIMEOUT)?;
        stream.set_read_timeout(Some(HEALTHCHECK_TIMEOUT))?;
        stream.set_write_timeout(Some(HEALTHCHECK_TIMEOUT))?;
        stream.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200"))
    })();
    if matches!(ok, Ok(true)) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
