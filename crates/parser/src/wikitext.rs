//! Helpers over the `parse-wiki-text-2` tree.

use parse_wiki_text_2::{Configuration, ConfigurationSource, Node, Parameter};

// From the wiki's siteinfo (`siprop=extensiontags|protocols|magicwords`), fetched 2026-10-09.
// Refresh from the API when MediaWiki or Fandom extensions change.
const EXTENSION_TAGS: [&str; 114] = [
    "activityfeed",
    "archiveorg",
    "bambuser",
    "bambuser_channel",
    "beam",
    "bing",
    "blip",
    "categorytree",
    "ce",
    "charinsert",
    "chem",
    "choose",
    "collegehumor",
    "comments",
    "coordinates",
    "dailymotion",
    "disclose",
    "discussions",
    "display_line",
    "display_map",
    "display_point",
    "display_points",
    "distance",
    "divshare",
    "dpl",
    "dynamicpagelist",
    "embedvideo",
    "evlplayer",
    "facebook",
    "fb:follow",
    "fb:like",
    "fb:like-box",
    "fb:page",
    "fb:share-button",
    "finddestination",
    "forum",
    "funnyordie",
    "gallery",
    "geocode",
    "geodistance",
    "gfycat",
    "helper",
    "imagemap",
    "imap",
    "indicator",
    "infobox",
    "inputbox",
    "interactive-map",
    "jwplayer",
    "kickstarter",
    "langconvert",
    "loggedin",
    "loggedout",
    "mainpage-endcolumn",
    "mainpage-leftcolumn-start",
    "mainpage-rightcolumn-start",
    "map",
    "maplib",
    "mapsdoc",
    "math",
    "mediacccde",
    "metacafe",
    "metadesc",
    "microsoftstream",
    "mixer",
    "mp3",
    "nico",
    "nowiki",
    "pageby",
    "pagetools",
    "poem",
    "poll",
    "pre",
    "pvxbig",
    "randomimage",
    "ref",
    "references",
    "rhtml",
    "rss",
    "rutube",
    "section",
    "slideboxlightshow",
    "smashcast",
    "soundcloud",
    "source",
    "spotify",
    "spotifyalbum",
    "spotifyartist",
    "spotifytrack",
    "staff",
    "syntaxhighlight",
    "tabber",
    "table-progress-tracking",
    "teachertube",
    "ted",
    "templatedata",
    "templatestyles",
    "timeline",
    "tubitv",
    "tudou",
    "tvpot",
    "twitterfeed",
    "verbatim",
    "videomaten",
    "vimeo",
    "vine",
    "vote",
    "vplayer",
    "widget",
    "yahoo",
    "youku",
    "youtube",
    "youtubeplaylist",
    "youtubevideolist",
];

const PROTOCOLS: [&str; 27] = [
    "ftp://",
    "ftps://",
    "geo:",
    "git://",
    "gopher://",
    "http://",
    "https://",
    "irc://",
    "ircs://",
    "magnet:",
    "mailto:",
    "matrix:",
    "mms://",
    "nntp://",
    "redis://",
    "sftp://",
    "sip:",
    "sips:",
    "sms:",
    "ssh://",
    "svn://",
    "tel:",
    "telnet://",
    "urn:",
    "worldwind://",
    "xmpp:",
    "//",
];

/// Built per call. Extension tag content is still parsed into nodes (checked for `pvxbig`).
pub(crate) fn configuration() -> Configuration {
    Configuration::new(&ConfigurationSource {
        category_namespaces: &["category"],
        extension_tags: &EXTENSION_TAGS,
        file_namespaces: &["file", "image"],
        link_trail: "abcdefghijklmnopqrstuvwxyz",
        magic_words: &[
            "FORCETOC",
            "NOEDITSECTION",
            "NOTOC",
            "STATICREDIRECT",
            "TOC",
        ],
        protocols: &PROTOCOLS,
        redirect_magic_words: &["REDIRECT"],
    })
}

/// Text for the search index: text nodes, link labels and template parameter values, with
/// markup dropped and whitespace collapsed to single spaces.
pub(crate) fn plain_text(nodes: &[Node]) -> String {
    let mut out = String::new();
    push_text(nodes, &mut out);
    collapse(&out)
}

/// Single spaces between words, without bare list markers: inside `<pvxbig>` the tree keeps
/// `*` / `#` / `:` / `;` line prefixes as text.
pub(crate) fn collapse(text: &str) -> String {
    text.split_whitespace()
        .filter(|word| !word.chars().all(|c| matches!(c, '*' | '#' | ':' | ';')))
        .collect::<Vec<_>>()
        .join(" ")
}

