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

//! Port of `org.apache.lucene.tests.codecs.asserting.AssertingTermVectorsFormat`.

use std::fmt;
use std::sync::Arc;

use rucene::codecs::{TermVectorsFormat, TermVectorsReader, TermVectorsWriter};
use rucene::error::Result;
use rucene::index::Fields;
use rucene::index::{FieldInfo, FieldInfos, SegmentInfo, TermVectors};
use rucene::store::{Directory, IOContext};
use rucene::util::{Accountable, BytesRef};

use crate::codecs::{CodecFormat, TermVectorsKind};
use crate::index::asserting_leaf_reader::AssertingFields;
use crate::util::TestUtil;

/// Just like the default vectors format but with additional asserts.
///
/// Port of `AssertingTermVectorsFormat`, over
/// `TestUtil.getDefaultCodec().termVectorsFormat()`.
///
/// The writer does not override `merge` or `addProx`, as Java's does not:
/// both run `TermVectorsWriter`'s defaults through the asserting calls.
pub struct AssertingTermVectorsFormat {
    r#in: CodecFormat<TermVectorsKind>,
}

impl AssertingTermVectorsFormat {
    /// `new AssertingTermVectorsFormat()`.
    pub fn new() -> Self {
        AssertingTermVectorsFormat {
            r#in: CodecFormat::new(TestUtil::get_default_codec()),
        }
    }
}

impl Default for AssertingTermVectorsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AssertingTermVectorsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertingTermVectorsFormat")
    }
}

impl TermVectorsFormat for AssertingTermVectorsFormat {
    /// Java's `TermVectorsFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.r#in.get().name()
    }

    fn vectors_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>> {
        Ok(Box::new(AssertingTermVectorsReader::new(
            self.r#in
                .get()
                .vectors_reader(directory, segment_info, field_infos, context)?,
        )))
    }

    fn vectors_writer(
        &self,
        directory: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>> {
        Ok(Box::new(AssertingTermVectorsWriter::new(
            self.r#in
                .get()
                .vectors_writer(directory, segment_info, context)?,
        )))
    }
}

/// Port of `AssertingTermVectorsFormat.AssertingTermVectorsReader`.
struct AssertingTermVectorsReader {
    r#in: Box<dyn TermVectorsReader>,
}

impl AssertingTermVectorsReader {
    fn new(r#in: Box<dyn TermVectorsReader>) -> Self {
        // Java's `assert toString() != null` cannot fail in Rust.
        AssertingTermVectorsReader { r#in }
    }
}

impl fmt::Debug for AssertingTermVectorsReader {
    /// `getClass().getSimpleName() + "(" + in + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertingTermVectorsReader({:?})", self.r#in)
    }
}

impl TermVectors for AssertingTermVectorsReader {
    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        Ok(match self.r#in.get(doc)? {
            None => None,
            Some(fields) => Some(Box::new(AssertingFields::new(fields))),
        })
    }
}

impl TermVectorsReader for AssertingTermVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        self.r#in.check_integrity()
    }

    fn clone_reader(&self) -> Result<Box<dyn TermVectorsReader>> {
        Ok(Box::new(AssertingTermVectorsReader::new(
            self.r#in.clone_reader()?,
        )))
    }

    fn get_merge_instance(&self) -> Result<Box<dyn TermVectorsReader>> {
        Ok(Box::new(AssertingTermVectorsReader::new(
            self.r#in.get_merge_instance()?,
        )))
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}

/// `AssertingTermVectorsFormat.Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Undefined,
    Started,
    Finished,
}

/// Port of `AssertingTermVectorsFormat.AssertingTermVectorsWriter`.
#[derive(Debug)]
struct AssertingTermVectorsWriter {
    r#in: Box<dyn TermVectorsWriter>,
    doc_status: Status,
    field_status: Status,
    term_status: Status,
    doc_count: i32,
    field_count: i32,
    term_count: i32,
    position_count: i32,
    has_positions: bool,
}

