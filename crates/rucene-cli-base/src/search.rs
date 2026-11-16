//! The read commands `search` and `count`: criteria to a `BooleanQuery`,
//! sort keys to a `Sort`, and a sorted top-hits search through
//! `IndexSearcher`.
//!
//! Each criterion becomes the query the field's Lucene class provides
//! (`KeywordField.newExactQuery`, `TermQuery` on a `StringField`,
//! `LongField.newRangeQuery`, ...), and each sort key the sort field the class
//! provides (`KeywordField.newSortField`, `DoubleField.newSortField`, ...).
//! Free text goes through `org.apache.lucene.util.QueryBuilder` with the
//! index-time [`PortugueseFoldingAnalyzer`], so query words are folded,
//! stemmed and stop-filtered exactly as the indexed text was.
//!
//! # Text over several fields
//!
//! `--text WORDS` without a field searches the index's default text fields
//! the way Lucene's `MultiFieldQueryParser` does with the `AND` operator:
//! each whitespace-separated word must match in **at least one** of the
//! fields (a `SHOULD` disjunction per word, the words `MUST`). `--phrase`
//! without a field matches the phrase in any default field.

use std::path::Path;
use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::document::{DoubleField, IntField, KeywordField, LongField};
use rucene::index::Term;
use rucene::search::{
    BooleanQueryBuilder, IndexSearcher, MatchAllDocsQuery, MissingValue, Occur, Query, Sort,
    SortField, SortFieldType, SortValue, SortedNumericSelectorType, SortedSetSelectorType,
    TermInSetQuery, TermQuery, TotalHitsRelation,
};
use rucene::util::{BytesRef, QueryBuilder, QueryBuilderOps};

use crate::analysis::PortugueseFoldingAnalyzer;
use crate::date::format_date;
use crate::error::{CliError, Result};
use crate::index_ops::{key_term, open_searcher};
use crate::schema::{stored_values, EntitySchema, FieldKind, FieldSpec, Value};

/// One search criterion.
#[derive(Clone, Debug, PartialEq)]
pub enum Criterion {
    /// Every analysed word must occur: in `field` when given, otherwise in at
    /// least one default text field per word.
    Text {
        /// The text or name field, or `None` for the defaults.
        field: Option<String>,
        /// The words, analysed with the pt-PT chain.
        words: String,
    },
    /// The analysed words must occur as a phrase: in `field` when given,
    /// otherwise in any default text field.
    Phrase {
        /// The text or name field, or `None` for the defaults.
        field: Option<String>,
        /// The phrase, analysed with the pt-PT chain.
        words: String,
    },
    /// The field must hold exactly `value`.
    Term {
        /// The field.
        field: String,
        /// The value, in the field's text form.
        value: String,
    },
    /// The field must hold one of `values`.
    Terms {
        /// The field.
        field: String,
        /// The values, in the field's text form.
        values: Vec<String>,
    },
    /// The numeric or date field must lie in the inclusive range; a missing
    /// bound is open.
    Range {
        /// The field.
        field: String,
        /// The inclusive lower bound, if any.
        lower: Option<String>,
        /// The inclusive upper bound, if any.
        upper: Option<String>,
    },
    /// The document key must be `key` (`TermQuery` on `_key`).
    Key(String),
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
    /// A field's doc values.
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
    /// The stored fields printed for each hit.
    pub fields: Vec<String>,
    /// When `true`, the total is counted with `IndexSearcher.count(Query)`,
    /// which is always exact; otherwise it is the collector's, exact only up
    /// to 1 000 hits.
    pub exact_total: bool,
}

/// One hit of a search.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// The Lucene document number.
    pub doc: i32,
    /// The score.
    pub score: f32,
    /// The value of each sort key, in text form.
    pub sort_values: Vec<String>,
    /// The requested fields and their stored values (empty when absent).
    pub fields: Vec<(String, Vec<String>)>,
}

/// The outcome of a search.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchResult {
    /// The number of matching documents.
    pub total_hits: i64,
    /// Whether `total_hits` is exact or a lower bound.
    pub exact: bool,
    /// A label per sort key, for column headers.
    pub sort_labels: Vec<String>,
    /// The top hits, in sort order.
    pub hits: Vec<Hit>,
}

