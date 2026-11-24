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

//! Port of `LongValueFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::fmt;
use std::sync::Arc;

use rucene::document::NumericValue;
use rucene::error::{LuceneError, Result};
use rucene::index::{DocValues, IndexReader};
use rucene::internal::hppc::LongIntHashMap;
use rucene::search::long_values_source::LongValuesSource;
use rucene::search::NO_MORE_DOCS;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};

use crate::facet_result::FacetResult;
use crate::facet_utils::{unwrap_singleton_numeric, Intersect2, NumericValuesView};
use crate::facets::{validate_top_n, Facets};
use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::label_and_value::LabelAndValue;
use crate::multi_long_values_source::MultiLongValuesSource;

/// `FacetResult` built by this class: no path, the total count.
fn result(
    field: &str,
    tot_count: i32,
    label_values: Vec<LabelAndValue>,
    child_count: i32,
) -> FacetResult {
    FacetResult::new(
        field,
        Vec::new(),
        NumericValue::Int(tot_count),
        label_values,
        child_count,
    )
}

/// `Facets` implementation that computes counts for all unique long values,
/// more efficiently counting small values (0-1023) using an int array, and
/// switching to a `HashMap` for values above 1023.
///
/// Equivalent to `org.apache.lucene.facet.LongValueFacetCounts`.
pub struct LongValueFacetCounts {
    /// Used for all values that are < 1K.
    counts: Option<Vec<i32>>,
    /// Used for all values that are >= 1K.
    hash_counts: Option<LongIntHashMap>,
    /// Whether-or-not counters have been initialized.
    initialized: bool,
    /// Field being counted.
    field: String,
    /// Total number of hits that were counted.
    tot_count: i32,
}

/// Holds a single entry.
///
/// Equivalent to `org.apache.lucene.facet.LongValueFacetCounts.Entry`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Entry {
    count: i32,
    value: i64,
}

struct EntryLessThan;

impl PriorityQueueComparator<Entry> for EntryLessThan {
    fn less_than(&self, a: &Entry, b: &Entry) -> bool {
        // sort by count descending, breaking ties by value ascending:
        a.count < b.count || (a.count == b.count && a.value > b.value)
    }
}

impl LongValueFacetCounts {
    fn empty(field: &str) -> Self {
        Self {
            counts: None,
            hash_counts: None,
            initialized: false,
            field: field.to_string(),
            tot_count: 0,
        }
    }

    /// Create `LongValueFacetCounts`, using either single-valued
    /// `NumericDocValues` or multi-valued `SortedNumericDocValues` from the
    /// specified field.
    ///
    /// Equivalent to `LongValueFacetCounts(String, FacetsCollector)`.
    pub fn new(field: &str, hits: &FacetsCollector) -> Result<Self> {
        Self::with_values_source(field, None, hits)
    }

    /// Create `LongValueFacetCounts`, using the provided `LongValuesSource`.
    ///
    /// Equivalent to `LongValueFacetCounts(String, LongValuesSource,
    /// FacetsCollector)`.
    pub fn with_values_source(
        field: &str,
        value_source: Option<Arc<dyn LongValuesSource>>,
        hits: &FacetsCollector,
    ) -> Result<Self> {
        let mut this = Self::empty(field);
        match value_source {
            Some(value_source) => this.count_source(value_source, hits.get_matching_docs()?)?,
            None => this.count_field(field, hits.get_matching_docs()?)?,
        }
        Ok(this)
    }

    /// Create `LongValueFacetCounts`, using the provided
    /// `MultiLongValuesSource`.
    ///
    /// Equivalent to `LongValueFacetCounts(String, MultiLongValuesSource,
    /// FacetsCollector)`.
    pub fn with_multi_values_source(
        field: &str,
        values_source: Option<Arc<dyn MultiLongValuesSource>>,
        hits: &FacetsCollector,
    ) -> Result<Self> {
        let mut this = Self::empty(field);
        match values_source {
            Some(values_source) => {
                match <dyn MultiLongValuesSource>::unwrap_singleton(&*values_source) {
                    Some(single_values) => {
                        this.count_source(single_values, hits.get_matching_docs()?)?
                    }
                    None => this.count_multi_source(&*values_source, hits.get_matching_docs()?)?,
                }
            }
            None => this.count_field(field, hits.get_matching_docs()?)?,
        }
        Ok(this)
    }

