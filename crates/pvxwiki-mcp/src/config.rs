//! Environment configuration, parsed once at startup.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;

use pvxwiki_server::ProxyList;

pub const DEFAULT_PORT: u16 = 8080;
/// Every interface, so the container port mapping reaches it.
pub const DEFAULT_LISTEN: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DEFAULT_PORT);
/// The admin page's port; compose publishes it on 127.0.0.1 only by default.
pub const DEFAULT_ADMIN_PORT: u16 = 8081;
pub const DEFAULT_ADMIN_LISTEN: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DEFAULT_ADMIN_PORT);
pub const DEFAULT_DATA_DIR: &str = "/data";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Json,
    Pretty,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    /// The admin page's own listener (`PVXWIKI_ADMIN_LISTEN`).
    pub admin_listen: SocketAddr,
    pub data_dir: PathBuf,
    pub allowed_hosts: Vec<String>,
    pub trusted_proxies: ProxyList,
    pub log_format: LogFormat,
    /// Operator contact for the scraper's User-Agent. Unset turns the scraper off.
    pub contact: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{var} is invalid: {reason}")]
    Invalid { var: &'static str, reason: String },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let allowed_hosts: Vec<String> = get("PVXWIKI_ALLOWED_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(str::to_string)
            .collect();
        if allowed_hosts.is_empty() {
            return Err(ConfigError::Missing("PVXWIKI_ALLOWED_HOSTS"));
        }
        Ok(Self {
            listen: listen(get("PVXWIKI_LISTEN").as_deref())?,
            admin_listen: socket_addr(
                "PVXWIKI_ADMIN_LISTEN",
                get("PVXWIKI_ADMIN_LISTEN").as_deref(),
                DEFAULT_ADMIN_LISTEN,
            )?,
            data_dir: PathBuf::from(
                get("PVXWIKI_DATA_DIR").unwrap_or_else(|| DEFAULT_DATA_DIR.into()),
            ),
            allowed_hosts,
            trusted_proxies: ProxyList::parse(&get("PVXWIKI_TRUSTED_PROXIES").unwrap_or_default())
                .map_err(|reason| ConfigError::Invalid {
                    var: "PVXWIKI_TRUSTED_PROXIES",
                    reason,
                })?,
            log_format: log_format(get("PVXWIKI_LOG_FORMAT").as_deref())?,
            contact: get("PVXWIKI_CONTACT")
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty()),
        })
    }
}

/// `PVXWIKI_LISTEN`, or the default. Also read by `healthcheck`, which needs nothing else.
pub fn listen(value: Option<&str>) -> Result<SocketAddr, ConfigError> {
    socket_addr("PVXWIKI_LISTEN", value, DEFAULT_LISTEN)
}

fn socket_addr(
    var: &'static str,
    value: Option<&str>,
    default: SocketAddr,
) -> Result<SocketAddr, ConfigError> {
    match value.map(str::trim) {
        None | Some("") => Ok(default),
        Some(value) => value
            .parse()
            .map_err(|e: std::net::AddrParseError| ConfigError::Invalid {
                var,
                reason: e.to_string(),
            }),
    }
}

/// Also used before the rest of the config is parsed, so config errors are logged in the
/// operator's chosen format.
pub fn log_format(value: Option<&str>) -> Result<LogFormat, ConfigError> {
    match value.map(str::trim) {
        None | Some("") | Some("json") => Ok(LogFormat::Json),
        Some("pretty") => Ok(LogFormat::Pretty),
        Some(other) => Err(ConfigError::Invalid {
            var: "PVXWIKI_LOG_FORMAT",
            reason: format!("{other:?} is not json or pretty"),
        }),
    }
}

/// Where `healthcheck` connects: the listen address, or loopback of the same family when the
/// server listens on every interface.
pub fn healthcheck_target(listen: SocketAddr) -> SocketAddr {
    let ip = match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, listen.port())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn defaults_apply_when_only_required_is_set() {
        let config =
            Config::from_lookup(lookup(&[("PVXWIKI_ALLOWED_HOSTS", "mcp.example.org")])).unwrap();
        assert_eq!(config.listen.to_string(), "0.0.0.0:8080");
        assert_eq!(config.admin_listen.to_string(), "0.0.0.0:8081");
        assert_eq!(config.data_dir, std::path::PathBuf::from(DEFAULT_DATA_DIR));
        assert_eq!(config.allowed_hosts, ["mcp.example.org"]);
        assert!(config.trusted_proxies.is_empty());
        assert_eq!(config.log_format, LogFormat::Json);
        assert_eq!(config.contact, None);
    }

    #[test]
    fn missing_allowed_hosts_names_the_variable() {
        let err = Config::from_lookup(lookup(&[])).unwrap_err();
        assert!(err.to_string().contains("PVXWIKI_ALLOWED_HOSTS"), "{err}");
        let err = Config::from_lookup(lookup(&[("PVXWIKI_ALLOWED_HOSTS", " , ")])).unwrap_err();
        assert!(err.to_string().contains("PVXWIKI_ALLOWED_HOSTS"), "{err}");
    }

    #[test]
    fn invalid_values_name_the_variable() {
        for (key, value) in [
            ("PVXWIKI_LISTEN", "nope"),
            ("PVXWIKI_ADMIN_LISTEN", "nope"),
            ("PVXWIKI_TRUSTED_PROXIES", "10.0.0.0/99"),
            ("PVXWIKI_LOG_FORMAT", "xml"),
        ] {
            let err = Config::from_lookup(lookup(&[("PVXWIKI_ALLOWED_HOSTS", "a"), (key, value)]))
                .unwrap_err();
            assert!(err.to_string().contains(key), "{err}");
        }
    }

    #[test]
    fn all_values_parse() {
        let config = Config::from_lookup(lookup(&[
            ("PVXWIKI_ALLOWED_HOSTS", "a.example, b.example"),
            ("PVXWIKI_LISTEN", "127.0.0.1:9000"),
            ("PVXWIKI_ADMIN_LISTEN", "127.0.0.1:9001"),
            ("PVXWIKI_DATA_DIR", "/tmp/x"),
            ("PVXWIKI_TRUSTED_PROXIES", "10.0.0.5,172.31.250.0/24"),
            ("PVXWIKI_LOG_FORMAT", "pretty"),
            ("PVXWIKI_CONTACT", "ops@example.org"),
        ]))
        .unwrap();
        assert_eq!(config.allowed_hosts, ["a.example", "b.example"]);
        assert_eq!(config.listen.port(), 9000);
        assert_eq!(config.admin_listen.port(), 9001);
        assert!(
            config
                .trusted_proxies
                .contains("172.31.250.9".parse().unwrap())
        );
        assert_eq!(config.log_format, LogFormat::Pretty);
        assert_eq!(config.contact.as_deref(), Some("ops@example.org"));
    }

    #[test]
    fn healthcheck_uses_the_listen_ip_and_loopback_only_for_unspecified() {
        let target = |listen: &str| healthcheck_target(listen.parse().unwrap()).to_string();
        assert_eq!(target("0.0.0.0:8080"), "127.0.0.1:8080");
        assert_eq!(target("[::]:9000"), "[::1]:9000");
        assert_eq!(target("10.1.2.3:8080"), "10.1.2.3:8080");
        assert_eq!(target("[fd00::5]:8081"), "[fd00::5]:8081");
    }
}
