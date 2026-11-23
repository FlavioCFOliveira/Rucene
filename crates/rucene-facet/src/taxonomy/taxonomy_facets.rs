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

//! Port of `TaxonomyFacets.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;

use rucene::document::NumericValue;
use rucene::error::{LuceneError, Result};
use rucene::internal::hppc::IntIntHashMap;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};
use rucene::util::java_collections::java_compare_to;

use crate::facet_result::FacetResult;
use crate::facets::validate_top_n;
use crate::facets_collector::MatchingDocs;
use crate::facets_config::{DimConfig, FacetsConfig};
use crate::label_and_value::{
    number_double_value, number_float_value, number_int_value, LabelAndValue,
};
use crate::taxonomy::facet_label::FacetLabel;
use crate::taxonomy::float_taxonomy_facets::FloatTaxonomyFacets;
use crate::taxonomy::int_taxonomy_facets::IntTaxonomyFacets;
use crate::taxonomy::parallel_taxonomy_arrays::{IntArray, ParallelTaxonomyArrays};
use crate::taxonomy::taxonomy_reader::{TaxonomyReader, INVALID_ORDINAL, ROOT_ORDINAL};
use crate::top_ord_and_float_queue::{OrdAndFloat, TopOrdAndFloatQueue};
use crate::top_ord_and_int_queue::{OrdAndInt, TopOrdAndIntQueue};
use crate::top_ord_and_number_queue::{OrdAndValue, TopOrdAndNumberQueue};

/// Intermediate result to store top children for a given path before
/// resolving labels, etc.
///
/// Equivalent to the record
/// `org.apache.lucene.facet.taxonomy.TaxonomyFacets.TopChildrenForPath`.
pub(crate) struct TopChildrenForPath {
    pub(crate) path_value: NumericValue,
    pub(crate) child_count: i32,
    pub(crate) child_queue: TopOrdAndNumberQueue,
}

/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyFacets.DimValue`.
#[derive(Debug, Clone)]
pub(crate) struct DimValue {
    dim: String,
    dim_ord: i32,
    value: NumericValue,
}

/// The `valueComparator` of the facets: `Comparator.comparingInt` for counts
/// and int associations, `Float.compare` over `floatValue()` for float
/// associations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueComparator {
    Int,
    Float,
}

impl ValueComparator {
    pub(crate) fn compare(self, a: &NumericValue, b: &NumericValue) -> Ordering {
        match self {
            ValueComparator::Int => number_int_value(a).cmp(&number_int_value(b)),
            ValueComparator::Float => {
                java_float_compare(number_float_value(a), number_float_value(b))
            }
        }
    }
}

/// `Float.compare(float, float)`.
pub(crate) fn java_float_compare(a: f32, b: f32) -> Ordering {
    if a < b {
        return Ordering::Less;
    }
    if a > b {
        return Ordering::Greater;
    }
    let bits = |v: f32| -> i32 {
        if v.is_nan() {
            0x7fc0_0000
        } else {
            v.to_bits() as i32
        }
    };
    bits(a).cmp(&bits(b))
}

/// The `DimValue` queue of `getTopDims`.
struct DimValueLessThan(ValueComparator);

impl PriorityQueueComparator<DimValue> for DimValueLessThan {
    fn less_than(&self, a: &DimValue, b: &DimValue) -> bool {
        let comparison = self.0.compare(&a.value, &b.value);
        if comparison == Ordering::Less {
            return true;
        }
        if comparison == Ordering::Greater {
            return false;
        }
        java_compare_to(&a.dim, &b.dim) == Ordering::Greater
    }
}

/// An accumulator for an aggregated value.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyFacets.AggregatedValue`;
/// its three subclasses (`AggregatedCount`, `IntAggregatedValue`,
/// `FloatAggregatedValue`) are the variants.
#[derive(Debug, Clone, Copy)]
pub enum AggregatedValue {
    /// `AggregatedCount`.
    Count(i32),
    /// `IntTaxonomyFacets.IntAggregatedValue`.
    Int(i32),
    /// `FloatTaxonomyFacets.FloatAggregatedValue`.
    Float(f32),
}

