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

//! Port of `FloatTaxonomyFacets.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::sync::Arc;

use rucene::internal::hppc::IntFloatHashMap;

use crate::taxonomy::association_aggregation_function::AssociationAggregationFunction;

/// Base class for all taxonomy-based facets that aggregate to float.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.FloatTaxonomyFacets`: the
/// state it adds to `TaxonomyFacets`, carried by
/// [`ValueStore::Float`](crate::taxonomy::taxonomy_facets::ValueStore::Float);
/// its overrides are the `Float` arms of `TaxonomyFacets`.
pub(crate) struct FloatTaxonomyFacets {
    /// Aggregation function used for combining values.
    pub(crate) aggregation_function: Arc<dyn AssociationAggregationFunction>,
    /// Per-ordinal value.
    pub(crate) values: Option<Vec<f32>>,
    /// Sparse ordinal values.
    pub(crate) sparse_values: Option<IntFloatHashMap>,
}

impl FloatTaxonomyFacets {
    /// Equivalent to the field initialization of `FloatTaxonomyFacets(String,
    /// TaxonomyReader, AssociationAggregationFunction, FacetsConfig,
    /// FacetsCollector)`.
    pub(crate) fn new(aggregation_function: Arc<dyn AssociationAggregationFunction>) -> Self {
        Self {
            aggregation_function,
            values: None,
            sparse_values: None,
        }
    }

    /// The part of `initializeValueCounters()` this class overrides.
    pub(crate) fn initialize(&mut self, sparse: bool, size: usize) {
        debug_assert!(self.sparse_values.is_none() && self.values.is_none());
        if sparse {
            self.sparse_values = Some(IntFloatHashMap::new());
        } else {
            self.values = Some(vec![0.0; size]);
        }
    }

    /// Set the value associated with this ordinal to `new_value`.
    ///
    /// Equivalent to `setValue(int, float)`.
    pub(crate) fn set_value(&mut self, ordinal: i32, new_value: f32) {
        if let Some(sparse_values) = &mut self.sparse_values {
            sparse_values.put(ordinal, new_value);
        } else {
            self.values.as_mut().expect("INVARIANT: initialized")[ordinal as usize] = new_value;
        }
    }

    /// Get the value associated with this ordinal.
    ///
    /// Equivalent to `getValue(int)`.
    pub(crate) fn get_value(&self, ordinal: i32) -> f32 {
        if let Some(sparse_values) = &self.sparse_values {
            sparse_values.get(ordinal)
        } else {
            self.values.as_ref().expect("INVARIANT: initialized")[ordinal as usize]
        }
    }
}

/// An accumulator for a float aggregated value.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.FloatTaxonomyFacets.FloatAggregatedValue`,
/// the [`AggregatedValue::Float`](crate::taxonomy::taxonomy_facets::AggregatedValue::Float)
/// variant.
pub struct FloatAggregatedValue;

impl FloatAggregatedValue {
    /// Equivalent to `FloatAggregatedValue(float)`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(value: f32) -> crate::taxonomy::taxonomy_facets::AggregatedValue {
        crate::taxonomy::taxonomy_facets::AggregatedValue::Float(value)
    }
}
