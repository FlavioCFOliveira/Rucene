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

//! Port of `DefaultSortedSetDocValuesReaderState.java`, of the Java package
//! `org.apache.lucene.facet.sortedset`.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use rucene::error::{LuceneError, Result};
use rucene::index::multi_doc_values::{MultiDocValues, MultiSortedSetDocValues, WithMapping};
use rucene::index::{DocValuesType, IndexReader, OrdinalMap, SortedSetDocValues};
use rucene::util::java_collections::JavaHashMap;
use rucene::util::{Accountable, Accountables};

use crate::facets_config::{FacetsConfig, DEFAULT_INDEX_FIELD_NAME};
use crate::sortedset::sorted_set_doc_values_reader_state::{
    DimTree, OrdRange, SortedSetDocValuesReaderState, INVALID_ORDINAL,
};
use crate::taxonomy::facet_label::arrays_to_string;

/// Default implementation of `SortedSetDocValuesFacetCounts`. You must ensure
/// the original `IndexReader` passed to the constructor is not closed whenever
/// you use this class!
///
/// Equivalent to `org.apache.lucene.facet.sortedset.DefaultSortedSetDocValuesReaderState`.
pub struct DefaultSortedSetDocValuesReaderState {
    field: String,
    value_count: i32,
    /// `IndexReader` passed to the constructor.
    pub reader: Arc<dyn IndexReader>,
    cached_ord_map: Mutex<JavaHashMap<Arc<OrdinalMap>>>,
    config: Arc<FacetsConfig>,
    /// Used for hierarchical dims.
    prefix_to_dim_tree: JavaHashMap<DimTree>,
    /// Used for flat dims.
    prefix_to_ord_range: JavaHashMap<OrdRange>,
}

impl DefaultSortedSetDocValuesReaderState {
    /// Creates this, pulling doc values from the default
    /// `FacetsConfig.DEFAULT_INDEX_FIELD_NAME`.
    ///
    /// Equivalent to `DefaultSortedSetDocValuesReaderState(IndexReader,
    /// FacetsConfig)`.
    pub fn new(reader: Arc<dyn IndexReader>, config: Arc<FacetsConfig>) -> Result<Self> {
        Self::with_field(reader, DEFAULT_INDEX_FIELD_NAME, config)
    }

    /// Creates this, pulling doc values from the specified field.
    ///
    /// Equivalent to `DefaultSortedSetDocValuesReaderState(IndexReader,
    /// String, FacetsConfig)`.
    pub fn with_field(reader: Arc<dyn IndexReader>, field: &str, config: Arc<FacetsConfig>) -> Result<Self> {
        let mut this = Self {
            field: field.to_string(),
            value_count: 0,
            reader,
            cached_ord_map: Mutex::new(JavaHashMap::new()),
            config,
            prefix_to_dim_tree: JavaHashMap::new(),
            prefix_to_ord_range: JavaHashMap::new(),
        };

        // We need this to create thread-safe MultiSortedSetDV per collector:
        let Some((mut dv, _)) = this.get_doc_values()? else {
            return Err(LuceneError::illegal_argument(format!(
                "field \"{field}\" was not indexed with SortedSetDocValues"
            )));
        };
        if dv.get_value_count()? > i32::MAX as i64 {
            return Err(LuceneError::illegal_argument(format!(
                "can only handle valueCount < Integer.MAX_VALUE; got {}",
                dv.get_value_count()?
            )));
        }
        this.value_count = dv.get_value_count()? as i32;

        let mut ord = 0;
        while ord != this.value_count {
            let term = dv.lookup_ord(ord as i64)?.utf8_to_string()?;
            let components = FacetsConfig::string_to_path(&term);
            let dim = &components[0];
            if this.config.get_dim_config(dim).hierarchical() {
                ord = this.create_one_hierarchical_facet_dim_state(&mut *dv, ord)? + 1;
            } else {
                ord = this.create_one_flat_facet_dim_state(&mut *dv, ord)? + 1;
            }
        }
        Ok(this)
    }