    /// Counts all facet values for this reader.
    ///
    /// Equivalent to `LongValueFacetCounts(String, IndexReader)`.
    pub fn count_all_docs(field: &str, reader: &Arc<dyn IndexReader>) -> Result<Self> {
        Self::count_all_docs_with_source(field, None, reader)
    }

    /// Counts all facet values for the provided `LongValuesSource`.
    ///
    /// Equivalent to `LongValueFacetCounts(String, LongValuesSource,
    /// IndexReader)`.
    pub fn count_all_docs_with_source(
        field: &str,
        value_source: Option<Arc<dyn LongValuesSource>>,
        reader: &Arc<dyn IndexReader>,
    ) -> Result<Self> {
        let mut this = Self::empty(field);
        this.initialize_counters();
        match value_source {
            Some(value_source) => this.count_all_source(reader, value_source)?,
            None => this.count_all_field(reader, field)?,
        }
        Ok(this)
    }

    /// Counts all facet values for the provided `MultiLongValuesSource`.
    ///
    /// Equivalent to `LongValueFacetCounts(String, MultiLongValuesSource,
    /// IndexReader)`.
    pub fn count_all_docs_with_multi_source(
        field: &str,
        values_source: Option<Arc<dyn MultiLongValuesSource>>,
        reader: &Arc<dyn IndexReader>,
    ) -> Result<Self> {
        let mut this = Self::empty(field);
        this.initialize_counters();
        match values_source {
            Some(values_source) => {
                match <dyn MultiLongValuesSource>::unwrap_singleton(&*values_source) {
                    Some(single_valued) => this.count_all_source(reader, single_valued)?,
                    None => this.count_all_multi_source(reader, &*values_source)?,
                }
            }
            None => this.count_all_field(reader, field)?,
        }
        Ok(this)
    }

    /// Equivalent to `initializeCounters()`.
    fn initialize_counters(&mut self) {
        if self.initialized {
            return;
        }
        debug_assert!(self.counts.is_none() && self.hash_counts.is_none());
        self.initialized = true;
        self.counts = Some(vec![0; 1024]);
        self.hash_counts = Some(LongIntHashMap::new());
    }

    /// Optimized version that directly counts a single-valued
    /// `LongValuesSource`.
    ///
    /// Equivalent to `count(LongValuesSource, List)`.
    fn count_source(
        &mut self,
        value_source: Arc<dyn LongValuesSource>,
        matching_docs: &[MatchingDocs],
    ) -> Result<()> {
        for hits in matching_docs {
            if hits.total_hits == 0 {
                continue;
            }
            self.initialize_counters();

            let mut fv = Arc::clone(&value_source).get_values(&hits.context, None)?;

            // NOTE: this is not as efficient as working directly with the doc
            // values APIs in the sparse case because we are doing a linear
            // scan across all hits, but this API is more flexible since a
            // LongValuesSource can compute interesting values at query time
            let mut docs = hits.bits.iterator()?;
            let mut doc = docs.next_doc()?;
            while doc != NO_MORE_DOCS {
                // Skip missing docs:
                if fv.advance_exact(doc)? {
                    let value = fv.long_value()?;
                    self.increment(value);
                    self.tot_count += 1;
                }
                doc = docs.next_doc()?;
            }
        }
        Ok(())
    }

    fn count_multi_doc(&mut self, limit: i64, mut next: impl FnMut() -> Result<i64>) -> Result<()> {
        if limit > 0 {
            self.tot_count += 1;
        }
        let mut previous_value = 0i64;
        for i in 0..limit {
            let value = next()?;
            // do not increment the count for duplicate values
            if i == 0 || value != previous_value {
                self.increment(value);
                previous_value = value;
            }
        }
        Ok(())
    }

