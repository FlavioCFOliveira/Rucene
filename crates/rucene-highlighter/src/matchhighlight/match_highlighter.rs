/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `MatchHighlighter.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};
use rucene::index::LeafReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::TopDocs;

use crate::java::utf16_len;
use crate::matchhighlight::match_region_retriever::{
    FieldHits, FieldPredicate, FieldValueProvider, MatchRegionRetriever,
};
use crate::matchhighlight::offset_range::OffsetRange;
use crate::matchhighlight::offsets_retrieval_strategy_supplier::OffsetsRetrievalStrategySupplier;

/// Actual per-field highlighter. Field highlighters are probed whether
/// they are applicable to a particular combination of (field, hasMatches)
/// pair. If a highlighter declares it is applicable, its `format` method is
/// invoked and the result is returned as the field's value.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.MatchHighlighter.FieldValueHighlighter`.
pub trait FieldValueHighlighter: Send + Sync {
    /// Check if this highlighter can be applied to a given field.
    ///
    /// Equivalent to `isApplicable(String, boolean)`.
    fn is_applicable(&self, field: &str, has_matches: bool) -> bool;

    /// Equivalent to `format(String, List, String, List, List)`;
    /// `match_offsets` is `None` for Java's `null` (no matches), and `None`
    /// is returned for Java's `null` (the field is skipped).
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the highlighter.
    fn format(
        &self,
        field: &str,
        values: &[String],
        contiguous_value: &str,
        value_ranges: &[OffsetRange],
        match_offsets: Option<&[QueryOffsetRange]>,
    ) -> Result<Option<Vec<String>>>;

    /// Returns a collection of fields that should be always returned by
    /// this highlighter.
    ///
    /// Equivalent to `alwaysFetchedFields()`.
    fn always_fetched_fields(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The anonymous highlighter of `FieldValueHighlighter.or`.
struct OrHighlighter {
    first: Arc<dyn FieldValueHighlighter>,
    second: Arc<dyn FieldValueHighlighter>,
    field_union: Vec<String>,
}

impl FieldValueHighlighter for OrHighlighter {
    fn is_applicable(&self, field: &str, has_matches: bool) -> bool {
        self.first.is_applicable(field, has_matches) || self.second.is_applicable(field, has_matches)
    }

    fn format(
        &self,
        field: &str,
        values: &[String],
        contiguous_value: &str,
        value_ranges: &[OffsetRange],
        match_offsets: Option<&[QueryOffsetRange]>,
    ) -> Result<Option<Vec<String>>> {
        let delegate = if self
            .first
            .is_applicable(field, match_offsets.is_some_and(|m| !m.is_empty()))
        {
            &self.first
        } else {
            &self.second
        };
        delegate.format(field, values, contiguous_value, value_ranges, match_offsets)
    }

    fn always_fetched_fields(&self) -> Vec<String> {
        self.field_union.clone()
    }
}

/// Returns a combined highlighter that applies `first` and falls back to
/// `second`.
///
/// Equivalent to the default method `FieldValueHighlighter.or(FieldValueHighlighter)`.
pub fn or(
    first: Arc<dyn FieldValueHighlighter>,
    second: Arc<dyn FieldValueHighlighter>,
) -> Arc<dyn FieldValueHighlighter> {
    let mut field_union: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for f in first
        .always_fetched_fields()
        .into_iter()
        .chain(second.always_fetched_fields())
    {
        if seen.insert(f.clone()) {
            field_union.push(f);
        }
    }
    Arc::new(OrHighlighter {
        first,
        second,
        field_union,
    })
}

/// Single document's highlights.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.MatchHighlighter.DocHighlights`;
/// `fields` is Java's `LinkedHashMap`, in insertion order.
#[derive(Debug, Clone)]
pub struct DocHighlights {
    /// Equivalent to the public final field `docId`.
    pub doc_id: i32,
    /// Equivalent to the public final field `fields`.
    pub fields: Vec<(String, Vec<String>)>,
}

impl DocHighlights {
    /// Equivalent to `new DocHighlights(int)`.
    pub fn new(doc_id: i32) -> Self {
        Self {
            doc_id,
            fields: Vec::new(),
        }
    }
}

/// An `OffsetRange` of a match, together with the source query that caused
/// it.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.MatchHighlighter.QueryOffsetRange`.
#[derive(Clone)]
pub struct QueryOffsetRange {
    /// Equivalent to the public final field `query`.
    pub query: Arc<dyn Query>,
    /// The inherited field `from`.
    pub from: i32,
    /// The inherited field `to`.
    pub to: i32,
}

impl fmt::Debug for QueryOffsetRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[from={}, to={}]", self.from, self.to)
    }
}