/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyFacets.AggregatedCount`.
pub(crate) struct AggregatedCount;

impl AggregatedCount {
    /// Equivalent to `AggregatedCount(int)`.
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(count: i32) -> AggregatedValue {
        AggregatedValue::Count(count)
    }
}

impl AggregatedValue {
    /// Aggregate the value corresponding to the given ordinal.
    ///
    /// Equivalent to `aggregate(int)`, dispatched per variant: Equivalent to
    /// `AggregatedCount.aggregate(int)`. Equivalent to
    /// `IntAggregatedValue.aggregate(int)`. Equivalent to
    /// `FloatAggregatedValue.aggregate(int)`.
    pub(crate) fn aggregate(&mut self, facets: &TaxonomyFacets, ord: i32) {
        match self {
            AggregatedValue::Count(count) => *count = count.wrapping_add(facets.get_count(ord)),
            AggregatedValue::Int(value) => {
                let int = facets.int_values();
                *value = int
                    .aggregation_function
                    .aggregate_int(*value, int.get_value(ord));
            }
            AggregatedValue::Float(value) => {
                let float = facets.float_values();
                *value = float
                    .aggregation_function
                    .aggregate_float(*value, float.get_value(ord));
            }
        }
    }

    /// Retrieve the aggregation value.
    ///
    /// Equivalent to `get()`, dispatched per variant: Equivalent to
    /// `AggregatedCount.get()`. Equivalent to `IntAggregatedValue.get()`.
    /// Equivalent to `FloatAggregatedValue.get()`.
    pub(crate) fn get(&self) -> NumericValue {
        match *self {
            AggregatedValue::Count(v) | AggregatedValue::Int(v) => NumericValue::Int(v),
            AggregatedValue::Float(v) => NumericValue::Float(v),
        }
    }
}

/// The values a subclass of `TaxonomyFacets` keeps besides the counts.
pub(crate) enum ValueStore {
    /// `TaxonomyFacets` itself (counts only).
    Counts,
    /// `IntTaxonomyFacets`.
    Int(IntTaxonomyFacets),
    /// `FloatTaxonomyFacets`.
    Float(FloatTaxonomyFacets),
}

/// Base class for all taxonomy-based facets impls.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyFacets`. Its
/// abstract subclasses `IntTaxonomyFacets` and `FloatTaxonomyFacets` are the
/// [`ValueStore`] it carries; their overrides are the dispatches on it.
pub(crate) struct TaxonomyFacets {
    /// Index field name provided to the constructor.
    pub(crate) index_field_name: String,
    /// `TaxonomyReader` provided to the constructor.
    pub(crate) taxo_reader: Arc<dyn TaxonomyReader>,
    /// `FacetsConfig` provided to the constructor.
    pub(crate) config: Arc<FacetsConfig>,
    /// The matching docs of the `FacetsCollector` provided to the
    /// constructor, `None` for Java's `null` collector.
    pub(crate) fc: Option<Vec<MatchingDocs>>,
    arrays: Arc<dyn ParallelTaxonomyArrays>,
    children_loaded: AtomicBool,
    siblings_loaded: AtomicBool,
    /// Dense ordinal counts.
    pub(crate) counts: Option<Vec<i32>>,
    /// Sparse ordinal counts.
    pub(crate) sparse_counts: Option<IntIntHashMap>,
    /// Have value counters been initialized.
    pub(crate) initialized: bool,
    pub(crate) values: ValueStore,
}

impl TaxonomyFacets {
    /// Sole constructor.
    ///
    /// Equivalent to `TaxonomyFacets(String, TaxonomyReader, FacetsConfig,
    /// FacetsCollector)`.
    pub(crate) fn new(
        index_field_name: &str,
        taxo_reader: Arc<dyn TaxonomyReader>,
        config: Arc<FacetsConfig>,
        fc: Option<&crate::facets_collector::FacetsCollector>,
        values: ValueStore,
    ) -> Result<Self> {
        let arrays = taxo_reader.get_parallel_taxonomy_arrays()?;
        let fc = match fc {
            Some(fc) => Some(fc.get_matching_docs()?.clone()),
            None => None,
        };
        Ok(Self {
            index_field_name: index_field_name.to_string(),
            taxo_reader,
            config,
            fc,
            arrays,
            children_loaded: AtomicBool::new(false),
            siblings_loaded: AtomicBool::new(false),
            counts: None,
            sparse_counts: None,
            initialized: false,
            values,
        })
    }

    /// The `valueComparator`.
    pub(crate) fn value_comparator(&self) -> ValueComparator {
        match self.values {
            ValueStore::Float(_) => ValueComparator::Float,
            _ => ValueComparator::Int,
        }
    }

    pub(crate) fn int_values(&self) -> &IntTaxonomyFacets {
        match &self.values {
            ValueStore::Int(v) => v,
            _ => unreachable!("an IntTaxonomyFacets value"),
        }
    }

    pub(crate) fn float_values(&self) -> &FloatTaxonomyFacets {
        match &self.values {
            ValueStore::Float(v) => v,
            _ => unreachable!("a FloatTaxonomyFacets value"),
        }
    }

    /// Returns true if the (costly, and lazily initialized) children int[]
    /// was initialized.
    ///
    /// Equivalent to `childrenLoaded()`.
    pub fn children_loaded(&self) -> bool {
        self.children_loaded.load(AtomicOrdering::SeqCst)
    }

    /// Returns true if the (costly, and lazily initialized) sibling int[] was
    /// initialized.
    ///
    /// Equivalent to `siblingsLoaded()`.
    pub fn siblings_loaded(&self) -> bool {
        self.siblings_loaded.load(AtomicOrdering::SeqCst)
    }

