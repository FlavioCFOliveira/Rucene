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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankyStoredFieldsFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::{MergeState, StoredFieldsFormat, StoredFieldsReader, StoredFieldsWriter};
use rucene::error::Result;
use rucene::index::{FieldInfo, FieldInfos, SegmentInfo, StoredFieldDataInput};
use rucene::store::{Directory, IOContext};
use rucene::util::Accountable;

use super::maybe_fail;
use crate::codecs::{CodecFormat, StoredFieldsKind};

/// Port of `CrankyStoredFieldsFormat`: `fieldsWriter` and the writer's
/// `finish` and `merge` fail one time in 100, `close` one time in 1000, the
/// per-document and per-field calls one time in 10000.
pub struct CrankyStoredFieldsFormat {
    delegate: CodecFormat<StoredFieldsKind>,
    random: Arc<dyn Random>,
}

impl CrankyStoredFieldsFormat {
    pub(crate) fn new(delegate: CodecFormat<StoredFieldsKind>, random: Arc<dyn Random>) -> Self {
        CrankyStoredFieldsFormat { delegate, random }
    }
}

impl fmt::Debug for CrankyStoredFieldsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CrankyStoredFieldsFormat")
    }
}

impl StoredFieldsFormat for CrankyStoredFieldsFormat {
    /// Java's `StoredFieldsFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.delegate.get().name()
    }

    fn fields_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>> {
        self.delegate
            .get()
            .fields_reader(directory, segment_info, field_infos, context)
    }

    fn fields_writer(
        &self,
        directory: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from StoredFieldsFormat.fieldsWriter()",
        )?;
        Ok(Box::new(CrankyStoredFieldsWriter {
            delegate: self
                .delegate
                .get()
                .fields_writer(directory, segment_info, context)?,
            random: Arc::clone(&self.random),
        }))
    }
}

/// Port of `CrankyStoredFieldsFormat.CrankyStoredFieldsWriter`.
struct CrankyStoredFieldsWriter {
    delegate: Box<dyn StoredFieldsWriter>,
    random: Arc<dyn Random>,
}

impl fmt::Debug for CrankyStoredFieldsWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CrankyStoredFieldsWriter({:?})", self.delegate)
    }
}

impl Accountable for CrankyStoredFieldsWriter {
    fn ram_bytes_used(&self) -> i64 {
        self.delegate.ram_bytes_used()
    }

    /// `delegate.getChildResources()`.
    fn child_resources(&self) -> Vec<Box<dyn Accountable + '_>> {
        self.delegate.child_resources()
    }
}

/// `super.merge(mergeState)`: the writer seen through a handle that forwards
/// every call but `merge`, so `StoredFieldsWriter`'s default merge runs over
/// the cranky per-document calls, as Java's `super.merge` does.
#[derive(Debug)]
struct SuperMerge<'w>(&'w mut CrankyStoredFieldsWriter);

impl Accountable for SuperMerge<'_> {
    fn ram_bytes_used(&self) -> i64 {
        self.0.ram_bytes_used()
    }
}

impl StoredFieldsWriter for SuperMerge<'_> {
    fn start_document(&mut self) -> Result<()> {
        self.0.start_document()
    }
    fn finish_document(&mut self) -> Result<()> {
        self.0.finish_document()
    }
    fn write_field_i32(&mut self, info: &FieldInfo, value: i32) -> Result<()> {
        self.0.write_field_i32(info, value)
    }
    fn write_field_i64(&mut self, info: &FieldInfo, value: i64) -> Result<()> {
        self.0.write_field_i64(info, value)
    }
    fn write_field_f32(&mut self, info: &FieldInfo, value: f32) -> Result<()> {
        self.0.write_field_f32(info, value)
    }
    fn write_field_f64(&mut self, info: &FieldInfo, value: f64) -> Result<()> {
        self.0.write_field_f64(info, value)
    }
    fn write_field_bytes(&mut self, info: &FieldInfo, value: &[u8]) -> Result<()> {
        self.0.write_field_bytes(info, value)
    }
    fn write_field_data_input(
        &mut self,
        info: &FieldInfo,
        value: &mut StoredFieldDataInput<'_>,
    ) -> Result<()> {
        self.0.write_field_data_input(info, value)
    }
    fn write_field_string(&mut self, info: &FieldInfo, value: &str) -> Result<()> {
        self.0.write_field_string(info, value)
    }
    fn finish(&mut self, num_docs: i32) -> Result<()> {
        self.0.finish(num_docs)
    }
    fn close(&mut self) -> Result<()> {
        self.0.close()
    }
}

impl StoredFieldsWriter for CrankyStoredFieldsWriter {
    fn finish(&mut self, num_docs: i32) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from StoredFieldsWriter.finish()",
        )?;
        self.delegate.finish(num_docs)
    }

    fn merge(&mut self, merge_state: &MergeState) -> Result<i32> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from StoredFieldsWriter.merge()",
        )?;
        SuperMerge(self).merge(merge_state)
    }

    fn close(&mut self) -> Result<()> {
        self.delegate.close()?;
        maybe_fail(
            &*self.random,
            1000,
            "Fake IOException from StoredFieldsWriter.close()",
        )
    }

    // per doc/field methods: lower probability since they are invoked so many times.

    fn start_document(&mut self) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from StoredFieldsWriter.startDocument()",
        )?;
        self.delegate.start_document()
    }

    fn finish_document(&mut self) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from StoredFieldsWriter.finishDocument()",
        )?;
        self.delegate.finish_document()
    }

    fn write_field_i32(&mut self, info: &FieldInfo, value: i32) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from StoredFieldsWriter.writeField()",
        )?;
        self.delegate.write_field_i32(info, value)
    }

    fn write_field_i64(&mut self, info: &FieldInfo, value: i64) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from StoredFieldsWriter.writeField()",
        )?;
        self.delegate.write_field_i64(info, value)
    }

    fn write_field_f32(&mut self, info: &FieldInfo, value: f32) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from StoredFieldsWriter.writeField()",
        )?;
        self.delegate.write_field_f32(info, value)
    }

    fn write_field_f64(&mut self, info: &FieldInfo, value: f64) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from StoredFieldsWriter.writeField()",
        )?;
        self.delegate.write_field_f64(info, value)
    }

    fn write_field_bytes(&mut self, info: &FieldInfo, value: &[u8]) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from StoredFieldsWriter.writeField()",
        )?;
        self.delegate.write_field_bytes(info, value)
    }

    fn write_field_data_input(
        &mut self,
        info: &FieldInfo,
        value: &mut StoredFieldDataInput<'_>,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from StoredFieldsWriter.writeField()",
        )?;
        self.delegate.write_field_data_input(info, value)
    }

    fn write_field_string(&mut self, info: &FieldInfo, value: &str) -> Result<()> {
        maybe_fail(
            &*self.random,
            10000,
            "Fake IOException from StoredFieldsWriter.writeField()",
        )?;
        self.delegate.write_field_string(info, value)
    }
}
