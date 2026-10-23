//! The admin page as maud markup. Pure functions of the data they are given.

use maud::{DOCTYPE, Markup, html};

use crate::BASE_PATH;
use pvxwiki_scraper::{Phase, RunRefused, RunnerStatus, ScraperState};
use pvxwiki_store::{ARCHIVE_NAMESPACE, IssuePage, ParseFailure, RunRecord};

const WIKI: &str = "https://gwpvx.fandom.com";

/// The page's URL, built from its title only (never from page content).
pub(crate) fn wiki_url(title: &str) -> String {
    let mut url = format!("{WIKI}/wiki/");
    for byte in title.replace(' ', "_").bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/:_".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

/// One revision of a page.
pub(crate) fn revision_url(revision: i64) -> String {
    format!("{WIKI}/index.php?oldid={revision}")
}

/// htmx sends no request bodies here; let it swap a 409 so the refusal reason shows at once.
const HTMX_CONFIG: &str = r#"{"responseHandling":[{"code":"204","swap":false},{"code":"[23]..","swap":true},{"code":"409","swap":true},{"code":"[45]..","swap":false,"error":true}]}"#;

/// The whole page around the three sections.
pub(crate) fn page(status: Markup, history: Markup, attention: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="htmx-config" content=(HTMX_CONFIG);
                title { "pvxwiki-mcp admin" }
                link rel="stylesheet" href={ (BASE_PATH) "/assets/admin.css" };
                script src={ (BASE_PATH) "/assets/htmx.min.js" } {}
            }
            body {
                main.shell {
                    header.top {
                        div.brand { h1 { "pvxwiki-mcp" } span { "Admin" } }
                        a href={ (WIKI) "/wiki/PvX_wiki" } { "PvX wiki" }
                    }
                    (status)
                    (history)
                    (attention)
                }
            }
        }
    }
}

/// htmx trigger for the status refresh while the page is open.
pub(crate) const PROGRESS_POLL: &str = "every 2s";
/// Rows in the history table; the run strip above it shows every stored run.
const HISTORY_ROWS: usize = 10;
/// htmx trigger for the history refresh; a run takes minutes, so this can be slow.
pub(crate) const HISTORY_POLL: &str = "every 10s";

/// The scraper's state, progress, the run button, and the next run, index size and last run.
/// `now` in unix seconds.
pub(crate) fn status_view(
    status: &RunnerStatus,
    counts: &[(i64, u64)],
    last: Option<&RunRecord>,
    now: i64,
) -> Markup {
    let (name, label, refused) = match &status.state {
        ScraperState::Off => ("off", "Off", Some(RunRefused::Off)),
        ScraperState::Idle => ("idle", "Idle", None),
        ScraperState::Running { .. } => ("running", "Running", Some(RunRefused::Running)),
        ScraperState::Blocked { .. } => ("blocked", "Blocked", Some(RunRefused::Blocked)),
    };
    html! {
        section #status .panel data-state=(name) hx-get={ (BASE_PATH) "/status" }
            hx-trigger=(PROGRESS_POLL) hx-swap="outerHTML" {
            div.panel-head {
                div.state {
                    span class={ "dot " (name) } aria-hidden="true" {}
                    strong { (label) }
                    p {
                        @match &status.state {
                            ScraperState::Off => "PVXWIKI_CONTACT is not set, so the index is not updated.",
                            ScraperState::Idle => "Waiting for the next run.",
                            ScraperState::Running { trigger, started_at, .. } => {
                                "A " (trigger.as_str()) " run started " (ago(now - started_at)) "."
                            }
                            ScraperState::Blocked { since } => {
                                "The wiki blocked the scraper " (ago(now - since))
                                ". Check the logs and the wiki's terms, then restart the service."
                            }
                        }
                    }
                }
                div.action {
                    // The sentence beside the state already says why; the tooltip repeats it.
                    button.btn hx-post={ (BASE_PATH) "/runs" } hx-target="#status" hx-swap="outerHTML"
                        title=[refused.map(|r| r.to_string())] disabled[refused.is_some()] { "Run now" }
                }
            }
            @if let ScraperState::Running { progress: Some(p), .. } = &status.state {
                div.progress {
                    @if p.total > 0 {
                        progress value=(p.done) max=(p.total) {}
                    }
                    p.muted {
                        @match p.phase {
                            Phase::Listing => "Listing namespaces: ",
                            Phase::Fetching => "Fetching changed pages: ",
                        }
                        (p.done) " of " (p.total)
                    }
                }
            }
            dl.facts {
                div {
                    dt { "Next run" }
                    dd { @match status.next_run_at {
                        Some(next) => { "in " (span(next - now)) }
                        None => "None scheduled",
                    } }
                }
                div {
                    dt { "Index" }
                    dd {
                        @if counts.is_empty() { "Empty" }
                        @for (i, (namespace, pages)) in counts.iter().enumerate() {
                            @if i > 0 { br; }
                            (pages) " " (namespace_name(*namespace))
                        }
                    }
                }
                div {
                    dt { "Last run" }
                    dd { @match last {
                        Some(run) => {
                            span class={ "outcome " (run.outcome.as_str()) } { (run.outcome.as_str()) }
                            " " (ago(now - run.started_at))
                        }
                        None => "None yet",
                    } }
                }
            }
        }
    }
}

