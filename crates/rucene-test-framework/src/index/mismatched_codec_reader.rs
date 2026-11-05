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

//! Port of `org.apache.lucene.tests.index.MismatchedCodecReader`: shuffles
//! field numbers around to try to trip bugs where field numbers are assumed
//! to always be consistent across segments.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::doc_values::DocValuesProducer;
use rucene::codecs::knn_vectors::KnnVectorsReader;
use rucene::codecs::norms::NormsProducer;
use rucene::codecs::points::PointsReader;
use rucene::codecs::postings::FieldsProducer;
use rucene::codecs::stored_fields::StoredFieldsReader;
use rucene::codecs::term_vectors::TermVectorsReader;
use rucene::error::Result;
use rucene::index::codec_reader::CodecReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::{
    BinaryDocValues, DocValuesSkipper, FieldInfo, FieldInfos, LeafReader, NumericDocValues,
    SortedDocValues, SortedNumericDocValues, SortedSetDocValues, StoredFieldVisitor,
};

use super::asserting_leaf_reader::npe;
use super::mismatched_leaf_reader::{shuffle_infos, MismatchedVisitor};
use super::shared_producers::shared_close_error;

/// Port of `org.apache.lucene.tests.index.MismatchedCodecReader`
/// (`extends FilterCodecReader`): the wrapped reader with its field infos
/// renumbered in a random order ([`shuffle_infos`]); its stored-fields
/// reader reports the renumbered infos to the visitor, and its doc-values and
/// norms producers map the renumbered infos back to the wrapped reader's.
pub struct MismatchedCodecReader {
    /// The `FilterCodecReader` this class extends: the reader state of this
    /// reader (its own core) and the members it inherits.
    base: rucene::index::filter_reader::FilterCodecReader,
    /// Java's protected `in`: the reader `base` wraps.
    r#in: Arc<dyn CodecReader>,
    shuffled: Arc<FieldInfos>,
}

impl MismatchedCodecReader {
    /// `new MismatchedCodecReader(in, random)`.
    ///
    /// # Errors
    ///
    /// What [`shuffle_infos`] raises.
    pub fn new(r#in: Arc<dyn CodecReader>, random: &dyn Random) -> Result<Self> {
        let shuffled = Arc::new(shuffle_infos(&r#in.get_field_infos(), random)?);
        Ok(MismatchedCodecReader {
            // `super(in)`
            base: rucene::index::filter_reader::FilterCodecReader::new(Arc::clone(&r#in)),
            r#in,
            shuffled,
        })
    }

    /// `getDelegate()`: the wrapped reader.
    pub fn get_delegate(&self) -> &Arc<dyn CodecReader> {
        &self.r#in
    }
}

impl fmt::Debug for MismatchedCodecReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MismatchedCodecReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for MismatchedCodecReader {
    /// Neither `MismatchedCodecReader` nor `FilterCodecReader` overrides
    /// `toString()`: `Object`'s applies.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.tests.index.MismatchedCodecReader",
            self,
        ))
    }
}

impl LeafReader for MismatchedCodecReader {
    forward_leaf_reader!(codec base;
        core,
        num_docs,
        max_doc,
        do_close,
        get_live_docs,
        check_integrity,
        get_meta_data,
    );

    codec_leaf_reader!(
        term_vectors,
        stored_fields,
        terms,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
        get_sorted_numeric_doc_values,
        get_sorted_set_doc_values,
        get_norm_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_point_values,
    );

    /// This reader is a `CodecReader`.
    fn as_codec_reader(self: Arc<Self>) -> Option<Arc<dyn CodecReader>> {
        Some(self)
    }

    fn get_field_infos(&self) -> FieldInfos {
        (*self.shuffled).clone()
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_core_cache_helper()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_reader_cache_helper()
    }
}

impl CodecReader for MismatchedCodecReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        Ok(self.r#in.get_fields_reader()?.map(|r#in| {
            Box::new(MismatchedStoredFieldsReader {
                r#in: Arc::from(r#in),
                shuffled: Arc::clone(&self.shuffled),
            }) as Box<dyn StoredFieldsReader>
        }))
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        self.r#in.get_term_vectors_reader()
    }

    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        Ok(self.r#in.get_norms_reader()?.map(|r#in| {
            Arc::new(MismatchedNormsProducer {
                r#in,
                shuffled: Arc::clone(&self.shuffled),
                orig: Arc::new(self.r#in.get_field_infos()),
            }) as Arc<dyn NormsProducer>
        }))
    }

    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        Ok(self.r#in.get_doc_values_reader()?.map(|r#in| {
            Arc::new(MismatchedDocValuesProducer {
                r#in,
                shuffled: Arc::clone(&self.shuffled),
                orig: Arc::new(self.r#in.get_field_infos()),
            }) as Arc<dyn DocValuesProducer>
        }))
    }

    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        self.r#in.get_postings_reader()
    }

    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        self.r#in.get_points_reader()
    }

    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        self.r#in.get_vector_reader()
    }

    /// `this instanceof FilterCodecReader`: the delegate.
    fn filter_codec_reader_delegate(&self) -> Option<&Arc<dyn CodecReader>> {
        Some(&self.r#in)
    }
}