    /// Counts directly from a `MultiLongValuesSource`.
    ///
    /// Equivalent to `count(MultiLongValuesSource, List)`.
    fn count_multi_source(
        &mut self,
        values_source: &dyn MultiLongValuesSource,
        matching_docs: &[MatchingDocs],
    ) -> Result<()> {
        for hits in matching_docs {
            if hits.total_hits == 0 {
                continue;
            }
            self.initialize_counters();

            let mut multi_values = values_source.get_values(&hits.context)?;

            let mut docs = hits.bits.iterator()?;
            let mut doc = docs.next_doc()?;
            while doc != NO_MORE_DOCS {
                if multi_values.advance_exact(doc)? {
                    let limit = multi_values.get_value_count()?;
                    self.count_multi_doc(limit, || multi_values.next_value())?;
                }
                doc = docs.next_doc()?;
            }
        }
        Ok(())
    }

    /// Counts from the provided field.
    ///
    /// Equivalent to `count(String, List)`.
    fn count_field(&mut self, field: &str, matching_docs: &[MatchingDocs]) -> Result<()> {
        for hits in matching_docs {
            if hits.total_hits == 0 {
                continue;
            }
            self.initialize_counters();

            let multi_values = DocValues::get_sorted_numeric(&*hits.context.leaf_reader(), field)?;
            let mut bits = hits.bits.iterator()?;
            match unwrap_singleton_numeric(multi_values)? {
                NumericValuesView::Single(mut single_values) => loop {
                    if Intersect2::new(&mut *bits, &mut *single_values).next_doc()? == NO_MORE_DOCS
                    {
                        break;
                    }
                    let value = single_values.long_value()?;
                    self.increment(value);
                    self.tot_count += 1;
                },
                NumericValuesView::Multi(mut multi_values) => loop {
                    if Intersect2::new(&mut *bits, &mut *multi_values).next_doc()? == NO_MORE_DOCS {
                        break;
                    }
                    let limit = multi_values.doc_value_count()? as i64;
                    self.count_multi_doc(limit, || multi_values.next_value())?;
                },
            }
        }
        Ok(())
    }

    /// Count everything in the provided `LongValuesSource`.
    ///
    /// Equivalent to `countAll(IndexReader, LongValuesSource)`.
    fn count_all_source(
        &mut self,
        reader: &Arc<dyn IndexReader>,
        value_source: Arc<dyn LongValuesSource>,
    ) -> Result<()> {
        for context in Arc::clone(reader).leaves()? {
            let mut fv = Arc::clone(&value_source).get_values(&context, None)?;
            let max_doc = context.leaf_reader().max_doc();

            for doc in 0..max_doc {
                // Skip missing docs:
                if fv.advance_exact(doc)? {
                    let value = fv.long_value()?;
                    self.increment(value);
                    self.tot_count += 1;
                }
            }
        }
        Ok(())
    }

    /// Count everything in the provided `MultiLongValuesSource`.
    ///
    /// Equivalent to `countAll(IndexReader, MultiLongValuesSource)`.
    fn count_all_multi_source(
        &mut self,
        reader: &Arc<dyn IndexReader>,
        value_source: &dyn MultiLongValuesSource,
    ) -> Result<()> {
        for context in Arc::clone(reader).leaves()? {
            let mut multi_values = value_source.get_values(&context)?;
            let max_doc = context.leaf_reader().max_doc();
            for doc in 0..max_doc {
                // Skip missing docs:
                if multi_values.advance_exact(doc)? {
                    let limit = multi_values.get_value_count()?;
                    self.count_multi_doc(limit, || multi_values.next_value())?;
                }
            }
        }
        Ok(())
    }

