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

//! Port of `TaxonomyFacetLabels.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{DocValues, LeafReaderContext, SortedNumericDocValues};

use crate::taxonomy::facet_label::FacetLabel;
use crate::taxonomy::parallel_taxonomy_arrays::ParallelTaxonomyArrays;
use crate::taxonomy::taxonomy_reader::{TaxonomyReader, INVALID_ORDINAL, ROOT_ORDINAL};

/// Utility class to easily retrieve previously indexed facet labels,
/// allowing you to skip also adding stored fields for these values, reducing
/// your index size.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyFacetLabels`.
pub struct TaxonomyFacetLabels {
    /// Taxonomy reader used to read the facet labels.
    taxo_reader: Arc<dyn TaxonomyReader>,
    /// field storing the taxonomy ordinals
    index_field_name: String,
}

impl TaxonomyFacetLabels {
    /// Sole constructor.
    ///
    /// Equivalent to `TaxonomyFacetLabels(TaxonomyReader, String)`.
    pub fn new(taxo_reader: Arc<dyn TaxonomyReader>, index_field_name: &str) -> Self {
        Self {
            taxo_reader,
            index_field_name: index_field_name.to_string(),
        }
    }

    /// Create and return an instance of `FacetLabelReader` to retrieve facet
    /// labels for multiple documents and (optionally) for a specific
    /// dimension.
    ///
    /// Equivalent to `getFacetLabelReader(LeafReaderContext)`.
    pub fn get_facet_label_reader(
        &self,
        reader_context: &LeafReaderContext,
    ) -> Result<FacetLabelReader> {
        // `DocValues.getSortedNumeric` never returns null, so Java's
        // `emptySortedNumeric()` fallback is unreachable.
        let ordinal_values =
            DocValues::get_sorted_numeric(&*reader_context.leaf_reader(), &self.index_field_name)?;
        Ok(FacetLabelReader::new(
            Arc::clone(&self.taxo_reader),
            ordinal_values,
        ))
    }
}

/// Facet labels reader for faceting.
///
/// Equivalent to the inner class
/// `org.apache.lucene.facet.taxonomy.TaxonomyFacetLabels.FacetLabelReader`,
/// which reads its enclosing instance's `taxoReader`.
pub struct FacetLabelReader {
    taxo_reader: Arc<dyn TaxonomyReader>,
    ordinal_values: Box<dyn SortedNumericDocValues>,
    current_doc_id: i32,
    current_doc_has_values: bool,
    current_pos: i32,
    current_doc_ordinal_count: i32,
    /// Lazily set when nextFacetLabel(int docId, String facetDimension) is
    /// first called.
    parents: Option<Arc<dyn ParallelTaxonomyArrays>>,
}

impl FacetLabelReader {
    /// Sole constructor.
    ///
    /// Equivalent to `FacetLabelReader(SortedNumericDocValues)`.
    pub fn new(
        taxo_reader: Arc<dyn TaxonomyReader>,
        ordinal_values: Box<dyn SortedNumericDocValues>,
    ) -> Self {
        Self {
            taxo_reader,
            ordinal_values,
            current_doc_id: -1,
            current_doc_has_values: false,
            current_pos: 0,
            current_doc_ordinal_count: 0,
            parents: None,
        }
    }

    fn position(&mut self, doc_id: i32) -> Result<()> {
        if self.current_doc_id != doc_id {
            if doc_id < self.current_doc_id {
                return Err(LuceneError::illegal_argument(format!(
                    "docs out of order: previous docId={} current docId={}",
                    self.current_doc_id, doc_id
                )));
            }
            self.current_doc_id = doc_id;
            self.current_doc_has_values = self.ordinal_values.advance_exact(doc_id)?;
            if self.current_doc_has_values {
                self.current_doc_ordinal_count = self.ordinal_values.doc_value_count()?;
                self.current_pos = 0;
            }
        }
        Ok(())
    }

    /// Retrieves the next `FacetLabel` for the specified `doc_id`, or `None`
    /// if there are no more.
    ///
    /// Equivalent to `nextFacetLabel(int)`.
    pub fn next_facet_label(&mut self, doc_id: i32) -> Result<Option<FacetLabel>> {
        self.position(doc_id)?;

        if !self.current_doc_has_values {
            return Ok(None);
        }

        debug_assert!(self.current_pos <= self.current_doc_ordinal_count);
        if self.current_pos == self.current_doc_ordinal_count {
            return Ok(None);
        }

        let ord = self.ordinal_values.next_value()? as i32;
        self.current_pos += 1;
        self.taxo_reader.get_path(ord)
    }

    /// Equivalent to `isDescendant(int, int)`.
    fn is_descendant(&self, mut ord: i32, ancestor_ord: i32) -> bool {
        let parents = self
            .parents
            .as_ref()
            .expect("INVARIANT: parents loaded before use")
            .parents();
        while ord != INVALID_ORDINAL && ord != ROOT_ORDINAL {
            if parents.get(ord) == ancestor_ord {
                return true;
            }
            ord = parents.get(ord);
        }
        false
    }

    /// Retrieves the next `FacetLabel` for the specified `doc_id` under the
    /// requested `facet_dimension`, or `None` if there are no more.
    ///
    /// Equivalent to `nextFacetLabel(int, String)`; Java's `null` dimension
    /// check is carried by the type.
    pub fn next_facet_label_for_dimension(
        &mut self,
        doc_id: i32,
        facet_dimension: &str,
    ) -> Result<Option<FacetLabel>> {
        let parent_ord = self
            .taxo_reader
            .get_ordinal(&FacetLabel::new([facet_dimension])?)?;
        if parent_ord == INVALID_ORDINAL {
            return Err(LuceneError::illegal_argument(format!(
                "Category ordinal not found for facet dimension: {facet_dimension}"
            )));
        }

        self.position(doc_id)?;

        if !self.current_doc_has_values {
            return Ok(None);
        }

        debug_assert!(self.current_pos <= self.current_doc_ordinal_count);
        if self.current_pos == self.current_doc_ordinal_count {
            return Ok(None);
        }

        if self.parents.is_none() {
            self.parents = Some(self.taxo_reader.get_parallel_taxonomy_arrays()?);
        }

        loop {
            let ord = self.ordinal_values.next_value()? as i32;
            self.current_pos += 1;
            if self.is_descendant(ord, parent_ord) {
                return self.taxo_reader.get_path(ord);
            }
            if self.current_pos >= self.current_doc_ordinal_count {
                break;
            }
        }
        Ok(None)
    }
}