    /// Equivalent to `parents`.
    pub(crate) fn parents(&self) -> &dyn IntArray {
        self.arrays.parents()
    }

    /// Equivalent to `getChildren()`.
    pub(crate) fn get_children(&self) -> &dyn IntArray {
        self.children_loaded.store(true, AtomicOrdering::SeqCst);
        self.arrays.children()
    }

    /// Equivalent to `getSiblings()`.
    pub(crate) fn get_siblings(&self) -> &dyn IntArray {
        self.siblings_loaded.store(true, AtomicOrdering::SeqCst);
        self.arrays.siblings()
    }

    /// Returns true if a hash table should be used for counting.
    ///
    /// Equivalent to `useHashTable(FacetsCollector, TaxonomyReader)`.
    fn use_hash_table(&self) -> Result<bool> {
        if self.taxo_reader.get_size()? < 1024 {
            // small number of unique values: use an array
            return Ok(false);
        }

        let Some(fc) = &self.fc else {
            // counting all docs: use an array
            return Ok(false);
        };

        let mut max_doc = 0i32;
        let mut sum_total_hits = 0i32;
        for docs in fc {
            sum_total_hits = sum_total_hits.wrapping_add(docs.total_hits);
            max_doc = max_doc.wrapping_add(docs.context.leaf_reader().max_doc());
        }

        // if our result set is < 10% of the index, we collect sparsely (use
        // hash map):
        Ok(sum_total_hits < max_doc / 10)
    }

    /// Initialize counters.
    ///
    /// Equivalent to `initializeValueCounters()` and its overrides.
    pub(crate) fn initialize_value_counters(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        self.initialized = true;
        debug_assert!(self.sparse_counts.is_none() && self.counts.is_none());
        if self.use_hash_table()? {
            self.sparse_counts = Some(IntIntHashMap::new());
        } else {
            self.counts = Some(vec![0; self.taxo_reader.get_size()?.max(0) as usize]);
        }
        let sparse = self.sparse_counts.is_some();
        let size = self.taxo_reader.get_size()?.max(0) as usize;
        match &mut self.values {
            ValueStore::Counts => {}
            ValueStore::Int(v) => v.initialize(sparse, size),
            ValueStore::Float(v) => v.initialize(sparse, size),
        }
        Ok(())
    }

    /// Set the count for this ordinal to `new_value`.
    ///
    /// Equivalent to `setCount(int, int)`.
    pub(crate) fn set_count(&mut self, ordinal: i32, new_value: i32) {
        if let Some(sparse_counts) = &mut self.sparse_counts {
            sparse_counts.put(ordinal, new_value);
        } else {
            self.counts.as_mut().expect("INVARIANT: initialized")[ordinal as usize] = new_value;
        }
    }

    /// Get the count for this ordinal.
    ///
    /// Equivalent to `getCount(int)`.
    pub(crate) fn get_count(&self, ordinal: i32) -> i32 {
        if let Some(sparse_counts) = &self.sparse_counts {
            sparse_counts.get(ordinal)
        } else {
            self.counts.as_ref().expect("INVARIANT: initialized")[ordinal as usize]
        }
    }

    /// Get the aggregation value for this ordinal.
    ///
    /// Equivalent to `getAggregationValue(int)` and its overrides.
    pub(crate) fn get_aggregation_value(&self, ordinal: i32) -> NumericValue {
        match &self.values {
            // By default, this is just the count.
            ValueStore::Counts => NumericValue::Int(self.get_count(ordinal)),
            ValueStore::Int(v) => NumericValue::Int(v.get_value(ordinal)),
            ValueStore::Float(v) => NumericValue::Float(v.get_value(ordinal)),
        }
    }

    /// Apply an aggregation to the two values and return the result.
    ///
    /// Equivalent to `aggregate(Number, Number)` and its overrides.
    pub(crate) fn aggregate(
        &self,
        existing_val: &NumericValue,
        new_val: &NumericValue,
    ) -> NumericValue {
        match &self.values {
            // By default, we are computing counts, so the values are
            // interpreted as integers and summed.
            ValueStore::Counts => NumericValue::Int(
                number_int_value(existing_val).wrapping_add(number_int_value(new_val)),
            ),
            ValueStore::Int(v) => NumericValue::Int(
                v.aggregation_function
                    .aggregate_int(number_int_value(existing_val), number_int_value(new_val)),
            ),
            ValueStore::Float(v) => NumericValue::Float(v.aggregation_function.aggregate_float(
                number_float_value(existing_val),
                number_float_value(new_val),
            )),
        }
    }

    /// Were any values actually aggregated during counting?
    ///
    /// Equivalent to `hasValues()`.
    pub(crate) fn has_values(&self) -> bool {
        self.initialized
    }

