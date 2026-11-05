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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextKnnVectorsReader`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rucene::codecs::{KnnCollector, KnnVectorsReader, SegmentReadState};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{
    ByteVectorValues, DocIndexIterator, FieldInfo, FieldInfos, FloatVectorValues, KnnVectorValues,
    VectorEncoding, VectorSimilarityFunction,
};
use rucene::search::vector_scorer::{
    default_bulk, SharedVectorScorer, VectorScorer, VectorScorerBulk,
};
use rucene::search::{AcceptDocs, DocIdSetIterator};
use rucene::store::{BufferedChecksumIndexInput, IndexInput, DEFAULT_IO_CONTEXT};
use rucene::util::java_fmt::java_utf8_to_string;

use super::knn_vectors_format::{META_EXTENSION, VECTOR_EXTENSION};
use super::knn_vectors_writer::{
    FIELD_NAME, FIELD_NUMBER, SIZE, VECTOR_DATA_LENGTH, VECTOR_DATA_OFFSET, VECTOR_DIMENSION,
};
use super::util::{self, java_split, parse_float, parse_int, parse_long, read_line, CHECKSUM};

const EMPTY: &[u8] = b"";

/// Reads vector values from a simple text format. All vectors are read up
/// front and cached in RAM in order to support random access.
///
/// **FOR RECREATIONAL USE ONLY**
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextKnnVectorsReader`.
///
/// `getMergeInstance()` returns the reader itself in Java; the port's
/// returns a handle that shares all of this reader's state.
#[derive(Clone)]
pub struct SimpleTextKnnVectorsReader {
    field_infos: Arc<FieldInfos>,
    data_in: Arc<Mutex<Option<Box<dyn IndexInput>>>>,
    /// `IntObjectHashMap<FieldEntry> fieldEntries`, keyed by field number.
    field_entries: Arc<HashMap<i32, Arc<FieldEntry>>>,
}

impl std::fmt::Debug for SimpleTextKnnVectorsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SimpleTextKnnVectorsReader")
    }
}

/// Port of the record `org.apache.lucene.codecs.simpletext.SimpleTextKnnVectorsReader.FieldEntry`.
#[derive(Debug)]
struct FieldEntry {
    dimension: i32,
    vector_data_offset: i64,
    vector_data_length: i64,
    ord_to_doc: Vec<i32>,
    similarity_function: VectorSimilarityFunction,
}

/// A field's entry and the slice of the vector data holding its vectors.
type EntryAndSlice = (Arc<FieldEntry>, Box<dyn IndexInput>);

impl FieldEntry {
    fn size(&self) -> i32 {
        self.ord_to_doc.len() as i32
    }
}

fn poisoned<T>(_: T) -> LuceneError {
    LuceneError::illegal_state("vector data lock was poisoned".to_string())
}

/// `stripPrefix`.
fn strip_prefix(scratch: &[u8], prefix: &[u8]) -> String {
    util::read_string(scratch, prefix.len())
}

/// `readInt(in, field)`.
fn read_int(input: &mut dyn IndexInput, scratch: &mut Vec<u8>, field: &[u8]) -> Result<i32> {
    read_line(input, scratch)?;
    debug_assert!(scratch.starts_with(field));
    parse_int(&strip_prefix(scratch, field))
}

/// `readLong(in, field)`.
fn read_long(input: &mut dyn IndexInput, scratch: &mut Vec<u8>, field: &[u8]) -> Result<i64> {
    read_line(input, scratch)?;
    debug_assert!(scratch.starts_with(field));
    parse_long(&strip_prefix(scratch, field))
}

/// `readString(in, field)`.
fn read_string(input: &mut dyn IndexInput, scratch: &mut Vec<u8>, field: &[u8]) -> Result<String> {
    read_line(input, scratch)?;
    Ok(strip_prefix(scratch, field))
}