/// Builds the query of `clauses`: a `BooleanQuery`, or `MatchAllDocsQuery`
/// when there are none. When every clause is `MUST_NOT`, a
/// `MatchAllDocsQuery` `MUST` clause is added, because a Lucene
/// `BooleanQuery` with only prohibited clauses matches nothing.
///
/// # Errors
///
/// Returns [`CliError::Input`] when a criterion names an unknown field, uses
/// a field kind it does not apply to, holds a value that does not parse, or
/// produces no terms after analysis.
pub fn build_query(schema: &EntitySchema, clauses: &[Clause]) -> Result<Arc<dyn Query>> {
    if clauses.is_empty() {
        return Ok(Arc::new(MatchAllDocsQuery::instance()));
    }
    let mut builder = BooleanQueryBuilder::new();
    for clause in clauses {
        builder.add(criterion_query(schema, &clause.criterion)?, clause.occur)?;
    }
    if clauses.iter().all(|clause| clause.occur == Occur::MUST_NOT) {
        builder.add(Arc::new(MatchAllDocsQuery::instance()), Occur::MUST)?;
    }
    Ok(Arc::new(builder.build()))
}

fn criterion_query(schema: &EntitySchema, criterion: &Criterion) -> Result<Arc<dyn Query>> {
    match criterion {
        Criterion::Text { field, words } => text_query(schema, field.as_deref(), words, false),
        Criterion::Phrase { field, words } => text_query(schema, field.as_deref(), words, true),
        Criterion::Key(key) => Ok(Arc::new(TermQuery::new(key_term(key)))),
        Criterion::Term { field, value } => {
            set_query(schema.require_field(field)?, std::slice::from_ref(value))
        }
        Criterion::Terms { field, values } => {
            if values.is_empty() {
                return Err(CliError::Input(format!(
                    "no values given for field {field:?}"
                )));
            }
            set_query(schema.require_field(field)?, values)
        }
        Criterion::Range {
            field,
            lower,
            upper,
        } => range_query(
            schema.require_field(field)?,
            lower.as_deref(),
            upper.as_deref(),
        ),
    }
}

fn query_builder() -> QueryBuilder {
    let analyzer: Arc<dyn Analyzer> = Arc::new(PortugueseFoldingAnalyzer::new());
    QueryBuilder::new(Some(analyzer))
}

/// The analysed query of `text` on one field: all words `MUST`
/// (`QueryBuilder.createBooleanQuery(field, text, MUST)`) or a phrase
/// (`QueryBuilder.createPhraseQuery`); `None` when analysis leaves no term.
fn analysed(
    builder: &QueryBuilder,
    field: &str,
    text: &str,
    phrase: bool,
) -> Result<Option<Arc<dyn Query>>> {
    Ok(if phrase {
        builder.create_phrase_query(Some(field), text)?
    } else {
        builder.create_boolean_query_with_operator(Some(field), text, Occur::MUST)?
    })
}

fn text_query(
    schema: &EntitySchema,
    field: Option<&str>,
    text: &str,
    phrase: bool,
) -> Result<Arc<dyn Query>> {
    let no_terms = || {
        CliError::Input(format!(
            "{text:?} produces no searchable terms after analysis"
        ))
    };
    let builder = query_builder();
    if let Some(name) = field {
        let spec = schema.require_field(name)?;
        if !matches!(spec.kind, FieldKind::Text | FieldKind::Name) {
            return Err(CliError::Input(format!(
                "field {name:?} is not a text or name field; use --term for exact values"
            )));
        }
        return analysed(&builder, spec.name, text, phrase)?.ok_or_else(no_terms);
    }
    let disjunction = |piece: &str| -> Result<Option<Arc<dyn Query>>> {
        let mut alternatives = Vec::new();
        for name in schema.text_fields {
            if let Some(query) = analysed(&builder, name, piece, phrase)? {
                alternatives.push(query);
            }
        }
        if alternatives.is_empty() {
            return Ok(None);
        }
        let mut any = BooleanQueryBuilder::new();
        for query in alternatives {
            any.add(query, Occur::SHOULD)?;
        }
        Ok(Some(Arc::new(any.build()) as Arc<dyn Query>))
    };
    if phrase {
        return disjunction(text)?.ok_or_else(no_terms);
    }
    let mut all = BooleanQueryBuilder::new();
    let mut words = 0;
    for word in text.split_whitespace() {
        if let Some(query) = disjunction(word)? {
            all.add(query, Occur::MUST)?;
            words += 1;
        }
    }
    if words == 0 {
        return Err(no_terms());
    }
    Ok(Arc::new(all.build()))
}