    /// Throws `IllegalArgumentException` if the dimension is not recognized.
    /// Otherwise, returns the `DimConfig` for this dimension.
    ///
    /// Equivalent to `verifyDim(String)`.
    pub(crate) fn verify_dim(&self, dim: &str) -> Result<DimConfig> {
        let dim_config = self.config.get_dim_config(dim);
        if self.config.is_dim_configured(dim)
            && dim_config.index_field_name() != self.index_field_name
        {
            return Err(LuceneError::illegal_argument(format!(
                "dimension \"{}\" cannot be found in field \"{}\", since it was configured to be indexed into field \"{}\"",
                dim, self.index_field_name, dim_config.index_field_name()
            )));
        }
        Ok(dim_config)
    }

    /// Roll-up the aggregation values from `child_ordinal` to `ordinal`.
    ///
    /// Equivalent to `updateValueFromRollup(int, int)` and its overrides.
    pub(crate) fn update_value_from_rollup(&mut self, ordinal: i32, child_ordinal: i32) {
        let count = self
            .get_count(ordinal)
            .wrapping_add(self.rollup_counts(child_ordinal));
        self.set_count(ordinal, count);
        match &self.values {
            ValueStore::Counts => {}
            ValueStore::Int(_) => {
                let current_value = self.int_values().get_value(ordinal);
                let rolled = self.rollup_int(child_ordinal);
                let new_value = self
                    .int_values()
                    .aggregation_function
                    .aggregate_int(current_value, rolled);
                if let ValueStore::Int(v) = &mut self.values {
                    v.set_value(ordinal, new_value);
                }
            }
            ValueStore::Float(_) => {
                let current_value = self.float_values().get_value(ordinal);
                let rolled = self.rollup_float(child_ordinal);
                let new_value = self
                    .float_values()
                    .aggregation_function
                    .aggregate_float(current_value, rolled);
                if let ValueStore::Float(v) = &mut self.values {
                    v.set_value(ordinal, new_value);
                }
            }
        }
    }

    /// Return a `TopOrdAndNumberQueue` of the appropriate type, i.e. a
    /// `TopOrdAndIntQueue` or a `TopOrdAndFloatQueue`.
    ///
    /// Equivalent to `makeTopOrdAndNumberQueue(int)` and its overrides.
    pub(crate) fn make_top_ord_and_number_queue(&self, top_n: i32) -> Result<TopOrdAndNumberQueue> {
        let size = self.taxo_reader.get_size()?.min(top_n);
        Ok(match &self.values {
            ValueStore::Float(_) => TopOrdAndFloatQueue::new(size)?.into_inner(),
            _ => TopOrdAndIntQueue::new(size)?.into_inner(),
        })
    }

    /// Get the aggregation value for a missing value.
    ///
    /// Equivalent to `missingAggregationValue()` and its overrides.
    pub(crate) fn missing_aggregation_value(&self) -> NumericValue {
        match &self.values {
            ValueStore::Float(_) => NumericValue::Float(-1.0),
            _ => NumericValue::Int(-1),
        }
    }

    /// Rolls up any single-valued hierarchical dimensions.
    ///
    /// Equivalent to `rollup()`.
    pub(crate) fn rollup(&mut self) -> Result<()> {
        if !self.initialized {
            return Ok(());
        }

        // Rollup any necessary dims:
        // a snapshot of the live map's entries (Java iterates the
        // ConcurrentHashMap weakly consistently; each dim rolls up
        // independently, so the order is immaterial)
        let dim_configs: Vec<(String, crate::facets_config::DimConfig)> = self
            .config
            .get_dim_configs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (dim, ft) in &dim_configs {
            if ft.hierarchical() && !ft.multi_valued() {
                let dim_root_ord = self
                    .taxo_reader
                    .get_ordinal(&FacetLabel::new([dim.as_str()])?)?;
                // It can be -1 if this field was declared in the config but
                // never indexed:
                if dim_root_ord > 0 {
                    let child = self.get_children().get(dim_root_ord);
                    self.update_value_from_rollup(dim_root_ord, child);
                }
            }
        }
        Ok(())
    }

    /// Equivalent to the private `rollup(int)`.
    fn rollup_counts(&mut self, mut ord: i32) -> i32 {
        let mut aggregated_value = 0i32;
        while ord != INVALID_ORDINAL {
            let current_value = self.get_count(ord);
            let child = self.get_children().get(ord);
            let new_value = current_value.wrapping_add(self.rollup_counts(child));
            self.set_count(ord, new_value);
            aggregated_value = aggregated_value.wrapping_add(self.get_count(ord));
            ord = self.get_siblings().get(ord);
        }
        aggregated_value
    }

    /// Equivalent to `IntTaxonomyFacets.rollup(int)`.
    fn rollup_int(&mut self, mut ord: i32) -> i32 {
        let mut aggregated_value = 0i32;
        while ord != INVALID_ORDINAL {
            let child = self.get_children().get(ord);
            self.update_value_from_rollup(ord, child);
            aggregated_value = self
                .int_values()
                .aggregation_function
                .aggregate_int(aggregated_value, self.int_values().get_value(ord));
            ord = self.get_siblings().get(ord);
        }
        aggregated_value
    }

