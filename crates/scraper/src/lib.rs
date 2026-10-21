//! Polite client for the PvX wiki's MediaWiki Action API (`.claude/rules/scraping.md`).

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, RETRY_AFTER};
use serde_json::Value;
use tokio::time::Instant;

/// The only wiki this service copies from.
pub const FANDOM_API: &str = "https://gwpvx.fandom.com/api.php";

/// How the client spaces and retries requests.
#[derive(Debug, Clone, Copy)]
pub struct Pacing {
    /// Between any two requests, retries included.
    pub min_delay: Duration,
    /// Retries per call before the run gives up; the next scheduled run tries again.
    pub max_retries: u32,
    /// First backoff after a retryable failure; doubles each retry.
    pub backoff_base: Duration,
    /// Longest single wait. A `Retry-After` beyond it ends the run instead.
    pub backoff_cap: Duration,
}

impl Pacing {
    /// Fandom's condition is fewer than one request per second, retries included.
    pub const FANDOM: Pacing = Pacing {
        min_delay: Duration::from_secs(2),
        max_retries: 5,
        backoff_base: Duration::from_secs(5),
        backoff_cap: Duration::from_secs(120),
    };
}

/// A request that has not answered by then counts as a network failure and is retried.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Sent on every request: JSON in the current format, and back off while replicas lag.
const FIXED_PARAMS: [(&str, &str); 4] = [
    ("action", "query"),
    ("format", "json"),
    ("formatversion", "2"),
    ("maxlag", "5"),
];

#[derive(Debug, thiserror::Error)]
pub enum ScrapeError {
    #[error("PVXWIKI_CONTACT must be a non-empty address on one line")]
    InvalidContact,
    #[error("could not build the HTTP client")]
    Client(#[source] reqwest::Error),
    #[error("the wiki refused the request (HTTP {status}); stopping the run, do not retry")]
    Blocked { status: u16 },
    #[error(
        "the wiki answered HTTP {status} with {content_type:?} instead of JSON, likely a challenge page; stopping the run"
    )]
    NotJson {
        status: u16,
        content_type: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("unexpected HTTP {status} from the wiki")]
    Status { status: u16 },
    #[error("the wiki rejected the query: {code}: {info}")]
    Api { code: String, info: String },
    #[error("the wiki asked us to wait {seconds} s, longer than the backoff cap; ending the run")]
    RetryAfterTooLong { seconds: u64 },
    #[error("gave up after {attempts} attempts")]
    RetriesExhausted {
        attempts: u32,
        #[source]
        last: Transient,
    },
}

/// A failure worth retrying after a wait.
#[derive(Debug, thiserror::Error)]
pub enum Transient {
    #[error("wiki replicas lag more than maxlag allows")]
    Maxlag,
    #[error("HTTP {0}")]
    Status(u16),
    #[error("could not reach the wiki")]
    Network(#[source] reqwest::Error),
}

pub struct ApiClient {
    http: reqwest::Client,
    endpoint: String,
    pacing: Pacing,
    last_request: Option<Instant>,
}

impl ApiClient {
    pub fn new(endpoint: &str, contact: &str, pacing: Pacing) -> Result<Self, ScrapeError> {
        let contact = contact.trim();
        if contact.is_empty() || contact.chars().any(char::is_control) {
            return Err(ScrapeError::InvalidContact);
        }
        let http = reqwest::Client::builder()
            .user_agent(user_agent(contact))
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(ScrapeError::Client)?;
        Ok(Self {
            http,
            endpoint: endpoint.to_string(),
            pacing,
            last_request: None,
        })
    }

    /// One `action=query` request. `params` adds the module parameters (`list`, `prop`, ...).
    pub async fn query(&mut self, params: &[(&str, &str)]) -> Result<Value, ScrapeError> {
        let mut attempts = 0;
        loop {
            self.wait_turn().await;
            attempts += 1;
            let Retry {
                reason: transient,
                after,
            } = match self.send(params).await? {
                Ok(value) => return Ok(value),
                Err(retry) => retry,
            };
            if let Some(after) = after.filter(|after| *after > self.pacing.backoff_cap) {
                return Err(ScrapeError::RetryAfterTooLong {
                    seconds: after.as_secs(),
                });
            }
            if attempts > self.pacing.max_retries {
                return Err(ScrapeError::RetriesExhausted {
                    attempts,
                    last: transient,
                });
            }
            let wait = self.backoff(attempts).max(after.unwrap_or_default());
            tracing::warn!(
                attempts,
                wait_ms = wait.as_millis(),
                reason = %transient,
                "wiki asked us to slow down; retrying"
            );
            tokio::time::sleep(wait).await;
        }
    }

    /// One request. The outer `Err` ends the run; the inner one is worth a retry.
    async fn send(&self, params: &[(&str, &str)]) -> Result<Result<Value, Retry>, ScrapeError> {
        let response = match self
            .http
            .get(&self.endpoint)
            .query(&FIXED_PARAMS)
            .query(params)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => return Ok(Err(Retry::network(err))),
        };
        let status = response.status();
        let after = retry_after(response.headers());
        if status == StatusCode::FORBIDDEN {
            return Err(ScrapeError::Blocked {
                status: status.as_u16(),
            });
        }
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            return Ok(Err(Retry {
                reason: Transient::Status(status.as_u16()),
                after,
            }));
        }
        if !status.is_success() {
            return Err(ScrapeError::Status {
                status: status.as_u16(),
            });
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(err) => return Ok(Err(Retry::network(err))),
        };
        let value: Value =
            serde_json::from_slice(&body).map_err(|source| ScrapeError::NotJson {
                status: status.as_u16(),
                content_type,
                source,
            })?;
        if let Some(error) = value.get("error") {
            let field = |name: &str| error[name].as_str().unwrap_or_default().to_string();
            let code = field("code");
            if code == "maxlag" {
                return Ok(Err(Retry {
                    reason: Transient::Maxlag,
                    after,
                }));
            }
            return Err(ScrapeError::Api {
                code,
                info: field("info"),
            });
        }
        Ok(Ok(value))
    }

    /// Doubles from `backoff_base` per attempt, up to `backoff_cap`.
    fn backoff(&self, attempts: u32) -> Duration {
        let factor = 2u32.saturating_pow(attempts.saturating_sub(1));
        self.pacing
            .backoff_base
            .saturating_mul(factor)
            .min(self.pacing.backoff_cap)
    }

    /// Holds every request, retries included, to one per `min_delay`.
    async fn wait_turn(&mut self) {
        if let Some(last) = self.last_request {
            tokio::time::sleep_until(last + self.pacing.min_delay).await;
        }
        self.last_request = Some(Instant::now());
    }
}

struct Retry {
    reason: Transient,
    /// The server's `Retry-After`, when it sent one in seconds.
    after: Option<Duration>,
}

impl Retry {
    fn network(err: reqwest::Error) -> Self {
        Self {
            reason: Transient::Network(err),
            after: None,
        }
    }
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(seconds))
}

/// Wikimedia's User-Agent convention: `name/version (url; contact)`.
fn user_agent(contact: &str) -> String {
    format!(
        "pvxwiki-mcp/{} ({}; {contact})",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY"),
    )
}
