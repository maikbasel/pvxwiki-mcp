//! Malformed and hostile input never panics and never loses bytes.

use pvxwiki_parser::{IssueKind, parse};

const INPUTS: &[&str] = &[
    "",
    "== only ==",
    "== a ==\n== b ==\n",
    "{{",
    "[[",
    "}}]]",
    "<pvxbig>",
    "<pvxbig>[build prof=A/E",
    "<pvxbig>[build prof=A/E sha=12][Dash]</pvxbig>",
    "<pvxbig>[build ][/build]</pvxbig>",
    "<pvxbig>[build prof=/][[][/build]</pvxbig>",
    "{{Real-Vetting|rating=",
    "{{Real-Vetting|=|==|reason99999999999999999999=x}}",
    "{{Variantbar|template=<pvxbig>[build prof=A][x][/build]</pvxbig>}}",
    "\u{feff}== Über ==\nümlaut ’quotes’ <pvxbig>[build prof=Ä/É ü=12][Ünknown][/build]</pvxbig>",
    "<pvxbig><pvxbig>[build prof=A][Dash][/build]</pvxbig></pvxbig>",
];

#[test]
fn malformed_input_parses_without_panic_and_stays_lossless() {
    for input in INPUTS {
        let page = parse(input);
        let joined: String = page.sections.iter().map(|s| s.wikitext.as_str()).collect();
        assert_eq!(&joined, input, "{input:?}");
    }
}

#[test]
fn unclosed_bar_in_pvxbig_is_reported() {
    let page = parse("<pvxbig>[build prof=A/E sha=12][Dash]</pvxbig>");
    assert!(
        page.issues
            .iter()
            .any(|i| i.kind == IssueKind::UnclosedBuildTag)
    );
    assert!(page.skill_bars.is_empty());
}

#[test]
fn nested_pvxbig_yields_each_bar_once() {
    let page = parse("<pvxbig><pvxbig>[build prof=A][Dash][/build]</pvxbig></pvxbig>");
    assert_eq!(page.skill_bars.len(), 1);
}

#[test]
fn very_long_page_parses() {
    let section = "== Usage ==\n* [[gww:Dash|Dash]] {{skill pvxicon|Dash}} text\n";
    let input = section.repeat(5_000);
    let page = parse(&input);
    assert_eq!(page.sections.len(), 5_001);
}

#[test]
fn deeply_nested_page_is_reported_not_a_stack_overflow() {
    let templates = "{{a|".repeat(13_000) + &"}}".repeat(13_000);
    let tags = "<pvxbig>".repeat(20_000) + &"</pvxbig>".repeat(20_000);
    for input in [templates, tags] {
        let page = parse(&input);
        assert!(
            page.issues
                .iter()
                .any(|i| i.kind == IssueKind::TooDeeplyNested)
        );
        let joined: String = page.sections.iter().map(|s| s.wikitext.as_str()).collect();
        assert_eq!(joined, input);
    }
}