impl SimpleTextKnnVectorsReader {
    /// Port of `SimpleTextKnnVectorsReader(SegmentReadState)`.
    ///
    /// # Errors
    ///
    /// Propagates the directory's and the parsers' errors.
    pub fn new(read_state: &SegmentReadState<'_>) -> Result<Self> {
        let meta_file_name = segment_file_name(
            &read_state.segment_info.name,
            &read_state.segment_suffix,
            META_EXTENSION,
        );
        let vector_file_name = segment_file_name(
            &read_state.segment_info.name,
            &read_state.segment_suffix,
            VECTOR_EXTENSION,
        );

        // `try (ChecksumIndexInput in = ...)`, whose body also opens `dataIn`
        // (`SimpleTextKnnVectorsReader.java:82-114`); on failure `this` — so
        // far nothing — is closed.
        let mut input = read_state.directory.open_checksum_input(&meta_file_name)?;
        let read = (|| -> Result<_> {
            let mut scratch = Vec::new();
            let mut field_entries: HashMap<i32, Arc<FieldEntry>> = HashMap::new();
            let mut field_number = read_int(input.as_mut(), &mut scratch, FIELD_NUMBER)?;
            while field_number != -1 {
                let field_name = read_string(input.as_mut(), &mut scratch, FIELD_NAME)?;
                let vector_data_offset =
                    read_long(input.as_mut(), &mut scratch, VECTOR_DATA_OFFSET)?;
                let vector_data_length =
                    read_long(input.as_mut(), &mut scratch, VECTOR_DATA_LENGTH)?;
                let dimension = read_int(input.as_mut(), &mut scratch, VECTOR_DIMENSION)?;
                let size = read_int(input.as_mut(), &mut scratch, SIZE)?;
                let mut doc_ids = Vec::with_capacity(size.max(0) as usize);
                for _ in 0..size {
                    doc_ids.push(read_int(input.as_mut(), &mut scratch, EMPTY)?);
                }
                debug_assert!(!field_entries.contains_key(&field_number));
                let similarity_function = read_state
                .field_infos
                .field_info(&field_name)
                .map(FieldInfo::get_vector_similarity_function)
                .ok_or_else(|| {
                    null_pointer(
                        "Cannot invoke \"org.apache.lucene.index.FieldInfo.getVectorSimilarityFunction()\" \
                         because the return value of \"org.apache.lucene.index.FieldInfos.fieldInfo(String)\" \
                         is null",
                    )
                })?;
                field_entries.insert(
                    field_number,
                    Arc::new(FieldEntry {
                        dimension,
                        vector_data_offset,
                        vector_data_length,
                        ord_to_doc: doc_ids,
                        similarity_function,
                    }),
                );
                field_number = read_int(input.as_mut(), &mut scratch, FIELD_NUMBER)?;
            }
            util::check_footer(input.as_mut())?;

            let data_in = read_state
                .directory
                .open_input(&vector_file_name, &*DEFAULT_IO_CONTEXT)?;
            Ok((field_entries, data_in))
        })();
        let (field_entries, data_in) = rucene::store::close_after(input.as_mut(), read)?;
        Ok(Self {
            field_infos: Arc::new(read_state.field_infos.clone()),
            data_in: Arc::new(Mutex::new(Some(data_in))),
            field_entries: Arc::new(field_entries),
        })
    }

    /// The common part of `getFloatVectorValues` and `getByteVectorValues`:
    /// the field's entry and the slice of its vectors, or `None`.
    fn entry_and_slice(&self, field: &str) -> Result<Option<EntryAndSlice>> {
        let Some(info) = self.field_infos.field_info(field) else {
            // mirror the handling in Lucene90VectorReader#getVectorValues
            // needed to pass TestSimpleTextKnnVectorsFormat#testDeleteAllVectorDocs
            return Ok(None);
        };
        let dimension = info.get_vector_dimension();
        if dimension == 0 {
            return Err(LuceneError::illegal_state(
                "KNN vectors readers should not be called on fields that don't enable KNN vectors"
                    .to_string(),
            ));
        }
        let Some(field_entry) = self.field_entries.get(&info.number) else {
            // mirror the handling in Lucene90VectorReader#getVectorValues
            // needed to pass TestSimpleTextKnnVectorsFormat#testDeleteAllVectorDocs
            return Ok(None);
        };
        if dimension != field_entry.dimension {
            return Err(LuceneError::illegal_state(format!(
                "Inconsistent vector dimension for field=\"{field}\"; {dimension} != {}",
                field_entry.dimension
            )));
        }
        let guard = self.data_in.lock().map_err(poisoned)?;
        let data_in = guard
            .as_ref()
            .ok_or_else(|| LuceneError::already_closed("this IndexInput is closed"))?;
        let bytes_slice = data_in.slice(
            "vector-data",
            field_entry.vector_data_offset,
            field_entry.vector_data_length,
        )?;
        Ok(Some((Arc::clone(field_entry), bytes_slice)))
    }