    /// Equivalent to `FloatTaxonomyFacets.rollup(int)`.
    fn rollup_float(&mut self, mut ord: i32) -> f32 {
        let mut aggregated_value = 0f32;
        while ord != INVALID_ORDINAL {
            let child = self.get_children().get(ord);
            self.update_value_from_rollup(ord, child);
            aggregated_value = self
                .float_values()
                .aggregation_function
                .aggregate_float(aggregated_value, self.float_values().get_value(ord));
            ord = self.get_siblings().get(ord);
        }
        aggregated_value
    }

    /// Create a FacetResult for the provided dim + path.
    ///
    /// Equivalent to `createFacetResult(TopChildrenForPath, String,
    /// String...)`.
    fn create_facet_result(
        &self,
        top_children_for_path: Option<TopChildrenForPath>,
        dim: &str,
        path: &[&str],
    ) -> Result<Option<FacetResult>> {
        // If the intermediate result is null or there are no children, we
        // return null:
        let Some(mut top_children_for_path) = top_children_for_path else {
            return Ok(None);
        };
        if top_children_for_path.child_count == 0 {
            return Ok(None);
        }

        let q = &mut top_children_for_path.child_queue;
        let len = q.size();
        let mut ordinals = vec![0i32; len];
        let mut counts = vec![0i32; len];
        let mut values: Vec<NumericValue> = vec![NumericValue::Int(0); len];

        for i in (0..len).rev() {
            let ord_and_value = q.pop().expect("INVARIANT: queue has len entries");
            ordinals[i] = ord_and_value.ord();
            values[i] = ord_and_value.get_value();
            counts[i] = self.get_count(ordinals[i]);
        }

        let bulk_path = self.taxo_reader.get_bulk_path(&mut ordinals.clone())?;
        // The path component we're interested in is the one immediately after
        // the provided path. We add 1 here to also account for the dim:
        let child_component_idx = path.len() + 1;
        let mut label_values = Vec::with_capacity(len);
        for i in 0..len {
            let label = bulk_path[i]
                .as_ref()
                .ok_or_else(LuceneError::null_pointer)?;
            label_values.push(LabelAndValue::with_count(
                label.components[child_component_idx].clone(),
                values[i],
                counts[i],
            ));
        }

        Ok(Some(FacetResult::new(
            dim,
            path.iter().map(|p| p.to_string()).collect(),
            top_children_for_path.path_value,
            label_values,
            top_children_for_path.child_count,
        )))
    }

    /// Equivalent to `getAllChildren(String, String...)`.
    pub(crate) fn get_all_children(&self, dim: &str, path: &[&str]) -> Result<Option<FacetResult>> {
        let dim_config = self.verify_dim(dim)?;
        let cp = FacetLabel::with_dim(dim, path)?;
        let dim_ord = self.taxo_reader.get_ordinal(&cp)?;
        if dim_ord == -1 {
            return Ok(None);
        }

        if !self.initialized {
            return Ok(None);
        }

        let mut aggregated_value = NumericValue::Int(0);
        let mut aggregated_count = 0i32;

        let mut ordinals: Vec<i32> = Vec::new();
        let mut ord_values: Vec<NumericValue> = Vec::new();

        if let Some(sparse_counts) = &self.sparse_counts {
            for ord_and_count in sparse_counts.iter() {
                let ord = ord_and_count.key;
                let count = ord_and_count.value;
                let value = self.get_aggregation_value(ord);
                if self.parents().get(ord) == dim_ord && count > 0 {
                    aggregated_count = aggregated_count.wrapping_add(count);
                    aggregated_value = self.aggregate(&aggregated_value, &value);
                    ordinals.push(ord);
                    ord_values.push(value);
                }
            }
        } else {
            let counts = self.counts.as_ref().expect("INVARIANT: initialized");
            let children = self.get_children();
            let siblings = self.get_siblings();
            let mut ord = children.get(dim_ord);
            while ord != INVALID_ORDINAL {
                let count = counts[ord as usize];
                let value = self.get_aggregation_value(ord);
                if count > 0 {
                    aggregated_count = aggregated_count.wrapping_add(count);
                    aggregated_value = self.aggregate(&aggregated_value, &value);
                    ordinals.push(ord);
                    ord_values.push(value);
                }
                ord = siblings.get(ord);
            }
        }

        if aggregated_count == 0 {
            return Ok(None);
        }

        if dim_config.multi_valued() {
            if dim_config.require_dim_count() {
                aggregated_value = self.get_aggregation_value(dim_ord);
            } else {
                // Our sum'd value is not correct, in general:
                aggregated_value = self.missing_aggregation_value();
            }
        } else {
            // Our sum'd dim value is accurate, so we keep it
        }

        let bulk_path = self.taxo_reader.get_bulk_path(&mut ordinals.clone())?;
        let mut label_values = Vec::with_capacity(ord_values.len());
        for i in 0..ord_values.len() {
            let label = bulk_path[i]
                .as_ref()
                .ok_or_else(LuceneError::null_pointer)?;
            label_values.push(LabelAndValue::with_count(
                label.components[cp.length].clone(),
                ord_values[i],
                self.get_count(ordinals[i]),
            ));
        }
        Ok(Some(FacetResult::new(
            dim,
            path.iter().map(|p| p.to_string()).collect(),
            aggregated_value,
            label_values,
            ordinals.len() as i32,
        )))
    }

