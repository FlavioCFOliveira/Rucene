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

//! Port of `TaxonomyFacetIntAssociations.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::DocValues;
use rucene::search::NO_MORE_DOCS;

use crate::facet_utils::Intersect2;
use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::facets_config::{FacetsConfig, DEFAULT_INDEX_FIELD_NAME};
use crate::taxonomy::association_aggregation_function::AssociationAggregationFunction;
use crate::taxonomy::int_taxonomy_facets::IntTaxonomyFacets;
use crate::taxonomy::taxonomy_facets::{taxonomy_facets_delegate, TaxonomyFacets, ValueStore};
use crate::taxonomy::taxonomy_reader::TaxonomyReader;

/// Aggregates int values previously indexed with `IntAssociationFacetField`,
/// assuming the default encoding.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyFacetIntAssociations`.
pub struct TaxonomyFacetIntAssociations {
    base: TaxonomyFacets,
}

impl TaxonomyFacetIntAssociations {
    /// Create `TaxonomyFacetIntAssociations` against the default index field.
    ///
    /// Equivalent to `TaxonomyFacetIntAssociations(TaxonomyReader,
    /// FacetsConfig, FacetsCollector, AssociationAggregationFunction)`.
    pub fn new(
        taxo_reader: Arc<dyn TaxonomyReader>,
        config: Arc<FacetsConfig>,
        fc: &FacetsCollector,
        aggregation_function: Arc<dyn AssociationAggregationFunction>,
    ) -> Result<Self> {
        Self::with_index_field_name(
            DEFAULT_INDEX_FIELD_NAME,
            taxo_reader,
            config,
            fc,
            aggregation_function,
        )
    }

    /// Create `TaxonomyFacetIntAssociations` against the specified index
    /// field.
    ///
    /// Equivalent to `TaxonomyFacetIntAssociations(String, TaxonomyReader,
    /// FacetsConfig, FacetsCollector, AssociationAggregationFunction)`.
    pub fn with_index_field_name(
        index_field_name: &str,
        taxo_reader: Arc<dyn TaxonomyReader>,
        config: Arc<FacetsConfig>,
        fc: &FacetsCollector,
        aggregation_function: Arc<dyn AssociationAggregationFunction>,
    ) -> Result<Self> {
        let base = TaxonomyFacets::new(
            index_field_name,
            taxo_reader,
            config,
            Some(fc),
            ValueStore::Int(IntTaxonomyFacets::new(Arc::clone(&aggregation_function))),
        )?;
        let mut this = Self { base };
        this.aggregate_values(&*aggregation_function, fc.get_matching_docs()?)?;
        Ok(this)
    }

    /// Aggregate from indexed association values.
    ///
    /// Equivalent to `aggregateValues(AssociationAggregationFunction, List)`.
    fn aggregate_values(
        &mut self,
        aggregation_function: &dyn AssociationAggregationFunction,
        matching_docs: &[MatchingDocs],
    ) -> Result<()> {
        for hits in matching_docs {
            if hits.total_hits == 0 {
                continue;
            }
            self.base.initialize_value_counters()?;

            let mut dv =
                DocValues::get_binary(&*hits.context.leaf_reader(), &self.base.index_field_name)?;
            let mut bits_it = hits.bits.iterator()?;
            loop {
                let doc = Intersect2::new(&mut *bits_it, &mut *dv).next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                let bytes_ref = dv.binary_value()?;
                let bytes = &bytes_ref.bytes;
                let end = bytes_ref.offset + bytes_ref.length;
                let mut offset = bytes_ref.offset;
                while offset < end {
                    let ord =
                        i32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"));
                    offset += 4;
                    let value =
                        i32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"));
                    offset += 4;
                    // TODO: Can we optimize the null check in setValue? See
                    // LUCENE-10373.
                    let ValueStore::Int(int) = &mut self.base.values else {
                        unreachable!("an IntTaxonomyFacets value");
                    };
                    let current_value = int.get_value(ord);
                    let new_value = aggregation_function.aggregate_int(current_value, value);
                    int.set_value(ord, new_value);
                    let count = self.base.get_count(ord) + 1;
                    self.base.set_count(ord, count);
                }
            }
        }
        Ok(())
    }
}

taxonomy_facets_delegate!(TaxonomyFacetIntAssociations, base);
