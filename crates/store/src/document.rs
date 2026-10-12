//! The stored and served shape of a build page, mapped from the parser's output.
//!
//! The `*_name` and `filter_key` functions define the names that are stored, filtered on and
//! shown to agents. Search filters and MCP input enums use the same ones.

use pvxwiki_parser::{
    IssueKind, ParseIssue, ParsedPage, Profession, Rating, Slot, Status, Vetting,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// MediaWiki namespace of archived builds on the PvX wiki.
pub const ARCHIVE_NAMESPACE: i64 = 104;

/// A page as `write_page` receives it.
#[derive(Debug, Clone)]
pub struct NewPage {
    pub title: String,
    pub namespace: i64,
    pub revision: i64,
    /// Unix seconds.
    pub fetched_at: i64,
    pub parsed: ParsedPage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildDocument {
    pub title: String,
    pub namespace: i64,
    pub revision: i64,
    pub vetting: VettingDoc,
    /// Page order; the first entry is the summary before the first heading.
    pub sections: Vec<SectionDoc>,
    pub skill_bars: Vec<BarDoc>,
    /// Problems the parser found; the rest of the page was still parsed.
    pub issues: Vec<IssueDoc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VettingDoc {
    /// "vetted", "stub" or "missing".
    pub kind: String,
    /// Lowercase: great, good, creative, trash, abandoned, testing, trial, archived, or as written.
    pub rating: Option<String>,
    /// Lowercase: meta, provisional, or as written.
    pub status: Option<String>,
    pub gameplay_types: Vec<String>,
    pub archive: Option<ArchiveDoc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveDoc {
    pub date: Option<String>,
    pub category: Option<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SectionDoc {
    pub heading: Option<String>,
    pub level: u8,
    /// Exact wiki source of the section, heading line included.
    pub wikitext: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BarDoc {
    /// Index into `sections`.
    pub section: usize,
    /// Exact `[build ...]...[/build]` source.
    pub raw: String,
    pub name: Option<String>,
    pub variant: Option<VariantDoc>,
    pub primary: Option<String>,
    pub secondary: Option<String>,
    pub professions_raw: Option<String>,
    pub attributes: Vec<AttributeDoc>,
    pub slots: Vec<SlotDoc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VariantDoc {
    pub name: Option<String>,
    pub position: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttributeDoc {
    pub name: Option<String>,
    pub raw_name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SlotDoc {
    /// `raw` is the whole bracketed slot as written, e.g. `[Flail@5]`.
    Skill {
        name: String,
        raw: String,
    },
    Optional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IssueDoc {
    /// Index into `sections`, when the problem has a location.
    pub section: Option<usize>,
    /// Byte offset into the page wikitext, when the problem has a location.
    pub offset: Option<usize>,
    /// "unclosed_build_tag", "unknown_profession", "unknown_attribute", "multiple_vetting_tags",
    /// "timed_out" or "too_deeply_nested".
    pub kind: String,
    /// The offending text, for "unknown_profession" and "unknown_attribute".
    pub detail: Option<String>,
}

impl BuildDocument {
    pub fn from_parsed(title: &str, namespace: i64, revision: i64, page: &ParsedPage) -> Self {
        Self {
            title: title.to_string(),
            namespace,
            revision,
            vetting: vetting(&page.vetting),
            sections: page
                .sections
                .iter()
                .map(|s| SectionDoc {
                    heading: s.heading.clone(),
                    level: s.level,
                    wikitext: s.wikitext.clone(),
                })
                .collect(),
            skill_bars: page
                .skill_bars
                .iter()
                .map(|b| BarDoc {
                    section: b.section,
                    raw: b.raw.clone(),
                    name: b.name.clone(),
                    variant: b.variant.as_ref().map(|v| VariantDoc {
                        name: v.name.clone(),
                        position: v.position.clone(),
                    }),
                    primary: b.primary.map(|p| profession_name(p).to_string()),
                    secondary: b.secondary.map(|p| profession_name(p).to_string()),
                    professions_raw: b.professions_raw.clone(),
                    attributes: b
                        .attributes
                        .iter()
                        .map(|a| AttributeDoc {
                            name: a.name.clone(),
                            raw_name: a.raw_name.clone(),
                            value: a.value.clone(),
                        })
                        .collect(),
                    slots: b
                        .slots
                        .iter()
                        .map(|s| match s {
                            Slot::Skill { name, raw } => SlotDoc::Skill {
                                name: name.clone(),
                                raw: raw.clone(),
                            },
                            Slot::Optional => SlotDoc::Optional,
                        })
                        .collect(),
                })
                .collect(),
            issues: page.issues.iter().map(issue).collect(),
        }
    }

    /// Archived pages live in the Archive namespace or carry `rating=archived`.
    pub fn is_archived(&self) -> bool {
        self.namespace == ARCHIVE_NAMESPACE || self.vetting.rating.as_deref() == Some("archived")
    }
}

fn issue(issue: &ParseIssue) -> IssueDoc {
    let (kind, detail) = match &issue.kind {
        IssueKind::UnclosedBuildTag => ("unclosed_build_tag", None),
        IssueKind::UnknownProfession(text) => ("unknown_profession", Some(text.clone())),
        IssueKind::UnknownAttribute(text) => ("unknown_attribute", Some(text.clone())),
        IssueKind::MultipleVettingTags => ("multiple_vetting_tags", None),
        IssueKind::TimedOut => ("timed_out", None),
        IssueKind::TooDeeplyNested => ("too_deeply_nested", None),
    };
    IssueDoc {
        section: issue.section,
        offset: issue.offset,
        kind: kind.to_string(),
        detail,
    }
}

fn vetting(vetting: &Vetting) -> VettingDoc {
    match vetting {
        Vetting::Vetted(v) => VettingDoc {
            kind: "vetted".into(),
            rating: v.rating.as_ref().map(rating_name),
            status: v.status.as_ref().map(status_name),
            gameplay_types: v.gameplay_types.clone(),
            archive: v.archive.as_ref().map(|a| ArchiveDoc {
                date: a.date.clone(),
                category: a.category.clone(),
                reasons: a.reasons.clone(),
            }),
        },
        Vetting::Stub => empty_vetting("stub"),
        Vetting::Missing => empty_vetting("missing"),
    }
}

fn empty_vetting(kind: &str) -> VettingDoc {
    VettingDoc {
        kind: kind.into(),
        rating: None,
        status: None,
        gameplay_types: Vec::new(),
        archive: None,
    }
}

/// Display name of a profession, as stored in `BarDoc::primary`. Filters match on
/// `filter_key` of it.
pub fn profession_name(profession: Profession) -> &'static str {
    match profession {
        Profession::Warrior => "Warrior",
        Profession::Ranger => "Ranger",
        Profession::Monk => "Monk",
        Profession::Necromancer => "Necromancer",
        Profession::Mesmer => "Mesmer",
        Profession::Elementalist => "Elementalist",
        Profession::Assassin => "Assassin",
        Profession::Ritualist => "Ritualist",
        Profession::Paragon => "Paragon",
        Profession::Dervish => "Dervish",
    }
}

/// Lowercase rating as stored in `pages.rating`; unknown ratings keep their text, lowercased.
pub fn rating_name(rating: &Rating) -> String {
    match rating {
        Rating::Great => "great".into(),
        Rating::Good => "good".into(),
        Rating::Creative => "creative".into(),
        Rating::Trash => "trash".into(),
        Rating::Abandoned => "abandoned".into(),
        Rating::Testing => "testing".into(),
        Rating::Trial => "trial".into(),
        Rating::Archived => "archived".into(),
        Rating::Unknown(other) => other.to_lowercase(),
    }
}

/// Lowercase status as stored in `pages.status`.
pub fn status_name(status: &Status) -> String {
    match status {
        Status::Meta => "meta".into(),
        Status::Provisional => "provisional".into(),
        Status::Unknown(other) => other.to_lowercase(),
    }
}

/// The form of a profession, gameplay type or skill name that the filter tables hold and
/// that search filters compare against.
pub fn filter_key(name: &str) -> String {
    name.to_lowercase()
}