    /// Set the value of `incoming_ord_and_value` to the value corresponding
    /// to the given ordinal.
    ///
    /// Equivalent to `setIncomingValue(OrdAndValue, int)` and its overrides.
    fn set_incoming_value(&self, incoming_ord_and_value: &mut dyn OrdAndValue, ord: i32) {
        match &self.values {
            ValueStore::Counts => {
                let any: &mut dyn std::any::Any = as_any_mut(incoming_ord_and_value);
                any.downcast_mut::<OrdAndInt>()
                    .expect("ClassCastException: not an OrdAndInt")
                    .value = self.get_count(ord);
            }
            ValueStore::Int(v) => {
                let any: &mut dyn std::any::Any = as_any_mut(incoming_ord_and_value);
                any.downcast_mut::<OrdAndInt>()
                    .expect("ClassCastException: not an OrdAndInt")
                    .value = v.get_value(ord);
            }
            ValueStore::Float(v) => {
                let any: &mut dyn std::any::Any = as_any_mut(incoming_ord_and_value);
                any.downcast_mut::<OrdAndFloat>()
                    .expect("ClassCastException: not an OrdAndFloat")
                    .value = v.get_value(ord);
            }
        }
    }

    /// Insert an ordinal and the value corresponding to it into the queue.
    ///
    /// Equivalent to `insertIntoQueue(TopOrdAndNumberQueue, OrdAndValue,
    /// int)`.
    fn insert_into_queue(
        &self,
        q: &mut TopOrdAndNumberQueue,
        incoming_ord_and_value: Option<Box<dyn OrdAndValue>>,
        ord: i32,
    ) -> Option<Box<dyn OrdAndValue>> {
        let mut incoming_ord_and_value =
            incoming_ord_and_value.unwrap_or_else(|| q.new_ord_and_value());
        incoming_ord_and_value.set_ord(ord);
        self.set_incoming_value(&mut *incoming_ord_and_value, ord);
        q.insert_with_overflow(incoming_ord_and_value)
    }

    /// Return a new (initialized) `AggregatedValue`.
    ///
    /// Equivalent to `newAggregatedValue()` and its overrides.
    fn new_aggregated_value(&self) -> AggregatedValue {
        match &self.values {
            ValueStore::Counts => AggregatedCount::new(0),
            ValueStore::Int(_) => AggregatedValue::Int(0),
            ValueStore::Float(_) => AggregatedValue::Float(0.0),
        }
    }

    /// Determine the top-n children for a specified dimension + path.
    ///
    /// Equivalent to `getTopChildrenForPath(DimConfig, int, int)`.
    fn get_top_children_for_path(
        &self,
        dim_config: &DimConfig,
        path_ord: i32,
        top_n: i32,
    ) -> Result<TopChildrenForPath> {
        let mut q = self.make_top_ord_and_number_queue(top_n)?;
        let mut aggregated_value = self.new_aggregated_value();

        let mut child_count = 0i32;
        let mut incoming_ord_and_value: Option<Box<dyn OrdAndValue>> = None;

        // TODO: would be faster if we had a "get the following children"
        // API?  then we can make a single pass over the hashmap
        if let Some(sparse_counts) = &self.sparse_counts {
            for c in sparse_counts.iter() {
                let ord = c.key;
                let count = c.value;
                if self.parents().get(ord) == path_ord && count > 0 {
                    aggregated_value.aggregate(self, ord);
                    child_count += 1;

                    incoming_ord_and_value =
                        self.insert_into_queue(&mut q, incoming_ord_and_value, ord);
                }
            }
        } else {
            let counts = self.counts.as_ref().expect("INVARIANT: initialized");
            let children = self.get_children();
            let siblings = self.get_siblings();
            let mut ord = children.get(path_ord);
            while ord != INVALID_ORDINAL {
                let count = counts[ord as usize];
                if count > 0 {
                    aggregated_value.aggregate(self, ord);
                    child_count += 1;

                    incoming_ord_and_value =
                        self.insert_into_queue(&mut q, incoming_ord_and_value, ord);
                }
                ord = siblings.get(ord);
            }
        }

        let mut aggregated_value_number = aggregated_value.get();
        if dim_config.multi_valued() {
            if dim_config.require_dim_count() {
                aggregated_value_number = self.get_aggregation_value(path_ord);
            } else {
                // Our aggregated value is not correct, in general:
                aggregated_value_number = self.missing_aggregation_value();
            }
        }

        Ok(TopChildrenForPath {
            path_value: aggregated_value_number,
            child_count,
            child_queue: q,
        })
    }

