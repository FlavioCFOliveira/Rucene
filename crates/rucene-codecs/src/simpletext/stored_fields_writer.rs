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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextStoredFieldsWriter`.

use rucene::codecs::StoredFieldsWriter;
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::FieldInfo;
use rucene::store::{Directory, IOContext, IndexOutput};
use rucene::util::java_fmt::{double_to_string, float_to_string};
use rucene::util::Accountable;

use super::util;

pub(crate) const FIELDS_EXTENSION: &str = "fld";

pub(crate) const TYPE_STRING: &[u8] = b"string";
pub(crate) const TYPE_BINARY: &[u8] = b"binary";
pub(crate) const TYPE_INT: &[u8] = b"int";
pub(crate) const TYPE_LONG: &[u8] = b"long";
pub(crate) const TYPE_FLOAT: &[u8] = b"float";
pub(crate) const TYPE_DOUBLE: &[u8] = b"double";

pub(crate) const END: &[u8] = b"END";
pub(crate) const DOC: &[u8] = b"doc ";
pub(crate) const FIELD: &[u8] = b"  field ";
pub(crate) const NAME: &[u8] = b"    name ";
pub(crate) const TYPE: &[u8] = b"    type ";
pub(crate) const VALUE: &[u8] = b"    value ";

/// Writes plain-text stored fields.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextStoredFieldsWriter`.
///
/// **FOR RECREATIONAL USE ONLY**
pub struct SimpleTextStoredFieldsWriter {
    num_docs_written: i32,
    out: Option<Box<dyn IndexOutput>>,
}

impl std::fmt::Debug for SimpleTextStoredFieldsWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleTextStoredFieldsWriter")
            .field("num_docs_written", &self.num_docs_written)
            .finish_non_exhaustive()
    }
}

impl SimpleTextStoredFieldsWriter {
    /// Creates the writer of `segment`'s `.fld` file.
    ///
    /// # Errors
    ///
    /// Propagates the directory's error.
    pub fn new(directory: &dyn Directory, segment: &str, context: &dyn IOContext) -> Result<Self> {
        let out =
            directory.create_output(&segment_file_name(segment, "", FIELDS_EXTENSION), context)?;
        Ok(Self {
            num_docs_written: 0,
            out: Some(out),
        })
    }

    fn out(&mut self) -> Result<&mut dyn IndexOutput> {
        match self.out.as_deref_mut() {
            Some(out) => Ok(out),
            None => Err(LuceneError::already_closed("this writer is closed")),
        }
    }

    fn write(&mut self, s: &str) -> Result<()> {
        util::write_str(self.out()?, s)
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        util::write_bytes(self.out()?, bytes)
    }

    fn new_line(&mut self) -> Result<()> {
        util::write_newline(self.out()?)
    }

    /// The private `writeField(FieldInfo)`: the field number and name.
    fn write_field_header(&mut self, info: &FieldInfo) -> Result<()> {
        self.write_bytes(FIELD)?;
        self.write(&info.number.to_string())?;
        self.new_line()?;

        self.write_bytes(NAME)?;
        self.write(&info.name)?;
        self.new_line()
    }

    fn write_typed(&mut self, info: &FieldInfo, type_: &[u8], value: &[u8]) -> Result<()> {
        self.write_field_header(info)?;

        self.write_bytes(TYPE)?;
        self.write_bytes(type_)?;
        self.new_line()?;

        self.write_bytes(VALUE)?;
        self.write_bytes(value)?;
        self.new_line()
    }
}

impl Accountable for SimpleTextStoredFieldsWriter {
    fn ram_bytes_used(&self) -> i64 {
        4 // Integer.BYTES: something > 0
    }
}

impl StoredFieldsWriter for SimpleTextStoredFieldsWriter {
    fn start_document(&mut self) -> Result<()> {
        self.write_bytes(DOC)?;
        let n = self.num_docs_written.to_string();
        self.write(&n)?;
        self.new_line()?;

        self.num_docs_written += 1;
        Ok(())
    }

    fn write_field_i32(&mut self, info: &FieldInfo, value: i32) -> Result<()> {
        self.write_typed(info, TYPE_INT, value.to_string().as_bytes())
    }

    fn write_field_i64(&mut self, info: &FieldInfo, value: i64) -> Result<()> {
        self.write_typed(info, TYPE_LONG, value.to_string().as_bytes())
    }

    fn write_field_f32(&mut self, info: &FieldInfo, value: f32) -> Result<()> {
        self.write_typed(info, TYPE_FLOAT, float_to_string(value).as_bytes())
    }

    fn write_field_f64(&mut self, info: &FieldInfo, value: f64) -> Result<()> {
        self.write_typed(info, TYPE_DOUBLE, double_to_string(value).as_bytes())
    }

    fn write_field_bytes(&mut self, info: &FieldInfo, value: &[u8]) -> Result<()> {
        self.write_typed(info, TYPE_BINARY, value)
    }

    fn write_field_string(&mut self, info: &FieldInfo, value: &str) -> Result<()> {
        self.write_typed(info, TYPE_STRING, value.as_bytes())
    }

    fn finish(&mut self, num_docs: i32) -> Result<()> {
        if self.num_docs_written != num_docs {
            let description = self.out.as_deref().map_or_else(
                || "null".to_string(),
                |o| o.resource_description().to_string(),
            );
            return Err(LuceneError::Other(format!(
                "mergeFields produced an invalid result: docCount is {num_docs} but only saw {} \
                 file={description}; now aborting this merge to prevent index corruption",
                self.num_docs_written
            )));
        }
        self.write_bytes(END)?;
        self.new_line()?;
        util::write_checksum(self.out()?)
    }

    fn close(&mut self) -> Result<()> {
        match self.out.take() {
            Some(mut out) => out.close(),
            None => Ok(()),
        }
    }
}
