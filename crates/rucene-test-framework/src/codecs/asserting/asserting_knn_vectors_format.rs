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

//! Port of `org.apache.lucene.tests.codecs.asserting.AssertingKnnVectorsFormat`.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::codecs::knn_vectors::{FieldVectorWriter, IORunnable};
use rucene::codecs::state::OwnedSegmentWriteState;
use rucene::codecs::{
    ByteVectorValues, FloatVectorValues, KnnCollector, KnnVectorsFormat, KnnVectorsReader,
    KnnVectorsWriter, MergeState, SegmentReadState, SorterDocMap,
};
use rucene::error::Result;
use rucene::index::VectorEncoding;
use rucene::index::{FieldInfo, FieldInfos};
use rucene::search::AcceptDocs;

use crate::index::asserting_leaf_reader::npe;
use crate::search::AssertingAcceptDocs;
use crate::util::TestUtil;

/// `KnnVectorsFormat.DEFAULT_MAX_DIMENSIONS`.
const DEFAULT_MAX_DIMENSIONS: i32 = 1024;

/// Wraps the default KnnVectorsFormat and provides additional assertions.
///
/// Port of `AssertingKnnVectorsFormat` (named `Asserting`, over
/// `TestUtil.getDefaultKnnVectorsFormat()`).
pub struct AssertingKnnVectorsFormat {
    delegate: Arc<dyn KnnVectorsFormat>,
}

impl AssertingKnnVectorsFormat {
    /// `new AssertingKnnVectorsFormat()`.
    pub fn new() -> Self {
        AssertingKnnVectorsFormat {
            delegate: TestUtil::get_default_knn_vectors_format(),
        }
    }
}

impl Default for AssertingKnnVectorsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AssertingKnnVectorsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KnnVectorsFormat(name=Asserting)")
    }
}

impl KnnVectorsFormat for AssertingKnnVectorsFormat {
    fn name(&self) -> &str {
        "Asserting"
    }

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        Ok(Box::new(AssertingKnnVectorsWriter::new(
            self.delegate.fields_writer(state)?,
        )?))
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(AssertingKnnVectorsReader::new(
            self.delegate.fields_reader(state)?,
            state.field_infos.clone(),
        )))
    }

    fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        DEFAULT_MAX_DIMENSIONS
    }
}

/// Port of `AssertingKnnVectorsFormat.AssertingKnnVectorsWriter`.
///
/// It does not override `merge`, as Java's does not: `KnnVectorsWriter`'s
/// default merge calls this writer's `mergeOneField` for every field.
#[derive(Debug)]
struct AssertingKnnVectorsWriter {
    delegate: Box<dyn KnnVectorsWriter>,
}

impl AssertingKnnVectorsWriter {
    fn new(delegate: Box<dyn KnnVectorsWriter>) -> Result<Self> {
        // `assert delegate != null`: a `Box` always is.
        Ok(AssertingKnnVectorsWriter { delegate })
    }
}

impl KnnVectorsWriter for AssertingKnnVectorsWriter {
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        self.delegate.add_field(field_info)
    }

    fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()> {
        self.delegate.flush(max_doc, sort_map)
    }

    fn finish(&mut self) -> Result<()> {
        self.delegate.finish()
    }

    fn merge_one_field<'m>(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &'m MergeState,
    ) -> Result<Option<Box<dyn IORunnable + 'm>>> {
        // `assert fieldInfo != null; assert mergeState != null;`: references
        // always are.
        self.delegate.merge_one_field(field_info, merge_state)
    }

    fn close(&mut self) -> Result<()> {
        self.delegate.close()
    }

    fn ram_bytes_used(&self) -> i64 {
        self.delegate.ram_bytes_used()
    }
}

/// The state of the anonymous `AssertingKnnVectorsReader` subclass that
/// `getMergeInstance()` returns.
#[derive(Debug)]
struct MergeInstance {
    finished: bool,
    parent_merge_finish_count: Arc<AtomicI32>,
}

/// Wraps a [`KnnVectorsReader`] providing additional assertions.
///
/// Port of `AssertingKnnVectorsFormat.AssertingKnnVectorsReader`; the reader
/// [`get_merge_instance`](KnnVectorsReader::get_merge_instance) returns stands
/// for Java's anonymous subclass (only usable for merging, finished once).
///
/// **Divergence.** Java's class also `implements HnswGraphProvider` by
/// casting its delegate. A `dyn KnnVectorsReader` cannot be narrowed to a
/// provider in this port, the limitation the HNSW merger already records
/// (`Lucene99HnswVectorsWriter::build_and_write_graph`), so the reader is not
/// a provider and a merge rebuilds the graph as Java does for any reader that
/// is not one.
#[derive(Debug)]
pub struct AssertingKnnVectorsReader {
    /// `delegate`.
    pub delegate: Box<dyn KnnVectorsReader>,
    fis: FieldInfos,
    merge_instance_count: Arc<AtomicI32>,
    finish_merge_count: Arc<AtomicI32>,
    merge_instance: Option<MergeInstance>,
}

