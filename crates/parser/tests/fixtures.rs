#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]

use std::fs;
use std::path::PathBuf;

use pvxwiki_parser::parse;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages")
}

/// Every recorded page as (file name, wikitext), sorted by name.
fn all_fixtures() -> Vec<(String, String)> {
    let mut pages: Vec<_> = fs::read_dir(fixture_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "wikitext"))
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&path).unwrap())
        })
        .collect();
    pages.sort();
    assert!(pages.len() >= 50, "fixtures missing: {}", pages.len());
    pages
}

#[test]
fn sections_reproduce_every_fixture_byte_for_byte() {
    for (name, text) in all_fixtures() {
        let page = parse(&text);
        let joined: String = page.sections.iter().map(|s| s.wikitext.as_str()).collect();
        assert_eq!(joined, text, "{name}");
    }
}

#[test]
fn vaettir_sections_follow_the_page() {
    let text = fs::read_to_string(fixture_dir().join("Build-A-Me_Vaettir_Farm__1306202.wikitext"))
        .unwrap();
    let page = parse(&text);
    let headings: Vec<_> = page
        .sections
        .iter()
        .map(|s| (s.level, s.heading.as_deref().unwrap_or("")))
        .collect();
    assert_eq!(
        headings,
        [
            (0, ""),
            (2, "Frequency"),
            (2, "Attributes and Skills"),
            (3, "Optionals"),
            (2, "Equipment"),
            (2, "Usage"),
            (2, "Counters"),
            (2, "Videos"),
            (2, "Other Farms"),
            (3, "Dragon Moss Farming"),
            (2, "See also"),
        ]
    );
    assert!(
        page.sections[0]
            .text
            .contains("Designed to farm Vaettir in Hard Mode for Glacial Stones")
    );
    assert!(page.sections[3].text.contains("Mantra of Earth"));
    assert!(!page.sections[4].text.contains("[["));
}

#[test]
fn every_build_tag_in_the_fixtures_becomes_a_skill_bar() {
    for (name, text) in all_fixtures() {
        let tags = text.to_ascii_lowercase().matches("[build ").count();
        assert_eq!(parse(&text).skill_bars.len(), tags, "{name}");
    }
}

#[test]
fn margonite_variants_are_named() {
    let text = fs::read_to_string(
        fixture_dir().join("Build-A-Me_Margonite_Gemstone_Farm__1295862.wikitext"),
    )
    .unwrap();
    let page = parse(&text);
    assert_eq!(
        page.skill_bars
            .iter()
            .filter(|b| b.variant.is_some())
            .count(),
        2
    );
    assert!(
        page.skill_bars
            .iter()
            .filter_map(|b| b.variant.as_ref())
            .all(|v| v.name.is_some())
    );
}
