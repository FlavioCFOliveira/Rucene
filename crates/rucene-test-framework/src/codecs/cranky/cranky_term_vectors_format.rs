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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankyTermVectorsFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::{MergeState, TermVectorsFormat, TermVectorsReader, TermVectorsWriter};
use rucene::error::Result;
use rucene::index::{FieldInfo, FieldInfos, SegmentInfo};
use rucene::store::{DataInput, Directory, IOContext};
use rucene::util::{Accountable, BytesRef};

use super::maybe_fail;
use crate::codecs::{CodecFormat, TermVectorsKind};

/// Port of `CrankyTermVectorsFormat`: `vectorsWriter` and the writer's
/// `merge`, `finish` and `close` fail one time in 100, the per-document,
/// per-field, per-term and per-position calls one time in 10000.
pub struct CrankyTermVectorsFormat {
    delegate: CodecFormat<TermVectorsKind>,
    random: Arc<dyn Random>,
}

impl CrankyTermVectorsFormat {
    pub(crate) fn new(delegate: CodecFormat<TermVectorsKind>, random: Arc<dyn Random>) -> Self {
        CrankyTermVectorsFormat { delegate, random }
    }
}

impl fmt::Debug for CrankyTermVectorsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CrankyTermVectorsFormat")
    }
}

impl TermVectorsFormat for CrankyTermVectorsFormat {
    /// Java's `TermVectorsFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.delegate.get().name()
    }

    fn vectors_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>> {
        self.delegate
            .get()
            .vectors_reader(directory, segment_info, field_infos, context)
    }

    fn vectors_writer(
        &self,
        directory: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from TermVectorsFormat.vectorsWriter()",
        )?;
        Ok(Box::new(CrankyTermVectorsWriter {
            delegate: self
                .delegate
                .get()
                .vectors_writer(directory, segment_info, context)?,
            random: Arc::clone(&self.random),
        }))
    }
}

/// Port of `CrankyTermVectorsFormat.CrankyTermVectorsWriter`.
struct CrankyTermVectorsWriter {
    delegate: Box<dyn TermVectorsWriter>,
    random: Arc<dyn Random>,
}

impl fmt::Debug for CrankyTermVectorsWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CrankyTermVectorsWriter({:?})", self.delegate)
    }
}

impl Accountable for CrankyTermVectorsWriter {
    fn ram_bytes_used(&self) -> i64 {
        self.delegate.ram_bytes_used()
    }

    /// `Collections.singleton(delegate)`.
    fn child_resources(&self) -> Vec<Box<dyn Accountable + '_>> {
        vec![Box::new(&*self.delegate)]
    }
}

/// `super.merge(mergeState)` and `super.addProx(...)`: the writer seen
/// through a handle that forwards every call but `merge` and `addProx`, so
/// `TermVectorsWriter`'s defaults run over the cranky calls, as Java's
/// `super` calls do.
#[derive(Debug)]
struct Super<'w>(&'w mut CrankyTermVectorsWriter);

impl Accountable for Super<'_> {
    fn ram_bytes_used(&self) -> i64 {
        self.0.ram_bytes_used()
    }
}

impl TermVectorsWriter for Super<'_> {
    fn start_document(&mut self, num_vector_fields: i32) -> Result<()> {
        self.0.start_document(num_vector_fields)
    }
    fn finish_document(&mut self) -> Result<()> {
        self.0.finish_document()
    }
    fn start_field(
        &mut self,
        info: &FieldInfo,
        num_terms: i32,
        positions: bool,
        offsets: bool,
        payloads: bool,
    ) -> Result<()> {
        self.0
            .start_field(info, num_terms, positions, offsets, payloads)
    }
    fn finish_field(&mut self) -> Result<()> {
        self.0.finish_field()
    }
    fn start_term(&mut self, term: &BytesRef, freq: i32) -> Result<()> {
        self.0.start_term(term, freq)
    }
    fn finish_term(&mut self) -> Result<()> {
        self.0.finish_term()
    }
    fn add_position(
        &mut self,
        position: i32,
        start_offset: i32,
        end_offset: i32,
        payload: Option<&BytesRef>,
    ) -> Result<()> {
        self.0
            .add_position(position, start_offset, end_offset, payload)
    }
    fn finish(&mut self, num_docs: i32) -> Result<()> {
        self.0.finish(num_docs)
    }
    fn close(&mut self) -> Result<()> {
        self.0.close()
    }
}

impl TermVectorsWriter for CrankyTermVectorsWriter {
    fn merge(&mut self, merge_state: &MergeState) -> Result<i32> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from TermVectorsWriter.merge()",
        )?;
        Super(self).merge(merge_state)
    }

    fn finish(&mut self, num_docs: i32) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from TermVectorsWriter.finish()",
        )?;
        self.delegate.finish(num_docs)
    }

    fn close(&mut self) -> Result<()> {
        self.delegate.close()?;
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from TermVectorsWriter.close()",
        )
    }

    // per doc/field methods: lower probability since they are invoked so many times.

    fn start_document(&mut self, num_vector_fields: i32) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from TermVectorsWriter.startDocument()",
        )?;
        self.delegate.start_document(num_vector_fields)
    }

    fn finish_document(&mut self) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from TermVectorsWriter.finishDocument()",
        )?;
        self.delegate.finish_document()
    }

    fn start_field(
        &mut self,
        info: &FieldInfo,
        num_terms: i32,
        positions: bool,
        offsets: bool,
        payloads: bool,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from TermVectorsWriter.startField()",
        )?;
        self.delegate
            .start_field(info, num_terms, positions, offsets, payloads)
    }

    fn finish_field(&mut self) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from TermVectorsWriter.finishField()",
        )?;
        self.delegate.finish_field()
    }

    fn start_term(&mut self, term: &BytesRef, freq: i32) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from TermVectorsWriter.startTerm()",
        )?;
        self.delegate.start_term(term, freq)
    }

    fn finish_term(&mut self) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from TermVectorsWriter.finishTerm()",
        )?;
        self.delegate.finish_term()
    }

    fn add_position(
        &mut self,
        position: i32,
        start_offset: i32,
        end_offset: i32,
        payload: Option<&BytesRef>,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from TermVectorsWriter.addPosition()",
        )?;
        self.delegate
            .add_position(position, start_offset, end_offset, payload)
    }

    fn add_prox(
        &mut self,
        num_prox: i32,
        positions: Option<&mut dyn DataInput>,
        offsets: Option<&mut dyn DataInput>,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from TermVectorsWriter.addProx()",
        )?;
        Super(self).add_prox(num_prox, positions, offsets)
    }
}
