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

//! Port of `MatchRegionRetriever.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};
use rucene::index::{
    FieldInfo, FieldInfos, IndexOptions, IndexReader, LeafReader, LeafReaderContext,
    StoredFieldVisitor, StoredFieldVisitorStatus,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::weight::Weight;
use rucene::search::TopDocs;
use rucene::util::java_fmt::{double_to_string, float_to_string};

use crate::matchhighlight::offset_range::OffsetRange;
use crate::matchhighlight::offsets_from_match_iterator::OffsetsFromMatchIterator;
use crate::matchhighlight::offsets_from_positions::OffsetsFromPositions;
use crate::matchhighlight::offsets_from_tokens::OffsetsFromTokens;
use crate::matchhighlight::offsets_retrieval_strategy::OffsetsRetrievalStrategy;
use crate::matchhighlight::offsets_retrieval_strategy_supplier::OffsetsRetrievalStrategySupplier;

/// `String.compareTo`: the UTF-16 code unit order.
fn java_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Java's `TreeMap<String, List<OffsetRange>>` of the highlights of one
/// document: entries sorted by field name (`String.compareTo`).
#[derive(Debug, Clone, Default)]
pub struct FieldHits {
    entries: Vec<(String, Vec<OffsetRange>)>,
}

impl FieldHits {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// `put(field, ranges)`.
    pub fn put(&mut self, field: &str, ranges: Vec<OffsetRange>) {
        match self
            .entries
            .binary_search_by(|(k, _)| java_compare(k, field))
        {
            Ok(i) => self.entries[i].1 = ranges,
            Err(i) => self.entries.insert(i, (field.to_string(), ranges)),
        }
    }

    /// `get(field)`.
    pub fn get(&self, field: &str) -> Option<&Vec<OffsetRange>> {
        self.entries
            .binary_search_by(|(k, _)| java_compare(k, field))
            .ok()
            .map(|i| &self.entries[i].1)
    }

    /// The entries in key order (`entrySet()`).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Vec<OffsetRange>)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Implement this interface to provide field values to the highlighting
/// code.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.MatchRegionRetriever.FieldValueProvider`,
/// an `Iterable<String>` of field names.
pub trait FieldValueProvider: Send + Sync {
    /// Equivalent to `getValues(String)`: the field's values, empty when
    /// absent.
    fn get_values(&self, field: &str) -> Vec<String>;

    /// Equivalent to `iterator()`: the field names.
    fn field_names(&self) -> Vec<String>;
}

/// A callback for accepting a single document (and its associated leaf
/// reader, leaf document ID) and its match offset ranges, as indicated by
/// the `Matches` interface retrieved for the query.
///
/// Equivalent to the functional interface
/// `org.apache.lucene.search.matchhighlight.MatchRegionRetriever.MatchOffsetsConsumer`.
pub trait MatchOffsetsConsumer {
    /// Equivalent to `accept(int, LeafReader, int, FieldValueProvider,
    /// Map)`.
    ///
    /// # Errors
    ///
    /// What the consumer raises.
    fn accept(
        &mut self,
        doc_id: i32,
        leaf_reader: &Arc<dyn LeafReader>,
        leaf_doc_id: i32,
        field_value_provider: &dyn FieldValueProvider,
        hits: &FieldHits,
    ) -> Result<()>;
}

impl<F> MatchOffsetsConsumer for F
where
    F: FnMut(i32, &Arc<dyn LeafReader>, i32, &dyn FieldValueProvider, &FieldHits) -> Result<()>,
{
    fn accept(
        &mut self,
        doc_id: i32,
        leaf_reader: &Arc<dyn LeafReader>,
        leaf_doc_id: i32,
        field_value_provider: &dyn FieldValueProvider,
        hits: &FieldHits,
    ) -> Result<()> {
        self(doc_id, leaf_reader, leaf_doc_id, field_value_provider, hits)
    }
}

/// `ToIntFunction<String>`: the maximum hits per field.
pub type MaxHitsPerField = Arc<dyn Fn(&str) -> i32 + Send + Sync>;

/// `Predicate<String>`.
pub type FieldPredicate = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The state the blocks of `highlightDocuments` share across threads.
struct Inner {
    leaves: Vec<Arc<LeafReaderContext>>,
    weight: Arc<dyn Weight>,
    offset_strategies: HashMap<String, Arc<dyn OffsetsRetrievalStrategy>>,
    query_affected_highlighted_fields: Vec<String>,
    should_load_stored_field: FieldPredicate,
}

/// Utility class to compute a list of "match regions" for a given query,
/// searcher and document(s) using `Matches` API.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.MatchRegionRetriever`.
pub struct MatchRegionRetriever {
    inner: Arc<Inner>,
    searcher: Arc<IndexSearcher>,
}

/// A `QueryVisitor` whose `acceptField` records the field: Java's
/// `acceptField` adds to the set and returns `false`; Rust's takes `&self`,
/// so the set is behind a lock.
struct RecordingFieldCollector<'a> {
    fields_to_load_if_with_hits: &'a FieldPredicate,
    fields: std::sync::Mutex<Vec<String>>,
}

impl QueryVisitor for RecordingFieldCollector<'_> {
    fn accept_field(&self, field: &str) -> bool {
        if (self.fields_to_load_if_with_hits)(field) {
            let mut fields = self
                .fields
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Err(i) = fields.binary_search_by(|f| java_compare(f, field)) {
                fields.insert(i, field.to_string());
            }
        }
        false
    }
}