/// `remapFieldInfo(field)` of the private producers: asserts that the
/// renumbered infos know `field` under its number, and returns the wrapped
/// reader's info of the same name (`null` when it has none, which the wrapped
/// producer then dereferences: the `NullPointerException`).
fn remap_field_info<'o>(
    shuffled: &FieldInfos,
    orig: &'o FieldInfos,
    field: &FieldInfo,
) -> Result<&'o FieldInfo> {
    let fi = shuffled.field_info(&field.name);
    java_assert!(fi.is_some_and(|fi| fi.number == field.number));
    orig.field_info(&field.name).ok_or_else(npe)
}

/// Port of the private `MismatchedCodecReader.MismatchedStoredFieldsReader`:
/// visits through a [`MismatchedVisitor`]. It does not override
/// `getMergeInstance()`, whose default returns `this`: a handle on the same
/// wrapped reader.
struct MismatchedStoredFieldsReader {
    r#in: Arc<dyn StoredFieldsReader>,
    shuffled: Arc<FieldInfos>,
}

impl fmt::Debug for MismatchedStoredFieldsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MismatchedStoredFieldsReader")
            .field("in", &self.r#in)
            .finish_non_exhaustive()
    }
}

impl StoredFieldsReader for MismatchedStoredFieldsReader {
    fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        self.r#in
            .document(doc_id, &mut MismatchedVisitor::new(visitor, &self.shuffled))
    }

    fn check_integrity(&self) -> Result<()> {
        self.r#in.check_integrity()
    }

    /// `new MismatchedStoredFieldsReader(in.clone(), shuffled)`.
    fn clone_reader(&self) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(MismatchedStoredFieldsReader {
            r#in: Arc::from(self.r#in.clone_reader()?),
            shuffled: Arc::clone(&self.shuffled),
        }))
    }

    /// The inherited `getMergeInstance()`: `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(MismatchedStoredFieldsReader {
            r#in: Arc::clone(&self.r#in),
            shuffled: Arc::clone(&self.shuffled),
        }))
    }

    /// `in.close()`; see [`shared_close_error`].
    fn close(&mut self) -> Result<()> {
        match Arc::get_mut(&mut self.r#in) {
            Some(r#in) => r#in.close(),
            None => Err(shared_close_error("stored fields reader")),
        }
    }
}

/// Port of the private `MismatchedCodecReader.MismatchedDocValuesProducer`.
struct MismatchedDocValuesProducer {
    r#in: Arc<dyn DocValuesProducer>,
    shuffled: Arc<FieldInfos>,
    orig: Arc<FieldInfos>,
}

impl fmt::Debug for MismatchedDocValuesProducer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MismatchedDocValuesProducer")
            .field("in", &self.r#in)
            .finish_non_exhaustive()
    }
}

impl DocValuesProducer for MismatchedDocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.r#in
            .get_numeric(remap_field_info(&self.shuffled, &self.orig, field)?)
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.r#in
            .get_binary(remap_field_info(&self.shuffled, &self.orig, field)?)
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.r#in
            .get_sorted(remap_field_info(&self.shuffled, &self.orig, field)?)
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.r#in
            .get_sorted_numeric(remap_field_info(&self.shuffled, &self.orig, field)?)
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.r#in
            .get_sorted_set(remap_field_info(&self.shuffled, &self.orig, field)?)
    }

    fn get_skipper(&self, field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.r#in
            .get_skipper(remap_field_info(&self.shuffled, &self.orig, field)?)
    }

    fn check_integrity(&self) -> Result<()> {
        self.r#in.check_integrity()
    }

    /// The inherited `getMergeInstance()`: `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(MismatchedDocValuesProducer {
            r#in: Arc::clone(&self.r#in),
            shuffled: Arc::clone(&self.shuffled),
            orig: Arc::clone(&self.orig),
        }))
    }

    /// `in.close()`; see [`shared_close_error`].
    fn close(&mut self) -> Result<()> {
        match Arc::get_mut(&mut self.r#in) {
            Some(r#in) => r#in.close(),
            None => Err(shared_close_error("doc values producer")),
        }
    }
}

/// Port of the private `MismatchedCodecReader.MismatchedNormsProducer`.
struct MismatchedNormsProducer {
    r#in: Arc<dyn NormsProducer>,
    shuffled: Arc<FieldInfos>,
    orig: Arc<FieldInfos>,
}

impl fmt::Debug for MismatchedNormsProducer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MismatchedNormsProducer")
            .field("in", &self.r#in)
            .finish_non_exhaustive()
    }
}

impl NormsProducer for MismatchedNormsProducer {
    fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.r#in
            .get_norms(remap_field_info(&self.shuffled, &self.orig, field)?)
    }

    fn check_integrity(&self) -> Result<()> {
        self.r#in.check_integrity()
    }

    /// The inherited `getMergeInstance()`: `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(MismatchedNormsProducer {
            r#in: Arc::clone(&self.r#in),
            shuffled: Arc::clone(&self.shuffled),
            orig: Arc::clone(&self.orig),
        }))
    }

    /// `in.close()`; see [`shared_close_error`].
    fn close(&mut self) -> Result<()> {
        match Arc::get_mut(&mut self.r#in) {
            Some(r#in) => r#in.close(),
            None => Err(shared_close_error("norms producer")),
        }
    }
}