impl AssertingKnnVectorsReader {
    fn new(delegate: Box<dyn KnnVectorsReader>, fis: FieldInfos) -> Self {
        // `assert delegate != null`: a `Box` always is.
        AssertingKnnVectorsReader {
            delegate,
            fis,
            merge_instance_count: Arc::new(AtomicI32::new(0)),
            finish_merge_count: Arc::new(AtomicI32::new(0)),
            merge_instance: None,
        }
    }

    fn check_field(&self, field: &str, encoding: VectorEncoding) -> Result<()> {
        let fi = self.fis.field_info(field);
        java_assert!(
            fi.is_some_and(|fi| fi.get_vector_dimension() > 0 && fi.vector_encoding == encoding)
        );
        Ok(())
    }
}

impl KnnVectorsReader for AssertingKnnVectorsReader {
    /// This reader answers with itself: Java's `instanceof
    /// AssertingKnnVectorsFormat.AssertingKnnVectorsReader` on it is `true`
    /// (`BaseKnnVectorsFormatTestCase.hasQuantized`, `hasHNSW`).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn check_integrity(&self) -> Result<()> {
        self.delegate.check_integrity()
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        self.check_field(field, VectorEncoding::FLOAT32)?;
        let float_values = self.delegate.get_float_vector_values(field)?;
        java_assert!(float_values.is_some());
        if let Some(values) = &float_values {
            java_assert!(values.iterator()?.doc_id() == -1);
            java_assert!(values.size() >= 0);
            java_assert!(values.dimension() > 0);
        }
        Ok(float_values)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        self.check_field(field, VectorEncoding::BYTE)?;
        let byte_values = self.delegate.get_byte_vector_values(field)?;
        java_assert!(byte_values.is_some());
        if let Some(values) = &byte_values {
            java_assert!(values.iterator()?.doc_id() == -1);
            java_assert!(values.size() >= 0);
            java_assert!(values.dimension() > 0);
        }
        Ok(byte_values)
    }

    fn search(
        &self,
        field: &str,
        target: &[f32],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        if self.merge_instance.is_some() {
            java_assert!(false, "This instance should only be used for merging");
            return Ok(());
        }
        self.check_field(field, VectorEncoding::FLOAT32)?;
        let mut accept_docs = AssertingAcceptDocs::wrap(accept_docs);
        self.delegate
            .search(field, target, knn_collector, &mut accept_docs)
    }

    fn search_byte(
        &self,
        field: &str,
        target: &[u8],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        if self.merge_instance.is_some() {
            java_assert!(false, "This instance should only be used for merging");
            return Ok(());
        }
        self.check_field(field, VectorEncoding::BYTE)?;
        let mut accept_docs = AssertingAcceptDocs::wrap(accept_docs);
        self.delegate
            .search_byte(field, target, knn_collector, &mut accept_docs)
    }

    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        if self.merge_instance.is_some() {
            java_assert!(false); // merging from a merge instance it not allowed
                                 // Java returns `null` here once assertions are disabled.
            return Err(npe());
        }
        let merge_vectors_reader = self.delegate.get_merge_instance()?;
        self.merge_instance_count.fetch_add(1, Ordering::SeqCst);
        let mut reader = AssertingKnnVectorsReader::new(merge_vectors_reader, self.fis.clone());
        reader.merge_instance = Some(MergeInstance {
            finished: false,
            parent_merge_finish_count: Arc::clone(&self.finish_merge_count),
        });
        Ok(Box::new(reader))
    }

    fn finish_merge(&mut self) -> Result<()> {
        match &mut self.merge_instance {
            None => {
                java_assert!(false); // can only finish merge on the merge instance
                Ok(())
            }
            Some(merge) => {
                java_assert!(!merge.finished, "Merging already finished");
                merge.finished = true;
                self.delegate.finish_merge()?;
                merge
                    .parent_merge_finish_count
                    .fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
    }

    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> HashMap<String, i64> {
        self.delegate.get_off_heap_byte_size(field_info)
    }

    fn close(&mut self) -> Result<()> {
        if self.merge_instance.is_some() {
            java_assert!(false); // closing the merge instance it not allowed
            return Ok(());
        }
        self.delegate.close()?;
        self.delegate.close()?; // impls should be able to handle multiple closes
        let finish_merge_count = self.finish_merge_count.load(Ordering::SeqCst);
        java_assert!(
            finish_merge_count <= 0
                || self.merge_instance_count.load(Ordering::SeqCst) == finish_merge_count
        );
        Ok(())
    }
}