    fn float_values(&self, field: &str) -> Result<Option<SimpleTextFloatVectorValues>> {
        match self.entry_and_slice(field)? {
            None => Ok(None),
            Some((entry, slice)) => Ok(Some(SimpleTextFloatVectorValues::new(entry, slice)?)),
        }
    }

    fn byte_values(&self, field: &str) -> Result<Option<SimpleTextByteVectorValues>> {
        match self.entry_and_slice(field)? {
            None => Ok(None),
            Some((entry, slice)) => Ok(Some(SimpleTextByteVectorValues::new(entry, slice)?)),
        }
    }
}

/// `new NullPointerException(message)`, as the JVM throws it.
fn null_pointer(message: &str) -> LuceneError {
    LuceneError::unchecked("java.lang.NullPointerException", message)
}

/// The `NullPointerException` Java's `search` throws when the field has no
/// vectors (`values` is `null`); `class` is the type of `values`.
fn null_values(class: &str) -> LuceneError {
    null_pointer(&format!(
        "Cannot invoke \"org.apache.lucene.index.{class}.dimension()\" because \"values\" is null"
    ))
}

/// The `NullPointerException` of `info.getVectorSimilarityFunction()` in
/// `search`; `getFloatVectorValues` and `getByteVectorValues` return `null`
/// for a field the field infos do not know, so `search` never reaches it.
fn null_info() -> LuceneError {
    null_pointer(
        "Cannot invoke \"org.apache.lucene.index.FieldInfo.getVectorSimilarityFunction()\" \
         because \"info\" is null",
    )
}

impl KnnVectorsReader for SimpleTextKnnVectorsReader {
    /// Port of `SimpleTextKnnVectorsReader.checkIntegrity`.
    fn check_integrity(&self) -> Result<()> {
        let guard = self.data_in.lock().map_err(poisoned)?;
        let data_in = guard
            .as_ref()
            .ok_or_else(|| LuceneError::already_closed("this IndexInput is closed"))?;
        let mut clone = data_in.clone_input()?;
        clone.seek(0)?;

        // checksum is fixed-width encoded with 20 bytes, plus 1 byte for
        // newline (the space is included in SimpleTextUtil.CHECKSUM):
        let footer_start_pos = data_in.length() - (CHECKSUM.len() as i64 + 21);
        let mut input = BufferedChecksumIndexInput::new(clone);

        // when there's no actual vector data written (e.g. tested in
        // TestSimpleTextKnnVectorsFormat#testDeleteAllVectorDocs) the first
        // line in dataInput will be, checksum 00000000000000000000
        if footer_start_pos == 0 {
            return util::check_footer(&mut input);
        }

        let mut scratch = Vec::new();
        loop {
            read_line(&mut input, &mut scratch)?;
            if input.file_pointer() >= footer_start_pos {
                // Make sure we landed at precisely the right location:
                if input.file_pointer() != footer_start_pos {
                    return Err(LuceneError::corrupt_index(
                        format!(
                            "SimpleText failure: footer does not start at expected position \
                             current={} vs expected={footer_start_pos}",
                            input.file_pointer()
                        ),
                        IndexInput::resource_description(&input),
                    ));
                }
                return util::check_footer(&mut input);
            }
        }
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        Ok(self
            .float_values(field)?
            .map(|v| Box::new(v) as Box<dyn FloatVectorValues>))
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        Ok(self
            .byte_values(field)?
            .map(|v| Box::new(v) as Box<dyn ByteVectorValues>))
    }

