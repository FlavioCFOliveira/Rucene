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
use super::util::{
    self, java_split, parse_float, parse_int, parse_long, read_line, JavaScratch, CHECKSUM,
};

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
fn strip_prefix(scratch: &JavaScratch, prefix: &[u8]) -> Result<String> {
    scratch.new_string(prefix.len())
}

/// `readInt(in, field)`.
fn read_int(input: &mut dyn IndexInput, scratch: &mut JavaScratch, field: &[u8]) -> Result<i32> {
    scratch.read_line(input)?;
    debug_assert!(scratch.starts_with(field));
    parse_int(&strip_prefix(scratch, field)?)
}

/// `readLong(in, field)`.
fn read_long(input: &mut dyn IndexInput, scratch: &mut JavaScratch, field: &[u8]) -> Result<i64> {
    scratch.read_line(input)?;
    debug_assert!(scratch.starts_with(field));
    parse_long(&strip_prefix(scratch, field)?)
}

/// `readString(in, field)`.
/// `readString(in, field)`: no prefix assertion, so a short line fails in
/// `new String(...)` even under `-ea`.
fn read_string(
    input: &mut dyn IndexInput,
    scratch: &mut JavaScratch,
    field: &[u8],
) -> Result<String> {
    scratch.read_line(input)?;
    strip_prefix(scratch, field)
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
            let mut scratch = JavaScratch::new();
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
                // `new int[size]` (`SimpleTextKnnVectorsReader.java:90`).
                if size < 0 {
                    return Err(util::negative_array_size(size));
                }
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
    /// This reader answers with itself: Java's `instanceof
    /// SimpleTextKnnVectorsReader` on it is `true`
    /// (`BaseKnnVectorsFormatTestCase.assertOffHeapByteSize`).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

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
            if input.file_pointer()? >= footer_start_pos {
                // Make sure we landed at precisely the right location:
                if input.file_pointer()? != footer_start_pos {
                    return Err(LuceneError::corrupt_index(
                        format!(
                            "SimpleText failure: footer does not start at expected position \
                             current={} vs expected={footer_start_pos}",
                            input.file_pointer()?
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
            return Err(LuceneError::illegal_argument(format!(
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
            return Err(LuceneError::illegal_argument(format!(
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
    fn get_off_heap_byte_size(&self, _field_info: &FieldInfo) -> Result<HashMap<String, i64>> {
        Ok(HashMap::new())
    }

    fn close(&self) -> Result<()> {
        let data_in = self.data_in.lock().map_err(poisoned)?.take();
        match data_in {
            Some(mut data_in) => data_in.close(),
            None => Ok(()),
        }
    }
}

/// Reads one `Arrays.toString` line: skip leading "[" and strip trailing
/// "]", split on ",", `Float.parseFloat` each part.
fn read_vector_parts(input: &mut dyn IndexInput, scratch: &mut Vec<u8>) -> Result<Vec<String>> {
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
    Ok(java_split(&s, ',')
        .into_iter()
        .map(str::to_string)
        .collect())
}

/// The loop of `readVector`: `value[i] = Float.parseFloat(floatStrings[i])`
/// for every part, the parse before the store, so a part past `dimension`
/// is parsed and then throws `ArrayIndexOutOfBoundsException`
/// (`SimpleTextKnnVectorsReader.java:420-429`).
fn read_vector_values(parts: &[String], dimension: usize) -> Result<Vec<f32>> {
    debug_assert_eq!(
        parts.len(),
        dimension,
        " read {} when expecting {dimension} floats",
        parts.join(",")
    );
    let mut value = vec![0f32; dimension];
    for (i, part) in parts.iter().enumerate() {
        let v = parse_float(part)?;
        *value
            .get_mut(i)
            .ok_or_else(|| util::array_index_out_of_bounds(i as i64, dimension))? = v;
    }
    Ok(value)
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
            let parts = read_vector_parts(input.as_mut(), &mut scratch)?;
            values.push(read_vector_values(&parts, entry.dimension.max(0) as usize)?);
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
            let parts = read_vector_parts(input.as_mut(), &mut scratch)?;
            let floats = read_vector_values(&parts, entry.dimension.max(0) as usize)?;
            // `(byte) Float.parseFloat(...)`: float to int (saturating, NaN
            // to 0), then int to byte.
            values.push(floats.iter().map(|v| (*v as i32) as i8 as u8).collect());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simpletext::test_support::unchecked;

    /// Regression (rmp #526), release builds (no `-ea`): each part is parsed
    /// before it is stored, so the part past `dimension` is parsed and then
    /// throws `ArrayIndexOutOfBoundsException`; a bad part after it is never
    /// reached. The port parsed every part first.
    #[cfg(not(debug_assertions))]
    #[test]
    fn extra_vector_parts_throw_after_their_parse() {
        let parts: Vec<String> = ["1", " 2", " 3", " x"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let error = read_vector_values(&parts, 2).unwrap_err();
        assert_eq!(
            unchecked(&error),
            (
                "java.lang.ArrayIndexOutOfBoundsException",
                "Index 2 out of bounds for length 2".to_string()
            )
        );
        let parts: Vec<String> = ["1", " 2", " x"].iter().map(|s| s.to_string()).collect();
        assert!(read_vector_values(&parts, 2).is_err());
    }

    /// Regression (rmp #526): `new int[size]` rejects a negative vector
    /// count in the `.gri` file.
    #[test]
    fn negative_vector_count_is_a_negative_array_size() {
        use crate::simpletext::test_support::{directory, segment_info, write_file};
        use rucene::index::FieldInfos;
        use rucene::store::DEFAULT_IO_CONTEXT;
        let dir = directory();
        write_file(
            dir.as_ref(),
            "_0.gri",
            &[
                "field-number 0",
                "field-name f",
                "vector-data-offset 0",
                "vector-data-length 0",
                "vector-dimension 2",
                "size -3",
            ],
            true,
        );
        let si = segment_info(&dir, "_0", 1);
        let infos = FieldInfos::new(Vec::new()).unwrap();
        let state = SegmentReadState::new(dir.as_ref(), &si, &infos, &*DEFAULT_IO_CONTEXT);
        let error = SimpleTextKnnVectorsReader::new(&state).unwrap_err();
        assert_eq!(
            unchecked(&error),
            ("java.lang.NegativeArraySizeException", "-3".to_string())
        );
    }
}

#[cfg(test)]
mod short_line_tests {
    use super::*;
    use crate::simpletext::test_support::{directory, segment_info, unchecked, write_file};
    use rucene::index::FieldInfos;
    use rucene::store::DEFAULT_IO_CONTEXT;

    /// Regression (rmp #526): `readString(in, FIELD_NAME)` has no prefix
    /// assertion, so a short line fails in `new String(scratch.bytes(), 11,
    /// -8)` with the scratch's capacity: `field-number 0` grew it to 16
    /// (`oversize(9, 1)`); the port returned an empty name.
    #[test]
    fn short_field_name_line_is_a_string_index_out_of_bounds() {
        let dir = directory();
        write_file(dir.as_ref(), "_0.gri", &["field-number 0", "fie"], true);
        let si = segment_info(&dir, "_0", 1);
        let infos = FieldInfos::new(Vec::new()).unwrap();
        let state = SegmentReadState::new(dir.as_ref(), &si, &infos, &*DEFAULT_IO_CONTEXT);
        let error = SimpleTextKnnVectorsReader::new(&state).unwrap_err();
        assert_eq!(
            unchecked(&error),
            (
                "java.lang.StringIndexOutOfBoundsException",
                "Range [11, 11 + -8) out of bounds for length 16".to_string()
            )
        );
    }
}
