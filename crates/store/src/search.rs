//! Search: FTS5 for text, plain indexed tables for filters, ranking and pagination in Rust.

use std::collections::{BTreeSet, HashMap};

use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};

use crate::fts;
use crate::sqlite::load_by_id;
use crate::{BuildDocument, StoreError, filter_key};

/// Best-ranked sections returned per hit.
pub const MAX_MATCHES_PER_HIT: usize = 3;
/// Tokens of context in a snippet.
const SNIPPET_TOKENS: i32 = 16;
/// BM25 column weights: a title or heading hit outranks a body hit.
const TITLE_WEIGHT: f64 = 10.0;
const HEADING_WEIGHT: f64 = 5.0;
const BODY_WEIGHT: f64 = 1.0;

/// Validated input. Filter strings are lowercase; empty lists mean "no filter".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: Option<String>,
    /// A page matches when one of its bars contains all of these skills.
    pub skills: Vec<String>,
    /// Primary profession of any bar.
    pub professions: Vec<String>,
    pub gameplay_types: Vec<String>,
    pub ratings: Vec<String>,
    pub include_archived: bool,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub document: BuildDocument,
    /// Empty when the query had no text.
    pub matches: Vec<SectionMatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionMatch {
    pub heading: Option<String>,
    /// Section text around the match; matched words wrapped in « ».
    pub snippet: String,
}

/// Sort key for ratings: great, good, creative, then everything else.
pub(crate) fn rating_rank(rating: Option<&str>) -> u8 {
    match rating {
        Some("great") => 0,
        Some("good") => 1,
        Some("creative") => 2,
        _ => 3,
    }
}

struct Candidate {
    id: i64,
    title: String,
    rating: Option<String>,
    best: f64,
    matches: Vec<(f64, SectionMatch)>,
}

pub(crate) fn search(conn: &Connection, query: &SearchQuery) -> Result<SearchPage, StoreError> {
    let match_text = query
        .text
        .as_deref()
        .map(fts::escape)
        .filter(|t| !t.is_empty());
    let mut params: Vec<Value> = Vec::new();
    let mut candidates = match &match_text {
        Some(match_text) => {
            params.push(Value::Text(match_text.clone()));
            let filters = filter_sql(query, &mut params);
            text_candidates(conn, &filters, &params)?
        }
        None => {
            let filters = filter_sql(query, &mut params);
            filter_candidates(conn, &filters, &params)?
        }
    };
    candidates.sort_by(|a, b| {
        a.best
            .total_cmp(&b.best)
            .then(rating_rank(a.rating.as_deref()).cmp(&rating_rank(b.rating.as_deref())))
            .then(a.title.cmp(&b.title))
    });
    let has_more = candidates.len() > query.offset.saturating_add(query.limit);
    let hits = candidates
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .map(|mut c| {
            c.matches.sort_by(|a, b| a.0.total_cmp(&b.0));
            Ok(SearchHit {
                document: load_by_id(conn, c.id)?,
                matches: c
                    .matches
                    .into_iter()
                    .take(MAX_MATCHES_PER_HIT)
                    .map(|(_, m)| m)
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    Ok(SearchPage { hits, has_more })
}

/// SQL fragments chosen by which filters are set; every value goes into `params`. The only
/// text formatted into SQL is a run of `?` placeholders.
fn filter_sql(query: &SearchQuery, params: &mut Vec<Value>) -> String {
    let mut sql = String::from(if query.include_archived {
        "1 = 1"
    } else {
        "p.archived = 0"
    });
    let mut list = |sql: &mut String, fragment: &str, values: &[String]| {
        if values.is_empty() {
            return;
        }
        sql.push_str(&fragment.replace("{}", &placeholders(values.len())));
        params.extend(values.iter().cloned().map(Value::Text));
    };
    let keys = |values: &[String]| values.iter().map(|v| filter_key(v)).collect::<Vec<_>>();
    list(&mut sql, " AND p.rating IN ({})", &keys(&query.ratings));
    list(
        &mut sql,
        " AND p.id IN (SELECT page_id FROM page_profs WHERE profession IN ({}))",
        &keys(&query.professions),
    );
    list(
        &mut sql,
        " AND p.id IN (SELECT page_id FROM page_types WHERE gameplay_type IN ({}))",
        &keys(&query.gameplay_types),
    );
    let skills: Vec<String> = keys(&query.skills)
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !skills.is_empty() {
        sql.push_str(&format!(
            " AND p.id IN (SELECT page_id FROM page_skills WHERE skill IN ({}) \
               GROUP BY page_id, bar HAVING COUNT(DISTINCT skill) = ?)",
            placeholders(skills.len())
        ));
        params.extend(skills.iter().cloned().map(Value::Text));
        params.push(Value::Integer(
            i64::try_from(skills.len()).unwrap_or(i64::MAX),
        ));
    }
    sql
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

fn text_candidates(
    conn: &Connection,
    filters: &str,
    params: &[Value],
) -> Result<Vec<Candidate>, StoreError> {
    let sql = format!(
        "SELECT p.id, p.title, p.rating, sections_fts.heading,
                snippet(sections_fts, 2, '«', '»', '…', {SNIPPET_TOKENS}),
                bm25(sections_fts, {TITLE_WEIGHT:?}, {HEADING_WEIGHT:?}, {BODY_WEIGHT:?})
         FROM sections_fts JOIN pages p ON p.id = sections_fts.page_id
         WHERE sections_fts MATCH ? AND {filters}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params_from_iter(params))?;
    let mut by_page: HashMap<i64, Candidate> = HashMap::new();
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let heading: String = row.get(3)?;
        let score: f64 = row.get(5)?;
        let entry = by_page.entry(id).or_insert(Candidate {
            id,
            title: row.get(1)?,
            rating: row.get(2)?,
            best: score,
            matches: Vec::new(),
        });
        entry.best = entry.best.min(score);
        entry.matches.push((
            score,
            SectionMatch {
                heading: (!heading.is_empty()).then_some(heading),
                snippet: row.get(4)?,
            },
        ));
    }
    Ok(by_page.into_values().collect())
}

fn filter_candidates(
    conn: &Connection,
    filters: &str,
    params: &[Value],
) -> Result<Vec<Candidate>, StoreError> {
    let sql = format!("SELECT p.id, p.title, p.rating FROM pages p WHERE {filters}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params), |r| {
        Ok(Candidate {
            id: r.get(0)?,
            title: r.get(1)?,
            rating: r.get(2)?,
            best: 0.0,
            matches: Vec::new(),
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
