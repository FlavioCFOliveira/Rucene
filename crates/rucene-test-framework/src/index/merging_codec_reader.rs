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

//! Port of `org.apache.lucene.tests.index.MergingCodecReader`: a
//! `CodecReader` wrapper that performs all reads using the merging instance
//! of the index formats.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use rucene::codecs::doc_values::DocValuesProducer;
use rucene::codecs::knn_vectors::KnnVectorsReader;
use rucene::codecs::norms::NormsProducer;
use rucene::codecs::points::PointsReader;
use rucene::codecs::postings::FieldsProducer;
use rucene::codecs::stored_fields::StoredFieldsReader;
use rucene::codecs::term_vectors::TermVectorsReader;
use rucene::error::{LuceneError, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::index_reader::{CacheHelper, IndexReaderCore};
use rucene::index::LeafReader;

use super::asserting_leaf_reader::npe;
use super::shared_producers::SharedStoredFieldsReader;

/// One value per thread, computed on a thread's first `get` (Java's
/// `CloseableThreadLocal` with an overridden `initialValue()`).
///
/// **Divergence.** Java's `CloseableThreadLocal` holds each thread's value
/// until the thread dies or the instance is closed or collected. Core's port
/// (`rucene::util::attribute::CloseableThreadLocal`) keeps the values in the
/// threads' own storage, where they outlive the reader until each thread
/// exits; the values here are merge instances holding index inputs, so they
/// are kept in the reader instead, keyed by thread, and freed with it. An
/// `initialValue()` that throws stores nothing, so the next `get` on that
/// thread computes it again, as Java's `ThreadLocal.get()` does.
struct PerThread<T: Clone> {
    values: Mutex<HashMap<ThreadId, T>>,
}

impl<T: Clone> PerThread<T> {
    fn new() -> Self {
        PerThread {
            values: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, initial_value: impl FnOnce() -> Result<T>) -> Result<T> {
        let thread = std::thread::current().id();
        let mut values = self
            .values
            .lock()
            .map_err(|_| LuceneError::illegal_state("thread-local lock poisoned".to_string()))?;
        if let Some(value) = values.get(&thread) {
            return Ok(value.clone());
        }
        let value = initial_value()?;
        values.insert(thread, value.clone());
        Ok(value)
    }
}

/// Port of `org.apache.lucene.tests.index.MergingCodecReader`
/// (`extends FilterCodecReader`): the stored fields, norms and doc values are
/// read through the merge instances of the wrapped reader's producers, one
/// per thread (`// TODO: other formats too`), and every leaf query goes
/// through them as through the `final` members of Java's `CodecReader`.
pub struct MergingCodecReader {
    core: IndexReaderCore,
    r#in: Arc<dyn CodecReader>,
    fields_reader: PerThread<Arc<dyn StoredFieldsReader>>,
    norms_reader: PerThread<Option<Arc<dyn NormsProducer>>>,
    doc_values_reader: PerThread<Option<Arc<dyn DocValuesProducer>>>,
}

impl MergingCodecReader {
    /// `new MergingCodecReader(in)`: wraps the given instance.
    pub fn new(r#in: Arc<dyn CodecReader>) -> Self {
        MergingCodecReader {
            core: IndexReaderCore::new(),
            r#in,
            fields_reader: PerThread::new(),
            norms_reader: PerThread::new(),
            doc_values_reader: PerThread::new(),
        }
    }

    /// `getDelegate()`: the wrapped reader.
    pub fn get_delegate(&self) -> &Arc<dyn CodecReader> {
        &self.r#in
    }
}

impl fmt::Debug for MergingCodecReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MergingCodecReader").finish_non_exhaustive()
    }
}

impl fmt::Display for MergingCodecReader {
    /// Neither `MergingCodecReader` nor `FilterCodecReader` overrides
    /// `toString()`: `Object`'s applies.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.tests.index.MergingCodecReader",
            self,
        ))
    }
}

impl LeafReader for MergingCodecReader {
    forward_leaf_reader!(codec r#in;
        core,
        num_docs,
        max_doc,
        do_close,
        get_field_infos,
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

    /// same content, we can delegate
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_core_cache_helper()
    }

    /// same content, we can delegate
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_reader_cache_helper()
    }
}

impl CodecReader for MergingCodecReader {
    /// This thread's `in.getFieldsReader().getMergeInstance()`; a reader
    /// without stored fields throws the `NullPointerException` of the
    /// dereference.
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        let reader = self.fields_reader.get(|| {
            let fields_reader = self.r#in.get_fields_reader()?.ok_or_else(npe)?;
            Ok(Arc::from(fields_reader.get_merge_instance()?))
        })?;
        Ok(Some(Box::new(SharedStoredFieldsReader(reader))))
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        self.r#in.get_term_vectors_reader()
    }

    /// This thread's merge instance of `in.getNormsReader()`, or `null`.
    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        self.norms_reader
            .get(|| match self.r#in.get_norms_reader()? {
                None => Ok(None),
                Some(norms) => Ok(Some(Arc::from(norms.get_merge_instance()?))),
            })
    }

    /// This thread's merge instance of `in.getDocValuesReader()`, or `null`.
    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        self.doc_values_reader
            .get(|| match self.r#in.get_doc_values_reader()? {
                None => Ok(None),
                Some(doc_values) => Ok(Some(Arc::from(doc_values.get_merge_instance()?))),
            })
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