impl QueryOffsetRange {
    /// Equivalent to `new QueryOffsetRange(Query, int, int)`.
    pub fn new(query: Arc<dyn Query>, from: i32, to: i32) -> Self {
        debug_assert!(from <= to, "A non-empty offset range is required: {from}-{to}");
        Self { query, from, to }
    }

    /// Equivalent to `slice(int, int)`.
    pub fn slice(&self, from: i32, to: i32) -> QueryOffsetRange {
        QueryOffsetRange::new(Arc::clone(&self.query), from, to)
    }

    /// This range as an `OffsetRange` (Java's upcast), carrying the query.
    pub fn as_offset_range(&self) -> OffsetRange {
        OffsetRange::with_query(Arc::clone(&self.query), self.from, self.to)
    }
}

/// Equivalent to the private nested class `MatchHighlighter.DocHit`.
pub(crate) struct DocHit {
    pub(crate) doc_id: i32,
    match_ranges: Vec<(String, Vec<QueryOffsetRange>)>,
    field_values: Vec<(String, Vec<String>)>,
}

impl DocHit {
    fn new(doc_id: i32, field_value_provider: &dyn FieldValueProvider) -> Self {
        let mut field_values = Vec::new();
        for field_name in field_value_provider.field_names() {
            let values = field_value_provider.get_values(&field_name);
            field_values.push((field_name, values));
        }
        Self {
            doc_id,
            match_ranges: Vec::new(),
            field_values,
        }
    }

    fn add_matches(&mut self, query: &Arc<dyn Query>, hits: &FieldHits) {
        for (field, offsets) in hits.iter() {
            let index = match self.match_ranges.iter().position(|(f, _)| f == field) {
                Some(index) => index,
                None => {
                    self.match_ranges.push((field.clone(), Vec::new()));
                    self.match_ranges.len() - 1
                }
            };
            let target = &mut self.match_ranges[index].1;
            for o in offsets {
                target.push(QueryOffsetRange::new(Arc::clone(query), o.from, o.to));
            }
        }
    }
}

/// An example highlighter that combines several lower-level highlighting
/// utilities in this package into a fully featured, ready-to-use component.
///
/// Note that if you need to customize or tweak the details of highlighting,
/// it is better to assemble your own highlighter using those low-level
/// building blocks, rather than extend or modify this one.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.MatchHighlighter`.
pub struct MatchHighlighter {
    searcher: Arc<IndexSearcher>,
    offsets_retrieval_strategies: Arc<dyn OffsetsRetrievalStrategySupplier>,
    analyzer: Arc<dyn Analyzer>,

    fields_always_returned: HashSet<String>,
    field_highlighters: Vec<Arc<dyn FieldValueHighlighter>>,
}

impl MatchHighlighter {
    /// Equivalent to `new MatchHighlighter(IndexSearcher, Analyzer)`.
    ///
    /// # Errors
    ///
    /// What merging the field infos raises.
    pub fn new(searcher: Arc<IndexSearcher>, analyzer: Arc<dyn Analyzer>) -> Result<Self> {
        let strategies = MatchRegionRetriever::compute_offset_retrieval_strategies(
            Arc::clone(searcher.get_index_reader()),
            Arc::clone(&analyzer),
        )?;
        Ok(Self::with_strategies(searcher, analyzer, strategies))
    }

    /// Equivalent to `new MatchHighlighter(IndexSearcher, Analyzer,
    /// OffsetsRetrievalStrategySupplier)`.
    pub fn with_strategies(
        searcher: Arc<IndexSearcher>,
        analyzer: Arc<dyn Analyzer>,
        offsets_retrieval_strategies: Arc<dyn OffsetsRetrievalStrategySupplier>,
    ) -> Self {
        Self {
            searcher,
            offsets_retrieval_strategies,
            analyzer,
            fields_always_returned: HashSet::new(),
            field_highlighters: Vec::new(),
        }
    }

    /// Append a new highlighter to field highlighters chain. The order of
    /// field highlighters is important (first-matching wins).
    ///
    /// Equivalent to `appendFieldHighlighter(FieldValueHighlighter)`.
    pub fn append_field_highlighter(&mut self, highlighter: Arc<dyn FieldValueHighlighter>) -> &mut Self {
        self.fields_always_returned
            .extend(highlighter.always_fetched_fields());
        self.field_highlighters.push(highlighter);
        self
    }

    /// Always fetch the given set of fields for all input documents.
    ///
    /// Equivalent to `alwaysFetchFields(String...)`.
    pub fn always_fetch_fields(&mut self, fields: &[&str]) {
        for fld in fields {
            self.fields_always_returned.insert((*fld).to_string());
        }
    }