impl MatchRegionRetriever {
    /// A constructor with the default offset strategy supplier.
    ///
    /// Equivalent to `new MatchRegionRetriever(IndexSearcher, Query,
    /// Analyzer, Predicate, Predicate)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn new(
        searcher: Arc<IndexSearcher>,
        query: Arc<dyn Query>,
        analyzer: Arc<dyn Analyzer>,
        fields_to_load_unconditionally: FieldPredicate,
        fields_to_load_if_with_hits: FieldPredicate,
    ) -> Result<Self> {
        let supplier =
            Self::compute_offset_retrieval_strategies(Arc::clone(searcher.get_index_reader()), analyzer)?;
        Self::with_supplier(
            searcher,
            query,
            supplier,
            fields_to_load_unconditionally,
            fields_to_load_if_with_hits,
        )
    }

    /// Equivalent to `new MatchRegionRetriever(IndexSearcher, Query,
    /// OffsetsRetrievalStrategySupplier, Predicate, Predicate)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn with_supplier(
        searcher: Arc<IndexSearcher>,
        query: Arc<dyn Query>,
        field_offset_strategy_supplier: Arc<dyn OffsetsRetrievalStrategySupplier>,
        fields_to_load_unconditionally: FieldPredicate,
        fields_to_load_if_with_hits: FieldPredicate,
    ) -> Result<Self> {
        let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
        debug_assert!(Self::check_order_consistency(&leaves));

        // We need full scoring mode so that we can receive matches from all
        // sub-clauses (no optimizations in Boolean queries take place).
        let weight = searcher.create_weight(Arc::clone(&query), ScoreMode::COMPLETE, 0.0)?;

        // Compute the subset of fields affected by this query so that we
        // don't load or scan fields that are irrelevant.
        let collector = RecordingFieldCollector {
            fields_to_load_if_with_hits: &fields_to_load_if_with_hits,
            fields: std::sync::Mutex::new(Vec::new()),
        };
        let mut collector = collector;
        query.visit(&mut collector);
        let query_affected_highlighted_fields = collector
            .fields
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Compute value offset retrieval strategy for all affected fields.
        let mut offset_strategies: HashMap<String, Arc<dyn OffsetsRetrievalStrategy>> = HashMap::new();
        for field in &query_affected_highlighted_fields {
            offset_strategies.insert(field.clone(), field_offset_strategy_supplier.apply(field));
        }

        let affected = query_affected_highlighted_fields.clone();
        let should_load_stored_field: FieldPredicate = Arc::new(move |field: &str| {
            fields_to_load_unconditionally(field)
                || affected.binary_search_by(|f| java_compare(f, field)).is_ok()
        });

        Ok(Self {
            inner: Arc::new(Inner {
                leaves,
                weight,
                offset_strategies,
                query_affected_highlighted_fields,
                should_load_stored_field,
            }),
            searcher,
        })
    }

    /// Processes all documents in the top documents, in increasing document
    /// ID order.
    ///
    /// Equivalent to `highlightDocuments(TopDocs, MatchOffsetsConsumer)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn highlight_documents(&self, top_docs: &TopDocs, consumer: &mut dyn MatchOffsetsConsumer) -> Result<()> {
        let mut doc_ids: Vec<i32> = top_docs.score_docs.iter().map(|sd| sd.doc).collect();
        doc_ids.sort_unstable();
        self.highlight_documents_with_limits(&doc_ids, consumer, Arc::new(|_| i32::MAX))
    }

    /// Low-level method for retrieving hit regions for documents.
    ///
    /// Equivalent to `highlightDocuments(PrimitiveIterator.OfInt,
    /// MatchOffsetsConsumer, ToIntFunction)`: blocks of 50 documents, as
    /// many in parallel as `ForkJoinPool.getCommonPoolParallelism()` (the
    /// available processors minus one, at least one).
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn highlight_documents_with_limits(
        &self,
        doc_ids: &[i32],
        consumer: &mut dyn MatchOffsetsConsumer,
        max_hits_per_field: MaxHitsPerField,
    ) -> Result<()> {
        const DEFAULT_MAX_BLOCK_SIZE: usize = 50;
        let parallelism = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .saturating_sub(1)
            .max(1);
        self.highlight_documents_in_blocks(
            doc_ids,
            consumer,
            max_hits_per_field,
            DEFAULT_MAX_BLOCK_SIZE,
            parallelism,
        )
    }

    /// Low-level method for retrieving hit regions for documents.
    ///
    /// Equivalent to `highlightDocuments(PrimitiveIterator.OfInt,
    /// MatchOffsetsConsumer, ToIntFunction, int, int)`: `doc_ids` must be
    /// sorted (increasing).
    ///
    /// # Errors
    ///
    /// Propagates I/O errors; Java's `RuntimeException` for unsorted
    /// document IDs.
    pub fn highlight_documents_in_blocks(
        &self,
        doc_ids: &[i32],
        consumer: &mut dyn MatchOffsetsConsumer,
        max_hits_per_field: MaxHitsPerField,
        max_block_size: usize,
        max_blocks_processed_in_parallel: usize,
    ) -> Result<()> {
        if self.inner.leaves.is_empty() {
            return Ok(());
        }

        let mut block_queue: Vec<Vec<i32>> = Vec::new();
        let mut previous_doc_id = -1;
        let mut block: Vec<i32> = Vec::with_capacity(max_block_size);
        let mut iter = doc_ids.iter().peekable();
        while let Some(&doc_id) = iter.next() {
            if doc_id < previous_doc_id {
                return Err(LuceneError::unchecked(
                    "java.lang.RuntimeException",
                    "Input document IDs must be sorted (increasing).",
                ));
            }
            previous_doc_id = doc_id;
            block.push(doc_id);
            if block.len() >= max_block_size || iter.peek().is_none() {
                block_queue.push(std::mem::take(&mut block));
                if block_queue.len() >= max_blocks_processed_in_parallel {
                    self.drain_queue(&mut block_queue, consumer, &max_hits_per_field, max_blocks_processed_in_parallel)?;
                }
            }
        }

        if !block_queue.is_empty() {
            self.drain_queue(&mut block_queue, consumer, &max_hits_per_field, max_blocks_processed_in_parallel)?;
        }
        Ok(())
    }

    fn drain_queue(
        &self,
        queue: &mut Vec<Vec<i32>>,
        consumer: &mut dyn MatchOffsetsConsumer,
        max_hits_per_field: &MaxHitsPerField,
        max_blocks_processed_in_parallel: usize,
    ) -> Result<()> {
        if max_blocks_processed_in_parallel == 1 {
            for id_block in queue.iter() {
                // `catch (Exception e) { throw new IOException(e); }`
                Inner::prepare_block(&self.inner, id_block, max_hits_per_field)
                    .and_then(|data| Self::process_block(&data, consumer))
                    .map_err(|e| LuceneError::from(std::io::Error::other(e.to_string())))?;
            }
        } else {
            let callables: Vec<Box<dyn FnOnce() -> Result<Vec<DocHighlightData>> + Send + 'static>> = queue
                .iter()
                .map(|id_block| {
                    let inner = Arc::clone(&self.inner);
                    let id_block = id_block.clone();
                    let max_hits = Arc::clone(max_hits_per_field);
                    Box::new(move || Inner::prepare_block(&inner, &id_block, &max_hits))
                        as Box<dyn FnOnce() -> Result<Vec<DocHighlightData>> + Send + 'static>
                })
                .collect();
            for highlight_data in self.searcher.get_task_executor().invoke_all(callables)? {
                Self::process_block(&highlight_data, consumer)?;
            }
        }
        queue.clear();
        Ok(())
    }

    fn process_block(doc_highlight_data: &[DocHighlightData], consumer: &mut dyn MatchOffsetsConsumer) -> Result<()> {
        for data in doc_highlight_data {
            consumer.accept(
                data.doc_id,
                &data.leaf_reader,
                data.leaf_doc_id,
                &data.field_value_provider,
                &data.hits,
            )?;
        }
        Ok(())
    }

    /// Low-level method for retrieving hit regions for a single document.
    ///
    /// Equivalent to `highlightDocument(LeafReaderContext, int,
    /// FieldValueProvider, ToIntFunction, Map)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn highlight_document(
        &self,
        leaf_reader_context: &LeafReaderContext,
        context_doc_id: i32,
        doc: &dyn FieldValueProvider,
        max_hits_per_field: &MaxHitsPerField,
        output_highlights: &mut FieldHits,
    ) -> Result<()> {
        self.inner.highlight_document(
            leaf_reader_context,
            context_doc_id,
            doc,
            max_hits_per_field,
            output_highlights,
        )
    }

    fn check_order_consistency(leaves: &[Arc<LeafReaderContext>]) -> bool {
        for i in 1..leaves.len() {
            let prev = &leaves[i - 1];
            let next = &leaves[i];
            debug_assert!(prev.doc_base() <= next.doc_base());
            debug_assert!(prev.doc_base() + prev.leaf_reader().max_doc() == next.doc_base());
        }
        true
    }

    /// Compute default strategies for retrieving offsets from
    /// `MatchesIterator` instances for a set of given fields.
    ///
    /// Equivalent to the static `computeOffsetRetrievalStrategies(IndexReader,
    /// Analyzer)`.
    ///
    /// # Errors
    ///
    /// What merging the field infos raises.
    pub fn compute_offset_retrieval_strategies(
        reader: Arc<dyn IndexReader>,
        analyzer: Arc<dyn Analyzer>,
    ) -> Result<Arc<dyn OffsetsRetrievalStrategySupplier>> {
        let field_infos: FieldInfos = FieldInfos::get_merged_field_infos(reader)?;
        Ok(Arc::new(move |field: &str| -> Arc<dyn OffsetsRetrievalStrategy> {
            let Some(field_info) = field_infos.field_info(field) else {
                let message = format!("FieldInfo is null for field: {field}");
                return Arc::new(move |_: &mut dyn MatchesIterator, _: &dyn FieldValueProvider| {
                    Err(LuceneError::from(std::io::Error::other(message.clone())))
                });
            };

            match field_info.get_index_options() {
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS => Arc::new(OffsetsFromMatchIterator::new(
                    field,
                    OffsetsFromPositions::new(field, Arc::clone(&analyzer)),
                )),
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS => {
                    Arc::new(OffsetsFromPositions::new(field, Arc::clone(&analyzer)))
                }
                IndexOptions::DOCS_AND_CUSTOM_FREQS | IndexOptions::DOCS_AND_FREQS | IndexOptions::DOCS => {
                    // By default use overlapping offsets from tokens.
                    Arc::new(OffsetsFromTokens::new(field, Arc::clone(&analyzer)))
                }
                other => {
                    let message = format!(
                        "Field is indexed without positions and/or offsets: {field}, {other:?}"
                    );
                    Arc::new(move |_: &mut dyn MatchesIterator, _: &dyn FieldValueProvider| {
                        Err(LuceneError::from(std::io::Error::other(message.clone())))
                    })
                }
            }
        }))
    }
}

