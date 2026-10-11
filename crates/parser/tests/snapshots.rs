//! One snapshot per page shape. Review changes with `cargo insta review`, or run with
//! `INSTA_UPDATE=always` and read the `.snap` diff.

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] fns")]

use std::fs;
use std::path::PathBuf;

use pvxwiki_parser::{ParsedPage, parse};

fn parse_fixture(file: &str) -> ParsedPage {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pages")
        .join(file);
    parse(&fs::read_to_string(path).unwrap())
}

#[test]
fn single_build_with_optionals_and_unknown_section() {
    insta::assert_debug_snapshot!(parse_fixture("Build-A-Me_Vaettir_Farm__1306202.wikitext"));
}

#[test]
fn single_build_with_variant_bars() {
    insta::assert_debug_snapshot!(parse_fixture(
        "Build-A-Me_Margonite_Gemstone_Farm__1295862.wikitext"
    ));
}

#[test]
fn page_without_vetting_tag() {
    insta::assert_debug_snapshot!(parse_fixture(
        "Build-A-any_Assassin_(Costume_Brawl_2007)__980585.wikitext"
    ));
}

#[test]
fn team_build() {
    insta::assert_debug_snapshot!(parse_fixture(
        "Build-Team_-_2_Hero_CoF_600-Smite__1295534.wikitext"
    ));
}

#[test]
fn archived_build() {
    insta::assert_debug_snapshot!(parse_fixture(
        "Archive-A-D_Death_Blossom_Farmer__1266398.wikitext"
    ));
}

#[test]
fn stub_build() {
    insta::assert_debug_snapshot!(parse_fixture(
        "Build-Team_-_NM_Cartographer_Skill-Capper__1306082.wikitext"
    ));
}
