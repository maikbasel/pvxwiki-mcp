//! Parses one PvX wiki build page from wikitext into sections, vetting data and skill bars.

mod build_tag;
mod lookup;
mod sections;
mod vetting;
mod wikitext;

use std::time::Duration;

use parse_wiki_text_2::ParseError;

/// Upper bound for the wikitext tree parse of one page. Real pages take well under 10 ms.
const PARSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Longest piece of page text put into a log event.
const SNIPPET_MAX_CHARS: usize = 80;

/// Deepest markup nesting parsed. Real pages stay in single digits; the tree recurses per level.
const MAX_NESTING: usize = 256;

/// Parses one build page. Never fails: problems are listed in `ParsedPage::issues`.
#[tracing::instrument(level = "debug", skip_all, fields(bytes = wikitext.len()))]
pub fn parse(wikitext: &str) -> ParsedPage {
    if let Some(offset) = wikitext::first_too_deep(wikitext, MAX_NESTING) {
        let page = unparsed(wikitext, offset);
        log_outcome(wikitext, &page);
        return page;
    }
    let (output, timed_out) =
        match wikitext::configuration().parse_with_timeout(wikitext, PARSE_TIMEOUT) {
            Ok(output) => (output, false),
            Err(ParseError::TimedOut { output, .. }) => (output, true),
        };
    for warning in &output.warnings {
        tracing::trace!(
            start = warning.start,
            end = warning.end,
            message = warning.message.message(),
            "wikitext warning"
        );
    }
    let mut issues = Vec::new();
    if timed_out {
        issues.push(ParseIssue {
            section: None,
            offset: None,
            kind: IssueKind::TimedOut,
        });
    }
    let (sections, starts) = sections::split(wikitext, &output.nodes);
    let collected = wikitext::collect(&output.nodes);
    let (vetting, duplicate) = vetting::extract(wikitext, &collected.templates);
    if let Some(offset) = duplicate {
        issues.push(ParseIssue {
            section: Some(sections::index_at(&starts, offset)),
            offset: Some(offset),
            kind: IssueKind::MultipleVettingTags,
        });
    }

    let mut skill_bars = Vec::new();
    for &(start, end) in &collected.pvxbig {
        let Some(text) = wikitext.get(start..end) else {
            continue;
        };
        // PvXCode sees the tag content after MediaWiki dropped comments and nowiki text.
        let visible = wikitext::masked(wikitext, start, end, &collected.hidden);
        for found in build_tag::find(&visible) {
            match found {
                build_tag::Found::Closed(range) => {
                    let offset = start + range.start;
                    let section = sections::index_at(&starts, offset);
                    let raw = text.get(range.clone()).unwrap_or("");
                    let bar = build_tag::parse(visible.get(range).unwrap_or(""));
                    issues.extend(bar.issues.into_iter().map(|kind| ParseIssue {
                        section: Some(section),
                        offset: Some(offset),
                        kind,
                    }));
                    skill_bars.push(SkillBar {
                        section,
                        raw: raw.to_string(),
                        name: bar.name,
                        variant: variant_at(wikitext, &collected.templates, offset),
                        primary: bar.primary,
                        secondary: bar.secondary,
                        professions_raw: bar.professions_raw,
                        attributes: bar.attributes,
                        slots: bar.slots,
                    });
                }
                build_tag::Found::Unclosed(at) => issues.push(ParseIssue {
                    section: Some(sections::index_at(&starts, start + at)),
                    offset: Some(start + at),
                    kind: IssueKind::UnclosedBuildTag,
                }),
            }
        }
    }
    let page = ParsedPage {
        vetting,
        sections,
        skill_bars,
        issues,
    };
    log_outcome(wikitext, &page);
    page
}

/// The whole page as one raw section, for input too deeply nested to build a tree from.
fn unparsed(wikitext: &str, offset: usize) -> ParsedPage {
    ParsedPage {
        vetting: Vetting::Missing,
        sections: vec![Section {
            heading: None,
            level: 0,
            wikitext: wikitext.to_string(),
            text: wikitext::collapse(wikitext),
        }],
        skill_bars: Vec::new(),
        issues: vec![ParseIssue {
            section: Some(0),
            offset: Some(offset),
            kind: IssueKind::TooDeeplyNested,
        }],
    }
}

/// The `{{Variantbar}}` (Template:Variantbar, rev 1285529) whose source contains `offset`.
fn variant_at(src: &str, templates: &[wikitext::TemplateRef], offset: usize) -> Option<Variant> {
    templates
        .iter()
        .find(|t| t.name == "variantbar" && t.start < offset && offset < t.end)
        .map(|t| Variant {
            name: wikitext::named(src, t.parameters, "name"),
            position: wikitext::named(src, t.parameters, "position"),
        })
}

