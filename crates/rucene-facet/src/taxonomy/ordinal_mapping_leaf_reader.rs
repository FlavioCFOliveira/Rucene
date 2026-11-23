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

//! Port of `OrdinalMappingLeafReader.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::{CacheHelper, StoredFields};
use rucene::index::{
    BinaryDocValues, DocValuesIterator, LeafReader, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues, TermVectors, Terms,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};

use crate::facets_config::{default_dim_config, FacetsConfig};

/// A `FilterLeafReader` for updating facets ordinal references, based on an
/// ordinal map.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.OrdinalMappingLeafReader`.
pub struct OrdinalMappingLeafReader {
    base: FilterLeafReader,
    ordinal_map: Arc<[i32]>,
    facet_fields: HashSet<String>,
}

impl OrdinalMappingLeafReader {
    /// Wraps a `LeafReader`, mapping ordinals according to the ordinal map.
    ///
    /// Equivalent to `OrdinalMappingLeafReader(LeafReader, int[],
    /// FacetsConfig)`.
    pub fn new(
        input: Arc<dyn LeafReader>,
        ordinal_map: Vec<i32>,
        src_config: &FacetsConfig,
    ) -> Result<Self> {
        let base = FilterLeafReader::new(input)?;
        let mut facet_fields = HashSet::new();
        for dc in src_config
            .get_dim_configs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
        {
            facet_fields.insert(dc.index_field_name());
        }
        // always add the default indexFieldName. This is because FacetsConfig
        // does not explicitly record dimensions that were indexed under the
        // default DimConfig, unless they have a custom DimConfig.
        facet_fields.insert(default_dim_config().index_field_name());
        Ok(Self {
            base,
            ordinal_map: Arc::from(ordinal_map),
            facet_fields,
        })
    }

    fn input(&self) -> &Arc<dyn LeafReader> {
        self.base.get_delegate()
    }
}

impl fmt::Debug for OrdinalMappingLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrdinalMappingLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for OrdinalMappingLeafReader {
    /// Inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.base, f)
    }
}

impl LeafReader for OrdinalMappingLeafReader {
    forward_leaf_reader!(filter base;
        core,
        filter_leaf_reader_delegate,
        do_close,
        term_vectors,
        num_docs,
        max_doc,
        stored_fields,
        terms,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
        get_sorted_set_doc_values,
        get_norm_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_field_infos,
        get_live_docs,
        get_point_values,
        check_integrity,
        get_meta_data,
    );

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let original = self.input().get_sorted_numeric_doc_values(field)?;
        match original {
            Some(original) if self.facet_fields.contains(field) => Ok(Some(Box::new(
                OrdinalMappingSortedNumericDocValues::new(original, Arc::clone(&self.ordinal_map)),
            ))),
            other => Ok(other),
        }
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

#[allow(unused_imports)]
use {
    BinaryDocValues as _, SortedDocValues as _, SortedSetDocValues as _, StoredFields as _,
    TermVectors as _, Terms as _,
};

/// Equivalent to
/// `org.apache.lucene.facet.taxonomy.OrdinalMappingLeafReader.OrdinalMappingSortedNumericDocValues`,
/// a `FilterSortedNumericDocValues` reading its enclosing reader's
/// `ordinalMap`.
pub(crate) struct OrdinalMappingSortedNumericDocValues {
    input: Box<dyn SortedNumericDocValues>,
    ordinal_map: Arc<[i32]>,
    current_values: Vec<i32>,
    curr_index: usize,
}

impl OrdinalMappingSortedNumericDocValues {
    /// Equivalent to `OrdinalMappingSortedNumericDocValues(SortedNumericDocValues)`.
    fn new(input: Box<dyn SortedNumericDocValues>, ordinal_map: Arc<[i32]>) -> Self {
        Self {
            input,
            ordinal_map,
            current_values: Vec::with_capacity(32),
            curr_index: 0,
        }
    }

    /// Inefficient, but we can't just read the values as they're sorted
    /// differently
    ///
    /// Equivalent to `reloadValues()`.
    fn reload_values(&mut self) -> Result<()> {
        self.curr_index = 0;
        self.current_values.clear();
        for _ in 0..self.input.doc_value_count()? {
            let original_ord = i32::try_from(self.input.next_value()?)
                .map_err(|_| LuceneError::integer_overflow())?;
            self.current_values
                .push(self.ordinal_map[original_ord as usize]);
        }
        self.current_values.sort_unstable();
        Ok(())
    }
}

impl DocIdSetIterator for OrdinalMappingSortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.input.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let result = self.input.next_doc()?;
        if result != NO_MORE_DOCS {
            self.reload_values()?;
        }
        Ok(result)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let result = self.input.advance(target)?;
        if result != NO_MORE_DOCS {
            self.reload_values()?;
        }
        Ok(result)
    }

    fn cost(&self) -> i64 {
        self.input.cost()
    }
}

impl DocValuesIterator for OrdinalMappingSortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let result = self.input.advance_exact(target)?;
        if result {
            self.reload_values()?;
        }
        Ok(result)
    }
}

impl SortedNumericDocValues for OrdinalMappingSortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        debug_assert!(self.curr_index < self.current_values.len());
        let actual = self.current_values[self.curr_index];
        self.curr_index += 1;
        Ok(actual as i64)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.current_values.len() as i32)
    }
}

#[allow(dead_code)]
fn _config_type(_: &FacetsConfig) {}