    /// Port of `search(String, float[], KnnCollector, AcceptDocs)`.
    fn search(
        &self,
        field: &str,
        target: &[f32],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        let values = self
            .float_values(field)?
            .ok_or_else(|| null_values("FloatVectorValues"))?;
        if target.len() as i32 != values.dimension() {
            return Err(LuceneError::IllegalArgument(format!(
                "vector query dimension: {} differs from field dimension: {}",
                target.len(),
                values.dimension()
            )));
        }
        let vector_similarity = self
            .field_infos
            .field_info(field)
            .map(FieldInfo::get_vector_similarity_function)
            .ok_or_else(null_info)?;
        for ord in 0..values.size() {
            let doc = values.ord_to_doc(ord);
            if let Some(bits) = accept_docs.bits()? {
                if !bits.try_get(doc as usize)? {
                    continue;
                }
            }

            if knn_collector.early_terminated() {
                break;
            }

            let vector = values.vector_value(ord)?;
            let score = vector_similarity.compare_f32(&vector, target)?;
            knn_collector.collect(doc, score);
            knn_collector.inc_visited_count(1);
        }
        Ok(())
    }

    /// Port of `search(String, byte[], KnnCollector, AcceptDocs)`.
    fn search_byte(
        &self,
        field: &str,
        target: &[u8],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        let values = self
            .byte_values(field)?
            .ok_or_else(|| null_values("ByteVectorValues"))?;
        if target.len() as i32 != values.dimension() {
            return Err(LuceneError::IllegalArgument(format!(
                "vector query dimension: {} differs from field dimension: {}",
                target.len(),
                values.dimension()
            )));
        }
        let vector_similarity = self
            .field_infos
            .field_info(field)
            .map(FieldInfo::get_vector_similarity_function)
            .ok_or_else(null_info)?;

        for ord in 0..values.size() {
            let doc = values.ord_to_doc(ord);
            if let Some(bits) = accept_docs.bits()? {
                if !bits.try_get(doc as usize)? {
                    continue;
                }
            }

            if knn_collector.early_terminated() {
                break;
            }

            let vector = values.vector_value(ord)?;
            let score = vector_similarity.compare_bytes(&vector, target)?;
            knn_collector.collect(doc, score);
            knn_collector.inc_visited_count(1);
        }
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(self.clone()))
    }

    /// Port of `getOffHeapByteSize`: `null` when the field has no entry,
    /// `Map.of()` (all in-heap) otherwise; both are empty here.
    fn get_off_heap_byte_size(&self, _field_info: &FieldInfo) -> HashMap<String, i64> {
        HashMap::new()
    }

    fn close(&mut self) -> Result<()> {
        let data_in = self.data_in.lock().map_err(poisoned)?.take();
        match data_in {
            Some(mut data_in) => data_in.close(),
            None => Ok(()),
        }
    }
}