    /// Equivalent to `getTopChildren(int, String, String...)`.
    pub(crate) fn get_top_children(
        &self,
        top_n: i32,
        dim: &str,
        path: &[&str],
    ) -> Result<Option<FacetResult>> {
        validate_top_n(top_n)?;
        let dim_config = self.verify_dim(dim)?;
        let cp = FacetLabel::with_dim(dim, path)?;
        let dim_ord = self.taxo_reader.get_ordinal(&cp)?;
        if dim_ord == -1 {
            return Ok(None);
        }

        if !self.initialized {
            return Ok(None);
        }

        let top_children_for_path = self.get_top_children_for_path(&dim_config, dim_ord, top_n)?;
        self.create_facet_result(Some(top_children_for_path), dim, path)
    }

    /// Equivalent to `getSpecificValue(String, String...)`.
    pub(crate) fn get_specific_value(&self, dim: &str, path: &[&str]) -> Result<NumericValue> {
        let dim_config = self.verify_dim(dim)?;
        if path.is_empty() {
            if dim_config.hierarchical() && !dim_config.multi_valued() {
                // ok: rolled up at search time
            } else if dim_config.require_dim_count() && dim_config.multi_valued() {
                // ok: we indexed all ords at index time
            } else {
                return Err(LuceneError::illegal_argument(
                    "cannot return dimension-level value alone; use getTopChildren instead",
                ));
            }
        }
        let ord = self
            .taxo_reader
            .get_ordinal(&FacetLabel::with_dim(dim, path)?)?;
        if ord < 0 {
            return Ok(NumericValue::Int(-1));
        }
        Ok(if self.initialized {
            self.get_aggregation_value(ord)
        } else {
            NumericValue::Int(0)
        })
    }

    /// Equivalent to `getAllDims(int)`.
    pub(crate) fn get_all_dims(&self, top_n: i32) -> Result<Vec<FacetResult>> {
        validate_top_n(top_n)?;
        if !self.has_values() {
            return Ok(Vec::new());
        }

        let children = self.get_children();
        let siblings = self.get_siblings();
        let mut ord = children.get(ROOT_ORDINAL);
        let mut results = Vec::new();
        while ord != INVALID_ORDINAL {
            let dim = self
                .taxo_reader
                .get_path(ord)?
                .ok_or_else(LuceneError::null_pointer)?
                .components[0]
                .clone();
            let dim_config = self.config.get_dim_config(&dim);
            if dim_config.index_field_name() == self.index_field_name {
                if let Some(result) = self.get_top_children(top_n, &dim, &[])? {
                    results.push(result);
                }
            }
            ord = siblings.get(ord);
        }

        // Sort by highest value, tie-break by dim:
        results.sort_by(|a, b| {
            let (av, bv) = (number_double_value(&a.value), number_double_value(&b.value));
            if av > bv {
                Ordering::Less
            } else if bv > av {
                Ordering::Greater
            } else {
                java_compare_to(&a.dim, &b.dim)
            }
        });
        Ok(results)
    }