/// One `debug` event per issue (issues are returned as data; the caller decides on `warn`),
/// then one summary.
fn log_outcome(src: &str, page: &ParsedPage) {
    for issue in &page.issues {
        tracing::debug!(
            section = issue.section,
            offset = issue.offset,
            kind = ?issue.kind,
            snippet = ?issue.offset.map(|offset| snippet(src, offset)),
            "page parse issue"
        );
    }
    tracing::debug!(
        sections = page.sections.len(),
        skill_bars = page.skill_bars.len(),
        issues = page.issues.len(),
        "page parsed"
    );
}

/// Up to `SNIPPET_MAX_CHARS` characters of `src` from byte `offset`; empty if `offset` is not
/// a char boundary inside `src`.
fn snippet(src: &str, offset: usize) -> &str {
    let Some(rest) = src.get(offset..) else {
        return "";
    };
    let end = rest
        .char_indices()
        .nth(SNIPPET_MAX_CHARS)
        .map_or(rest.len(), |(i, _)| i);
    &rest[..end]
}

/// One parsed build page. Title, revision ID and categories come from the API, not from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPage {
    pub vetting: Vetting,
    /// Page order. Index 0 is the summary before the first heading (heading `None`, level 0),
    /// present even when empty. Concatenating every `wikitext` gives back the input exactly.
    pub sections: Vec<Section>,
    /// Every `[build]` tag inside `<pvxbig>`, in page order.
    pub skill_bars: Vec<SkillBar>,
    pub issues: Vec<ParseIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vetting {
    Vetted(Vetted),
    /// `{{build-stub}}`: the build is still being written.
    Stub,
    /// No vetting tag on the page.
    Missing,
}