impl Inner {
    fn prepare_block(
        this: &Arc<Inner>,
        id_block: &[i32],
        max_hits_per_field: &MaxHitsPerField,
    ) -> Result<Vec<DocHighlightData>> {
        let mut doc_data: Vec<DocHighlightData> = Vec::with_capacity(id_block.len());

        let mut ctx = this.leaves.iter();
        let mut current_context = ctx
            .next()
            .ok_or_else(|| LuceneError::unchecked("java.util.NoSuchElementException", ""))?;
        let mut reader = current_context.leaf_reader();
        for &doc_id in id_block {
            while doc_id >= current_context.doc_base() + reader.max_doc() {
                current_context = ctx
                    .next()
                    .ok_or_else(|| LuceneError::unchecked("java.util.NoSuchElementException", ""))?;
                reader = current_context.leaf_reader();
            }

            let context_relative_doc_id = doc_id - current_context.doc_base();

            // Only preload fields we may potentially need.
            let mut field_visitor = StoredFieldsVisitor::new(Arc::clone(&this.should_load_stored_field));
            let stored_fields = reader.stored_fields()?;
            stored_fields.document_with_visitor(context_relative_doc_id, &mut field_visitor)?;

            let mut highlights = FieldHits::new();
            this.highlight_document(
                current_context,
                context_relative_doc_id,
                &field_visitor,
                max_hits_per_field,
                &mut highlights,
            )?;
            doc_data.push(DocHighlightData {
                doc_id,
                leaf_reader: Arc::clone(&reader),
                leaf_doc_id: context_relative_doc_id,
                field_value_provider: field_visitor,
                hits: highlights,
            });
        }
        Ok(doc_data)
    }

