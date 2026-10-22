//! The admin page as maud markup. Pure functions of the data they are given.

use maud::{DOCTYPE, Markup, html};
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
                link rel="stylesheet" href="/assets/pico.min.css";
                script src="/assets/htmx.min.js" {}
            }
            body {
                main.container {
                    h1 { "pvxwiki-mcp admin" }
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
/// htmx trigger for the history refresh; a run takes minutes, so this can be slow.
pub(crate) const HISTORY_POLL: &str = "every 10s";

/// The scraper's state, progress, next run, index size and the run button. `now` in unix seconds.
pub(crate) fn status_view(status: &RunnerStatus, counts: &[(i64, u64)], now: i64) -> Markup {
    let (name, refused) = match &status.state {
        ScraperState::Off => ("off", Some(RunRefused::Off)),
        ScraperState::Idle => ("idle", None),
        ScraperState::Running { .. } => ("running", Some(RunRefused::Running)),
        ScraperState::Blocked { .. } => ("blocked", Some(RunRefused::Blocked)),
    };
    html! {
        article #status data-state=(name) hx-get="/status" hx-trigger=(PROGRESS_POLL) hx-swap="outerHTML" {
            header { strong { "Scraper: " (name) } }
            @match &status.state {
                ScraperState::Off => p { "The scraper is off because PVXWIKI_CONTACT is not set. The index is not updated." },
                ScraperState::Idle => p { "Waiting for the next run." },
                ScraperState::Running { trigger, started_at, progress } => {
                    p { "A " (trigger.as_str()) " run started " (ago(now - started_at)) "." }
                    @match progress {
                        Some(p) => {
                            progress value=(p.done) max=(p.total) {}
                            p { small {
                                @match p.phase {
                                    Phase::Listing => "Listing namespaces: ",
                                    Phase::Fetching => "Fetching changed pages: ",
                                }
                                (p.done) " of " (p.total)
                            } }
                        }
                        None => progress {},
                    }
                }
                ScraperState::Blocked { since } => p {
                    "The wiki blocked the scraper " (ago(now - since)) ". No runs until the service restarts. "
                    "Check the logs and the wiki's terms before restarting."
                },
            }
            @if let Some(next) = status.next_run_at {
                p { "Next scheduled run in " (span(next - now)) "." }
            }
            p {
                "Index: "
                @for (i, (namespace, pages)) in counts.iter().enumerate() {
                    @if i > 0 { ", " }
                    (pages) " " (namespace_name(*namespace))
                }
                @if counts.is_empty() { "empty" }
                "."
            }
            footer {
                button hx-post="/runs" hx-target="#status" hx-swap="outerHTML" disabled[refused.is_some()] {
                    "Run now"
                }
                @if let Some(reason) = refused {
                    " " small { (reason) }
                }
            }
        }
    }
}

/// Newest runs first. `now` in unix seconds.
pub(crate) fn history(runs: &[RunRecord], now: i64) -> Markup {
    html! {
        section #history hx-get="/history" hx-trigger=(HISTORY_POLL) hx-swap="outerHTML" {
            h2 { "Run history" }
            @if runs.is_empty() {
                p { "No runs yet." }
            } @else {
                figure {
                    table {
                        thead { tr {
                            th { "Started" } th { "Took" } th { "Trigger" } th { "Outcome" }
                            th { "Checked" } th { "Updated" } th { "Kept" } th { "Removed" }
                        } }
                        tbody {
                            @for run in runs {
                                tr data-trigger=(run.trigger.as_str()) data-outcome=(run.outcome.as_str())
                                    data-checked=(run.counts.checked) {
                                    td { (ago(now - run.started_at)) }
                                    td { @match run.finished_at {
                                        Some(end) => (span(end - run.started_at)),
                                        None => "-",
                                    } }
                                    td { (run.trigger.as_str()) }
                                    td {
                                        (run.outcome.as_str())
                                        @if let Some(error) = &run.error { br; small { (error) } }
                                    }
                                    td { (run.counts.checked) }
                                    td { (run.counts.updated) }
                                    td { (run.counts.kept) }
                                    td { (run.counts.removed) }
                                }
                            }
                        }
                    }
                }
            }
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
    issue_counts: &[(String, u64)],
    kind: &str,
    pages: &[IssuePage],
    total: u64,
) -> Markup {
    html! {
        section {
            h2 { "Pages needing attention" }
            h3 { "Newest revision failed to parse" }
            @if failures.is_empty() {
                p { "None. Every stored page is at its newest revision." }
            } @else {
                p { "The stored copy is older than the wiki's. Fix the parser for the failed revision." }
                table {
                    thead { tr { th { "Page" } th { "Stored" } th { "Failed" } } }
                    tbody {
                        @for f in failures {
                            tr {
                                td { a href=(wiki_url(&f.title)) { (f.title) } }
                                td { a href=(revision_url(f.stored_revision)) { (f.stored_revision) } }
                                td { a href=(revision_url(f.failed_revision)) { (f.failed_revision) } }
                            }
                        }
                    }
                }
            }
            h3 { "Stored with parse issues" }
            @if issue_counts.is_empty() {
                p { "None." }
            } @else {
                nav {
                    ul {
                        @for (name, count) in issue_counts {
                            li {
                                a href={ "?kind=" (name) } hx-get={ "/issues?kind=" (name) }
                                    hx-target="#issues" hx-swap="outerHTML"
                                    aria-current=[(name == kind).then_some("true")] {
                                    (name) " (" (count) ")"
                                }
                            }
                        }
                    }
                }
                (issues(kind, pages, total))
            }
        }
    }
}

/// The capped list of pages with issues of `kind`.
pub(crate) fn issues(kind: &str, pages: &[IssuePage], total: u64) -> Markup {
    let more = total.saturating_sub(u64::try_from(pages.len()).unwrap_or(u64::MAX));
    html! {
        div #issues data-kind=(kind) {
            ul {
                @for page in pages {
                    li {
                        a href=(wiki_url(&page.title)) { (page.title) }
                        " at "
                        a href=(revision_url(page.revision)) { (page.revision) }
                    }
                }
            }
            @if more > 0 {
                p { small { "and " (more) " more" } }
            }
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
        let html = attention(&[failure("Build:<script>")], &[], "timed_out", &[], 0).into_string();

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
        status_view(&status, &[(100, 3), (104, 1)], 1_000).into_string()
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
        assert!(idle.contains(r#"hx-post="/runs""#), "{idle}");
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
}