/// Reads one `Arrays.toString` line: skip leading "[" and strip trailing
/// "]", split on ",", `Float.parseFloat` each part.
fn read_vector_parts(input: &mut dyn IndexInput, scratch: &mut Vec<u8>) -> Result<Vec<f32>> {
    read_line(input, scratch)?;
    // `new BytesRef(scratch.bytes(), 1, scratch.length() - 2).utf8ToString()`.
    let length = scratch.len() as i64 - 2;
    if length < 0 {
        // `assert isValid()` in the `BytesRef` constructor, which throws
        // the `IllegalStateException` itself (`BytesRef.java:54-58`,
        // `:184-189`); without assertions `utf8ToString` allocates
        // `new char[length]` (`:137-138`).
        return Err(if cfg!(debug_assertions) {
            LuceneError::illegal_state(format!("length is negative: {length}"))
        } else {
            LuceneError::unchecked("java.lang.NegativeArraySizeException", length.to_string())
        });
    }
    let inner = &scratch[1..scratch.len() - 1];
    let s = java_utf8_to_string(inner);
    java_split(&s, ',').into_iter().map(parse_float).collect()
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextKnnVectorsReader.SimpleTextFloatVectorValues`.
#[derive(Clone)]
struct SimpleTextFloatVectorValues {
    entry: Arc<FieldEntry>,
    values: Arc<Vec<Vec<f32>>>,
}

impl SimpleTextFloatVectorValues {
    fn new(entry: Arc<FieldEntry>, mut input: Box<dyn IndexInput>) -> Result<Self> {
        let mut scratch = Vec::new();
        let mut values = Vec::with_capacity(entry.size() as usize);
        for _ in 0..entry.size() {
            let value = read_vector_parts(input.as_mut(), &mut scratch)?;
            debug_assert_eq!(value.len() as i32, entry.dimension);
            let mut fixed = vec![0f32; entry.dimension.max(0) as usize];
            for (slot, v) in fixed.iter_mut().zip(value) {
                *slot = v;
            }
            values.push(fixed);
        }
        Ok(Self {
            entry,
            values: Arc::new(values),
        })
    }
}

impl KnnVectorValues for SimpleTextFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.entry.dimension
    }

    fn size(&self) -> i32 {
        self.entry.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.entry.ord_to_doc[ord as usize]
    }

    /// `copy()` returns `this` in Java.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(self.create_sparse_iterator())
    }
}

impl FloatVectorValues for SimpleTextFloatVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        Ok(self.values[ord as usize].clone())
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn scorer(&self, target: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        if self.size() == 0 {
            return Ok(None);
        }
        let values = self.clone();
        let iterator = values.iterator()?;
        let target = target.to_vec();
        Ok(Some(Box::new(TextVectorScorer {
            iterator,
            score: Box::new(move |ord| {
                values
                    .entry
                    .similarity_function
                    .compare_f32(&values.values[ord as usize], &target)
            }),
        })))
    }
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextKnnVectorsReader.SimpleTextByteVectorValues`.
#[derive(Clone)]
struct SimpleTextByteVectorValues {
    entry: Arc<FieldEntry>,
    values: Arc<Vec<Vec<u8>>>,
}

impl SimpleTextByteVectorValues {
    fn new(entry: Arc<FieldEntry>, mut input: Box<dyn IndexInput>) -> Result<Self> {
        let mut scratch = Vec::new();
        let mut values = Vec::with_capacity(entry.size() as usize);
        for _ in 0..entry.size() {
            let value = read_vector_parts(input.as_mut(), &mut scratch)?;
            debug_assert_eq!(value.len() as i32, entry.dimension);
            let mut fixed = vec![0u8; entry.dimension.max(0) as usize];
            for (slot, v) in fixed.iter_mut().zip(value) {
                // `(byte) Float.parseFloat(...)`: float to int (saturating,
                // NaN to 0), then int to byte.
                *slot = (v as i32) as i8 as u8;
            }
            values.push(fixed);
        }
        Ok(Self {
            entry,
            values: Arc::new(values),
        })
    }
}

impl KnnVectorValues for SimpleTextByteVectorValues {
    fn dimension(&self) -> i32 {
        self.entry.dimension
    }

    fn size(&self) -> i32 {
        self.entry.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.entry.ord_to_doc[ord as usize]
    }

    /// `copy()` returns `this` in Java.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(self.create_sparse_iterator())
    }
}

impl ByteVectorValues for SimpleTextByteVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        Ok(self.values[ord as usize].clone())
    }

    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn scorer(&self, target: &[u8]) -> Result<Option<Box<dyn VectorScorer>>> {
        if self.size() == 0 {
            return Ok(None);
        }
        let values = self.clone();
        let iterator = values.iterator()?;
        let target = target.to_vec();
        Ok(Some(Box::new(TextVectorScorer {
            iterator,
            score: Box::new(move |ord| {
                values
                    .entry
                    .similarity_function
                    .compare_bytes(&values.values[ord as usize], &target)
            }),
        })))
    }
}

/// The anonymous `VectorScorer`s of `scorer(float[])` and `scorer(byte[])`:
/// the score of the iterator's current ordinal.
struct TextVectorScorer {
    iterator: Box<dyn DocIndexIterator>,
    score: Box<dyn Fn(i32) -> Result<f32>>,
}

impl VectorScorer for TextVectorScorer {
    fn score(&mut self) -> Result<f32> {
        let ord = self.iterator.index();
        (self.score)(ord)
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.iterator.as_mut()
    }

    fn bulk(
        self: Box<Self>,
        matching_docs: Option<Box<dyn DocIdSetIterator>>,
    ) -> Result<Box<dyn VectorScorerBulk>> {
        default_bulk(SharedVectorScorer::new(self), matching_docs)
    }

    fn doc_index_iterator(&mut self) -> Option<&mut dyn DocIndexIterator> {
        Some(self.iterator.as_mut())
    }
}
