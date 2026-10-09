//! PvXCode's `[build]` tag, which is not wikitext: `[build <header>][slot]...[/build]`.
//!
//! Grammar from github.com/Wikia/pvxcode, gwbbcode/gwbbcode.inc.php at commit afe42e3:
//! `build_replace` (`\[build ([^\]]*)\](.*?)\[/build\]`, case-insensitive),
//! `gws_build_profession`, `gws_attributes_clean`, `attribute_list_raw`, `skill_name_replace`.

use std::ops::Range;

use crate::lookup;
use crate::{Attribute, IssueKind, Profession, Slot};

const OPEN: &str = "[build ";
const CLOSE: &str = "[/build]";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Found {
    Closed(Range<usize>),
    /// Byte offset of a `[build ` with no header `]` or no `[/build]` after it.
    Unclosed(usize),
}

pub(crate) struct Bar {
    pub name: Option<String>,
    pub professions_raw: Option<String>,
    pub primary: Option<Profession>,
    pub secondary: Option<Profession>,
    pub attributes: Vec<Attribute>,
    pub slots: Vec<Slot>,
    pub issues: Vec<IssueKind>,
}

/// Finds bars in `text`. A bar cut off before the next `[build ` is reported and scanning
/// resumes there; one with no `]` or `[/build]` after it ends the scan.
pub(crate) fn find(text: &str) -> Vec<Found> {
    // ASCII lowercasing keeps every byte offset valid in `text`.
    let lower = text.to_ascii_lowercase();
    let mut found = Vec::new();
    let mut pos = 0;
    while let Some(at) = lower[pos..].find(OPEN) {
        let start = pos + at;
        let Some(header_len) = lower[start..].find(']') else {
            found.push(Found::Unclosed(start));
            break;
        };
        let header_end = start + header_len;
        let Some(close) = lower[header_end..].find(CLOSE) else {
            found.push(Found::Unclosed(start));
            break;
        };
        let close_start = header_end + close;
        // A `[build ` before our `[/build]` means this bar was cut off; the next one owns it.
        if let Some(inner) = lower[start + OPEN.len()..close_start].find(OPEN) {
            found.push(Found::Unclosed(start));
            pos = start + OPEN.len() + inner;
            continue;
        }
        let end = close_start + CLOSE.len();
        found.push(Found::Closed(start..end));
        pos = end;
    }
    found
}

/// Parses one bar. `raw` starts with `[build ` (any case) and ends with `[/build]`.
pub(crate) fn parse(raw: &str) -> Bar {
    let header_end = raw.find(']').unwrap_or(raw.len());
    let header = raw.get(OPEN.len()..header_end).unwrap_or("");
    let body = raw
        .get(header_end + 1..raw.len().saturating_sub(CLOSE.len()))
        .unwrap_or("");

    let mut issues = Vec::new();
    let mut rest = header.to_string();
    let name = take_quoted(&mut rest, "name");
    take_quoted(&mut rest, "desc");
    let professions_raw = take_prof(&mut rest);

    let (primary, secondary) = match professions_raw.as_deref() {
        Some(value) => professions(value, &mut issues),
        None => (None, None),
    };

    let attributes = rest
        .split_whitespace()
        .filter_map(|token| {
            let (raw_name, value) = token.split_once('=')?;
            if value.contains('=') {
                return None;
            }
            let name = lookup::attribute(raw_name);
            if name.is_none() {
                issues.push(IssueKind::UnknownAttribute(raw_name.to_string()));
            }
            Some(Attribute {
                name: name.map(str::to_string),
                raw_name: raw_name.to_string(),
                value: value.to_string(),
            })
        })
        .collect();

    Bar {
        name,
        professions_raw,
        primary,
        secondary,
        attributes,
        slots: slots(body),
        issues,
    }
}

/// Removes `key="value"` (value non-empty) from `header` and returns the value.
fn take_quoted(header: &mut String, key: &str) -> Option<String> {
    let marker = format!("{key}=\"");
    let start = header.find(&marker)?;
    let value_start = start + marker.len();
    let value_len = header[value_start..].find('"')?;
    if value_len == 0 {
        return None;
    }
    let value = header[value_start..value_start + value_len].to_string();
    header.replace_range(start..value_start + value_len + 1, " ");
    Some(value)
}