fn push_text(nodes: &[Node], out: &mut String) {
    for node in nodes {
        match node {
            Node::Text { value, .. } => out.push_str(value),
            Node::CharacterEntity { character, .. } => out.push(*character),
            Node::Link { target, text, .. } => {
                if text.is_empty() {
                    out.push_str(target);
                } else {
                    push_text(text, out);
                }
            }
            Node::ExternalLink { nodes, .. } | Node::Preformatted { nodes, .. } => {
                push_text(nodes, out)
            }
            Node::Heading { nodes, .. } | Node::Tag { nodes, .. } => {
                out.push(' ');
                push_text(nodes, out);
                out.push(' ');
            }
            Node::Image { text, .. } => push_text(text, out),
            Node::Template { parameters, .. } => {
                for parameter in parameters {
                    out.push(' ');
                    push_text(&parameter.value, out);
                }
                out.push(' ');
            }
            Node::UnorderedList { items, .. } | Node::OrderedList { items, .. } => {
                for item in items {
                    out.push(' ');
                    push_text(&item.nodes, out);
                }
                out.push(' ');
            }
            Node::DefinitionList { items, .. } => {
                for item in items {
                    out.push(' ');
                    push_text(&item.nodes, out);
                }
                out.push(' ');
            }
            Node::Table { captions, rows, .. } => {
                for caption in captions {
                    out.push(' ');
                    push_text(&caption.content, out);
                }
                for row in rows {
                    for cell in &row.cells {
                        out.push(' ');
                        push_text(&cell.content, out);
                    }
                }
                out.push(' ');
            }
            Node::ParagraphBreak { .. }
            | Node::StartTag { .. }
            | Node::EndTag { .. }
            | Node::HorizontalDivider { .. } => out.push(' '),
            Node::Redirect { target, .. } => out.push_str(target),
            Node::Bold { .. }
            | Node::BoldItalic { .. }
            | Node::Italic { .. }
            | Node::Category { .. }
            | Node::Comment { .. }
            | Node::MagicWord { .. }
            | Node::Parameter { .. } => {}
        }
    }
}

pub(crate) struct TemplateRef<'n, 'a> {
    /// Normalised with `template_name`.
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub parameters: &'n [Parameter<'a>],
}

pub(crate) struct Collected<'n, 'a> {
    /// Every template in the tree, in document order.
    pub templates: Vec<TemplateRef<'n, 'a>>,
    /// Byte ranges of every `<pvxbig>` element, tags included, outermost only, in order.
    pub pvxbig: Vec<(usize, usize)>,
    /// Byte ranges of comments and `<nowiki>` elements: text the wiki never renders as markup.
    pub hidden: Vec<(usize, usize)>,
}

pub(crate) fn collect<'n, 'a>(nodes: &'n [Node<'a>]) -> Collected<'n, 'a> {
    let mut collected = Collected {
        templates: Vec::new(),
        pvxbig: Vec::new(),
        hidden: Vec::new(),
    };
    walk(nodes, &mut collected);
    collected.pvxbig.sort_unstable();
    let mut outermost: Vec<(usize, usize)> = Vec::new();
    for range in collected.pvxbig {
        if outermost.last().is_none_or(|last| range.0 >= last.1) {
            outermost.push(range);
        }
    }
    collected.pvxbig = outermost;
    collected
}

fn walk<'n, 'a>(nodes: &'n [Node<'a>], acc: &mut Collected<'n, 'a>) {
    for node in nodes {
        match node {
            Node::Template {
                name,
                parameters,
                start,
                end,
            } => {
                acc.templates.push(TemplateRef {
                    name: template_name(name),
                    start: *start,
                    end: *end,
                    parameters,
                });
                for parameter in parameters {
                    walk(&parameter.value, acc);
                }
            }
            Node::Tag {
                name,
                nodes,
                start,
                end,
            } => {
                if name.eq_ignore_ascii_case("nowiki") {
                    acc.hidden.push((*start, *end));
                    continue;
                }
                if name.eq_ignore_ascii_case("pvxbig") {
                    acc.pvxbig.push((*start, *end));
                }
                walk(nodes, acc);
            }
            Node::Comment { start, end } => acc.hidden.push((*start, *end)),
            Node::Heading { nodes, .. }
            | Node::ExternalLink { nodes, .. }
            | Node::Preformatted { nodes, .. } => walk(nodes, acc),
            Node::Link { text, .. } | Node::Image { text, .. } => walk(text, acc),
            Node::UnorderedList { items, .. } | Node::OrderedList { items, .. } => {
                for item in items {
                    walk(&item.nodes, acc);
                }
            }
            Node::DefinitionList { items, .. } => {
                for item in items {
                    walk(&item.nodes, acc);
                }
            }
            Node::Table { rows, .. } => {
                for row in rows {
                    for cell in &row.cells {
                        walk(&cell.content, acc);
                    }
                }
            }
            _ => {}
        }
    }
}

/// MediaWiki treats `_` and space alike in names; case is normalised fully here because no two
/// PvX templates differ only by case.
pub(crate) fn template_name(name: &[Node]) -> String {
    plain_text(name).replace('_', " ").to_lowercase()
}

