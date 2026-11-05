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

//! Port of `org.apache.lucene.tests.index.MergeReaderWrapper`: a
//! `LeafReader` over a `CodecReader` that reads through the merge instances
//! of its producers.

use std::fmt;
use std::sync::{Arc, Mutex};

use rucene::codecs::doc_values::DocValuesProducer;
use rucene::codecs::norms::NormsProducer;
use rucene::codecs::postings::FieldsProducer;
use rucene::codecs::stored_fields::StoredFieldsReader;
use rucene::codecs::term_vectors::TermVectorsReader;
use rucene::error::Result;
use rucene::index::codec_reader::CodecReader;
use rucene::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
use rucene::index::leaf_reader::{EmptyTermVectors, TermVectors};
use rucene::index::{
    BinaryDocValues, ByteVectorValues, DocValuesSkipper, DocValuesType, FieldInfo, FieldInfos,
    FloatVectorValues, LeafMetaData, LeafReader, NumericDocValues, PointValues, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues, Terms,
};
use rucene::search::knn::KnnCollector;
use rucene::search::AcceptDocs;
use rucene::util::Bits;

use super::asserting_leaf_reader::npe;
use super::shared_producers::{NullStoredFields, StoredFieldsReaderView, TermVectorsReaderView};

/// Port of the package-private `org.apache.lucene.tests.index.MergeReaderWrapper`
/// (`extends LeafReader`): terms, norms, doc values, stored fields and term
/// vectors are read through the merge instances (`getMergeInstance()`) of
/// the wrapped codec reader's producers, taken once when the wrapper is
/// built; points, vectors, live docs and field infos are the wrapped
/// reader's. It is a reader of its own (its own reference count), whose close
/// closes the wrapped reader.
///
/// As in Java, a doc-values or norms accessor of a reader without that
/// producer dereferences the missing producer for a field that has such
/// values: the `NullPointerException`.
///
/// **Visibility.** Java declares the class package-private (its one user is
/// `MockRandomMergePolicy`); it is public here so that the crate's tests
/// reach it.
pub struct MergeReaderWrapper {
    core: IndexReaderCore,
    r#in: Arc<dyn CodecReader>,
    fields: Option<Box<dyn FieldsProducer>>,
    norms: Option<Box<dyn NormsProducer>>,
    doc_values: Option<Box<dyn DocValuesProducer>>,
    store: Option<Arc<dyn StoredFieldsReader>>,
    vectors: Option<Arc<Mutex<Box<dyn TermVectorsReader>>>>,
}

impl MergeReaderWrapper {
    /// `new MergeReaderWrapper(in)`: takes the merge instance of each
    /// producer `in` has.
    ///
    /// # Errors
    ///
    /// What asking `in` for a producer, or a producer for its merge
    /// instance, raises.
    pub fn new(r#in: Arc<dyn CodecReader>) -> Result<Self> {
        let fields = match r#in.get_postings_reader()? {
            Some(fields) => Some(fields.get_merge_instance()?),
            None => None,
        };
        let norms = match r#in.get_norms_reader()? {
            Some(norms) => Some(norms.get_merge_instance()?),
            None => None,
        };
        let doc_values = match r#in.get_doc_values_reader()? {
            Some(doc_values) => Some(doc_values.get_merge_instance()?),
            None => None,
        };
        let store = match r#in.get_fields_reader()? {
            Some(store) => Some(Arc::from(store.get_merge_instance()?)),
            None => None,
        };
        let vectors = match r#in.get_term_vectors_reader()? {
            Some(vectors) => Some(Arc::new(Mutex::new(vectors.get_merge_instance()?))),
            None => None,
        };
        Ok(MergeReaderWrapper {
            core: IndexReaderCore::new(),
            r#in,
            fields,
            norms,
            doc_values,
            store,
            vectors,
        })
    }

    /// The wrapped codec reader (Java's package-private `in` field).
    pub fn get_delegate(&self) -> &Arc<dyn CodecReader> {
        &self.r#in
    }

    /// `getFieldInfos().fieldInfo(field)` when the field has doc values of
    /// `doc_values_type`.
    fn dv_field(&self, field: &str, doc_values_type: DocValuesType) -> Option<FieldInfo> {
        let infos = self.get_field_infos();
        let fi = infos.field_info(field)?;
        if fi.get_doc_values_type() != doc_values_type {
            // Field was not indexed with doc values
            return None;
        }
        Some(fi.clone())
    }

    /// `docValues`, whose dereference throws the `NullPointerException` when
    /// the wrapped reader has no doc-values producer.
    fn doc_values(&self) -> Result<&dyn DocValuesProducer> {
        self.doc_values.as_deref().ok_or_else(npe)
    }
}

