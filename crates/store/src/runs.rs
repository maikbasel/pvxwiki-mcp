use crate::{SqliteStore, StoreError};

/// Newest runs kept in the history.
pub const RUN_HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Scheduled,
    Manual,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Running,
    Finished,
    Failed,
    Blocked,
    Interrupted,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Finished => "finished",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunCounts {
    pub checked: u64,
    pub updated: u64,
    pub kept: u64,
    pub removed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    pub id: i64,
    pub trigger: Trigger,
    pub outcome: Outcome,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub counts: RunCounts,
    pub error: Option<String>,
}

impl Trigger {
    fn parse(text: &str) -> Result<Self, StoreError> {
        match text {
            "scheduled" => Ok(Self::Scheduled),
            "manual" => Ok(Self::Manual),
            other => Err(StoreError::UnknownRunValue {
                field: "trigger",
                value: other.to_string(),
            }),
        }
    }
}

impl Outcome {
    fn parse(text: &str) -> Result<Self, StoreError> {
        match text {
            "running" => Ok(Self::Running),
            "finished" => Ok(Self::Finished),
            "failed" => Ok(Self::Failed),
            "blocked" => Ok(Self::Blocked),
            "interrupted" => Ok(Self::Interrupted),
            other => Err(StoreError::UnknownRunValue {
                field: "outcome",
                value: other.to_string(),
            }),
        }
    }
}

fn count(value: i64) -> u64 {
    // Counts are written from u64 and never negative; a corrupt negative reads as 0.
    u64::try_from(value).unwrap_or(0)
}

impl SqliteStore {
    /// Records a run as `running` and returns its id.
    pub fn start_run(&self, trigger: Trigger, started_at: i64) -> Result<i64, StoreError> {
        let mut writer = self.writer.lock().map_err(|_| StoreError::Poisoned)?;
        let tx = writer.transaction()?;
        let id: i64 = tx.query_row(
            "INSERT INTO runs (trigger, outcome, started_at) VALUES (?1, 'running', ?2) RETURNING id",
            rusqlite::params![trigger.as_str(), started_at],
            |r| r.get(0),
        )?;
        tx.execute(
            "DELETE FROM runs WHERE outcome <> 'running'
               AND id NOT IN (SELECT id FROM runs ORDER BY id DESC LIMIT ?1)",
            [i64::try_from(RUN_HISTORY_LIMIT).unwrap_or(i64::MAX)],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn finish_run(
        &self,
        id: i64,
        outcome: Outcome,
        finished_at: i64,
        counts: RunCounts,
        error: Option<&str>,
    ) -> Result<(), StoreError> {
        let to_sql = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
        let writer = self.writer.lock().map_err(|_| StoreError::Poisoned)?;
        writer.execute(
            "UPDATE runs SET outcome = ?2, finished_at = ?3, checked = ?4, updated = ?5, kept = ?6, removed = ?7, error = ?8
             WHERE id = ?1",
            rusqlite::params![
                id,
                outcome.as_str(),
                finished_at,
                to_sql(counts.checked),
                to_sql(counts.updated),
                to_sql(counts.kept),
                to_sql(counts.removed),
                error,
            ],
        )?;
        Ok(())
    }

    /// Marks runs left `running` by a killed process as interrupted. Returns how many.
    pub fn interrupt_running_runs(&self) -> Result<usize, StoreError> {
        let writer = self.writer.lock().map_err(|_| StoreError::Poisoned)?;
        Ok(writer.execute(
            "UPDATE runs SET outcome = 'interrupted' WHERE outcome = 'running'",
            [],
        )?)
    }

    /// Newest first.
    pub async fn recent_runs(&self, limit: usize) -> Result<Vec<RunRecord>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, trigger, outcome, started_at, finished_at, checked, updated, kept, removed, error
                 FROM runs ORDER BY id DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    [r.get::<_, i64>(5)?, r.get(6)?, r.get(7)?, r.get(8)?],
                    r.get::<_, Option<String>>(9)?,
                ))
            })?;
            rows.map(|row| {
                let (id, trigger, outcome, started_at, finished_at, c, error) = row?;
                Ok(RunRecord {
                    id,
                    trigger: Trigger::parse(&trigger)?,
                    outcome: Outcome::parse(&outcome)?,
                    started_at,
                    finished_at,
                    counts: RunCounts {
                        checked: count(c[0]),
                        updated: count(c[1]),
                        kept: count(c[2]),
                        removed: count(c[3]),
                    },
                    error,
                })
            })
            .collect()
        })
        .await
    }
}