    /// Equivalent to `createOneHierarchicalFacetDimState(SortedSetDocValues,
    /// int)`.
    fn create_one_hierarchical_facet_dim_state(
        &mut self,
        dv: &mut dyn SortedSetDocValues,
        dim_start_ord: i32,
    ) -> Result<i32> {
        let mut has_children: Vec<bool> = Vec::new();
        let mut siblings: Vec<i32> = Vec::new();

        // stack of paths with unfulfilled siblings
        let mut sibling_stack: Vec<OrdAndComponent> = Vec::new();

        let mut dim_end_ord = dim_start_ord;

        let mut next_components =
            FacetsConfig::string_to_path(&dv.lookup_ord(dim_end_ord as i64)?.utf8_to_string()?);
        let dim = next_components[0].clone();

        loop {
            let components = next_components;

            let ord = dim_end_ord - dim_start_ord;

            while let Some(top) = sibling_stack.last() {
                if top.component.len() < components.len() {
                    break;
                }
                let possible_sibling = sibling_stack.pop().expect("INVARIANT: peeked");
                if possible_sibling.component.len() == components.len() {
                    // lengths are equal, all non-sibling paths of this length
                    // have been popped off the stack, so this must be a
                    // sibling
                    siblings[possible_sibling.ord as usize] = ord;
                }
            }

            if dim_end_ord + 1 == self.value_count {
                // current ord is the last ord, so it has no children and no
                // siblings
                siblings.push(-1);
                has_children.push(false);
                break;
            }

            next_components = FacetsConfig::string_to_path(
                &dv.lookup_ord(dim_end_ord as i64 + 1)?.utf8_to_string()?,
            );

            if next_components[0] != components[0] {
                // current ord is last ord in dimension, so it has no children
                // and no siblings
                siblings.push(-1);
                has_children.push(false);
                break;
            }

            if components.len() < next_components.len() {
                // next ord must be a direct child of current ord, this is
                // because we are indexing all ancestral paths
                has_children.insert(ord as usize, true);
                // we don't know the sibling of the current ord, but it could
                // be one of the ords after its children, so add it to the
                // stack
                sibling_stack.push(OrdAndComponent::new(ord, components));
                // we know the current ord is an ancestor of the next ord, but
                // it could also be the sibling of a previous ord
                siblings.insert(ord as usize, INVALID_ORDINAL);
            } else if components.len() == next_components.len() {
                // the current ord has no children, and its sibling is the
                // next ord
                siblings.insert(ord as usize, ord + 1);
                has_children.insert(ord as usize, false);
            } else {
                // components.length > nextComponents.length
                // the current ord has no children and no sibling
                siblings.insert(ord as usize, INVALID_ORDINAL);
                has_children.insert(ord as usize, false);
            }
            dim_end_ord += 1;
        }
        self.prefix_to_dim_tree
            .insert(dim, DimTree::new(dim_start_ord, siblings, has_children)?);

        Ok(dim_end_ord)
    }

    /// Equivalent to `createOneFlatFacetDimState(SortedSetDocValues, int)`.
    fn create_one_flat_facet_dim_state(
        &mut self,
        dv: &mut dyn SortedSetDocValues,
        dim_start_ord: i32,
    ) -> Result<i32> {
        let mut dim_end_ord = dim_start_ord;

        let next_term = dv.lookup_ord(dim_end_ord as i64)?.utf8_to_string()?;
        let mut next_components = FacetsConfig::string_to_path(&next_term);

        if next_components.len() > 2 {
            return Err(LuceneError::illegal_argument(format!(
                "dimension not configured to handle hierarchical field; got: {} {}",
                arrays_to_string(&next_components),
                next_term
            )));
        }

        let dim = next_components[0].clone();

        loop {
            let components = next_components;

            if dim_end_ord + 1 == self.value_count {
                break;
            }

            let next_term = dv.lookup_ord(dim_end_ord as i64 + 1)?.utf8_to_string()?;
            next_components = FacetsConfig::string_to_path(&next_term);

            if next_components[0] != components[0] {
                break;
            }

            if next_components.len() != 2 {
                return Err(LuceneError::illegal_argument(format!(
                    "dimension not configured to handle hierarchical field; got: {} {}",
                    arrays_to_string(&next_components),
                    next_term
                )));
            }

            dim_end_ord += 1;
        }
        self.prefix_to_ord_range
            .insert(dim, OrdRange::new(dim_start_ord, dim_end_ord));

        Ok(dim_end_ord)
    }
}

impl Accountable for DefaultSortedSetDocValuesReaderState {
    fn ram_bytes_used(&self) -> i64 {
        let cached = self.cached_ord_map.lock().unwrap_or_else(PoisonError::into_inner);
        match cached.get(&self.field) {
            None => 0,
            Some(map) => map.ram_bytes_used(),
        }
    }

    fn child_resources(&self) -> Vec<Box<dyn Accountable + '_>> {
        let cached = self.cached_ord_map.lock().unwrap_or_else(PoisonError::into_inner);
        let entries: Vec<(String, i64)> = cached
            .iter()
            .map(|(k, v)| (k.clone(), v.ram_bytes_used()))
            .collect();
        drop(cached);
        let mut resources: Vec<Box<dyn Accountable + '_>> = Vec::new();
        let mut sorted = entries;
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        for (k, bytes) in sorted {
            resources.push(Box::new(Accountables::named_accountable(
                &format!("DefaultSortedSetDocValuesReaderState '{k}'"),
                bytes,
            )));
        }
        resources
    }
}

