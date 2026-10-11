//! `{{Real-Vetting}}` and `{{build-stub}}`. Values from Template:Real-Vetting (rev 1299291),
//! which lowercases `rating` and `status` before using them.

use parse_wiki_text_2::Parameter;

use crate::wikitext::{TemplateRef, param_name, param_value};
use crate::{Archive, Rating, Status, Vetted, Vetting};

const REAL_VETTING: &str = "real-vetting";
const BUILD_STUB: &str = "build-stub";

/// The first vetting tag wins. The second value is the byte offset of a second one, if any.
pub(crate) fn extract(src: &str, templates: &[TemplateRef]) -> (Vetting, Option<usize>) {
    let mut tags = templates
        .iter()
        .filter(|t| t.name == REAL_VETTING || t.name == BUILD_STUB);
    let Some(first) = tags.next() else {
        return (Vetting::Missing, None);
    };
    let duplicate = tags.next().map(|t| t.start);
    if first.name == BUILD_STUB {
        return (Vetting::Stub, duplicate);
    }
    (Vetting::Vetted(vetted(src, first.parameters)), duplicate)
}

fn vetted(src: &str, parameters: &[Parameter]) -> Vetted {
    let mut vetted = Vetted {
        rating: None,
        status: None,
        gameplay_types: Vec::new(),
        archive: None,
    };
    let mut date = None;
    let mut category = None;
    let mut reasons: Vec<(u32, String)> = Vec::new();
    for parameter in parameters {
        let value = param_value(src, parameter);
        if value.is_empty() {
            continue;
        }
        match param_name(src, parameter).as_deref() {
            None => vetted.gameplay_types.push(value.to_string()),
            Some("rating") => vetted.rating = Some(rating(value)),
            Some("status") => vetted.status = Some(status(value)),
            Some("date") => date = Some(value.to_string()),
            Some("category") => category = Some(value.to_string()),
            Some(other) => {
                if let Some(n) = other.strip_prefix("reason").and_then(|n| n.parse().ok()) {
                    reasons.push((n, value.to_string()));
                }
            }
        }
    }
    reasons.sort_by_key(|(n, _)| *n);
    if date.is_some() || category.is_some() || !reasons.is_empty() {
        vetted.archive = Some(Archive {
            date,
            category,
            reasons: reasons.into_iter().map(|(_, reason)| reason).collect(),
        });
    }
    vetted
}

fn rating(value: &str) -> Rating {
    match value.to_lowercase().as_str() {
        "great" => Rating::Great,
        "good" => Rating::Good,
        "creative" => Rating::Creative,
        "trash" => Rating::Trash,
        "abandoned" => Rating::Abandoned,
        "testing" => Rating::Testing,
        "trial" => Rating::Trial,
        "archived" => Rating::Archived,
        _ => Rating::Unknown(value.to_string()),
    }
}

fn status(value: &str) -> Status {
    match value.to_lowercase().as_str() {
        "meta" => Status::Meta,
        "provisional" => Status::Provisional,
        _ => Status::Unknown(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use crate::{Archive, IssueKind, Rating, Status, Vetted, Vetting, parse};

    #[test]
    fn reads_rating_status_and_gameplay_types() {
        let page = parse("{{Real-Vetting|status=meta|rating=Great| farming |PvE team|}}\nText");
        assert_eq!(
            page.vetting,
            Vetting::Vetted(Vetted {
                rating: Some(Rating::Great),
                status: Some(Status::Meta),
                gameplay_types: vec!["farming".into(), "PvE team".into()],
                archive: None,
            })
        );
    }

    #[test]
    fn empty_status_is_none_and_unknown_values_are_kept() {
        let page = parse("{{Real-Vetting|running|quest|status=|rating=Shiny}}");
        let Vetting::Vetted(v) = page.vetting else {
            panic!("{:?}", page.vetting)
        };
        assert_eq!(v.status, None);
        assert_eq!(v.rating, Some(Rating::Unknown("Shiny".into())));
        assert_eq!(v.gameplay_types, ["running", "quest"]);
    }

    #[test]
    fn archived_pages_keep_date_category_and_ordered_reasons() {
        let page = parse(
            "{{Real-Vetting|rating=archived|category=great|date=1 May 2020|reason2=b|reason1=a|AB}}",
        );
        let Vetting::Vetted(v) = page.vetting else {
            panic!("{:?}", page.vetting)
        };
        assert_eq!(v.rating, Some(Rating::Archived));
        assert_eq!(
            v.archive,
            Some(Archive {
                date: Some("1 May 2020".into()),
                category: Some("great".into()),
                reasons: vec!["a".into(), "b".into()],
            })
        );
    }

    #[test]
    fn stub_and_missing() {
        assert_eq!(parse("{{build-stub}}\nx").vetting, Vetting::Stub);
        assert_eq!(parse("{{Build-stub}}").vetting, Vetting::Stub);
        assert_eq!(parse("{{section-stub}}\nx").vetting, Vetting::Missing);
        assert_eq!(parse("{{Costume Brawl build}}").vetting, Vetting::Missing);
    }

    #[test]
    fn template_name_spelling_variants_match() {
        for src in [
            "{{real-vetting|rating=good}}",
            "{{Real-Vetting |rating=good}}",
            "{{ Real-Vetting|rating=good}}",
        ] {
            let Vetting::Vetted(v) = parse(src).vetting else {
                panic!("{src}")
            };
            assert_eq!(v.rating, Some(Rating::Good), "{src}");
        }
    }

    #[test]
    fn first_of_several_vetting_tags_wins() {
        let page = parse("{{build-stub}}\n{{Real-Vetting|rating=good}}");
        assert_eq!(page.vetting, Vetting::Stub);
        let issue = page
            .issues
            .iter()
            .find(|i| i.kind == IssueKind::MultipleVettingTags)
            .unwrap();
        assert_eq!(issue.offset, Some(15));
    }
}