    fn highlight_document(
        &self,
        leaf_reader_context: &LeafReaderContext,
        context_doc_id: i32,
        doc: &dyn FieldValueProvider,
        max_hits_per_field: &MaxHitsPerField,
        output_highlights: &mut FieldHits,
    ) -> Result<()> {
        let Some(matches) = self.weight.matches(leaf_reader_context, context_doc_id)? else {
            return Ok(()); // No matches for this doc.
        };

        for field in &self.query_affected_highlighted_fields {
            let Some(mut matches_iterator) = matches.get_matches(field)? else {
                // No matches on this field, even though the field was part
                // of the query. This may be possible with complex queries
                // that source non-text fields (have no "hit regions" in any
                // textual representation). Skip.
                continue;
            };
            let Some(offset_strategy) = self.offset_strategies.get(field) else {
                return Err(LuceneError::from(std::io::Error::other(format!(
                    "Non-empty matches but no offset retrieval strategy for field: {field}"
                ))));
            };
            let max_hits = max_hits_per_field(field);
            let ranges = if max_hits != i32::MAX {
                let mut limited = MatchesIteratorWithLimit::new(&mut *matches_iterator, max_hits)?;
                offset_strategy.get(&mut limited, doc)?
            } else {
                offset_strategy.get(&mut *matches_iterator, doc)?
            };
            if !ranges.is_empty() {
                output_highlights.put(field, ranges);
            }
        }
        Ok(())
    }
}