/// From `{{Real-Vetting}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vetted {
    pub rating: Option<Rating>,
    pub status: Option<Status>,
    /// Positional arguments as written, empty ones skipped: "farming", "RA", "PvE team".
    pub gameplay_types: Vec<String>,
    /// Set when the tag has any of `date`, `category`, `reasonN`.
    pub archive: Option<Archive>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rating {
    Great,
    Good,
    Creative,
    Trash,
    Abandoned,
    Testing,
    Trial,
    Archived,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Meta,
    Provisional,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub date: Option<String>,
    pub category: Option<String>,
    /// `reason1`..`reasonN` in number order.
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Heading text, trimmed. `None` for the summary.
    pub heading: Option<String>,
    /// 0 for the summary, else the heading level (2 for `==`).
    pub level: u8,
    /// Exact source slice, heading line included.
    pub wikitext: String,
    /// Markup stripped and whitespace collapsed. For the search index only.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillBar {
    /// Index into `ParsedPage::sections`.
    pub section: usize,
    /// Exact `[build ...]...[/build]` source.
    pub raw: String,
    /// `name="..."` from the tag header.
    pub name: Option<String>,
    /// Set when the bar is inside `{{Variantbar}}`.
    pub variant: Option<Variant>,
    pub primary: Option<Profession>,
    /// `None` for "any", for a missing secondary, and when equal to the primary.
    pub secondary: Option<Profession>,
    /// The `prof=` value as written.
    pub professions_raw: Option<String>,
    pub attributes: Vec<Attribute>,
    pub slots: Vec<Slot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub name: Option<String>,
    /// Team slot this variant replaces (`position=`), as written.
    pub position: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Profession {
    Warrior,
    Ranger,
    Monk,
    Necromancer,
    Mesmer,
    Elementalist,
    Assassin,
    Ritualist,
    Paragon,
    Dervish,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// Canonical name as the wiki renders it ("Shadow Arts"); `None` if the name matched nothing.
    pub name: Option<String>,
    /// Name as written ("sha").
    pub raw_name: String,
    /// Value as written ("12+1+3").
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// `name` is the text before `@`, `|` or `:`, trimmed, page casing. `raw` is the whole
    /// bracketed slot. Abbreviations are not expanded.
    Skill { name: String, raw: String },
    /// `[Optional]`: an open slot.
    Optional,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIssue {
    /// Index into `ParsedPage::sections`, when the problem has a location.
    pub section: Option<usize>,
    /// Byte offset into the page where the problem starts, when it has a location.
    pub offset: Option<usize>,
    pub kind: IssueKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueKind {
    /// A `[build ` inside `<pvxbig>` without a closing `]` or `[/build]`.
    UnclosedBuildTag,
    /// A `prof=` part that matched no profession and is not "any".
    UnknownProfession(String),
    /// An `attribute=value` pair whose name matched no attribute.
    UnknownAttribute(String),
    /// More than one `{{Real-Vetting}}` / `{{build-stub}}`; the first one is used.
    MultipleVettingTags,
    /// The wikitext parser hit `PARSE_TIMEOUT`; output is built from the partial tree.
    TimedOut,
    /// Markup nests deeper than `MAX_NESTING`; the page is kept as one raw section, unparsed.
    TooDeeplyNested,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_is_capped_and_stays_on_char_boundaries() {
        let src = format!("ab{}", "ü".repeat(200));
        let s = snippet(&src, 2);
        assert_eq!(s.chars().count(), SNIPPET_MAX_CHARS);
        assert!(s.starts_with('ü'));
        assert_eq!(snippet("abc", 99), "");
        assert_eq!(snippet("aü", 2), "");
    }

    #[test]
    fn bars_inside_pvxbig_are_parsed_with_their_section() {
        let page = parse(
            "Intro\n== Attributes and Skills ==\n<pvxbig>\n[build prof=A/E sha=12+1+3][Dash][Optional][/build]\n</pvxbig>\n",
        );
        assert_eq!(page.skill_bars.len(), 1);
        let bar = &page.skill_bars[0];
        assert_eq!(bar.section, 1);
        assert_eq!(
            bar.raw,
            "[build prof=A/E sha=12+1+3][Dash][Optional][/build]"
        );
        assert_eq!(
            (bar.primary, bar.secondary),
            (Some(Profession::Assassin), Some(Profession::Elementalist))
        );
        assert_eq!(bar.variant, None);
        assert_eq!(bar.slots.len(), 2);
    }

    #[test]
    fn bars_outside_pvxbig_are_ignored() {
        assert!(
            parse("[build prof=A/E][Dash][/build]")
                .skill_bars
                .is_empty()
        );
    }

    #[test]
    fn variantbar_bars_carry_name_and_position() {
        let page = parse(
            "<pvxbig>\n{{Variantbar|name=Fast run|position=2|template=\n[build prof=A/E sha=12][Dash][/build]\n*note\n}}\n</pvxbig>",
        );
        assert_eq!(
            page.skill_bars[0].variant,
            Some(Variant {
                name: Some("Fast run".into()),
                position: Some("2".into())
            })
        );
    }

    #[test]
    fn unclosed_build_tag_is_an_issue_and_the_rest_still_parses() {
        let page = parse(
            "== A ==\n<pvxbig>[build prof=A/E][Dash][/build]</pvxbig>\n== B ==\n<pvxbig>[build prof=W/E][Frenzy]</pvxbig>\n== C ==\ntext",
        );
        assert_eq!(page.skill_bars.len(), 1);
        assert_eq!(page.sections.len(), 4);
        let issue = page
            .issues
            .iter()
            .find(|i| i.kind == IssueKind::UnclosedBuildTag)
            .unwrap();
        assert_eq!(issue.section, Some(2));
        assert!(text_at(&page, issue.offset).starts_with("[build prof=W/E]"));
    }

    #[test]
    fn cut_off_bar_next_to_another_bar_is_reported_and_the_next_bar_survives() {
        let src = "<pvxbig>\n[build prof=A/E sha=12][Dash][Shadow Form]\n[build prof=W/E str=12][Frenzy][/build]\n</pvxbig>";
        let page = parse(src);
        assert_eq!(page.skill_bars.len(), 1);
        assert_eq!(page.skill_bars[0].primary, Some(Profession::Warrior));
        let issue = page
            .issues
            .iter()
            .find(|i| i.kind == IssueKind::UnclosedBuildTag)
            .unwrap();
        assert!(src[issue.offset.unwrap()..].starts_with("[build prof=A/E"));
    }

    #[test]
    fn commented_out_and_nowiki_bars_are_not_skill_bars() {
        assert!(
            parse("<pvxbig><!-- [build prof=A][Dash][/build] --></pvxbig>")
                .skill_bars
                .is_empty()
        );
        assert!(
            parse("<pvxbig><nowiki>[build prof=A][Dash][/build]</nowiki></pvxbig>")
                .skill_bars
                .is_empty()
        );
    }

    #[test]
    fn commented_out_slots_and_attributes_are_ignored_but_raw_is_exact() {
        let src =
            "<pvxbig>[build prof=A/E <!-- sha=12 -->][Dash]<!-- [Old Skill] -->[/build]</pvxbig>";
        let page = parse(src);
        let bar = &page.skill_bars[0];
        assert!(bar.attributes.is_empty());
        assert_eq!(
            bar.slots,
            [Slot::Skill {
                name: "Dash".into(),
                raw: "[Dash]".into()
            }]
        );
        assert_eq!(
            bar.raw,
            "[build prof=A/E <!-- sha=12 -->][Dash]<!-- [Old Skill] -->[/build]"
        );
    }

    #[test]
    fn bar_issues_point_at_their_section_and_bar() {
        let src = "== A ==\n<pvxbig>[build prof=Q/E][Dash][/build]</pvxbig>";
        let page = parse(src);
        let issue = page
            .issues
            .iter()
            .find(|i| i.kind == IssueKind::UnknownProfession("Q".into()))
            .unwrap();
        assert_eq!(issue.section, Some(1));
        assert!(src[issue.offset.unwrap()..].starts_with("[build prof=Q/E]"));
    }

    /// The page text at an issue offset, rebuilt from the lossless sections.
    fn text_at(page: &ParsedPage, offset: Option<usize>) -> String {
        let all: String = page.sections.iter().map(|s| s.wikitext.as_str()).collect();
        all[offset.unwrap()..].to_string()
    }
}
