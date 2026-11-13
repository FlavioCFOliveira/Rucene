//! The search command: criteria to a `BooleanQuery`, sort keys to a `Sort`,
//! and a sorted top-hits search through `IndexSearcher`.
//!
//! Each criterion becomes the Lucene query the field's class provides
//! (`KeywordField.newExactQuery`, `LongField.newRangeQuery`,
//! `LatLonPoint.newDistanceQuery`, ...), and each sort key the sort field the
//! class provides (`KeywordField.newSortField`, `DoubleField.newSortField`,
//! `LatLonDocValuesField.newDistanceSort`, ...), so the example exercises the
//! same entry points a Lucene application uses. Free text goes through
//! `org.apache.lucene.util.QueryBuilder`, the analysis-to-query bridge that
//! lives in Lucene core (the query parser is a separate module).

use std::path::Path;
use std::sync::Arc;

use rucene::analysis::StandardAnalyzer;
use rucene::document::geo_fields::{LatLonDocValuesField, LatLonPoint};
use rucene::document::{DoubleField, IntField, KeywordField, LongField};
use rucene::index::directory_reader;
use rucene::index::IndexReader;
use rucene::search::{
    BooleanQueryBuilder, IndexSearcher, MatchAllDocsQuery, MissingValue, Occur, Query, Sort,
    SortField, SortFieldType, SortValue, SortedNumericSelectorType, SortedSetSelectorType,
    TotalHitsRelation,
};
use rucene::util::{BytesRef, QueryBuilder, QueryBuilderOps};

use crate::datetime::format_iso8601;
use crate::error::{CliError, Result};
use crate::index_ops::open_existing;
use crate::schema::{
    exact_field_of, require_field, stored_value, FieldKind, FieldSpec, Value, ID_FIELD,
    LOCATION_FIELD, PLACE_FIELD,
};

/// One search criterion.
#[derive(Clone, Debug, PartialEq)]
pub enum Criterion {
    /// Every analysed word of the text must occur in `place`
    /// (`QueryBuilder.createBooleanQuery(place, text, MUST)`).
    Text(String),
    /// The analysed words must occur in `place` as a phrase
    /// (`QueryBuilder.createPhraseQuery`).
    Phrase(String),
    /// The field must hold exactly `value` (an exact term for keywords, an
    /// exact point query for numbers and dates).
    Term {
        /// The column.
        field: String,
        /// The value, in the column's text form.
        value: String,
    },
    /// The field must hold one of `values` (`TermInSetQuery` for keywords, a
    /// point set query for numbers and dates).
    Terms {
        /// The column.
        field: String,
        /// The values, in the column's text form.
        values: Vec<String>,
    },
    /// The numeric or date field must lie in the inclusive range; a missing
    /// bound is open.
    Range {
        /// The column.
        field: String,
        /// The inclusive lower bound, if any.
        lower: Option<String>,
        /// The inclusive upper bound, if any.
        upper: Option<String>,
    },
    /// The location must lie in the box (`LatLonPoint.newBoxQuery`).
    Box {
        /// Southern latitude.
        min_latitude: f64,
        /// Northern latitude.
        max_latitude: f64,
        /// Western longitude (greater than `max_longitude` crosses the date
        /// line).
        min_longitude: f64,
        /// Eastern longitude.
        max_longitude: f64,
    },
    /// The location must lie within `kilometers` of the point
    /// (`LatLonPoint.newDistanceQuery`).
    Distance {
        /// Latitude of the centre.
        latitude: f64,
        /// Longitude of the centre.
        longitude: f64,
        /// Radius in kilometres.
        kilometers: f64,
    },
    /// The document key must be `id` (`KeywordField.newExactQuery("id", id)`).
    Id(String),
}

/// A criterion with its `BooleanClause.Occur`.
#[derive(Clone, Debug, PartialEq)]
pub struct Clause {
    /// `MUST`, `SHOULD` or `MUST_NOT`.
    pub occur: Occur,
    /// The criterion.
    pub criterion: Criterion,
}

/// What a sort key orders by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SortKey {
    /// Relevance (`SortField.FIELD_SCORE`).
    Score,
    /// Index order (`SortField.FIELD_DOC`).
    Doc,
    /// Distance from the origin (`LatLonDocValuesField.newDistanceSort`).
    Distance,
    /// A column's doc values.
    Field(String),
}

