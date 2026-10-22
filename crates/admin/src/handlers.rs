//! Admin endpoints: gather from the runner and the store, render with `views`.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use maud::Markup;
use pvxwiki_store::{RUN_HISTORY_LIMIT, StoreError};

use crate::{AdminState, guard, views};

/// Issue kinds shown first: the parser gave up on these pages as a whole.
const SERIOUS_KINDS: [&str; 2] = ["too_deeply_nested", "timed_out"];
/// Longer than any issue kind; a longer `kind` query is refused.
const MAX_KIND_LEN: usize = 64;

/// A store failure, answered without leaking internals.
pub(crate) struct AdminError(StoreError);

impl From<StoreError> for AdminError {
    fn from(err: StoreError) -> Self {
        Self(err)
    }
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        tracing::warn!(error = ?self.0, "admin page could not read the database");
        let status = match self.0 {
            StoreError::Busy | StoreError::Timeout => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, "the database is not available; try again shortly").into_response()
    }
}

type Page = Result<Markup, AdminError>;

pub(crate) async fn index(
    State(state): State<AdminState>,
    Query(query): Query<HashMap<String, String>>,
) -> Page {
    Ok(views::page(
        status_markup(&state).await?,
        history_markup(&state).await?,
        attention_markup(&state, query.get("kind").map(String::as_str)).await?,
    ))
}

pub(crate) async fn status(State(state): State<AdminState>) -> Page {
    status_markup(&state).await
}

pub(crate) async fn history(State(state): State<AdminState>) -> Page {
    history_markup(&state).await
}

pub(crate) async fn issues(
    State(state): State<AdminState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, AdminError> {
    let kind = query.get("kind").map(String::as_str).unwrap_or_default();
    if kind.len() > MAX_KIND_LEN {
        return Ok((StatusCode::BAD_REQUEST, "kind is too long").into_response());
    }
    let (pages, total) = state.store.pages_with_issue(kind).await?;
    Ok(views::issues(kind, &pages, total).into_response())
}

/// 403 for anything a cross-site page could send; 202 when a run was requested, 409 when
/// refused. Both answer with the fresh status fragment.
pub(crate) async fn run_now(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<Response, AdminError> {
    if !guard::same_origin_htmx(&headers) {
        tracing::warn!("cross-site or non-htmx run request; 403 returned");
        return Ok((StatusCode::FORBIDDEN, "cross-site request refused").into_response());
    }
    let code = match state.runner.run_now() {
        Ok(()) => StatusCode::ACCEPTED,
        Err(refused) => {
            tracing::info!(reason = %refused, "manual run refused; 409 returned");
            StatusCode::CONFLICT
        }
    };
    Ok((code, status_markup(&state).await?).into_response())
}

async fn status_markup(state: &AdminState) -> Page {
    let counts = state.store.namespace_counts().await?;
    Ok(views::status_view(&state.runner.status(), &counts, now()))
}

async fn history_markup(state: &AdminState) -> Page {
    let runs = state.store.recent_runs(RUN_HISTORY_LIMIT).await?;
    Ok(views::history(&runs, now()))
}

async fn attention_markup(state: &AdminState, kind: Option<&str>) -> Page {
    let failures = state.store.parse_failures().await?;
    let counts = state.store.issue_counts().await?;
    let kind = kind
        .filter(|k| k.len() <= MAX_KIND_LEN)
        .or_else(|| {
            SERIOUS_KINDS
                .into_iter()
                .find(|k| counts.iter().any(|(name, _)| name == k))
        })
        .or_else(|| counts.first().map(|(name, _)| name.as_str()))
        .unwrap_or(SERIOUS_KINDS[0]);
    let (pages, total) = state.store.pages_with_issue(kind).await?;
    Ok(views::attention(&failures, &counts, kind, &pages, total))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