impl fmt::Display for DefaultSortedSetDocValuesReaderState {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DefaultSortedSetDocValuesReaderState(field={} reader={})",
            self.field, self.reader
        )
    }
}

impl SortedSetDocValuesReaderState for DefaultSortedSetDocValuesReaderState {
    /// Return top-level doc values.
    fn get_doc_values(&self) -> Result<Option<WithMapping<dyn SortedSetDocValues>>> {
        // TODO: this is dup'd from slow composite reader wrapper ... can we
        // factor it out to share?
        let map;
        {
            let mut cached = self.cached_ord_map.lock().unwrap_or_else(PoisonError::into_inner);
            match cached.get(&self.field) {
                Some(m) => map = Arc::clone(m),
                None => {
                    // uncached, or not a multi dv
                    let Some((dv, mapping)) =
                        MultiDocValues::get_sorted_set_values_and_mapping(&self.reader, &self.field)?
                    else {
                        return Ok(None);
                    };
                    if let Some(mapping) = &mapping {
                        let cache_helper = self.reader.get_reader_cache_helper();
                        if let Some(cache_helper) = cache_helper {
                            if mapping.owner.as_ref() == Some(cache_helper.get_key()) {
                                cached.insert(self.field.clone(), Arc::clone(mapping));
                            }
                        }
                    }
                    return Ok(Some((dv, mapping)));
                }
            }
        }

        let leaves = Arc::clone(&self.reader).leaves()?;
        let size = leaves.len();
        let mut values: Vec<Box<dyn SortedSetDocValues>> = Vec::with_capacity(size);
        let mut starts = vec![0i32; size + 1];
        let mut cost = 0i64;
        for (i, context) in leaves.iter().enumerate() {
            let reader = context.leaf_reader();
            let field_infos = reader.get_field_infos();
            if let Some(field_info) = field_infos.field_info(&self.field) {
                if field_info.get_doc_values_type() != DocValuesType::SORTED_SET {
                    return Ok(None);
                }
            }
            let v: Box<dyn SortedSetDocValues> = match reader.get_sorted_set_doc_values(&self.field)? {
                Some(v) => v,
                None => Box::new(rucene::index::DocValues::empty_sorted_set()),
            };
            cost += rucene::search::DocIdSetIterator::cost(&*v);
            values.push(v);
            starts[i] = context.doc_base();
        }
        starts[size] = self.reader.max_doc();
        Ok(Some((
            Box::new(MultiSortedSetDocValues::new(values, starts, Arc::clone(&map), cost)),
            Some(map),
        )))
    }

    fn get_field(&self) -> &str {
        &self.field
    }

    fn get_reader(&self) -> &Arc<dyn IndexReader> {
        &self.reader
    }

    fn get_size(&self) -> i32 {
        self.value_count
    }

    fn get_facets_config(&self) -> &Arc<FacetsConfig> {
        &self.config
    }

    /// Returns all dims, hierarchical dims first then flat ones, each in
    /// Java's `HashMap` key order.
    fn get_dims(&self) -> Vec<String> {
        self.prefix_to_dim_tree
            .keys()
            .chain(self.prefix_to_ord_range.keys())
            .cloned()
            .collect()
    }

    /// Return the memoized `OrdRange` for the specified dimension.
    fn get_prefix_to_ord_range(&self) -> Vec<(String, OrdRange)> {
        self.prefix_to_ord_range
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    fn get_ord_range(&self, dim: &str) -> Result<Option<OrdRange>> {
        if self.config.get_dim_config(dim).hierarchical() {
            return Err(LuceneError::unsupported_operation(
                "This operation is only supported for flat dimensions",
            ));
        }
        Ok(self.prefix_to_ord_range.get(dim).copied())
    }

    fn get_dim_tree(&self, dim: &str) -> Result<Option<DimTree>> {
        if !self.config.get_dim_config(dim).hierarchical() {
            return Err(LuceneError::unsupported_operation(
                "This operation is only supported for hierarchical facets",
            ));
        }
        Ok(self.prefix_to_dim_tree.get(dim).cloned())
    }
}

/// Helper class to store ordinals allong with their path components.
///
/// Equivalent to
/// `org.apache.lucene.facet.sortedset.DefaultSortedSetDocValuesReaderState.OrdAndComponent`.
pub(crate) struct OrdAndComponent {
    ord: i32,
    component: Vec<String>,
}

impl OrdAndComponent {
    /// Equivalent to `OrdAndComponent(int, String[])`.
    fn new(ord: i32, component: Vec<String>) -> Self {
        Self { ord, component }
    }
}
