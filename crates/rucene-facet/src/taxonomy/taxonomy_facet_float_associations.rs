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

//! Port of `TaxonomyFacetFloatAssociations.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::DocValues;
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::NO_MORE_DOCS;

use crate::facet_utils::Intersect2;
use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::facets_config::{FacetsConfig, DEFAULT_INDEX_FIELD_NAME};
use crate::taxonomy::association_aggregation_function::AssociationAggregationFunction;
use crate::taxonomy::float_taxonomy_facets::FloatTaxonomyFacets;
use crate::taxonomy::taxonomy_facets::{taxonomy_facets_delegate, TaxonomyFacets, ValueStore};
use crate::taxonomy::taxonomy_reader::TaxonomyReader;

/// Aggregates float values associated with facet fields. Supports two
/// different approaches: 1. Fields can be indexed with
/// `FloatAssociationFacetField` to associate float values; 2. Values can be
/// computed from a `DoubleValuesSource`.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyFacetFloatAssociations`.
pub struct TaxonomyFacetFloatAssociations {
    base: TaxonomyFacets,
}

impl TaxonomyFacetFloatAssociations {
    /// Create an instance using the default index field.
    ///
    /// Equivalent to `TaxonomyFacetFloatAssociations(TaxonomyReader,
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

    /// Create an instance for the default index field computing values from
    /// a `DoubleValuesSource`.
    ///
    /// Equivalent to `TaxonomyFacetFloatAssociations(TaxonomyReader,
    /// FacetsConfig, FacetsCollector, AssociationAggregationFunction,
    /// DoubleValuesSource)`.
    pub fn with_values_source(
        taxo_reader: Arc<dyn TaxonomyReader>,
        config: Arc<FacetsConfig>,
        fc: &FacetsCollector,
        aggregation_function: Arc<dyn AssociationAggregationFunction>,
        values_source: Arc<dyn DoubleValuesSource>,
    ) -> Result<Self> {
        Self::with_index_field_name_and_values_source(
            DEFAULT_INDEX_FIELD_NAME,
            taxo_reader,
            config,
            fc,
            aggregation_function,
            values_source,
        )
    }

    /// Create an instance for a specific index field.
    ///
    /// Equivalent to `TaxonomyFacetFloatAssociations(String, TaxonomyReader,
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
            ValueStore::Float(FloatTaxonomyFacets::new(Arc::clone(&aggregation_function))),
        )?;
        let mut this = Self { base };
        this.aggregate_values(&*aggregation_function, fc.get_matching_docs())?;
        Ok(this)
    }

    /// Create an instance for a specific index field computing values from a
    /// `DoubleValuesSource`.
    ///
    /// Equivalent to `TaxonomyFacetFloatAssociations(String, TaxonomyReader,
    /// FacetsConfig, FacetsCollector, AssociationAggregationFunction,
    /// DoubleValuesSource)`.
    pub fn with_index_field_name_and_values_source(
        index_field_name: &str,
        taxo_reader: Arc<dyn TaxonomyReader>,
        config: Arc<FacetsConfig>,
        fc: &FacetsCollector,
        aggregation_function: Arc<dyn AssociationAggregationFunction>,
        values_source: Arc<dyn DoubleValuesSource>,
    ) -> Result<Self> {
        let base = TaxonomyFacets::new(
            index_field_name,
            taxo_reader,
            config,
            Some(fc),
            ValueStore::Float(FloatTaxonomyFacets::new(Arc::clone(&aggregation_function))),
        )?;
        let mut this = Self { base };
        this.aggregate_values_from_source(
            &*aggregation_function,
            fc.get_matching_docs(),
            fc.get_keep_scores(),
            values_source,
        )?;
        Ok(this)
    }

    fn add(&mut self, aggregation_function: &dyn AssociationAggregationFunction, ord: i32, value: f32) {
        let ValueStore::Float(float) = &mut self.base.values else {
            unreachable!("a FloatTaxonomyFacets value");
        };
        let current_value = float.get_value(ord);
        let new_value = aggregation_function.aggregate_float(current_value, value);
        float.set_value(ord, new_value);
        let count = self.base.get_count(ord) + 1;
        self.base.set_count(ord, count);
    }

    /// Aggregate using the provided `DoubleValuesSource`.
    ///
    /// Equivalent to `aggregateValues(AssociationAggregationFunction, List,
    /// boolean, DoubleValuesSource)`.
    fn aggregate_values_from_source(
        &mut self,
        aggregation_function: &dyn AssociationAggregationFunction,
        matching_docs: &[MatchingDocs],
        keep_scores: bool,
        value_source: Arc<dyn DoubleValuesSource>,
    ) -> Result<()> {
        for hits in matching_docs {
            if hits.total_hits == 0 {
                continue;
            }
            self.base.initialize_value_counters()?;

            let mut ordinal_values =
                DocValues::get_sorted_numeric(&*hits.context.leaf_reader(), &self.base.index_field_name)?;
            let scores: Option<Box<dyn DoubleValues + '_>> = if keep_scores {
                Some(Box::new(ScoresValues {
                    scores: hits.scores.clone().unwrap_or_default(),
                    index: -1,
                }))
            } else {
                None
            };
            let mut function_values = Arc::clone(&value_source).get_values(&hits.context, scores)?;
            let mut bits_it = hits.bits.iterator()?;

            loop {
                let doc = Intersect2::new(&mut *bits_it, &mut *ordinal_values).next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                if function_values.advance_exact(doc)? {
                    let value = function_values.double_value()? as f32;
                    let ordinal_count = ordinal_values.doc_value_count()?;
                    for _ in 0..ordinal_count {
                        let ord = ordinal_values.next_value()? as i32;
                        self.add(aggregation_function, ord, value);
                    }
                }
            }
        }

        // Hierarchical dimensions are supported when using a value source, so
        // we need to rollup:
        self.base.rollup()
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

            let mut dv = DocValues::get_binary(&*hits.context.leaf_reader(), &self.base.index_field_name)?;
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
                    let ord = i32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"));
                    offset += 4;
                    let value = f32::from_bits(u32::from_be_bytes(
                        bytes[offset..offset + 4].try_into().expect("4 bytes"),
                    ));
                    offset += 4;
                    // TODO: Can we optimize the null check in setValue? See
                    // LUCENE-10373.
                    self.add(aggregation_function, ord, value);
                }
            }
        }
        Ok(())
    }
}

/// The anonymous `DoubleValues` of `scores(MatchingDocs)`.
struct ScoresValues {
    scores: Vec<f32>,
    index: i32,
}

impl DoubleValues for ScoresValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(self.scores[self.index as usize] as f64)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.index = doc;
        Ok(true)
    }
}

taxonomy_facets_delegate!(TaxonomyFacetFloatAssociations, base);
