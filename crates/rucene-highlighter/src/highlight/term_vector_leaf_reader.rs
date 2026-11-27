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

//! Port of `TermVectorLeafReader.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
use rucene::index::{
    BinaryDocValues, ByteVectorValues, DocValuesSkipIndexType, DocValuesSkipper, DocValuesType,
    FieldInfo, FieldInfos, Fields, FloatVectorValues, IndexOptions, LeafMetaData, LeafReader,
    NumericDocValues, PointValues, SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
    StoredFieldVisitor, TermVectors, Terms, VectorEncoding, VectorSimilarityFunction,
};
use rucene::search::knn::KnnCollector;
use rucene::search::AcceptDocs;
use rucene::util::extra::Version;
use rucene::util::Bits;

/// The anonymous `Fields` of `TermVectorLeafReader`: one field.
#[derive(Clone)]
struct SingleFields {
    field: String,
    terms: Arc<dyn Terms>,
}

impl fmt::Debug for SingleFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fields")
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl Fields for SingleFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(std::iter::once(self.field.clone()))
    }

    fn terms(&self, fld: &str) -> Result<Option<Arc<dyn Terms>>> {
        if self.field != fld {
            return Ok(None);
        }
        Ok(Some(Arc::clone(&self.terms)))
    }

    fn size(&self) -> i32 {
        1
    }
}

/// Wraps a Terms with a `LeafReader`, typically from term vectors.
///
/// Equivalent to `org.apache.lucene.search.highlight.TermVectorLeafReader`.
pub struct TermVectorLeafReader {
    core: IndexReaderCore,
    fields: SingleFields,
    field_infos: FieldInfos,
}

impl fmt::Debug for TermVectorLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TermVectorLeafReader")
            .field("field", &self.fields.field)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for TermVectorLeafReader {
    /// `Object.toString()`, without its hash code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("org.apache.lucene.search.highlight.TermVectorLeafReader")
    }
}

impl TermVectorLeafReader {
    /// Equivalent to `new TermVectorLeafReader(String, Terms)`.
    pub fn new(field: &str, terms: Arc<dyn Terms>) -> Self {
        let index_options = if !terms.has_freqs() {
            IndexOptions::DOCS
        } else if !terms.has_positions() {
            IndexOptions::DOCS_AND_FREQS
        } else if !terms.has_offsets() {
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
        } else {
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS
        };
        let field_info = FieldInfo::new_full(
            field.to_string(),
            0,
            true,
            true,
            terms.has_payloads(),
            index_options,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            -1,
            Default::default(),
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        )
        .expect("INVARIANT: the field info of a term vector is consistent");
        let field_infos = FieldInfos::new(vec![field_info])
            .expect("INVARIANT: a single field is a valid FieldInfos");
        Self {
            core: IndexReaderCore::new(),
            fields: SingleFields {
                field: field.to_string(),
                terms,
            },
            field_infos,
        }
    }
}

/// The anonymous `TermVectors` of `termVectors()`.
#[derive(Debug)]
struct SingleTermVectors {
    fields: SingleFields,
}

impl TermVectors for SingleTermVectors {
    fn get(&self, doc_id: i32) -> Result<Option<Box<dyn Fields>>> {
        if doc_id != 0 {
            Ok(None)
        } else {
            Ok(Some(Box::new(self.fields.clone())))
        }
    }
}

/// The anonymous `StoredFields` of `storedFields()`.
#[derive(Debug)]
struct EmptyStoredFields;

impl StoredFields for EmptyStoredFields {
    fn document_with_visitor(
        &self,
        _doc_id: i32,
        _visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        Ok(())
    }
}

impl LeafReader for TermVectorLeafReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn do_close(&self) -> Result<()> {
        Ok(())
    }

    fn terms(&self, field: &str) -> Result<Option<Arc<dyn Terms>>> {
        self.fields.terms(field)
    }

    fn get_numeric_doc_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(None)
    }

    fn get_binary_doc_values(&self, _field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        Ok(None)
    }

    fn get_sorted_doc_values(&self, _field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        Ok(None)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        Ok(None)
    }

    fn get_sorted_set_doc_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        Ok(None)
    }

    fn get_doc_values_skipper(&self, _field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Ok(None)
    }

    fn get_norm_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(None) // Is this needed?  See MemoryIndex for a way to do it.
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.field_infos.clone()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        None
    }

    fn get_point_values(&self, _field_name: &str) -> Result<Option<Box<dyn PointValues>>> {
        Ok(None)
    }

    fn get_float_vector_values(
        &self,
        _field_name: &str,
    ) -> Result<Option<Box<dyn FloatVectorValues>>> {
        Ok(None)
    }

    fn get_byte_vector_values(
        &self,
        _field_name: &str,
    ) -> Result<Option<Box<dyn ByteVectorValues>>> {
        Ok(None)
    }

    fn search_nearest_vectors(
        &self,
        _field: &str,
        _target: &[f32],
        _knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn search_nearest_vectors_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        Ok(Box::new(SingleTermVectors {
            fields: self.fields.clone(),
        }))
    }

    fn num_docs(&self) -> i32 {
        1
    }

    fn max_doc(&self) -> i32 {
        1
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        Ok(Box::new(EmptyStoredFields))
    }

    fn get_meta_data(&self) -> LeafMetaData {
        LeafMetaData::new(Version::LATEST.major as i32, None, None, false)
            .expect("INVARIANT: the latest major version is a valid created version")
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}
