# Parser crate and workspace layout

Status: draft for review. Date: 2026-10-09.

## Goal

A pure function that turns one PvX build page's wikitext into a typed `ParsedPage`. The agent reads
prose itself; the parser extracts only what search and filtering need and keeps every byte of the
page. Unexpected content (extra sections, odd headings, user-added markup) is kept as section text,
never dropped and never guessed at.

Template codes are out of scope: not extracted, not generated.

## Workspace layout

The repository becomes a Cargo workspace. A crate is created when its work starts; this spec
creates `parser` and moves the bin.

```
Cargo.toml                 # [workspace]: members, [workspace.package], [workspace.dependencies], [workspace.lints]
crates/
  parser/                  # pvxwiki-parser (lib). Deps: parse-wiki-text-2, tracing. No I/O.
  pvxwiki-mcp/             # the bin. Today's src/main.rs moves here unchanged.
  scraper/, store/, server/  # later, each in its own spec
```

Dependency direction: `server` never depends on `scraper`; `parser` has no I/O dependencies. Edition,
`rust-version`, licence (MIT) and lints are set once at the root and inherited with
`workspace = true`.

## Sources for the page grammar

- `PvXwiki:Style and formatting` (rev 1293733): page skeleton, section order, team build layout,
  gameplay type codes.
- `Template:Real-Vetting` (rev 1299291): rating and status values, archive parameters.
- `Template:Variantbar` (rev 1285529): variant bars, `name`, `position`, `prof`, `template`.
- PvXCode extension, `github.com/Wikia/pvxcode` `gwbbcode/gwbbcode.inc.php` (commit afe42e3):
  `[build]` tag, skill slot syntax, profession and attribute lookup tables.
- 50 recorded pages from the API probe (namespace 100, first 50 titles).

## Page grammar the parser relies on

- **Vetting tag.** At most one per page: `{{Real-Vetting|rating=..|status=..|type|type|..}}` or
  `{{build-stub}}`. The template lowercases rating and status. Ratings: great, good, creative,
  trash, abandoned, testing, trial, archived. Statuses: meta, provisional; often empty. Positional
  arguments are gameplay types (AB, FA, JQ, GvG, HA, RA, PvP team, general, farming, running, hero,
  SC, PvE team); pages also use others such as `quest`. Archived pages add `date`, `category`,
  `reason1`..`reason9`.
- **Sections.** `==`-style headings. Text before the first heading is the summary. Heading
  spelling varies ("Attributes and Skills" / "Skills and Attributes", "Variant" / "Variants");
  the parser does not normalise headings.
- **Skill bars.** `[build <header>][slot]...[/build]`, interpreted only inside `<pvxbig>`. Header:
  `prof=P/S`, optional `name="..."`, `desc="..."`, and `attribute=value` pairs (value like
  `12+1+3`). Slots: `[Name]`, `[Name@12]`, `[Name|shown text]`, `[[Name]` (no icon), `[Optional]`.
- **Variants.** `{{Variantbar|name=..|position=N|prof=..|template=<[build] tag and notes>}}` inside
  `<pvxbig>`. `position` names the team slot it replaces.
- **Professions and attributes** resolve the way PvXCode does: lowercase, strip `_` (attributes)
  and match the first table entry the input is a prefix of. `rt` is special-cased to Ritualist.
  Table order matters (`earth` resolves to Earth Magic, `earthp` to Earth Prayers). `any` and an
  unmatched secondary mean no secondary profession.

## Public API