    /// Equivalent to `getTopDims(int, int)`.
    pub(crate) fn get_top_dims(
        &self,
        top_n_dims: i32,
        top_n_children: i32,
    ) -> Result<Vec<FacetResult>> {
        if top_n_dims <= 0 || top_n_children <= 0 {
            return Err(LuceneError::illegal_argument("topN must be > 0"));
        }

        if !self.initialized {
            return Ok(Vec::new());
        }

        // get children and siblings ordinal array from TaxonomyFacets
        let children = self.get_children();
        let siblings = self.get_siblings();

        let comparator = self.value_comparator();
        // Create priority queue to store top dimensions and sort by their
        // aggregated values/hits and string values.
        let mut pq: PriorityQueue<DimValue, DimValueLessThan> =
            PriorityQueue::new(top_n_dims as usize, DimValueLessThan(comparator))?;

        // create Map to store the intermediate results. Lazily initialized.
        let mut intermediate_results: Option<HashMap<String, TopChildrenForPath>> = None;

        // iterate over children and siblings ordinals for all dims
        let mut ord = children.get(ROOT_ORDINAL);
        while ord != INVALID_ORDINAL {
            let dim = self
                .taxo_reader
                .get_path(ord)?
                .ok_or_else(LuceneError::null_pointer)?
                .components[0]
                .clone();
            let dim_config = self.config.get_dim_config(&dim);
            if dim_config.index_field_name() == self.index_field_name {
                let cp = FacetLabel::new([dim.as_str()])?;
                let dim_ord = self.taxo_reader.get_ordinal(&cp)?;
                // if dimOrd = -1, we skip this dim, else call
                // getTopChildrenForPath()
                if dim_ord != -1 {
                    let dim_value: NumericValue;
                    if dim_config.multi_valued() {
                        if dim_config.require_dim_count() {
                            // If the dim is configured as multi-valued and
                            // requires dim counts, we can access
                            // dim value directly from values array.
                            dim_value = self.get_aggregation_value(dim_ord);
                        } else {
                            // If the dim is configured as multi-valued but
                            // not requiring dim counts, we cannot compute the
                            // dim count.
                            dim_value = NumericValue::Int(-1);
                        }
                    } else {
                        // Single-valued dims require aggregating descendant
                        // paths to get accurate dim counts since we don't
                        // directly access ancestry paths:
                        let top_children_for_path =
                            self.get_top_children_for_path(&dim_config, dim_ord, top_n_children)?;
                        dim_value = top_children_for_path.path_value;
                        intermediate_results
                            .get_or_insert_with(HashMap::new)
                            .insert(dim.clone(), top_children_for_path);
                    }
                    // add dimension and its value to priority queue
                    if comparator.compare(&dim_value, &NumericValue::Int(0)) != Ordering::Equal {
                        if pq.size() < top_n_dims as usize {
                            pq.add(DimValue {
                                dim: dim.clone(),
                                dim_ord,
                                value: dim_value,
                            });
                        } else {
                            let top = pq.top().expect("INVARIANT: queue is full");
                            let cmp = comparator.compare(&dim_value, &top.value);
                            if cmp == Ordering::Greater
                                || (cmp == Ordering::Equal
                                    && java_compare_to(&dim, &top.dim) == Ordering::Less)
                            {
                                let bottom_dim = pq.top_mut().expect("INVARIANT: queue is full");
                                bottom_dim.dim = dim.clone();
                                bottom_dim.value = dim_value;
                                pq.update_top();
                            }
                        }
                    }
                }
            }
            ord = siblings.get(ord);
        }

        // use priority queue to get top-n dims and create FacetResults
        let mut results: Vec<Option<FacetResult>> = (0..pq.size()).map(|_| None).collect();
        while pq.size() > 0 {
            let dim_value = pq.pop().expect("INVARIANT: size > 0");
            let dim = dim_value.dim;
            let mut top_children_for_path =
                intermediate_results.as_mut().and_then(|m| m.remove(&dim));
            if top_children_for_path.is_none() {
                let dim_config = self.config.get_dim_config(&dim);
                top_children_for_path = Some(self.get_top_children_for_path(
                    &dim_config,
                    dim_value.dim_ord,
                    top_n_children,
                )?);
            }
            let facet_result = self.create_facet_result(top_children_for_path, &dim, &[])?;
            debug_assert!(facet_result.is_some());
            results[pq.size()] = facet_result;
        }
        Ok(results.into_iter().flatten().collect())
    }
}

/// Casts a queue entry back to its concrete class, as Java's casts in the
/// `setIncomingValue` overrides do.
fn as_any_mut(v: &mut dyn OrdAndValue) -> &mut dyn std::any::Any {
    v.as_any_mut()
}

/// Implements the `Facets` methods of a concrete subclass by delegating to
/// its `TaxonomyFacets` base.
macro_rules! taxonomy_facets_delegate {
    ($ty:ty, $base:ident) => {
        impl crate::facets::Facets for $ty {
            /// This instance, for Java's `instanceof` tests and casts.
            fn as_any(&self) -> Option<&dyn std::any::Any> {
                Some(self)
            }

            fn get_all_children(
                &self,
                dim: &str,
                path: &[&str],
            ) -> rucene::error::Result<Option<crate::facet_result::FacetResult>> {
                self.$base.get_all_children(dim, path)
            }

            fn get_top_children(
                &self,
                top_n: i32,
                dim: &str,
                path: &[&str],
            ) -> rucene::error::Result<Option<crate::facet_result::FacetResult>> {
                self.$base.get_top_children(top_n, dim, path)
            }

            fn get_specific_value(
                &self,
                dim: &str,
                path: &[&str],
            ) -> rucene::error::Result<rucene::document::NumericValue> {
                self.$base.get_specific_value(dim, path)
            }

            fn get_all_dims(
                &self,
                top_n: i32,
            ) -> rucene::error::Result<Vec<crate::facet_result::FacetResult>> {
                self.$base.get_all_dims(top_n)
            }

            fn get_top_dims(
                &self,
                top_n_dims: i32,
                top_n_children: i32,
            ) -> rucene::error::Result<Vec<crate::facet_result::FacetResult>> {
                self.$base.get_top_dims(top_n_dims, top_n_children)
            }
        }

        impl $ty {
            /// Equivalent to `TaxonomyFacets.childrenLoaded()`.
            pub fn children_loaded(&self) -> bool {
                self.$base.children_loaded()
            }

            /// Equivalent to `TaxonomyFacets.siblingsLoaded()`.
            pub fn siblings_loaded(&self) -> bool {
                self.$base.siblings_loaded()
            }
        }
    };
}
pub(crate) use taxonomy_facets_delegate;
