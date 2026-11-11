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

//! Port of `org.apache.lucene.tests.codecs.asserting.AssertingNormsFormat`.

use std::fmt;

use randomizedtesting::jdk::thread::Thread;
use rucene::codecs::{
    NormsConsumer, NormsFormat, NormsProducer, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene::index::FieldInfo;
use rucene::index::NumericDocValues;
use rucene::search::NO_MORE_DOCS;

use super::AssertingCodec;
use crate::codecs::{CodecFormat, NormsKind};
use crate::index::asserting_leaf_reader::{npe, AssertingNumericDocValues};
use crate::util::TestUtil;

/// Just like the default norms format but with additional asserts.
///
/// Port of `AssertingNormsFormat`, over
/// `TestUtil.getDefaultCodec().normsFormat()`.
pub struct AssertingNormsFormat {
    r#in: CodecFormat<NormsKind>,
}

impl AssertingNormsFormat {
    /// `new AssertingNormsFormat()`.
    pub fn new() -> Self {
        AssertingNormsFormat {
            r#in: CodecFormat::new(TestUtil::get_default_codec()),
        }
    }
}

impl Default for AssertingNormsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AssertingNormsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertingNormsFormat")
    }
}

impl NormsFormat for AssertingNormsFormat {
    /// Java's `NormsFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.r#in.get().name()
    }

    fn norms_consumer(&self, state: &SegmentWriteState) -> Result<Box<dyn NormsConsumer>> {
        let consumer = self.r#in.get().norms_consumer(state)?;
        Ok(Box::new(AssertingNormsConsumer {
            r#in: consumer,
            max_doc: state.segment_info.max_doc()?,
        }))
    }

    fn norms_producer(&self, state: &SegmentReadState) -> Result<Box<dyn NormsProducer>> {
        java_assert!(state.field_infos.has_norms());
        let producer = self.r#in.get().norms_producer(state)?;
        Ok(Box::new(AssertingNormsProducer::new(
            producer,
            state.segment_info.max_doc()?,
            false,
        )))
    }
}

/// Port of `AssertingNormsFormat.AssertingNormsConsumer`.
#[derive(Debug)]
struct AssertingNormsConsumer {
    r#in: Box<dyn NormsConsumer>,
    max_doc: i32,
}

impl NormsConsumer for AssertingNormsConsumer {
    fn add_norms_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn NormsProducer,
    ) -> Result<()> {
        let mut values = values_producer.get_norms(field)?.ok_or_else(npe)?;
        let mut last_doc_id = -1;
        loop {
            let doc_id = values.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            java_assert!(doc_id >= 0 && doc_id < self.max_doc);
            java_assert!(doc_id > last_doc_id);
            last_doc_id = doc_id;
            values.long_value()?;
        }
        self.r#in.add_norms_field(field, values_producer)
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}

/// Port of `AssertingNormsFormat.AssertingNormsProducer`.
struct AssertingNormsProducer {
    r#in: Box<dyn NormsProducer>,
    max_doc: i32,
    merging: bool,
    creation_thread: Thread,
}

impl AssertingNormsProducer {
    fn new(r#in: Box<dyn NormsProducer>, max_doc: i32, merging: bool) -> Self {
        // Java's `assert toString() != null` cannot fail in Rust.
        AssertingNormsProducer {
            r#in,
            max_doc,
            merging,
            creation_thread: Thread::current(),
        }
    }
}

impl fmt::Debug for AssertingNormsProducer {
    /// `getClass().getSimpleName() + "(" + in + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertingNormsProducer({:?})", self.r#in)
    }
}

impl NormsProducer for AssertingNormsProducer {
    fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        if self.merging {
            AssertingCodec::assert_thread("NormsProducer", &self.creation_thread)?;
        }
        java_assert!(field.has_norms());
        let values = self.r#in.get_norms(field)?;
        java_assert!(values.is_some());
        let values = values.ok_or_else(npe)?;
        Ok(Some(Box::new(AssertingNumericDocValues::new(
            values,
            self.max_doc,
        )?)))
    }

    fn check_integrity(&self) -> Result<()> {
        self.r#in.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(AssertingNormsProducer::new(
            self.r#in.get_merge_instance()?,
            self.max_doc,
            true,
        )))
    }

    fn close(&self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}