/// The data of one highlighted document.
///
/// Equivalent to the private record `MatchRegionRetriever.DocHighlightData`.
pub(crate) struct DocHighlightData {
    doc_id: i32,
    leaf_reader: Arc<dyn LeafReader>,
    leaf_doc_id: i32,
    field_value_provider: StoredFieldsVisitor,
    hits: FieldHits,
}

/// Equivalent to the private nested class
/// `MatchRegionRetriever.MatchesIteratorWithLimit`, a `FilterMatchesIterator`
/// over an iterator it only borrows.
pub(crate) struct MatchesIteratorWithLimit<'a> {
    inner: &'a mut dyn MatchesIterator,
    limit: i32,
}

impl<'a> MatchesIteratorWithLimit<'a> {
    fn new(matches_iterator: &'a mut dyn MatchesIterator, limit: i32) -> Result<Self> {
        if limit < 0 {
            return Err(LuceneError::illegal_argument(String::new()));
        }
        Ok(Self {
            inner: matches_iterator,
            limit,
        })
    }
}

impl MatchesIterator for MatchesIteratorWithLimit<'_> {
    fn next(&mut self) -> Result<bool> {
        if self.limit == 0 {
            return Ok(false);
        }
        self.limit -= 1;
        self.inner.next()
    }

    fn start_position(&self) -> i32 {
        self.inner.start_position()
    }

    fn end_position(&self) -> i32 {
        self.inner.end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.inner.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.inner.end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.inner.get_sub_matches()
    }

    fn get_query(&self) -> Option<Arc<dyn Query>> {
        self.inner.get_query()
    }
}