/// One sort key and its direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortSpec {
    /// What to order by.
    pub key: SortKey,
    /// Descending when `true`.
    pub descending: bool,
}

/// A complete search request.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchRequest {
    /// The criteria; none matches every document.
    pub clauses: Vec<Clause>,
    /// The sort keys; none sorts by relevance.
    pub sort: Vec<SortSpec>,
    /// The maximum number of hits returned.
    pub limit: usize,
    /// The columns printed for each hit.
    pub fields: Vec<String>,
    /// The origin of a distance sort, when given explicitly; otherwise the
    /// centre of the first `Distance` criterion.
    pub origin: Option<(f64, f64)>,
    /// When `true`, the total is also counted with `IndexSearcher.count(Query)`,
    /// which is always exact. When `false`, the total is the collector's, which
    /// Lucene counts exactly only up to 1 000 hits.
    pub exact_total: bool,
}

/// One hit of a search.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// The Lucene document number.
    pub doc: i32,
    /// The score (`NaN` is never produced: scores are always computed).
    pub score: f32,
    /// The value of each sort key, in text form.
    pub sort_values: Vec<String>,
    /// The requested columns and their stored values (`None` when absent).
    pub fields: Vec<(String, Option<String>)>,
}

/// The outcome of a search.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchResult {
    /// The number of matching documents.
    pub total_hits: i64,
    /// Whether `total_hits` is exact (`TotalHits.Relation.EQUAL_TO`) or a
    /// lower bound.
    pub exact: bool,
    /// A label per sort key, for column headers.
    pub sort_labels: Vec<String>,
    /// The top hits, in sort order.
    pub hits: Vec<Hit>,
}

/// Builds the query of `request`: a `BooleanQuery` of its clauses, or
/// `MatchAllDocsQuery` when it has none.
///
/// When every clause is `MUST_NOT`, a `MatchAllDocsQuery` `MUST` clause is
/// added, because a Lucene `BooleanQuery` with only prohibited clauses
/// matches nothing.
///
/// # Errors
///
/// Returns [`CliError::Input`] when a criterion names an unknown field, uses
/// a field kind it does not apply to, holds a value that does not parse, or
/// produces no terms after analysis.
pub fn build_query(request: &SearchRequest) -> Result<Arc<dyn Query>> {
    if request.clauses.is_empty() {
        return Ok(Arc::new(MatchAllDocsQuery::instance()));
    }
    let mut builder = BooleanQueryBuilder::new();
    for clause in &request.clauses {
        builder.add(criterion_query(&clause.criterion)?, clause.occur)?;
    }
    if request
        .clauses
        .iter()
        .all(|clause| clause.occur == Occur::MUST_NOT)
    {
        builder.add(Arc::new(MatchAllDocsQuery::instance()), Occur::MUST)?;
    }
    Ok(Arc::new(builder.build()))
}

fn criterion_query(criterion: &Criterion) -> Result<Arc<dyn Query>> {
    match criterion {
        Criterion::Text(text) => analysed_query(text, false),
        Criterion::Phrase(text) => analysed_query(text, true),
        Criterion::Id(id) => Ok(KeywordField::new_exact_query_string(ID_FIELD, id)),
        Criterion::Term { field, value } => {
            set_query(require_field(field)?, std::slice::from_ref(value))
        }
        Criterion::Terms { field, values } => {
            if values.is_empty() {
                return Err(CliError::Input(format!(
                    "no values given for field {field:?}"
                )));
            }
            set_query(require_field(field)?, values)
        }
        Criterion::Range {
            field,
            lower,
            upper,
        } => range_query(require_field(field)?, lower.as_deref(), upper.as_deref()),
        Criterion::Box {
            min_latitude,
            max_latitude,
            min_longitude,
            max_longitude,
        } => Ok(LatLonPoint::new_box_query(
            Some(LOCATION_FIELD),
            *min_latitude,
            *max_latitude,
            *min_longitude,
            *max_longitude,
        )?),
        Criterion::Distance {
            latitude,
            longitude,
            kilometers,
        } => Ok(LatLonPoint::new_distance_query(
            Some(LOCATION_FIELD),
            *latitude,
            *longitude,
            kilometers * 1000.0,
        )?),
    }
}