/// Lowercased, trimmed name of a named parameter; `None` for positional ones.
pub(crate) fn param_name(src: &str, parameter: &Parameter) -> Option<String> {
    parameter.name.as_ref()?;
    let raw = src.get(parameter.start..parameter.end)?;
    raw.split_once('=')
        .map(|(name, _)| name.trim().to_lowercase())
}

/// Trimmed source text of a parameter's value.
pub(crate) fn param_value<'s>(src: &'s str, parameter: &Parameter) -> &'s str {
    let raw = src.get(parameter.start..parameter.end).unwrap_or("");
    let value = if parameter.name.is_some() {
        raw.split_once('=').map_or("", |(_, value)| value)
    } else {
        raw
    };
    value.trim()
}

/// Value of the named parameter `key`, if present and non-empty.
pub(crate) fn named(src: &str, parameters: &[Parameter], key: &str) -> Option<String> {
    parameters
        .iter()
        .find(|p| param_name(src, p).as_deref() == Some(key))
        .map(|p| param_value(src, p))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// HTML tags that never take a closing tag, so they never nest.
const VOID_TAGS: [&str; 4] = ["br", "hr", "img", "wbr"];

/// Byte offset of the opener (`{{`, `[[` or a start tag) that first takes nesting past `max`.
/// A linear scan, run before the tree is built: the tree and its `Drop` recurse per level, so
/// thousands of levels from a vandalised page would overflow the stack.
pub(crate) fn first_too_deep(src: &str, max: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let rest = &bytes[i..];
        let opened = if rest.starts_with(b"{{") || rest.starts_with(b"[[") {
            i += 2;
            Some(i - 2)
        } else if rest.starts_with(b"}}") || rest.starts_with(b"]]") || rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i += 2;
            None
        } else if rest.first() == Some(&b'<') && rest.get(1).is_some_and(u8::is_ascii_alphabetic) {
            let name_len = rest[1..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric())
                .count();
            let name = &rest[1..1 + name_len];
            let close = rest.iter().position(|&b| b == b'>');
            let self_closing = close.is_some_and(|c| rest[..c].ends_with(b"/"));
            let void = VOID_TAGS
                .iter()
                .any(|v| name.eq_ignore_ascii_case(v.as_bytes()));
            i += 1 + name_len;
            (close.is_some() && !self_closing && !void).then_some(i - 1 - name_len)
        } else {
            i += 1;
            None
        };
        if let Some(at) = opened {
            depth += 1;
            if depth > max {
                return Some(at);
            }
        }
    }
    None
}

/// `src[start..end]` with every `hidden` byte range replaced by spaces. Byte length is
/// unchanged, so offsets into the result are offsets into `src`.
pub(crate) fn masked(src: &str, start: usize, end: usize, hidden: &[(usize, usize)]) -> String {
    let mut bytes = src.as_bytes().get(start..end).unwrap_or_default().to_vec();
    for &(h_start, h_end) in hidden {
        let from = h_start.max(start) - start;
        let to = h_end.min(end).saturating_sub(start);
        if let Some(range) = bytes.get_mut(from..to) {
            range.fill(b' ');
        }
    }
    // Hidden ranges come from the parse tree and sit on char boundaries, so this holds.
    String::from_utf8(bytes).unwrap_or_else(|_| src.get(start..end).unwrap_or("").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(src: &str) -> String {
        let output = configuration().parse(src).unwrap();
        plain_text(&output.nodes)
    }

    #[test]
    fn nesting_past_the_limit_reports_where_it_got_too_deep() {
        let src = "{{a|".repeat(3) + "[[x|" + "<b>";
        assert_eq!(first_too_deep(&src, 4), Some(src.len() - 3));
        assert_eq!(first_too_deep(&src, 5), None);
    }

    #[test]
    fn closed_void_and_self_closing_tags_do_not_add_depth() {
        let src = "{{a}} [[b]] <big>x</big> <br> <br/> <ref name=x/> <!-- c --> ".repeat(500);
        assert_eq!(first_too_deep(&src, 1), None);
    }

    #[test]
    fn plain_text_keeps_link_labels_and_template_values() {
        assert_eq!(
            text_of(
                "Farm [[gww:Glacial Stone|Glacial Stones]] with {{skill pvxicon|Mantra of Earth|6}}."
            ),
            "Farm Glacial Stones with Mantra of Earth 6 ."
        );
    }

    #[test]
    fn plain_text_drops_markup_and_comments() {
        assert_eq!(
            text_of("'''Bold''' <!-- hidden --> fish &amp; chips<br />done"),
            "Bold fish & chips done"
        );
    }

    #[test]
    fn plain_text_reads_inside_pvxbig() {
        assert_eq!(
            text_of("<pvxbig>\n* {{skill pvxicon|Dash}} - fast\n</pvxbig>"),
            "Dash - fast"
        );
    }
}