/// This field value provider loads the values of the fields it is asked
/// for, keyed in the order they are visited.
///
/// Equivalent to the private nested class
/// `MatchRegionRetriever.StoredFieldsVisitor`.
pub(crate) struct StoredFieldsVisitor {
    needs_field: FieldPredicate,
    field_values: Vec<(String, Vec<String>)>,
}

impl StoredFieldsVisitor {
    fn new(should_load_stored_field: FieldPredicate) -> Self {
        Self {
            needs_field: should_load_stored_field,
            field_values: Vec::new(),
        }
    }

    fn add_field(&mut self, field: &FieldInfo, value: String) {
        match self.field_values.iter_mut().find(|(k, _)| *k == field.name) {
            Some((_, values)) => values.push(value),
            None => self.field_values.push((field.name.clone(), vec![value])),
        }
    }
}

impl FieldValueProvider for StoredFieldsVisitor {
    fn get_values(&self, field: &str) -> Vec<String> {
        self.field_values
            .iter()
            .find(|(k, _)| k == field)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    fn field_names(&self) -> Vec<String> {
        self.field_values.iter().map(|(k, _)| k.clone()).collect()
    }
}

impl StoredFieldVisitor for StoredFieldsVisitor {
    fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        Ok(if (self.needs_field)(&field_info.name) {
            StoredFieldVisitorStatus::Yes
        } else {
            StoredFieldVisitorStatus::No
        })
    }

    fn string_field(&mut self, field_info: &Arc<FieldInfo>, value: String) -> Result<()> {
        self.add_field(field_info, value);
        Ok(())
    }

    fn int_field(&mut self, field_info: &Arc<FieldInfo>, value: i32) -> Result<()> {
        self.add_field(field_info, value.to_string());
        Ok(())
    }

    fn long_field(&mut self, field_info: &Arc<FieldInfo>, value: i64) -> Result<()> {
        self.add_field(field_info, value.to_string());
        Ok(())
    }

    fn float_field(&mut self, field_info: &Arc<FieldInfo>, value: f32) -> Result<()> {
        self.add_field(field_info, float_to_string(value));
        Ok(())
    }

    fn double_field(&mut self, field_info: &Arc<FieldInfo>, value: f64) -> Result<()> {
        self.add_field(field_info, double_to_string(value));
        Ok(())
    }
}