fn analysed_query(text: &str, phrase: bool) -> Result<Arc<dyn Query>> {
    let builder = QueryBuilder::new(Some(Arc::new(StandardAnalyzer::new())));
    let query = if phrase {
        builder.create_phrase_query(PLACE_FIELD, text)?
    } else {
        builder.create_boolean_query_with_operator(PLACE_FIELD, text, Occur::MUST)?
    };
    query.ok_or_else(|| {
        CliError::Input(format!(
            "{text:?} produces no searchable terms after analysis"
        ))
    })
}

/// The query matching documents whose `spec` value is one of `raw`: a single
/// value uses the field class's exact query, several its set query.
fn set_query(spec: &FieldSpec, raw: &[String]) -> Result<Arc<dyn Query>> {
    let values = raw
        .iter()
        .map(|value| Value::parse(spec, value))
        .collect::<Result<Vec<_>>>()?;
    let single = values.len() == 1;
    Ok(match spec.kind {
        FieldKind::Keyword | FieldKind::Text => {
            let field = exact_field_of(spec);
            if single {
                KeywordField::new_exact_query_string(field, &raw[0])
            } else {
                KeywordField::new_set_query(
                    field,
                    raw.iter()
                        .map(|value| BytesRef::new(value.as_bytes().to_vec()))
                        .collect(),
                )?
            }
        }
        FieldKind::Date => {
            let millis: Vec<i64> = values.iter().map(as_date).collect();
            if single {
                LongField::new_exact_query(spec.name, millis[0])?
            } else {
                LongField::new_set_query(spec.name, &millis)?
            }
        }
        FieldKind::Double => {
            let numbers: Vec<f64> = values.iter().map(as_double).collect();
            if single {
                DoubleField::new_exact_query(spec.name, numbers[0])?
            } else {
                DoubleField::new_set_query(spec.name, &numbers)?
            }
        }
        FieldKind::Int => {
            let numbers: Vec<i32> = values.iter().map(as_int).collect();
            if single {
                IntField::new_exact_query(spec.name, numbers[0])?
            } else {
                IntField::new_set_query(spec.name, &numbers)?
            }
        }
    })
}

/// The inclusive range query of a numeric or date field; an open bound is
/// the type's extreme (`Long.MIN_VALUE`, `Double.NEGATIVE_INFINITY`, ...), as
/// the Lucene javadoc of `newRangeQuery` prescribes.
fn range_query(
    spec: &FieldSpec,
    lower: Option<&str>,
    upper: Option<&str>,
) -> Result<Arc<dyn Query>> {
    let parse = |raw: Option<&str>| raw.map(|raw| Value::parse(spec, raw)).transpose();
    let (lower, upper) = (parse(lower)?, parse(upper)?);
    Ok(match spec.kind {
        FieldKind::Date => LongField::new_range_query(
            spec.name,
            lower.as_ref().map_or(i64::MIN, as_date),
            upper.as_ref().map_or(i64::MAX, as_date),
        )?,
        FieldKind::Double => DoubleField::new_range_query(
            spec.name,
            lower.as_ref().map_or(f64::NEG_INFINITY, as_double),
            upper.as_ref().map_or(f64::INFINITY, as_double),
        )?,
        FieldKind::Int => IntField::new_range_query(
            spec.name,
            lower.as_ref().map_or(i32::MIN, as_int),
            upper.as_ref().map_or(i32::MAX, as_int),
        )?,
        FieldKind::Keyword | FieldKind::Text => {
            return Err(CliError::Input(format!(
                "field {:?} is not numeric or a date; ranges apply to numeric and date fields",
                spec.name
            )))
        }
    })
}

fn as_date(value: &Value) -> i64 {
    match value {
        Value::Date(millis) => *millis,
        other => unreachable!("Value::parse of a Date field gave {other:?}"),
    }
}

fn as_double(value: &Value) -> f64 {
    match value {
        Value::Double(number) => *number,
        other => unreachable!("Value::parse of a Double field gave {other:?}"),
    }
}

fn as_int(value: &Value) -> i32 {
    match value {
        Value::Int(number) => *number,
        other => unreachable!("Value::parse of an Int field gave {other:?}"),
    }
}

