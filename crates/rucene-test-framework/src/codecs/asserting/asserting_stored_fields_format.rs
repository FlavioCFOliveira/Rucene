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

//! Port of `org.apache.lucene.tests.codecs.asserting.AssertingStoredFieldsFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::thread::Thread;
use rucene::codecs::{StoredFieldsFormat, StoredFieldsReader, StoredFieldsWriter};
use rucene::error::Result;
use rucene::index::StoredFieldDataInput;
use rucene::index::{FieldInfo, FieldInfos, SegmentInfo, StoredFieldVisitor};
use rucene::store::{Directory, IOContext};
use rucene::util::Accountable;

use super::AssertingCodec;
use crate::codecs::{CodecFormat, StoredFieldsKind};
use crate::util::TestUtil;

/// Just like the default stored fields format but with additional asserts.
///
/// Port of `AssertingStoredFieldsFormat`, over
/// `TestUtil.getDefaultCodec().storedFieldsFormat()`.
///
/// The writer does not override `merge`, as Java's does not: a merge writes
/// every document through the asserting `startDocument` / `writeField` /
/// `finishDocument` calls of `StoredFieldsWriter`'s default `merge`, not
/// through the delegate's own (bulk-copying) merge.
pub struct AssertingStoredFieldsFormat {
    r#in: CodecFormat<StoredFieldsKind>,
}

impl AssertingStoredFieldsFormat {
    /// `new AssertingStoredFieldsFormat()`.
    pub fn new() -> Self {
        AssertingStoredFieldsFormat {
            r#in: CodecFormat::new(TestUtil::get_default_codec()),
        }
    }
}

impl Default for AssertingStoredFieldsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AssertingStoredFieldsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertingStoredFieldsFormat")
    }
}

impl StoredFieldsFormat for AssertingStoredFieldsFormat {
    /// Java's `StoredFieldsFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.r#in.get().name()
    }

    fn fields_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(AssertingStoredFieldsReader::new(
            self.r#in
                .get()
                .fields_reader(directory, segment_info, field_infos, context)?,
            segment_info.max_doc()?,
            false,
        )))
    }

    fn fields_writer(
        &self,
        directory: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>> {
        Ok(Box::new(AssertingStoredFieldsWriter::new(
            self.r#in
                .get()
                .fields_writer(directory, segment_info, context)?,
        )))
    }
}

/// Port of `AssertingStoredFieldsFormat.AssertingStoredFieldsReader`.
struct AssertingStoredFieldsReader {
    r#in: Box<dyn StoredFieldsReader>,
    max_doc: i32,
    merging: bool,
    creation_thread: Thread,
}

impl AssertingStoredFieldsReader {
    fn new(r#in: Box<dyn StoredFieldsReader>, max_doc: i32, merging: bool) -> Self {
        // Java's `assert toString() != null` cannot fail in Rust.
        AssertingStoredFieldsReader {
            r#in,
            max_doc,
            merging,
            creation_thread: Thread::current(),
        }
    }
}

impl fmt::Debug for AssertingStoredFieldsReader {
    /// `getClass().getSimpleName() + "(" + in + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertingStoredFieldsReader({:?})", self.r#in)
    }
}

impl StoredFieldsReader for AssertingStoredFieldsReader {
    fn document(&self, n: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        AssertingCodec::assert_thread("StoredFieldsReader", &self.creation_thread)?;
        java_assert!(n >= 0 && n < self.max_doc);
        self.r#in.document(n, visitor)
    }

    fn check_integrity(&self) -> Result<()> {
        self.r#in.check_integrity()
    }

    fn clone_reader(&self) -> Result<Box<dyn StoredFieldsReader>> {
        java_assert!(!self.merging, "Merge instances do not support cloning");
        Ok(Box::new(AssertingStoredFieldsReader::new(
            self.r#in.clone_reader()?,
            self.max_doc,
            false,
        )))
    }

    fn get_merge_instance(&self) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(AssertingStoredFieldsReader::new(
            self.r#in.get_merge_instance()?,
            self.max_doc,
            true,
        )))
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}

/// `AssertingStoredFieldsFormat.Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Undefined,
    Started,
    Finished,
}

/// Port of `AssertingStoredFieldsFormat.AssertingStoredFieldsWriter`.
#[derive(Debug)]
struct AssertingStoredFieldsWriter {
    r#in: Box<dyn StoredFieldsWriter>,
    num_written: i32,
    doc_status: Status,
}

impl AssertingStoredFieldsWriter {
    fn new(r#in: Box<dyn StoredFieldsWriter>) -> Self {
        AssertingStoredFieldsWriter {
            r#in,
            num_written: 0,
            doc_status: Status::Undefined,
        }
    }
}

impl Accountable for AssertingStoredFieldsWriter {
    fn ram_bytes_used(&self) -> i64 {
        self.r#in.ram_bytes_used()
    }

    /// `in.getChildResources()`.
    fn child_resources(&self) -> Vec<Box<dyn Accountable + '_>> {
        self.r#in.child_resources()
    }
}

impl StoredFieldsWriter for AssertingStoredFieldsWriter {
    fn start_document(&mut self) -> Result<()> {
        java_assert!(self.doc_status != Status::Started);
        self.r#in.start_document()?;
        self.num_written += 1;
        self.doc_status = Status::Started;
        Ok(())
    }

    fn finish_document(&mut self) -> Result<()> {
        java_assert!(self.doc_status == Status::Started);
        self.r#in.finish_document()?;
        self.doc_status = Status::Finished;
        Ok(())
    }

    fn write_field_i32(&mut self, info: &FieldInfo, value: i32) -> Result<()> {
        java_assert!(self.doc_status == Status::Started);
        self.r#in.write_field_i32(info, value)
    }

    fn write_field_i64(&mut self, info: &FieldInfo, value: i64) -> Result<()> {
        java_assert!(self.doc_status == Status::Started);
        self.r#in.write_field_i64(info, value)
    }

    fn write_field_f32(&mut self, info: &FieldInfo, value: f32) -> Result<()> {
        java_assert!(self.doc_status == Status::Started);
        self.r#in.write_field_f32(info, value)
    }

    fn write_field_f64(&mut self, info: &FieldInfo, value: f64) -> Result<()> {
        java_assert!(self.doc_status == Status::Started);
        self.r#in.write_field_f64(info, value)
    }

    fn write_field_bytes(&mut self, info: &FieldInfo, value: &[u8]) -> Result<()> {
        java_assert!(self.doc_status == Status::Started);
        self.r#in.write_field_bytes(info, value)
    }

    fn write_field_data_input(
        &mut self,
        info: &FieldInfo,
        value: &mut StoredFieldDataInput<'_>,
    ) -> Result<()> {
        java_assert!(self.doc_status == Status::Started);
        self.r#in.write_field_data_input(info, value)
    }

    fn write_field_string(&mut self, info: &FieldInfo, value: &str) -> Result<()> {
        java_assert!(self.doc_status == Status::Started);
        self.r#in.write_field_string(info, value)
    }

    fn finish(&mut self, num_docs: i32) -> Result<()> {
        java_assert!(
            self.doc_status
                == if num_docs > 0 {
                    Status::Finished
                } else {
                    Status::Undefined
                }
        );
        self.r#in.finish(num_docs)?;
        java_assert!(num_docs == self.num_written);
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}