    /// Count everything in the specified field.
    ///
    /// Equivalent to `countAll(IndexReader, String)`.
    fn count_all_field(&mut self, reader: &Arc<dyn IndexReader>, field: &str) -> Result<()> {
        for context in Arc::clone(reader).leaves()? {
            let leaf_reader = context.leaf_reader();
            let multi_values = DocValues::get_sorted_numeric(&*leaf_reader, field)?;
            let live_docs = leaf_reader.get_live_docs();
            let next = |values: &mut dyn rucene::search::DocIdSetIterator| -> Result<i32> {
                // `FacetUtils.liveDocsDISI(valuesIt, liveDocs)`.
                let mut doc = values.next_doc()?;
                if let Some(live_docs) = &live_docs {
                    while doc != NO_MORE_DOCS && !live_docs.get(doc as usize) {
                        doc = values.next_doc()?;
                    }
                }
                Ok(doc)
            };
            match unwrap_singleton_numeric(multi_values)? {
                NumericValuesView::Single(mut single_values) => loop {
                    if next(&mut *single_values)? == NO_MORE_DOCS {
                        break;
                    }
                    self.tot_count += 1;
                    let value = single_values.long_value()?;
                    self.increment(value);
                },
                NumericValuesView::Multi(mut multi_values) => loop {
                    if next(&mut *multi_values)? == NO_MORE_DOCS {
                        break;
                    }
                    let limit = multi_values.doc_value_count()? as i64;
                    self.count_multi_doc(limit, || multi_values.next_value())?;
                },
            }
        }
        Ok(())
    }

    /// Equivalent to `increment(long)`.
    fn increment(&mut self, value: i64) {
        let counts = self.counts.as_mut().expect("INVARIANT: initialized");
        if value >= 0 && value < counts.len() as i64 {
            counts[value as usize] += 1;
        } else {
            self.hash_counts
                .as_mut()
                .expect("INVARIANT: initialized")
                .add_to(value, 1);
        }
    }

    /// Reusable top-N facet result... in Java's terms: returns all unique
    /// values seen, sorted by value.
    ///
    /// Equivalent to `getAllChildrenSortByValue()`.
    pub fn get_all_children_sort_by_value(&self) -> FacetResult {
        if !self.initialized {
            debug_assert!(self.tot_count == 0);
            return result(&self.field, self.tot_count, Vec::new(), 0);
        }
        let counts = self.counts.as_ref().expect("INVARIANT: initialized");
        let hash = self.hash_counts.as_ref().expect("INVARIANT: initialized");

        let mut label_values = Vec::new();

        // compact & sort hash table's arrays by value
        let mut entries: Vec<(i32, i64)> = Vec::with_capacity(hash.size() as usize);
        for c in hash.iter() {
            if c.value != 0 {
                entries.push((c.value, c.key));
            }
        }
        let upto = entries.len();
        debug_assert!(
            upto == hash.size() as usize,
            "upto={} hashCounts.size={}",
            upto,
            hash.size()
        );

        // `InPlaceMergeSorter` by value: stable.
        entries.sort_by_key(|e| e.1);

        let mut counts_added = false;
        for &(count, value) in &entries {
            if !counts_added && value >= counts.len() as i64 {
                counts_added = true;
                Self::append_counts(counts, &mut label_values);
            }
            label_values.push(LabelAndValue::new(
                value.to_string(),
                NumericValue::Int(count),
            ));
        }

        if !counts_added {
            Self::append_counts(counts, &mut label_values);
        }

        let child_count = label_values.len() as i32;
        result(&self.field, self.tot_count, label_values, child_count)
    }

    /// Equivalent to `appendCounts(List)`.
    fn append_counts(counts: &[i32], label_values: &mut Vec<LabelAndValue>) {
        for (i, &count) in counts.iter().enumerate() {
            if count != 0 {
                label_values.push(LabelAndValue::new(i.to_string(), NumericValue::Int(count)));
            }
        }
    }

    /// Equivalent to `validateDimAndPathForGetChildren(String, String...)`.
    fn validate_dim_and_path_for_get_children(&self, dim: &str, path: &[&str]) -> Result<()> {
        if dim != self.field {
            return Err(LuceneError::illegal_argument(format!(
                "invalid dim \"{dim}\"; should be \"{}\"",
                self.field
            )));
        }
        if !path.is_empty() {
            return Err(LuceneError::illegal_argument("path.length should be 0"));
        }
        Ok(())
    }
}