/// Builds the `Sort` of `request`; no keys is `new Sort()` (relevance).
///
/// Documents missing a field sort after those holding it, in both
/// directions (the missing value is set to the type's extreme, or to
/// `STRING_LAST`/`STRING_FIRST`). Lucene breaks remaining ties by document
/// number.
///
/// # Errors
///
/// Returns [`CliError::Input`] for an unknown field, a descending distance
/// sort (`LatLonPointSortField` is ascending only), or a distance sort with
/// no origin.
pub fn build_sort(request: &SearchRequest) -> Result<Sort> {
    if request.sort.is_empty() {
        return Ok(Sort::new());
    }
    let fields = request
        .sort
        .iter()
        .map(|spec| sort_field(spec, request))
        .collect::<Result<Vec<_>>>()?;
    Ok(Sort::new_fields(fields)?)
}

fn sort_field(spec: &SortSpec, request: &SearchRequest) -> Result<SortField> {
    let reverse = spec.descending;
    Ok(match &spec.key {
        // Relevance sorts by decreasing score unless reversed.
        SortKey::Score => SortField::new_reverse(None, SortFieldType::Score, !reverse)?,
        SortKey::Doc => SortField::new_reverse(None, SortFieldType::Doc, reverse)?,
        SortKey::Distance => {
            if reverse {
                return Err(CliError::Input(
                    "_distance sorts ascending only (LatLonDocValuesField.newDistanceSort)"
                        .to_owned(),
                ));
            }
            let (latitude, longitude) = distance_origin(request).ok_or_else(|| {
                CliError::Input(
                    "_distance needs an origin: give --origin LAT,LON or a --distance criterion"
                        .to_owned(),
                )
            })?;
            LatLonDocValuesField::new_distance_sort(LOCATION_FIELD, latitude, longitude)?
                .sort_field()
                .clone()
        }
        SortKey::Field(name) => {
            let field = require_field(name)?;
            match field.kind {
                FieldKind::Keyword | FieldKind::Text => KeywordField::new_sort_field_with_missing(
                    Some(exact_field_of(field)),
                    reverse,
                    Some(SortedSetSelectorType::MIN),
                    Some(if reverse {
                        MissingValue::StringFirst
                    } else {
                        MissingValue::StringLast
                    }),
                )?
                .into(),
                FieldKind::Date => LongField::new_sort_field_with_missing(
                    field.name,
                    reverse,
                    SortedNumericSelectorType::MIN,
                    if reverse { i64::MIN } else { i64::MAX },
                )?
                .into_sort_field(),
                FieldKind::Double => DoubleField::new_sort_field_with_missing(
                    field.name,
                    reverse,
                    SortedNumericSelectorType::MIN,
                    if reverse {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    },
                )?
                .into_sort_field(),
                FieldKind::Int => IntField::new_sort_field_with_missing(
                    field.name,
                    reverse,
                    SortedNumericSelectorType::MIN,
                    if reverse { i32::MIN } else { i32::MAX },
                )?
                .into_sort_field(),
            }
        }
    })
}

/// The origin of a distance sort: `--origin`, else the centre of the first
/// distance criterion.
fn distance_origin(request: &SearchRequest) -> Option<(f64, f64)> {
    request.origin.or_else(|| {
        request
            .clauses
            .iter()
            .find_map(|clause| match clause.criterion {
                Criterion::Distance {
                    latitude,
                    longitude,
                    ..
                } => Some((latitude, longitude)),
                _ => None,
            })
    })
}

fn sort_label(spec: &SortSpec) -> String {
    let name = match &spec.key {
        SortKey::Score => "_score".to_owned(),
        SortKey::Doc => "_doc".to_owned(),
        SortKey::Distance => "_distance(m)".to_owned(),
        SortKey::Field(name) => name.clone(),
    };
    format!(
        "sort:{name}:{}",
        if spec.descending { "desc" } else { "asc" }
    )
}

fn format_sort_value(spec: &SortSpec, value: &SortValue) -> String {
    let is_date = matches!(&spec.key, SortKey::Field(name)
        if crate::schema::field(name).is_some_and(|field| field.kind == FieldKind::Date));
    match value {
        SortValue::Null => String::new(),
        SortValue::Long(millis) if is_date => format_iso8601(*millis),
        SortValue::Int(number) => number.to_string(),
        SortValue::Long(number) => number.to_string(),
        SortValue::Float(number) => number.to_string(),
        SortValue::Double(number) => number.to_string(),
        SortValue::Bytes(bytes) => {
            String::from_utf8_lossy(&bytes.bytes[bytes.offset..bytes.offset + bytes.length])
                .into_owned()
        }
    }
}

