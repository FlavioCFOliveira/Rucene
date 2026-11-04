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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextKnnVectorsWriter`.

use rucene::codecs::state::OwnedSegmentWriteState;
use rucene::codecs::BufferingKnnVectorsWriterImpl;
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{ByteVectorValues, FieldInfo, FloatVectorValues};
use rucene::search::NO_MORE_DOCS;
use rucene::store::IndexOutput;
use rucene::util::java_fmt::float_to_string;

use super::knn_vectors_format::{META_EXTENSION, VECTOR_EXTENSION};
use super::util;

pub(crate) const FIELD_NUMBER: &[u8] = b"field-number ";
pub(crate) const FIELD_NAME: &[u8] = b"field-name ";
pub(crate) const VECTOR_DATA_OFFSET: &[u8] = b"vector-data-offset ";
pub(crate) const VECTOR_DATA_LENGTH: &[u8] = b"vector-data-length ";
pub(crate) const VECTOR_DIMENSION: &[u8] = b"vector-dimension ";
pub(crate) const SIZE: &[u8] = b"size ";

/// Writes vector-valued fields in a plain text format.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextKnnVectorsWriter`,
/// a `BufferingKnnVectorsWriter` subclass: the base class is
/// [`rucene::codecs::BufferingKnnVectorsWriter`], which wraps this.
pub struct SimpleTextKnnVectorsWriter {
    meta: Option<Box<dyn IndexOutput>>,
    vector_data: Option<Box<dyn IndexOutput>>,
}

impl std::fmt::Debug for SimpleTextKnnVectorsWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SimpleTextKnnVectorsWriter")
    }
}

/// `Arrays.toString(float[])`.
fn floats_to_string(value: &[f32]) -> String {
    let parts: Vec<String> = value.iter().map(|v| float_to_string(*v)).collect();
    format!("[{}]", parts.join(", "))
}

/// `Arrays.toString(byte[])`.
fn bytes_to_string(value: &[u8]) -> String {
    let parts: Vec<String> = value.iter().map(|v| (*v as i8).to_string()).collect();
    format!("[{}]", parts.join(", "))
}

fn closed() -> LuceneError {
    LuceneError::already_closed("this writer is closed")
}

impl SimpleTextKnnVectorsWriter {
    /// Port of `SimpleTextKnnVectorsWriter(SegmentWriteState)`.
    ///
    /// # Errors
    ///
    /// Propagates the directory's error; the output already created is closed
    /// first, as Java's `closeWhileHandlingException(this)` does.
    pub fn new(state: &OwnedSegmentWriteState) -> Result<Self> {
        let meta_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta = state
            .directory
            .create_output(&meta_file_name, state.context.as_ref())?;

        let vector_data_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            VECTOR_EXTENSION,
        );
        let vector_data = match state
            .directory
            .create_output(&vector_data_file_name, state.context.as_ref())
        {
            Ok(vector_data) => vector_data,
            Err(e) => {
                let _ = meta.close();
                return Err(e);
            }
        };
        Ok(Self {
            meta: Some(meta),
            vector_data: Some(vector_data),
        })
    }

    fn vector_data(&mut self) -> Result<&mut dyn IndexOutput> {
        match self.vector_data.as_deref_mut() {
            Some(out) => Ok(out),
            None => Err(closed()),
        }
    }

    fn meta(&mut self) -> Result<&mut dyn IndexOutput> {
        match self.meta.as_deref_mut() {
            Some(out) => Ok(out),
            None => Err(closed()),
        }
    }

    /// Port of the private `writeMeta`.
    fn write_meta(
        &mut self,
        field: &FieldInfo,
        vector_data_offset: i64,
        vector_data_length: i64,
        doc_ids: &[i32],
    ) -> Result<()> {
        let meta = self.meta()?;
        write_field(meta, FIELD_NUMBER, &field.number.to_string())?;
        write_field(meta, FIELD_NAME, &field.name)?;
        write_field(meta, VECTOR_DATA_OFFSET, &vector_data_offset.to_string())?;
        write_field(meta, VECTOR_DATA_LENGTH, &vector_data_length.to_string())?;
        write_field(
            meta,
            VECTOR_DIMENSION,
            &field.get_vector_dimension().to_string(),
        )?;
        write_field(meta, SIZE, &doc_ids.len().to_string())?;
        for doc_id in doc_ids {
            util::write_str(meta, &doc_id.to_string())?;
            util::write_newline(meta)?;
        }
        Ok(())
    }
}