```rust
pub fn parse(wikitext: &str) -> ParsedPage   // infallible; problems go into `issues`

pub struct ParsedPage {
    pub vetting: Vetting,
    pub sections: Vec<Section>,     // page order; index 0 is the summary (heading None)
    pub skill_bars: Vec<SkillBar>,  // page order
    pub issues: Vec<ParseIssue>,
}

pub enum Vetting { Vetted(Vetted), Stub, Missing }

pub struct Vetted {
    pub rating: Option<Rating>,
    pub status: Option<Status>,
    pub gameplay_types: Vec<String>,    // as written, empty args skipped
    pub archive: Option<Archive>,       // present when any of date/category/reasonN is set
}
pub enum Rating { Great, Good, Creative, Trash, Abandoned, Testing, Trial, Archived, Unknown(String) }
pub enum Status { Meta, Provisional, Unknown(String) }   // empty `status=` is None
pub struct Archive { pub date: Option<String>, pub category: Option<String>, pub reasons: Vec<String> }

pub struct Section {
    pub heading: Option<String>,    // trimmed heading text; None for the summary
    pub level: u8,                  // 0 for the summary, else 2..=6
    pub wikitext: String,           // exact source slice, heading line included
    pub text: String,               // markup stripped, for the search index
}

pub struct SkillBar {
    pub section: usize,             // index into `sections`
    pub raw: String,                // exact "[build ...]...[/build]"
    pub name: Option<String>,
    pub variant: Option<Variant>,
    pub primary: Option<Profession>,
    pub secondary: Option<Profession>,
    pub professions_raw: Option<String>,   // the `prof=` value as written
    pub attributes: Vec<Attribute>,
    pub slots: Vec<Slot>,
}
pub struct Variant { pub name: Option<String>, pub position: Option<String> }
pub enum Profession { Warrior, Ranger, Monk, Necromancer, Mesmer, Elementalist, Assassin, Ritualist, Paragon, Dervish }
pub struct Attribute { pub name: Option<String>, pub raw_name: String, pub value: String }  // name: canonical, None if unmatched
pub enum Slot { Skill { name: String, raw: String }, Optional }   // name: before `@`/`|`, page casing

pub struct ParseIssue { pub section: Option<usize>, pub offset: Option<usize>, pub kind: IssueKind }  // offset: byte in page
pub enum IssueKind { UnclosedBuildTag, UnknownProfession(String), UnknownAttribute(String),
                     MultipleVettingTags, TimedOut, TooDeeplyNested }
```

Strings in the output are owned so `ParsedPage` outlives the input. The exact enum variants of
`IssueKind` may grow during implementation; adding one is not a breaking change for callers that
match with a wildcard.

## What the parser does not do

- Title, revision ID, namespace and categories come from the API, not the wikitext (categories
  are mostly set by templates such as `{{FarmCat}}`). The scraper requests them in the same batched
  call.
- No extraction from prose. Skills mentioned in Usage or Optionals text (`{{skill pvxicon|...}}`)
  stay in `Section.text`.