/// Removes `prof=<value>` from `header` and returns the value (`[^ \]]*` in PvXCode).
fn take_prof(header: &mut String) -> Option<String> {
    let start = header.find("prof=")?;
    let value_start = start + "prof=".len();
    let value_end = header[value_start..]
        .find(char::is_whitespace)
        .map_or(header.len(), |len| value_start + len);
    let value = header[value_start..value_end].to_string();
    header.replace_range(start..value_end, " ");
    (!value.is_empty()).then_some(value)
}

fn professions(
    value: &str,
    issues: &mut Vec<IssueKind>,
) -> (Option<Profession>, Option<Profession>) {
    let (primary_raw, secondary_raw) = match value.split_once('/') {
        Some((p, s)) if !p.is_empty() && !s.is_empty() => (p, Some(s)),
        Some((p, _)) => (p, None),
        None => (value, None),
    };
    let mut resolve = |raw: &str| {
        let found = lookup::profession(raw);
        if found.is_none() && !raw.is_empty() && !raw.eq_ignore_ascii_case("any") {
            issues.push(IssueKind::UnknownProfession(raw.to_string()));
        }
        found
    };
    let primary = resolve(primary_raw);
    let secondary = secondary_raw
        .and_then(&mut resolve)
        .filter(|s| Some(*s) != primary);
    (primary, secondary)
}