fn bytes(value: &str) -> BytesRef {
    BytesRef::new(value.as_bytes().to_vec())
}

/// The query matching documents whose `spec` value is one of `raw`.
fn set_query(spec: &FieldSpec, raw: &[String]) -> Result<Arc<dyn Query>> {
    let values = raw
        .iter()
        .map(|value| Value::parse_query(spec, value))
        .collect::<Result<Vec<_>>>()?;
    let single = values.len() == 1;
    Ok(match spec.kind {
        FieldKind::Exact => {
            if single {
                Arc::new(TermQuery::new(Term::new(
                    spec.name,
                    bytes(&values[0].display()),
                )))
            } else {
                Arc::new(TermInSetQuery::new(
                    spec.name,
                    values.iter().map(|value| bytes(&value.display())).collect(),
                )?)
            }
        }
        FieldKind::Keyword | FieldKind::Flag | FieldKind::Name => {
            let field = spec.exact_field();
            if single {
                KeywordField::new_exact_query_string(&field, &values[0].display())
            } else {
                KeywordField::new_set_query(
                    &field,
                    values.iter().map(|value| bytes(&value.display())).collect(),
                )?
            }
        }
        FieldKind::Text => {
            return Err(CliError::Input(format!(
                "field {:?} is analysed text with no exact copy; use --text-in or --phrase-in",
                spec.name
            )))
        }
        FieldKind::Stored => {
            return Err(CliError::Input(format!(
                "field {:?} is stored only and not searchable",
                spec.name
            )))
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
/// the type's extreme, as the Lucene javadoc of `newRangeQuery` prescribes.
fn range_query(
    spec: &FieldSpec,
    lower: Option<&str>,
    upper: Option<&str>,
) -> Result<Arc<dyn Query>> {
    if spec.kind.is_textual() {
        return Err(CliError::Input(format!(
            "field {:?} is not numeric or a date; ranges apply to numeric and date fields",
            spec.name
        )));
    }
    let parse = |raw: Option<&str>| raw.map(|raw| Value::parse_query(spec, raw)).transpose();
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
        _ => IntField::new_range_query(
            spec.name,
            lower.as_ref().map_or(i32::MIN, as_int),
            upper.as_ref().map_or(i32::MAX, as_int),
        )?,
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
/// directions. A multi-valued field sorts by its smallest value (`MIN`
/// selector). Lucene breaks remaining ties by document number.
///
/// # Errors
///
/// Returns [`CliError::Input`] for an unknown field, or one with no doc
/// values (exact ids, text without a copy, stored-only fields).
pub fn build_sort(schema: &EntitySchema, request: &SearchRequest) -> Result<Sort> {
    if request.sort.is_empty() {
        return Ok(Sort::new());
    }
    let fields = request
        .sort
        .iter()
        .map(|spec| sort_field(schema, spec))
        .collect::<Result<Vec<_>>>()?;
    Ok(Sort::new_fields(fields)?)
}

fn sort_field(schema: &EntitySchema, spec: &SortSpec) -> Result<SortField> {
    let reverse = spec.descending;
    Ok(match &spec.key {
        // Relevance sorts by decreasing score unless reversed.
        SortKey::Score => SortField::new_reverse(None, SortFieldType::Score, !reverse)?,
        SortKey::Doc => SortField::new_reverse(None, SortFieldType::Doc, reverse)?,
        SortKey::Field(name) => {
            let field = schema.require_field(name)?;
            match field.kind {
                FieldKind::Keyword | FieldKind::Flag | FieldKind::Name => {
                    KeywordField::new_sort_field_with_missing(
                        Some(&field.exact_field()),
                        reverse,
                        Some(SortedSetSelectorType::MIN),
                        Some(if reverse {
                            MissingValue::StringFirst
                        } else {
                            MissingValue::StringLast
                        }),
                    )?
                    .into()
                }
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
                FieldKind::Exact | FieldKind::Text | FieldKind::Stored => {
                    return Err(CliError::Input(format!(
                        "field {name:?} ({}) has no doc values and cannot be sorted",
                        field.kind.label()
                    )))
                }
            }
        }
    })
}

fn sort_label(spec: &SortSpec) -> String {
    let name = match &spec.key {
        SortKey::Score => "_score",
        SortKey::Doc => "_doc",
        SortKey::Field(name) => name,
    };
    format!(
        "sort:{name}:{}",
        if spec.descending { "desc" } else { "asc" }
    )
}

fn format_sort_value(schema: &EntitySchema, spec: &SortSpec, value: &SortValue) -> String {
    let is_date = matches!(&spec.key, SortKey::Field(name)
        if schema.field(name).is_some_and(|field| field.kind == FieldKind::Date));
    match value {
        SortValue::Null => String::new(),
        SortValue::Long(millis) if is_date => format_date(*millis),
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

/// Checks that every name in `fields` is a stored field of `schema`.
fn output_specs(schema: &EntitySchema, fields: &[String]) -> Result<Vec<&'static FieldSpec>> {
    fields
        .iter()
        .map(|name| {
            let spec = schema.require_field(name)?;
            if spec.derived {
                return Err(CliError::Input(format!(
                    "field {name:?} is derived and not stored; it can be searched and sorted, not printed"
                )));
            }
            Ok(spec)
        })
        .collect()
}

/// Runs `request` against the index of `schema` under `root`.
///
/// Opens a `DirectoryReader`, wraps it in an `IndexSearcher`, and calls
/// `IndexSearcher.search(query, limit, sort, true)` (the top-field collector
/// path, with scores), then loads each hit's stored fields. With
/// [`SearchRequest::exact_total`] the total is `IndexSearcher.count(query)`.
///
/// # Errors
///
/// Returns [`CliError::Input`] when no index exists or the request is
/// invalid (see [`build_query`] and [`build_sort`]), and propagates any
/// `rucene` error.
pub fn search(root: &Path, schema: &EntitySchema, request: &SearchRequest) -> Result<SearchResult> {
    let field_specs = output_specs(schema, &request.fields)?;
    if request.limit == 0 {
        return Err(CliError::Input("--limit must be at least 1".to_owned()));
    }
    let limit = i32::try_from(request.limit)
        .map_err(|_| CliError::Input(format!("--limit {} is too large", request.limit)))?;
    let query = build_query(schema, &request.clauses)?;
    let sort = build_sort(schema, request)?;

    let searcher = open_searcher(root, schema)?;
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
                .map(|(spec, value)| format_sort_value(schema, spec, value))
                .collect(),
            None => Vec::new(),
        };
        hits.push(Hit {
            doc: field_doc.doc(),
            score: field_doc.score(),
            sort_values,
            fields: field_specs
                .iter()
                .map(|spec| {
                    (
                        spec.name.to_owned(),
                        stored_values(&document, spec)
                            .iter()
                            .map(|value| value.text().to_owned())
                            .collect(),
                    )
                })
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

/// Counts the documents matching `clauses` exactly (`IndexSearcher.count`).
///
/// # Errors
///
/// As [`build_query`] and [`open_searcher`].
pub fn count(root: &Path, schema: &EntitySchema, clauses: &[Clause]) -> Result<i32> {
    let query = build_query(schema, clauses)?;
    let searcher: IndexSearcher = open_searcher(root, schema)?;
    Ok(searcher.count(query)?)
}

/// The separator between the values of a multi-valued field in a TSV cell.
pub const VALUE_SEPARATOR: &str = " | ";

/// Writes `result` as tab-separated values: a `# total hits` line, a header
/// row, then one row per hit (requested fields, sort values, score).
///
/// Multi-valued fields are joined with [`VALUE_SEPARATOR`]; tabs and line
/// breaks inside values are replaced by spaces so every hit stays on one
/// row.
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
            .map(|(_, values)| clean(&values.join(VALUE_SEPARATOR)))
            .collect();
        row.extend(hit.sort_values.iter().map(|value| clean(value)));
        row.push(hit.score.to_string());
        writeln!(out, "{}", row.join("\t"))?;
    }
    Ok(())
}