impl fmt::Debug for MergeReaderWrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MergeReaderWrapper").finish_non_exhaustive()
    }
}

impl fmt::Display for MergeReaderWrapper {
    /// `"MergeReaderWrapper(" + in + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MergeReaderWrapper({})", self.r#in)
    }
}

impl LeafReader for MergeReaderWrapper {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.core.ensure_open()?;
        // We could check the FieldInfo IndexOptions but there's no point since
        //   PostingsReader will simply return null for fields that don't exist or that have no terms
        // index.
        match &self.fields {
            None => Ok(None),
            Some(fields) => fields.terms(field),
        }
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.core.ensure_open()?;
        match self.dv_field(field, DocValuesType::NUMERIC) {
            None => Ok(None),
            Some(fi) => self.doc_values()?.get_numeric(&fi),
        }
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.core.ensure_open()?;
        match self.dv_field(field, DocValuesType::BINARY) {
            None => Ok(None),
            Some(fi) => self.doc_values()?.get_binary(&fi),
        }
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.core.ensure_open()?;
        match self.dv_field(field, DocValuesType::SORTED) {
            None => Ok(None),
            Some(fi) => self.doc_values()?.get_sorted(&fi),
        }
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.core.ensure_open()?;
        match self.dv_field(field, DocValuesType::SORTED_NUMERIC) {
            None => Ok(None),
            Some(fi) => self.doc_values()?.get_sorted_numeric(&fi),
        }
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.core.ensure_open()?;
        match self.dv_field(field, DocValuesType::SORTED_SET) {
            None => Ok(None),
            Some(fi) => self.doc_values()?.get_sorted_set(&fi),
        }
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.core.ensure_open()?;
        let infos = self.get_field_infos();
        match infos.field_info(field) {
            // Field does not exist or does not index norms
            Some(fi) if fi.has_norms() => self.norms.as_deref().ok_or_else(npe)?.get_norms(fi),
            _ => Ok(None),
        }
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.core.ensure_open()?;
        let infos = self.get_field_infos();
        match infos.field_info(field) {
            // Field does not exist
            None => Ok(None),
            Some(fi) => self.doc_values()?.get_skipper(fi),
        }
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.r#in.get_field_infos()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.r#in.get_live_docs()
    }

    fn check_integrity(&self) -> Result<()> {
        self.r#in.check_integrity()
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.core.ensure_open()?;
        Ok(match &self.vectors {
            None => Box::new(EmptyTermVectors),
            Some(vectors) => Box::new(TermVectorsReaderView(Arc::clone(vectors))),
        })
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        self.r#in.get_point_values(field)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        self.r#in.get_float_vector_values(field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        self.r#in.get_byte_vector_values(field)
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.r#in
            .search_nearest_vectors(field, target, collector, accept_docs)
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        self.r#in
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }

    fn num_docs(&self) -> i32 {
        self.r#in.num_docs()
    }

    fn max_doc(&self) -> i32 {
        self.r#in.max_doc()
    }

    /// The `store` merge instance itself, or the `null` Java returns for a
    /// reader without stored fields.
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.core.ensure_open()?;
        Ok(match &self.store {
            None => Box::new(NullStoredFields),
            Some(store) => Box::new(StoredFieldsReaderView(Arc::clone(store))),
        })
    }

    /// `in.close()`.
    fn do_close(&self) -> Result<()> {
        Arc::clone(&self.r#in).as_index_reader().close()
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_core_cache_helper()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_reader_cache_helper()
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.r#in.get_meta_data()
    }
}