impl Facets for LongValueFacetCounts {
    /// This instance, for Java's `instanceof` tests and casts.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Returns all unique values seen, sorted by value.
    fn get_all_children(&self, dim: &str, path: &[&str]) -> Result<Option<FacetResult>> {
        self.validate_dim_and_path_for_get_children(dim, path)?;

        if !self.initialized {
            // nothing was counted (either no hits or no values for all hits):
            debug_assert!(self.tot_count == 0);
            return Ok(Some(result(&self.field, self.tot_count, Vec::new(), 0)));
        }

        let counts = self.counts.as_ref().expect("INVARIANT: initialized");
        let hash = self.hash_counts.as_ref().expect("INVARIANT: initialized");
        let mut label_values = Vec::new();
        for (i, &count) in counts.iter().enumerate() {
            if count != 0 {
                label_values.push(LabelAndValue::new(i.to_string(), NumericValue::Int(count)));
            }
        }

        if hash.size() != 0 {
            for c in hash.iter() {
                let count = c.value;
                if count != 0 {
                    label_values.push(LabelAndValue::new(
                        c.key.to_string(),
                        NumericValue::Int(c.value),
                    ));
                }
            }
        }

        let child_count = label_values.len() as i32;
        Ok(Some(result(
            &self.field,
            self.tot_count,
            label_values,
            child_count,
        )))
    }

    fn get_top_children(
        &self,
        top_n: i32,
        dim: &str,
        path: &[&str],
    ) -> Result<Option<FacetResult>> {
        validate_top_n(top_n)?;
        self.validate_dim_and_path_for_get_children(dim, path)?;

        if !self.initialized {
            // nothing was counted (either no hits or no values for all hits):
            debug_assert!(self.tot_count == 0);
            return Ok(Some(result(&self.field, self.tot_count, Vec::new(), 0)));
        }

        let counts = self.counts.as_ref().expect("INVARIANT: initialized");
        let hash = self.hash_counts.as_ref().expect("INVARIANT: initialized");

        let size = (top_n as i64).min(counts.len() as i64 + hash.size() as i64);
        let mut pq: PriorityQueue<Entry, EntryLessThan> =
            PriorityQueue::new(size as usize, EntryLessThan)?;

        let mut child_count = 0i32;
        let mut e: Option<Entry> = None;
        for (i, &count) in counts.iter().enumerate() {
            if count != 0 {
                child_count += 1;
                let mut entry = e.take().unwrap_or_default();
                entry.value = i as i64;
                entry.count = count;
                e = pq.insert_with_overflow(entry);
            }
        }

        if hash.size() != 0 {
            child_count += hash.size();
            for c in hash.iter() {
                let count = c.value;
                if count != 0 {
                    let mut entry = e.take().unwrap_or_default();
                    entry.value = c.key;
                    entry.count = count;
                    e = pq.insert_with_overflow(entry);
                }
            }
        }

        let mut results: Vec<Option<LabelAndValue>> = (0..pq.size()).map(|_| None).collect();
        while pq.size() != 0 {
            let entry = pq.pop().expect("INVARIANT: size > 0");
            results[pq.size()] = Some(LabelAndValue::new(
                entry.value.to_string(),
                NumericValue::Int(entry.count),
            ));
        }

        Ok(Some(result(
            &self.field,
            self.tot_count,
            results.into_iter().flatten().collect(),
            child_count,
        )))
    }

    fn get_specific_value(&self, _dim: &str, _path: &[&str]) -> Result<NumericValue> {
        // TODO: should we impl this?
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn get_all_dims(&self, top_n: i32) -> Result<Vec<FacetResult>> {
        validate_top_n(top_n)?;
        Ok(self
            .get_top_children(top_n, &self.field.clone(), &[])?
            .into_iter()
            .collect())
    }
}

impl fmt::Display for LongValueFacetCounts {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LongValueFacetCounts totCount={}", self.tot_count)?;
        if self.initialized {
            f.write_str(":\n")?;
            let counts = self.counts.as_ref().expect("INVARIANT: initialized");
            for (i, &count) in counts.iter().enumerate() {
                if count != 0 {
                    writeln!(f, "  {i} -> count={count}")?;
                }
            }
            let hash = self.hash_counts.as_ref().expect("INVARIANT: initialized");
            if hash.size() != 0 {
                for c in hash.iter() {
                    if c.value != 0 {
                        writeln!(f, "  {} -> count={}", c.key, c.value)?;
                    }
                }
            }
        }
        Ok(())
    }
}