    /// Single document's highlights.
    ///
    /// Equivalent to `highlight(TopDocs, Query...)`: the documents in the
    /// order of `top_docs` (Java returns a `Stream`).
    ///
    /// # Errors
    ///
    /// Propagates I/O errors; Java's `RuntimeException` when no field
    /// highlighter matches a field.
    pub fn highlight(&self, top_docs: &TopDocs, queries: &[Arc<dyn Query>]) -> Result<Vec<DocHighlights>> {
        // We want to preserve the order of documents in TopDocs on output.
        // Remap.
        let mut doc_hits: Vec<(i32, Option<DocHit>)> = Vec::new();
        for score_doc in &top_docs.score_docs {
            if !doc_hits.iter().any(|(d, _)| *d == score_doc.doc) {
                doc_hits.push((score_doc.doc, None));
            }
        }

        let always = self.fields_always_returned.clone();
        let fields_to_load_unconditionally: FieldPredicate = Arc::new(move |f: &str| always.contains(f));
        let highlighters = self.field_highlighters.clone();
        let fields_to_load_if_with_hits: FieldPredicate = Arc::new(move |field_name: &str| {
            highlighters.iter().any(|highlighter| {
                highlighter.is_applicable(field_name, true) || highlighter.is_applicable(field_name, false)
            })
        });

        // Compute match highlights for all queries.
        for q in queries {
            let highlighter = MatchRegionRetriever::with_supplier(
                Arc::clone(&self.searcher),
                self.searcher.rewrite(Arc::clone(q))?,
                Arc::clone(&self.offsets_retrieval_strategies),
                Arc::clone(&fields_to_load_unconditionally),
                Arc::clone(&fields_to_load_if_with_hits),
            )?;
            let mut consumer = |doc_id: i32,
                                _leaf_reader: &Arc<dyn LeafReader>,
                                _leaf_doc_id: i32,
                                field_value_provider: &dyn FieldValueProvider,
                                hits: &FieldHits|
             -> Result<()> {
                let slot = match doc_hits.iter().position(|(d, _)| *d == doc_id) {
                    Some(i) => i,
                    None => {
                        doc_hits.push((doc_id, None));
                        doc_hits.len() - 1
                    }
                };
                let doc_hit = doc_hits[slot]
                    .1
                    .get_or_insert_with(|| DocHit::new(doc_id, field_value_provider));
                doc_hit.add_matches(q, hits);
                Ok(())
            };
            highlighter.highlight_documents(top_docs, &mut consumer)?;
        }

        let mut result = Vec::new();
        for (_, doc_hit) in doc_hits {
            // This should always the case?
            if let Some(doc_hit) = doc_hit {
                result.push(self.compute_doc_field_values(&doc_hit)?);
            }
        }
        Ok(result)
    }

    fn compute_doc_field_values(&self, doc_hit: &DocHit) -> Result<DocHighlights> {
        let mut doc_highlights = DocHighlights::new(doc_hit.doc_id);

        for (field, values) in &doc_hit.field_values {
            let contiguous_value = self.contiguous_field_value(field, values);
            let value_ranges = self.compute_value_ranges(field, values);
            let offsets = doc_hit
                .match_ranges
                .iter()
                .find(|(f, _)| f == field)
                .map(|(_, o)| o.as_slice());

            let formatted_values = self.field_value_highlighter(field, offsets.is_some())?.format(
                field,
                values,
                &contiguous_value,
                &value_ranges,
                offsets,
            )?;

            if let Some(formatted_values) = formatted_values {
                doc_highlights.fields.push((field.clone(), formatted_values));
            }
        }

        Ok(doc_highlights)
    }

    fn compute_value_ranges(&self, field: &str, values: &[String]) -> Vec<OffsetRange> {
        let mut value_ranges = Vec::new();
        let mut offset = 0;
        for v in values {
            let len = utf16_len(v);
            value_ranges.push(OffsetRange::new(offset, offset + len));
            offset += len;
            offset += self.analyzer.get_offset_gap(Some(field));
        }
        value_ranges
    }

    fn contiguous_field_value(&self, field: &str, values: &[String]) -> String {
        if values.len() == 1 {
            values[0].clone()
        } else {
            // TODO: This can be inefficient if offset gap is large but the
            // logic of applying offsets would get much more complicated so
            // leaving for now (would have to recalculate all offsets to
            // omit gaps).
            let field_gap_padding = " ".repeat(self.analyzer.get_offset_gap(Some(field)).max(0) as usize);
            values.join(&field_gap_padding)
        }
    }

    fn field_value_highlighter(&self, field: &str, has_matches: bool) -> Result<&Arc<dyn FieldValueHighlighter>> {
        for highlighter in &self.field_highlighters {
            if highlighter.is_applicable(field, has_matches) {
                return Ok(highlighter);
            }
        }
        Err(LuceneError::unchecked(
            "java.lang.RuntimeException",
            format!("No field highlighter could be matched to field: {field}"),
        ))
    }
}