/// Runs `request` against the index at `path`.
///
/// Opens a `DirectoryReader`, wraps it in an `IndexSearcher`, and calls
/// `IndexSearcher.search(query, limit, sort, true)` (the top-field collector
/// path, with scores), then loads each hit's stored fields.
///
/// As in Lucene, that call counts hits exactly only up to
/// `IndexSearcher.TOTAL_HITS_THRESHOLD` (1 000); above it the total may be a
/// lower bound. When [`SearchRequest::exact_total`] is set, the total is
/// replaced by `IndexSearcher.count(query)`, which is exact.
///
/// # Errors
///
/// Returns [`CliError::Input`] when no index exists at `path` or the request
/// is invalid (see [`build_query`] and [`build_sort`]), and propagates any
/// `rucene` error.
pub fn search(path: &Path, request: &SearchRequest) -> Result<SearchResult> {
    let field_specs = request
        .fields
        .iter()
        .map(|name| require_field(name))
        .collect::<Result<Vec<_>>>()?;
    if request.limit == 0 {
        return Err(CliError::Input("--limit must be at least 1".to_owned()));
    }
    let limit = i32::try_from(request.limit)
        .map_err(|_| CliError::Input(format!("--limit {} is too large", request.limit)))?;
    let query = build_query(request)?;
    let sort = build_sort(request)?;

    let directory = open_existing(path)?;
    let reader = directory_reader::open(directory)?;
    let searcher = IndexSearcher::new(reader as Arc<dyn IndexReader>)?;
    let top = searcher.search_with_sort_and_scores(Arc::clone(&query), limit, &sort, true)?;
    let (total_hits, exact) = if request.exact_total {
        (i64::from(searcher.count(query)?), true)
    } else {
        (
            top.total_hits.value(),
            top.total_hits.relation() == TotalHitsRelation::EQUAL_TO,
        )
    };
    let stored = searcher.stored_fields()?;

    let mut hits = Vec::with_capacity(top.score_docs.len());
    for field_doc in &top.score_docs {
        let document = stored.document(field_doc.doc())?;
        let sort_values = match &field_doc.fields {
            Some(values) => request
                .sort
                .iter()
                .zip(values)
                .map(|(spec, value)| format_sort_value(spec, value))
                .collect(),
            None => Vec::new(),
        };
        hits.push(Hit {
            doc: field_doc.doc(),
            score: field_doc.score(),
            sort_values,
            fields: field_specs
                .iter()
                .map(|spec| (spec.name.to_owned(), stored_value(&document, spec)))
                .collect(),
        });
    }
    Ok(SearchResult {
        total_hits,
        exact,
        sort_labels: request.sort.iter().map(sort_label).collect(),
        hits,
    })
}

/// Writes `result` as tab-separated values: a `# total hits` line, a header
/// row, then one row per hit (requested columns, sort values, score).
///
/// Tabs and line breaks inside values are replaced by spaces so every hit
/// stays on one row.
///
/// # Errors
///
/// Propagates any error writing to `out`.
pub fn write_tsv(
    result: &SearchResult,
    fields: &[String],
    out: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    writeln!(
        out,
        "# total hits: {}{}",
        result.total_hits,
        if result.exact { "" } else { " (lower bound)" }
    )?;
    let mut header: Vec<String> = fields.to_vec();
    header.extend(result.sort_labels.iter().cloned());
    header.push("score".to_owned());
    writeln!(out, "{}", header.join("\t"))?;
    let clean = |value: &str| value.replace(['\t', '\n', '\r'], " ");
    for hit in &result.hits {
        let mut row: Vec<String> = hit
            .fields
            .iter()
            .map(|(_, value)| clean(value.as_deref().unwrap_or("")))
            .collect();
        row.extend(hit.sort_values.iter().map(|value| clean(value)));
        row.push(hit.score.to_string());
        writeln!(out, "{}", row.join("\t"))?;
    }
    Ok(())
}