/// Newest runs first. `now` in unix seconds.
pub(crate) fn history(runs: &[RunRecord], now: i64) -> Markup {
    html! {
        section #history .block hx-get={ (BASE_PATH) "/history" } hx-trigger=(HISTORY_POLL)
            hx-swap="outerHTML" {
            div.block-head {
                h2 { "Run history" }
                @if runs.len() > HISTORY_ROWS {
                    span.muted { "Table shows the newest " (HISTORY_ROWS) " of " (runs.len()) }
                }
            }
            @if runs.is_empty() {
                p.empty { "No runs yet. The first starts with the service once PVXWIKI_CONTACT is set." }
            } @else {
                ol.strip aria-label="Runs, oldest first" {
                    @for run in runs.iter().rev() {
                        li class=(run.outcome.as_str()) title={
                            (run.outcome.as_str()) ", " (run.trigger.as_str()) " run, "
                            (ago(now - run.started_at)) ", " (run.counts.updated) " updated"
                        } {}
                    }
                }
                div.table-wrap {
                    table {
                        thead { tr {
                            th { "Started" } th { "Took" } th { "Trigger" } th { "Outcome" }
                            th.num { "Checked" } th.num { "Updated" } th.num { "Kept" } th.num { "Removed" }
                        } }
                        tbody {
                            @for run in runs.iter().take(HISTORY_ROWS) {
                                tr data-trigger=(run.trigger.as_str()) data-outcome=(run.outcome.as_str())
                                    data-checked=(run.counts.checked) {
                                    td { (ago(now - run.started_at)) }
                                    td { @match run.finished_at {
                                        Some(end) => (span(end - run.started_at)),
                                        None => "-",
                                    } }
                                    td { (run.trigger.as_str()) }
                                    td {
                                        span class={ "outcome " (run.outcome.as_str()) } { (run.outcome.as_str()) }
                                        @if let Some(error) = &run.error { span.error { (error) } }
                                    }
                                    td.num { (run.counts.checked) }
                                    td.num { (run.counts.updated) }
                                    td.num { (run.counts.kept) }
                                    td.num { (run.counts.removed) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// "and N more" when a capped list shows fewer than `total`.
fn more(shown: usize, total: u64) -> Markup {
    let hidden = total.saturating_sub(u64::try_from(shown).unwrap_or(u64::MAX));
    html! {
        @if hidden > 0 {
            p.more { "and " (hidden) " more" }
        }
    }
}

fn namespace_name(namespace: i64) -> String {
    match namespace {
        100 => "build pages".into(),
        ARCHIVE_NAMESPACE => "archived pages".into(),
        other => format!("pages in namespace {other}"),
    }
}

/// "3 min ago", "just now".
fn ago(seconds: i64) -> String {
    if seconds < 1 {
        "just now".into()
    } else {
        format!("{} ago", span(seconds))
    }
}

/// A coarse, readable length of time: "42 s", "5 min", "2 h 10 min", "3 d 4 h".
fn span(seconds: i64) -> String {
    let s = seconds.max(0);
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        3600..86_400 => format!("{} h {} min", s / 3600, s % 3600 / 60),
        _ => format!("{} d {} h", s / 86_400, s % 86_400 / 3600),
    }
}

/// Pages needing attention: failed parses, then pages with parse issues of one `kind`.
pub(crate) fn attention(
    failures: &[ParseFailure],
    failure_total: u64,
    issue_counts: &[(String, u64)],
    kind: &str,
    pages: &[IssuePage],
    total: u64,
) -> Markup {
    html! {
        section.block {
            div.block-head { h2 { "Pages needing attention" } }
            div.columns {
                div {
                    h3 { "Newest revision failed to parse" }
                    @if failures.is_empty() {
                        p.empty { "None. Every stored page is at its newest revision." }
                    } @else {
                        p { "The stored copy is older than the wiki's. The failed revision is the one to fix the parser for." }
                        div.table-wrap {
                            table {
                                thead { tr { th { "Page" } th.num { "Stored" } th.num { "Failed" } } }
                                tbody {
                                    @for f in failures {
                                        tr {
                                            td { a href=(wiki_url(&f.title)) { (f.title) } }
                                            td.num { a href=(revision_url(f.stored_revision)) { (f.stored_revision) } }
                                            td.num { a href=(revision_url(f.failed_revision)) { (f.failed_revision) } }
                                        }
                                    }
                                }
                            }
                        }
                        (more(failures.len(), failure_total))
                    }
                }
                div {
                    h3 { "Stored with parse issues" }
                    @if issue_counts.is_empty() {
                        p.empty { "None." }
                    } @else {
                        ul.chips {
                            @for (name, count) in issue_counts {
                                li {
                                    a href={ "?kind=" (name) } hx-get={ (BASE_PATH) "/issues?kind=" (name) }
                                        hx-target="#issues" hx-swap="outerHTML"
                                        aria-current=[(name == kind).then_some("true")] {
                                        (name) " " (count)
                                    }
                                }
                            }
                        }
                        (issues(kind, pages, total))
                    }
                }
            }
        }
    }
}

/// The capped list of pages with issues of `kind`.
pub(crate) fn issues(kind: &str, pages: &[IssuePage], total: u64) -> Markup {
    html! {
        div #issues data-kind=(kind) {
            ul.pages {
                @for page in pages {
                    li {
                        a href=(wiki_url(&page.title)) { (page.title) }
                        a.muted href=(revision_url(page.revision)) { "rev " (page.revision) }
                    }
                }
            }
            (more(pages.len(), total))
        }
    }
}

#[cfg(test)]
mod tests {
    use pvxwiki_scraper::Progress;
    use pvxwiki_store::{Outcome, RunCounts, Trigger};

    use super::*;

    #[test]
    fn wiki_url_encodes_titles() {
        assert_eq!(
            wiki_url("Build:A/Me \"Q&A\" #1?"),
            "https://gwpvx.fandom.com/wiki/Build:A/Me_%22Q%26A%22_%231%3F"
        );
        assert_eq!(
            revision_url(1306202),
            "https://gwpvx.fandom.com/index.php?oldid=1306202"
        );
    }

    fn failure(title: &str) -> ParseFailure {
        ParseFailure {
            title: title.into(),
            stored_revision: 1,
            failed_revision: 2,
            first_seen: 0,
        }
    }

    #[test]
    fn attention_escapes_titles_and_links_both_revisions() {
        let html =
            attention(&[failure("Build:<script>")], 1, &[], "timed_out", &[], 0).into_string();

        assert!(html.contains("Build:&lt;script&gt;"), "{html}");
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains(&revision_url(1)), "{html}");
        assert!(html.contains(&revision_url(2)), "{html}");
    }

    #[test]
    fn issue_list_says_how_many_more() {
        let pages: Vec<IssuePage> = (0..50)
            .map(|i| IssuePage {
                title: format!("Build:{i}"),
                revision: i,
            })
            .collect();

        let html = issues("timed_out", &pages, 51).into_string();

        assert!(html.contains("and 1 more"));
        assert!(
            !issues("timed_out", &pages, 50)
                .into_string()
                .contains("more")
        );
    }

    fn render(state: ScraperState) -> String {
        let status = RunnerStatus {
            state,
            next_run_at: None,
        };
        status_view(&status, &[(100, 3), (104, 1)], None, 1_000).into_string()
    }

    #[test]
    fn status_disables_run_now_with_the_reason() {
        let off = render(ScraperState::Off);
        assert!(off.contains(r#"data-state="off""#), "{off}");
        assert!(off.contains("PVXWIKI_CONTACT"), "{off}");
        assert!(off.contains("disabled"), "{off}");

        let running = render(ScraperState::Running {
            trigger: Trigger::Manual,
            started_at: 940,
            progress: Some(Progress {
                phase: Phase::Fetching,
                done: 3,
                total: 7,
            }),
        });
        assert!(running.contains(r#"data-state="running""#), "{running}");
        assert!(
            running.contains(r#"value="3""#) && running.contains(r#"max="7""#),
            "{running}"
        );
        assert!(running.contains("disabled"), "{running}");

        let blocked = render(ScraperState::Blocked { since: 900 });
        assert!(blocked.contains(r#"data-state="blocked""#), "{blocked}");
        assert!(blocked.contains("disabled"), "{blocked}");

        let idle = render(ScraperState::Idle);
        assert!(idle.contains(r#"data-state="idle""#), "{idle}");
        assert!(!idle.contains("disabled"), "{idle}");
        assert!(idle.contains(r#"hx-post="/admin/runs""#), "{idle}");
    }

    #[test]
    fn history_shows_outcome_trigger_counts_and_error() {
        let failed = RunRecord {
            id: 2,
            trigger: Trigger::Manual,
            outcome: Outcome::Failed,
            started_at: 900,
            finished_at: Some(960),
            counts: RunCounts {
                checked: 4,
                ..RunCounts::default()
            },
            error: Some("unexpected HTTP 404 from the wiki".into()),
        };
        let interrupted = RunRecord {
            id: 1,
            trigger: Trigger::Scheduled,
            outcome: Outcome::Interrupted,
            started_at: 100,
            finished_at: None,
            counts: RunCounts::default(),
            error: None,
        };

        let html = history(&[failed, interrupted], 1_000).into_string();

        assert!(html.contains(r#"data-trigger="manual""#), "{html}");
        assert!(html.contains(r#"data-outcome="failed""#), "{html}");
        assert!(html.contains(r#"data-checked="4""#), "{html}");
        assert!(html.contains("unexpected HTTP 404 from the wiki"), "{html}");
        assert!(html.contains(r#"data-outcome="interrupted""#), "{html}");
        assert!(html.contains("Run history"), "{html}");
    }

    #[test]
    fn failure_list_says_how_many_more() {
        let html = attention(&[failure("Build:A")], 3, &[], "timed_out", &[], 0).into_string();

        assert!(html.contains("and 2 more"), "{html}");
    }

    #[test]
    fn nothing_to_fetch_renders_no_empty_progress_bar() {
        let html = render(ScraperState::Running {
            trigger: Trigger::Scheduled,
            started_at: 990,
            progress: Some(Progress {
                phase: Phase::Fetching,
                done: 0,
                total: 0,
            }),
        });

        assert!(!html.contains(r#"max="0""#), "{html}");
    }
}
