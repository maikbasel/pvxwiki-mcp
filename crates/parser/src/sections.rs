//! Splits the page at top-level headings. Sections are contiguous byte ranges of the input, so
//! concatenating their wikitext gives back the input exactly.

use parse_wiki_text_2::Node;

use crate::Section;
use crate::wikitext::{collapse, plain_text};

/// Returns the sections and the start byte offset of each.
pub(crate) fn split(src: &str, nodes: &[Node]) -> (Vec<Section>, Vec<usize>) {
    struct Draft {
        heading: Option<String>,
        level: u8,
        start: usize,
        text: String,
    }
    let mut drafts = vec![Draft {
        heading: None,
        level: 0,
        start: 0,
        text: String::new(),
    }];
    for node in nodes {
        if let Node::Heading {
            start,
            level,
            nodes: title,
            ..
        } = node
        {
            drafts.push(Draft {
                heading: Some(plain_text(title)),
                level: *level,
                start: *start,
                text: String::new(),
            });
        }
        if let Some(draft) = drafts.last_mut() {
            draft.text.push(' ');
            draft.text.push_str(&plain_text(std::slice::from_ref(node)));
        }
    }

    let starts: Vec<usize> = drafts.iter().map(|d| d.start).collect();
    let sections = drafts
        .into_iter()
        .enumerate()
        .map(|(i, draft)| {
            let end = starts.get(i + 1).copied().unwrap_or(src.len());
            Section {
                heading: draft.heading,
                level: draft.level,
                wikitext: src.get(draft.start..end).unwrap_or("").to_string(),
                text: collapse(&draft.text),
            }
        })
        .collect();
    (sections, starts)
}

/// Index of the section containing byte `offset`. `starts[0]` is 0.
pub(crate) fn index_at(starts: &[usize], offset: usize) -> usize {
    starts
        .partition_point(|&start| start <= offset)
        .saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use crate::parse;

    #[test]
    fn page_without_headings_is_one_summary_section() {
        let page = parse("Just text.\n* a list");
        assert_eq!(page.sections.len(), 1);
        assert_eq!(page.sections[0].heading, None);
        assert_eq!(page.sections[0].level, 0);
        assert_eq!(page.sections[0].wikitext, "Just text.\n* a list");
        assert_eq!(page.sections[0].text, "Just text. a list");
    }

    #[test]
    fn page_starting_with_a_heading_has_an_empty_summary() {
        let page = parse("== A ==\nx\n== B ==\n");
        let parts: Vec<_> = page.sections.iter().map(|s| s.wikitext.as_str()).collect();
        assert_eq!(parts, ["", "== A ==\nx\n", "== B ==\n"]);
        assert_eq!(page.sections[1].heading.as_deref(), Some("A"));
        assert_eq!(page.sections[1].text, "A x");
    }

    #[test]
    fn empty_page_has_one_empty_section() {
        let page = parse("");
        assert_eq!(page.sections.len(), 1);
        assert_eq!(page.sections[0].wikitext, "");
    }

    #[test]
    fn index_at_maps_offsets_to_sections() {
        let starts = [0, 10, 20];
        assert_eq!(super::index_at(&starts, 0), 0);
        assert_eq!(super::index_at(&starts, 9), 0);
        assert_eq!(super::index_at(&starts, 10), 1);
        assert_eq!(super::index_at(&starts, 25), 2);
    }
}