/// `[Name]`, `[Name@12]`, `[Name|shown]`, `[[Name]` (no icon), `[Optional]`. Text between
/// slots is ignored.
fn slots(body: &str) -> Vec<Slot> {
    let mut slots = Vec::new();
    let mut rest = body;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        let raw = &rest[open..open + 1 + close + 1];
        let inner = after[..close].trim_start_matches('[').trim();
        rest = &after[close + 1..];
        // PvXCode's abbreviation table maps these three to an empty slot.
        if ["optional", "opt", "empty"]
            .iter()
            .any(|open| inner.eq_ignore_ascii_case(open))
        {
            slots.push(Slot::Optional);
            continue;
        }
        let name = inner.split(['@', '|', ':']).next().unwrap_or("").trim();
        if !name.is_empty() {
            slots.push(Slot::Skill {
                name: name.to_string(),
                raw: raw.to_string(),
            });
        }
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Profession::*;

    fn closed(text: &str) -> Vec<&str> {
        find(text)
            .into_iter()
            .map(|f| match f {
                Found::Closed(r) => &text[r],
                Found::Unclosed(at) => panic!("unclosed at {at}"),
            })
            .collect()
    }

    #[test]
    fn find_returns_each_bar_case_insensitively() {
        let text = "x [build prof=A/E][Dash][/build] y [BUILD prof=W][Frenzy][/Build] z";
        assert_eq!(
            closed(text),
            [
                "[build prof=A/E][Dash][/build]",
                "[BUILD prof=W][Frenzy][/Build]"
            ]
        );
    }

    #[test]
    fn find_ignores_text_without_bars() {
        assert!(find("[[Link]] [build=OwBj] [buildx]").is_empty());
    }

    #[test]
    fn find_reports_unclosed_bars() {
        assert_eq!(find("ok [build prof=A/E sha=12"), vec![Found::Unclosed(3)]);
        assert_eq!(find("[build prof=A/E][Dash]"), vec![Found::Unclosed(0)]);
        assert_eq!(find("[build x[/build]"), vec![Found::Unclosed(0)]);
    }

    #[test]
    fn find_reports_a_cut_off_bar_and_still_finds_the_next_one() {
        let text = "[build prof=A/E][Dash]\n[build prof=W][Frenzy][/build]";
        assert_eq!(
            find(text),
            vec![Found::Unclosed(0), Found::Closed(23..text.len())]
        );
    }

    #[test]
    fn find_keeps_offsets_on_char_boundaries_after_non_ascii() {
        let text = "Über ’ [build prof=A][Dash][/build]";
        assert_eq!(closed(text), ["[build prof=A][Dash][/build]"]);
    }

    #[test]
    fn parses_the_vaettir_header_and_slots() {
        let bar = parse(
            "[build prof=assas/mesmer shadow=12+1+3 Domination=11 Inspiration=6 deadly=2+3 ]\
             [deadly paradox][shadow form][shroud of distress][Optional][Optional]\
             [Arcane Echo][Wastrel's Demise][Channeling][/build]",
        );
        assert_eq!(bar.professions_raw.as_deref(), Some("assas/mesmer"));
        assert_eq!((bar.primary, bar.secondary), (Some(Assassin), Some(Mesmer)));
        let attrs: Vec<_> = bar
            .attributes
            .iter()
            .map(|a| (a.name.as_deref(), a.raw_name.as_str(), a.value.as_str()))
            .collect();
        assert_eq!(
            attrs,
            [
                (Some("Shadow Arts"), "shadow", "12+1+3"),
                (Some("Domination Magic"), "Domination", "11"),
                (Some("Inspiration Magic"), "Inspiration", "6"),
                (Some("Deadly Arts"), "deadly", "2+3"),
            ]
        );
        assert_eq!(bar.slots.len(), 8);
        assert_eq!(
            bar.slots[0],
            Slot::Skill {
                name: "deadly paradox".into(),
                raw: "[deadly paradox]".into()
            }
        );
        assert_eq!(bar.slots[3], Slot::Optional);
        assert!(bar.issues.is_empty());
    }

    #[test]
    fn reads_quoted_name_and_skips_desc() {
        let bar = parse(
            "[build name=\"Sanctum Cay Runner (NM)\" prof=A/P desc=\"fast run\" sha=12][Dash][/build]",
        );
        assert_eq!(bar.name.as_deref(), Some("Sanctum Cay Runner (NM)"));
        assert_eq!(
            (bar.primary, bar.secondary),
            (Some(Assassin), Some(Paragon))
        );
        assert_eq!(bar.attributes.len(), 1);
        assert!(bar.issues.is_empty());
    }

    #[test]
    fn any_missing_and_duplicate_secondaries_are_none() {
        assert_eq!(parse("[build prof=A/any][/build]").secondary, None);
        assert_eq!(parse("[build prof=A][/build]").secondary, None);
        assert_eq!(parse("[build prof=A/][/build]").secondary, None);
        assert_eq!(parse("[build prof=A/Assassin][/build]").secondary, None);
        assert!(parse("[build prof=A/any][/build]").issues.is_empty());
    }

    #[test]
    fn missing_prof_leaves_professions_unset() {
        let bar = parse("[build sha=12][Dash][/build]");
        assert_eq!(
            (bar.primary, bar.secondary, bar.professions_raw),
            (None, None, None)
        );
    }

    #[test]
    fn unknown_names_are_kept_and_reported() {
        let bar = parse("[build prof=Q/E foo=3 nosave][/build]");
        assert_eq!(bar.primary, None);
        assert_eq!(bar.secondary, Some(Elementalist));
        assert_eq!(
            bar.attributes,
            [Attribute {
                name: None,
                raw_name: "foo".into(),
                value: "3".into()
            }]
        );
        assert_eq!(
            bar.issues,
            [
                IssueKind::UnknownProfession("Q".into()),
                IssueKind::UnknownAttribute("foo".into())
            ]
        );
    }

    #[test]
    fn opt_and_empty_are_open_slots_like_optional() {
        let bar = parse("[build prof=A][opt][Empty][OPTIONAL][Dash][/build]");
        assert_eq!(
            bar.slots[..3],
            [Slot::Optional, Slot::Optional, Slot::Optional]
        );
    }

    #[test]
    fn slot_syntax_variants() {
        let bar = parse(
            "[build prof=E][Conjure Frost@10][Shock|a knockdown][[Aura of Restoration]\n [optional] [Signet: x][/build]",
        );
        let names: Vec<_> = bar
            .slots
            .iter()
            .map(|s| match s {
                Slot::Skill { name, .. } => name.as_str(),
                Slot::Optional => "<optional>",
            })
            .collect();
        assert_eq!(
            names,
            [
                "Conjure Frost",
                "Shock",
                "Aura of Restoration",
                "<optional>",
                "Signet"
            ]
        );
        assert_eq!(
            bar.slots[2],
            Slot::Skill {
                name: "Aura of Restoration".into(),
                raw: "[[Aura of Restoration]".into()
            }
        );
    }
}
