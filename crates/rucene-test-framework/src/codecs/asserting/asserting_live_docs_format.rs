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

//! Port of `org.apache.lucene.tests.codecs.asserting.AssertingLiveDocsFormat`.

use std::fmt;

use rucene::codecs::LiveDocsFormat;
use rucene::error::Result;
use rucene::index::SegmentCommitInfo;
use rucene::search::DocIdSetIterator;
use rucene::store::{Directory, IOContext};
use rucene::util::bit_set::LiveDocs;
use rucene::util::{Bits, FixedBitSet};

use crate::codecs::{CodecFormat, LiveDocsKind};
use crate::util::TestUtil;

/// Just like the default live docs format but with additional asserts.
///
/// Port of `AssertingLiveDocsFormat`, over
/// `TestUtil.getDefaultCodec().liveDocsFormat()`.
///
/// Java returns an `AssertingLiveDocs` when the bits it read are
/// `LiveDocs` (`instanceof`) and an `AssertingBits` otherwise. A
/// `Box<dyn Bits>` cannot be narrowed in this port without consuming it, so
/// the bits come back as an `AssertingBits` whose
/// [`Bits::into_live_docs`] — the port's narrowing of `Bits` to `LiveDocs` —
/// answers an `AssertingLiveDocs` exactly when the bits it wraps are
/// `LiveDocs`: a caller sees Java's two classes where Java's `instanceof`
/// would.
pub struct AssertingLiveDocsFormat {
    r#in: CodecFormat<LiveDocsKind>,
}

impl AssertingLiveDocsFormat {
    /// `new AssertingLiveDocsFormat()`.
    pub fn new() -> Self {
        AssertingLiveDocsFormat {
            r#in: CodecFormat::new(TestUtil::get_default_codec()),
        }
    }

    fn check(bits: &dyn Bits, expected_length: i32, expected_delete_count: i32) -> Result<()> {
        java_assert!(bits.length() as i64 == i64::from(expected_length));
        let mut deleted_count = 0;
        for i in 0..bits.length() {
            if !bits.get(i) {
                deleted_count += 1;
            }
        }
        java_assert!(
            deleted_count == expected_delete_count,
            "deleted: {} != expected: {}",
            deleted_count,
            expected_delete_count
        );
        Ok(())
    }
}

impl Default for AssertingLiveDocsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AssertingLiveDocsFormat {
    /// `"Asserting(" + in + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Asserting({:?})", self.r#in)
    }
}

impl LiveDocsFormat for AssertingLiveDocsFormat {
    /// Java's `LiveDocsFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.r#in.get().name()
    }

    fn read_live_docs(
        &self,
        dir: &dyn Directory,
        info: &SegmentCommitInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn Bits>> {
        let raw = self.r#in.get().read_live_docs(dir, info, context)?;
        Self::check(&*raw, info.info.max_doc()?, info.get_del_count())?;
        Ok(Box::new(AssertingBits::new(raw)?))
    }

    fn write_live_docs(
        &self,
        bits: &dyn Bits,
        dir: &dyn Directory,
        info: &SegmentCommitInfo,
        new_del_count: i32,
        context: &dyn IOContext,
    ) -> Result<()> {
        Self::check(
            bits,
            info.info.max_doc()?,
            info.get_del_count() + new_del_count,
        )?;
        self.r#in
            .get()
            .write_live_docs(bits, dir, info, new_del_count, context)
    }

    fn files(&self, info: &SegmentCommitInfo, files: &mut Vec<String>) -> Result<()> {
        self.r#in.get().files(info, files)
    }
}

/// Port of `AssertingLiveDocsFormat.AssertingBits`.
#[derive(Debug)]
struct AssertingBits {
    r#in: Box<dyn Bits>,
}

impl AssertingBits {
    fn new(r#in: Box<dyn Bits>) -> Result<Self> {
        // `assert in.length() >= 0`: a `usize` always is.
        Ok(AssertingBits { r#in })
    }
}

impl Bits for AssertingBits {
    fn get(&self, index: usize) -> bool {
        java_assert_unchecked!(
            index < self.r#in.length(),
            "index={} vs in.length()={}",
            index,
            self.r#in.length()
        );
        self.r#in.get(index)
    }

    fn try_get(&self, index: usize) -> Result<bool> {
        java_assert!(
            index < self.r#in.length(),
            "index={} vs in.length()={}",
            index,
            self.r#in.length()
        );
        self.r#in.try_get(index)
    }

    fn length(&self) -> usize {
        self.r#in.length()
    }

    fn apply_mask(&self, bit_set: &mut FixedBitSet, offset: usize) -> Result<()> {
        // `assert offset >= 0`: a `usize` always is.
        self.r#in.apply_mask(bit_set, offset)
    }

    fn into_live_docs(self: Box<Self>) -> Option<Box<dyn LiveDocs>> {
        self.r#in
            .into_live_docs()
            .map(|live_docs| Box::new(AssertingLiveDocs { r#in: live_docs }) as Box<dyn LiveDocs>)
    }
}

/// Port of `AssertingLiveDocsFormat.AssertingLiveDocs` (`extends
/// AssertingBits implements LiveDocs`).
#[derive(Debug)]
struct AssertingLiveDocs {
    r#in: Box<dyn LiveDocs>,
}

impl Bits for AssertingLiveDocs {
    fn get(&self, index: usize) -> bool {
        java_assert_unchecked!(
            index < self.r#in.length(),
            "index={} vs in.length()={}",
            index,
            self.r#in.length()
        );
        self.r#in.get(index)
    }

    fn try_get(&self, index: usize) -> Result<bool> {
        java_assert!(
            index < self.r#in.length(),
            "index={} vs in.length()={}",
            index,
            self.r#in.length()
        );
        self.r#in.try_get(index)
    }

    fn length(&self) -> usize {
        self.r#in.length()
    }

    fn apply_mask(&self, bit_set: &mut FixedBitSet, offset: usize) -> Result<()> {
        self.r#in.apply_mask(bit_set, offset)
    }

    fn into_live_docs(self: Box<Self>) -> Option<Box<dyn LiveDocs>> {
        Some(self)
    }
}

impl LiveDocs for AssertingLiveDocs {
    fn live_docs_iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        self.r#in.live_docs_iterator()
    }

    fn deleted_docs_iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        self.r#in.deleted_docs_iterator()
    }

    fn deleted_count(&self) -> i32 {
        let count = self.r#in.deleted_count();
        java_assert_unchecked!(count >= 0);
        java_assert_unchecked!(count as usize <= self.length());
        count
    }
}