/// The private `writeField(IndexOutput, BytesRef, ...)` overloads: the
/// name, the value as text, a newline.
fn write_field(out: &mut dyn IndexOutput, field_name: &[u8], value: &str) -> Result<()> {
    util::write_bytes(out, field_name)?;
    util::write_str(out, value)?;
    util::write_newline(out)
}

impl BufferingKnnVectorsWriterImpl for SimpleTextKnnVectorsWriter {
    /// Port of `writeField(FieldInfo, FloatVectorValues, int)`.
    fn write_field_float(
        &mut self,
        field_info: &FieldInfo,
        float_vector_values: &dyn FloatVectorValues,
        _max_doc: i32,
    ) -> Result<()> {
        let vector_data_offset = self.vector_data()?.file_pointer();
        let mut doc_ids = Vec::new();
        let mut iter = float_vector_values.iterator()?;
        let mut doc_id = iter.next_doc()?;
        while doc_id != NO_MORE_DOCS {
            // writeFloatVectorValue: write vector value
            let value = float_vector_values.vector_value(iter.index())?;
            debug_assert_eq!(value.len() as i32, float_vector_values.dimension());
            let vector_data = self.vector_data()?;
            util::write_str(vector_data, &floats_to_string(&value))?;
            util::write_newline(vector_data)?;
            doc_ids.push(doc_id);
            doc_id = iter.next_doc()?;
        }
        let vector_data_length = self.vector_data()?.file_pointer() - vector_data_offset;
        self.write_meta(field_info, vector_data_offset, vector_data_length, &doc_ids)
    }

    /// Port of `writeField(FieldInfo, ByteVectorValues, int)`.
    fn write_field_byte(
        &mut self,
        field_info: &FieldInfo,
        byte_vector_values: &dyn ByteVectorValues,
        _max_doc: i32,
    ) -> Result<()> {
        let vector_data_offset = self.vector_data()?.file_pointer();
        let mut doc_ids = Vec::new();
        let mut it = byte_vector_values.iterator()?;
        let mut doc_v = it.next_doc()?;
        while doc_v != NO_MORE_DOCS {
            // writeByteVectorValue: write vector value
            let value = byte_vector_values.vector_value(it.index())?;
            debug_assert_eq!(value.len() as i32, byte_vector_values.dimension());
            let vector_data = self.vector_data()?;
            util::write_str(vector_data, &bytes_to_string(&value))?;
            util::write_newline(vector_data)?;
            doc_ids.push(doc_v);
            doc_v = it.next_doc()?;
        }
        let vector_data_length = self.vector_data()?.file_pointer() - vector_data_offset;
        self.write_meta(field_info, vector_data_offset, vector_data_length, &doc_ids)
    }

    /// Port of `SimpleTextKnnVectorsWriter.finish`.
    fn finish(&mut self) -> Result<()> {
        let meta = self.meta()?;
        write_field(meta, FIELD_NUMBER, "-1")?;
        util::write_checksum(meta)?;
        util::write_checksum(self.vector_data()?)
    }

    /// Port of `SimpleTextKnnVectorsWriter.close`: `IOUtils.close(vectorData,
    /// meta)`.
    fn close(&mut self) -> Result<()> {
        let first = match self.vector_data.take() {
            Some(mut out) => out.close(),
            None => Ok(()),
        };
        let second = match self.meta.take() {
            Some(mut out) => out.close(),
            None => Ok(()),
        };
        first.and(second)
    }
}