- No skill abbreviation expansion (that needs PvXCode's skill database; out of scope).
- No heading normalisation. Consumers that want "the equipment section" match headings
  case-insensitively.

## Implementation notes

- `parse-wiki-text-2` 0.2.0 provides headings, templates with parameters, and tags with byte
  positions. Its default configuration is Wikipedia's. The parser builds a `Configuration` from
  the wiki's own `siteinfo` (`extensiontags`, `protocols`, `magicwords`, fetched 2026-10-09):
  114 extension tags including `<pvxbig>`, whose content is still parsed into nodes. The lists are
  consts with that source in a comment.
  Checked against all 50 probe pages with the full tag list: no parse errors, all 109 `<pvxbig>` blocks found, no nested headings, every `[build]` inside
  `<pvxbig>`, every `{{Variantbar}}` found.
- The crate parses with a wall-clock timeout. `parse` uses a named `PARSE_TIMEOUT` const; on
  timeout it uses the partial tree the crate returns and records `IssueKind::TimedOut`.
- Template names compare after trimming, lowercasing and `_` to space. The stub tag matches
  `build-stub` exactly (`{{section-stub}}` also exists and is not a vetting tag).
- The `[build]` grammar is hand-written: it is not wikitext and no crate knows it.
- Bars are found in the `<pvxbig>` source with comments and `<nowiki>` content blanked to spaces
  (same byte length), because PvXCode sees the content after MediaWiki removed them. `raw` keeps
  the original bytes.
- A bar cut off before the next `[build ` is `UnclosedBuildTag`; scanning resumes at the next bar.
- Before building the tree, a linear scan measures markup nesting. Past `MAX_NESTING` (256) the
  page is returned as one raw summary section with `TooDeeplyNested`: the tree and its `Drop`
  recurse per level, and a vandalised page could otherwise overflow the stack.
- Profession and attribute tables are consts copied from PvXCode, with the source file and commit
  in a comment.
- `Section.text` strips markup by walking the node tree: text nodes, link labels, template names
  dropped except `{{skill pvxicon|Name}}`-style skill templates, which contribute the skill name.

## Logging

Per `.claude/rules/logging.md`. The parser emits `tracing` events and never installs a subscriber;
its output does not depend on logging.

- `parse` is `#[instrument(level = "debug", skip_all, fields(bytes))]`. The caller's page span
  (scraper) supplies title and revision ID; the parser does not know them.
- On return: one `debug!` with counts (sections, skill bars, issues).
- Each `ParseIssue`: one `debug!` with section, byte offset, kind, and a snippet of the page at
  the offset (at most `SNIPPET_MAX_CHARS`, logged with `?`). Issues are returned as data, so the
  parser never logs them at `warn`; the scraper logs one `warn` per page with problems, in its own
  spec.
- Each warning from `parse-wiki-text-2` (unknown tag, odd nesting): one `trace!` with byte range
  and message. 198 such warnings across the 50 probe pages, mostly `<big>`, so not higher.
- Timeout: `IssueKind::TimedOut` plus its `debug!` like any issue.

## Error handling

`parse` never panics and never fails as a whole. Each problem is a `ParseIssue` tied to a section
where possible; the rest of the page still parses. Missing values are `None`, never defaulted.

## Testing

Fixtures in `crates/parser/tests/fixtures/pages/`, one raw `.wikitext` file per page, named
`<title>__<revid>.wikitext` with `/` and `:` replaced by `-`. A `README.md` there states the
licence (CC BY-NC-SA 2.5), credits PvX wiki contributors, and lists each file's source URL and
revision ID.

| Shape | Page |
|---|---|
| Single build, Optionals, unknown section | `Build:A/Me Vaettir Farm` (rev 1306202, from the probe) |
| Single build with two `{{Variantbar}}` variants | `Build:A/Me Margonite Gemstone Farm` (rev 1295862, from the probe) |
| No vetting tag | `Build:A/any Assassin (Costume Brawl 2007)` (rev 980585, from the probe) |
| Team build with positioned variants | to record: a `Build:Team - ...` page |
| Archived build | to record: a page in namespace 104 (`Archive`) |
| Stub | to record if one exists |

Recording the missing pages is a manual step on the homelab: one `allpages` call per namespace or
prefix and one batched `revisions` call, with the probe's User-Agent and pacing.

Tests:

- **Snapshots.** Each fixture is parsed and checked with `insta::assert_debug_snapshot!`. Snapshots
  are reviewed with `cargo insta review` and committed. `insta` is a dev-dependency.
- **Lossless.** For every fixture file (the shape fixtures plus all 50 probe pages, each its own
  file), concatenating `Section.wikitext` equals the input byte for byte.
- **No panic.** Truncated and malformed inputs (cut-off `[build`, unclosed `{{`, unclosed
  `<pvxbig>`, empty string, a page with only headings) return a `ParsedPage` with issues.
- **Unit tests** next to the code for the lookup tables (`sha`, `earth`, `earthp`, `rt`, `any`),
  the slot grammar, the `[build]` header, and vetting normalisation.

No acceptance test in this spec: there is no MCP tool yet. The first acceptance test comes with the
walking skeleton.

## Project file changes

- Root `Cargo.toml` becomes the workspace; `src/main.rs` moves to `crates/pvxwiki-mcp/src/main.rs`.
- `deny.toml`: `parse-wiki-text-2` declares `license-file`; cargo-deny detects its text as MIT-0.
  Add `MIT-0` to `allow`.
- `probity.config.ts`: globs become `crates/*/src/**/*.rs` and `crates/*/tests/**/*.rs`.
- `.claude/rules/testing.md`: path globs and the fixture path follow the workspace layout.
- CI and the `CLAUDE.md` command list use `--workspace` where it matters.
- `CLAUDE.md`: Out of scope says template codes are neither decoded nor stored; the Stack section
  records the workspace layout and `parse-wiki-text-2`.
- `.claude/rules/logging.md` (done 2026-10-09): applies to every crate; spans for context,
  structured fields, log once where handled, escaped and length-capped outside text.
- `.claude/rules/data-fidelity.md`: the parser bullet allows `tracing` events (done 2026-10-09);
  drop the template-code bullet; the strength-of-recommendation
  bullet covers what the markup gives (main bar, `Optional` slots, named variants), with prose
  recommendations kept as section text.
