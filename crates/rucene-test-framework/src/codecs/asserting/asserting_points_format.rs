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

//! Port of `org.apache.lucene.tests.codecs.asserting.AssertingPointsFormat`.

use std::fmt;

use randomizedtesting::jdk::thread::Thread;
use rucene::codecs::{
    MergeState, PointsFormat, PointsReader, PointsWriter, SegmentReadState, SegmentWriteState,
};
use rucene::error::{LuceneError, Result};
use rucene::index::PointValues;
use rucene::index::{FieldInfo, FieldInfos};

use super::AssertingCodec;
use crate::codecs::{CodecFormat, PointsKind};
use crate::index::asserting_leaf_reader::AssertingPointValues;
use crate::util::TestUtil;

/// The points format an [`AssertingPointsFormat`] wraps: the default codec's,
/// or one given to the expert constructor.
enum InFormat {
    Codec(CodecFormat<PointsKind>),
    Given(Box<dyn PointsFormat>),
}

impl InFormat {
    fn get(&self) -> &dyn PointsFormat {
        match self {
            InFormat::Codec(format) => format.get(),
            InFormat::Given(format) => &**format,
        }
    }
}

/// Just like the default point format but with additional asserts.
///
/// Port of `AssertingPointsFormat`.
pub struct AssertingPointsFormat {
    r#in: InFormat,
}

impl AssertingPointsFormat {
    /// `new AssertingPointsFormat()`: over
    /// `TestUtil.getDefaultCodec().pointsFormat()`.
    pub fn new() -> Self {
        AssertingPointsFormat {
            r#in: InFormat::Codec(CodecFormat::new(TestUtil::get_default_codec())),
        }
    }

    /// `new AssertingPointsFormat(in)`. Expert: only intended to pass special
    /// parameters for testing.
    pub fn with_format(r#in: Box<dyn PointsFormat>) -> Self {
        AssertingPointsFormat {
            r#in: InFormat::Given(r#in),
        }
    }
}

impl Default for AssertingPointsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AssertingPointsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertingPointsFormat")
    }
}

impl PointsFormat for AssertingPointsFormat {
    /// Java's `PointsFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.r#in.get().name()
    }

    fn fields_writer(&self, state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Ok(Box::new(AssertingPointsWriter {
            r#in: self.r#in.get().fields_writer(state)?,
        }))
    }

    fn fields_reader(&self, state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(AssertingPointsReader::new(
            state.segment_info.max_doc()?,
            self.r#in.get().fields_reader(state)?,
            state.field_infos.clone(),
            false,
        )))
    }
}

/// Port of `AssertingPointsFormat.AssertingPointsReader`.
struct AssertingPointsReader {
    r#in: Box<dyn PointsReader>,
    max_doc: i32,
    fis: FieldInfos,
    merging: bool,
    creation_thread: Thread,
}

impl AssertingPointsReader {
    fn new(max_doc: i32, r#in: Box<dyn PointsReader>, fis: FieldInfos, merging: bool) -> Self {
        // Java's `assert toString() != null` cannot fail in Rust.
        AssertingPointsReader {
            r#in,
            max_doc,
            fis,
            merging,
            creation_thread: Thread::current(),
        }
    }
}

impl fmt::Debug for AssertingPointsReader {
    /// `getClass().getSimpleName() + "(" + in + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertingPointsReader({:?})", self.r#in)
    }
}

impl PointsReader for AssertingPointsReader {
    fn check_integrity(&self) -> Result<()> {
        self.r#in.check_integrity()
    }

    fn get_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        let fi = self.fis.field_info(field);
        java_assert!(fi.is_some_and(|fi| fi.get_point_dimension_count() > 0));
        if self.merging {
            AssertingCodec::assert_thread("PointsReader", &self.creation_thread)?;
        }
        let Some(values) = self.r#in.get_values(field)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(AssertingPointValues::new(
            values,
            self.max_doc,
        )?)))
    }

    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(AssertingPointsReader::new(
            self.max_doc,
            self.r#in.get_merge_instance()?,
            self.fis.clone(),
            true,
        )))
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}

/// Port of `AssertingPointsFormat.AssertingPointsWriter`.
#[derive(Debug)]
struct AssertingPointsWriter {
    r#in: Box<dyn PointsWriter>,
}

impl PointsWriter for AssertingPointsWriter {
    fn write_field(&mut self, field_info: &FieldInfo, values: &dyn PointsReader) -> Result<()> {
        if field_info.get_point_dimension_count() == 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "writing field=\"{}\" but pointDimensionalCount is 0",
                field_info.name
            )));
        }
        self.r#in.write_field(field_info, values)
    }

    fn merge(&mut self, merge_state: &MergeState) -> Result<()> {
        self.r#in.merge(merge_state)
    }

    fn finish(&mut self) -> Result<()> {
        self.r#in.finish()
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}
