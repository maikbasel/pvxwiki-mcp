//! The one place that starts scrape runs: on a schedule or on request, one at a time.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use pvxwiki_store::{Outcome, RunCounts, SqliteStore, Trigger};
use tokio::sync::Notify;
use tokio::time::Instant;
use tracing::Instrument;

use crate::run::unix_now;
use crate::{ApiClient, RunReport, ScrapeError};

/// How far a running run has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Listing namespaces; `done` and `total` count namespaces.
    Listing,
    /// Fetching changed pages; `done` and `total` count pages.
    Fetching,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScraperState {
    /// No `PVXWIKI_CONTACT`: the scraper never runs.
    Off,
    Idle,
    Running {
        trigger: Trigger,
        started_at: i64,
        progress: Option<Progress>,
    },
    /// The wiki refused us (403 or a challenge page); no runs until restart.
    Blocked {
        since: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerStatus {
    pub state: ScraperState,
    /// Unix seconds of the next scheduled run; `None` when no run will be scheduled.
    pub next_run_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RunRefused {
    #[error("the scraper is off: PVXWIKI_CONTACT is not set")]
    Off,
    #[error("a run is already going")]
    Running,
    #[error("the wiki blocked the scraper; restart the service after checking why")]
    Blocked,
}

#[derive(Clone)]
pub struct Runner {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<SqliteStore>,
    interval: Duration,
    /// Taken by `run_forever`, the only task that talks to the wiki.
    client: Mutex<Option<ApiClient>>,
    /// Never held across an `.await`.
    shared: Mutex<Shared>,
    /// Wakes `run_forever` for a manual run.
    wake: Notify,
}

struct Shared {
    state: ScraperState,
    next_run_at: Option<i64>,
    /// Set by `run_now`, cleared when the run starts; a second request in between is refused.
    manual_requested: bool,
}

impl Runner {
    /// `client` is `None` when no contact is configured: the scraper is then off for good.
    pub fn new(store: Arc<SqliteStore>, client: Option<ApiClient>, interval: Duration) -> Self {
        let state = if client.is_some() {
            ScraperState::Idle
        } else {
            ScraperState::Off
        };
        Self {
            inner: Arc::new(Inner {
                store,
                interval,
                client: Mutex::new(client),
                shared: Mutex::new(Shared {
                    state,
                    next_run_at: None,
                    manual_requested: false,
                }),
                wake: Notify::new(),
            }),
        }
    }

    pub fn status(&self) -> RunnerStatus {
        let shared = self.shared();
        RunnerStatus {
            state: shared.state.clone(),
            next_run_at: shared.next_run_at,
        }
    }

    /// Asks `run_forever` for a manual run. Checked and marked under one lock, so of two
    /// concurrent requests only one is accepted.
    pub fn run_now(&self) -> Result<(), RunRefused> {
        let mut shared = self.shared();
        match shared.state {
            ScraperState::Off => return Err(RunRefused::Off),
            ScraperState::Blocked { .. } => return Err(RunRefused::Blocked),
            ScraperState::Running { .. } => return Err(RunRefused::Running),
            _ if shared.manual_requested => return Err(RunRefused::Running),
            _ => {}
        }
        shared.manual_requested = true;
        drop(shared);
        tracing::info!(trigger = "manual", "manual run requested");
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Runs at once, then every `interval`, until the wiki blocks us. Run as one spawned task.
    pub async fn run_forever(self) {
        let client = self
            .inner
            .client
            .lock()
            .map(|mut c| c.take())
            .ok()
            .flatten();
        let Some(mut client) = client else {
            return;
        };
        match self.inner.store.interrupt_running_runs() {
            Ok(0) => {}
            Ok(closed) => tracing::warn!(
                closed,
                "previous process stopped mid-run; its runs are recorded as interrupted"
            ),
            Err(err) => tracing::warn!(error = ?err, "could not close interrupted runs"),
        }
        let mut next = Instant::now();
        loop {
            self.shared().next_run_at = Some(unix_now() + secs_until(next));
            let trigger = tokio::select! {
                biased;
                () = tokio::time::sleep_until(next) => {
                    next += self.inner.interval;
                    Trigger::Scheduled
                }
                () = self.inner.wake.notified() => {
                    // A permit left from a request a scheduled run already served.
                    if !self.shared().manual_requested {
                        continue;
                    }
                    Trigger::Manual
                }
            };
            // Published before the run, so status never shows a time already past.
            self.shared().next_run_at = Some(unix_now() + secs_until(next));
            if self.execute(&mut client, trigger).await == Outcome::Blocked {
                return;
            }
            // A run longer than the interval skips the ticks it covered.
            if next <= Instant::now() {
                next = Instant::now() + self.inner.interval;
            }
        }
    }

    /// One run, wrapped in its history row. Returns the outcome it recorded.
    async fn execute(&self, client: &mut ApiClient, trigger: Trigger) -> Outcome {
        let started_at = unix_now();
        {
            let mut shared = self.shared();
            shared.manual_requested = false;
            shared.state = ScraperState::Running {
                trigger,
                started_at,
                progress: None,
            };
        }
        let store = &self.inner.store;
        let id = store.start_run(trigger, started_at);
        let (outcome, report, error) = match crate::run(client, store, &|p| self.set_progress(p))
            .instrument(tracing::info_span!(
                "scrape_run",
                trigger = trigger.as_str()
            ))
            .await
        {
            Ok(report) => (Outcome::Finished, report, None),
            Err(err @ (ScrapeError::Blocked { .. } | ScrapeError::NotJson { .. })) => {
                tracing::warn!(
                    error = ?err,
                    "the wiki blocked the scraper; no further runs until restart"
                );
                (Outcome::Blocked, RunReport::default(), Some(chain(&err)))
            }
            Err(err) => {
                tracing::warn!(
                    error = ?err,
                    next_run_in_s = self.inner.interval.as_secs(),
                    "scrape run failed; the stored index stays as it was"
                );
                (Outcome::Failed, RunReport::default(), Some(chain(&err)))
            }
        };
        // State first: whoever sees the closed row also sees the runner past it.
        {
            let mut shared = self.shared();
            if outcome == Outcome::Blocked {
                shared.state = ScraperState::Blocked { since: unix_now() };
                shared.next_run_at = None;
            } else {
                shared.state = ScraperState::Idle;
            }
        }
        let finished = id.and_then(|id| {
            store.finish_run(id, outcome, unix_now(), counts(&report), error.as_deref())
        });
        if let Err(err) = finished {
            tracing::warn!(error = ?err, "could not record the run in the history");
        }
        outcome
    }

    fn set_progress(&self, now: Progress) {
        if let ScraperState::Running { progress, .. } = &mut self.shared().state {
            *progress = Some(now);
        }
    }

    fn shared(&self) -> MutexGuard<'_, Shared> {
        // A panic while holding this lock leaves plain data behind; keep serving it.
        self.inner
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The error and its sources, for the history row.
fn chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(next) = source {
        text.push_str(": ");
        text.push_str(&next.to_string());
        source = next.source();
    }
    text
}

fn secs_until(at: Instant) -> i64 {
    i64::try_from(at.saturating_duration_since(Instant::now()).as_secs()).unwrap_or(i64::MAX)
}

fn counts(report: &RunReport) -> RunCounts {
    let n = |v: usize| u64::try_from(v).unwrap_or(u64::MAX);
    RunCounts {
        checked: n(report.checked),
        updated: n(report.updated),
        kept: n(report.kept),
        removed: n(report.removed),
    }
}