impl AssertingTermVectorsWriter {
    fn new(r#in: Box<dyn TermVectorsWriter>) -> Self {
        AssertingTermVectorsWriter {
            r#in,
            doc_status: Status::Undefined,
            field_status: Status::Undefined,
            term_status: Status::Undefined,
            doc_count: 0,
            field_count: 0,
            term_count: 0,
            position_count: 0,
            has_positions: false,
        }
    }
}

impl Accountable for AssertingTermVectorsWriter {
    fn ram_bytes_used(&self) -> i64 {
        self.r#in.ram_bytes_used()
    }

    /// `Collections.singleton(in)`.
    fn child_resources(&self) -> Vec<Box<dyn Accountable + '_>> {
        vec![Box::new(&*self.r#in)]
    }
}

impl TermVectorsWriter for AssertingTermVectorsWriter {
    fn start_document(&mut self, num_vector_fields: i32) -> Result<()> {
        java_assert!(self.field_count == 0);
        java_assert!(self.doc_status != Status::Started);
        self.r#in.start_document(num_vector_fields)?;
        self.doc_status = Status::Started;
        self.field_count = num_vector_fields;
        self.doc_count += 1;
        Ok(())
    }

    fn finish_document(&mut self) -> Result<()> {
        java_assert!(self.field_count == 0);
        java_assert!(self.doc_status == Status::Started);
        self.r#in.finish_document()?;
        self.doc_status = Status::Finished;
        Ok(())
    }

    fn start_field(
        &mut self,
        info: &FieldInfo,
        num_terms: i32,
        positions: bool,
        offsets: bool,
        payloads: bool,
    ) -> Result<()> {
        java_assert!(self.term_count == 0);
        java_assert!(self.doc_status == Status::Started);
        java_assert!(self.field_status != Status::Started);
        self.r#in
            .start_field(info, num_terms, positions, offsets, payloads)?;
        self.field_status = Status::Started;
        self.term_count = num_terms;
        self.has_positions = positions || offsets || payloads;
        Ok(())
    }

    fn finish_field(&mut self) -> Result<()> {
        java_assert!(self.term_count == 0);
        java_assert!(self.field_status == Status::Started);
        self.r#in.finish_field()?;
        self.field_status = Status::Finished;
        self.field_count -= 1;
        Ok(())
    }

    fn start_term(&mut self, term: &BytesRef, freq: i32) -> Result<()> {
        java_assert!(self.doc_status == Status::Started);
        java_assert!(self.field_status == Status::Started);
        java_assert!(self.term_status != Status::Started);
        self.r#in.start_term(term, freq)?;
        self.term_status = Status::Started;
        self.position_count = if self.has_positions { freq } else { 0 };
        Ok(())
    }

    fn finish_term(&mut self) -> Result<()> {
        java_assert!(self.position_count == 0);
        java_assert!(self.doc_status == Status::Started);
        java_assert!(self.field_status == Status::Started);
        java_assert!(self.term_status == Status::Started);
        self.r#in.finish_term()?;
        self.term_status = Status::Finished;
        self.term_count -= 1;
        Ok(())
    }

    fn add_position(
        &mut self,
        position: i32,
        start_offset: i32,
        end_offset: i32,
        payload: Option<&BytesRef>,
    ) -> Result<()> {
        java_assert!(self.doc_status == Status::Started);
        java_assert!(self.field_status == Status::Started);
        java_assert!(self.term_status == Status::Started);
        self.r#in
            .add_position(position, start_offset, end_offset, payload)?;
        self.position_count -= 1;
        Ok(())
    }

    fn finish(&mut self, num_docs: i32) -> Result<()> {
        java_assert!(self.doc_count == num_docs);
        java_assert!(
            self.doc_status
                == if num_docs > 0 {
                    Status::Finished
                } else {
                    Status::Undefined
                }
        );
        java_assert!(self.field_status != Status::Started);
        java_assert!(self.term_status != Status::Started);
        self.r#in.finish(num_docs)
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}
