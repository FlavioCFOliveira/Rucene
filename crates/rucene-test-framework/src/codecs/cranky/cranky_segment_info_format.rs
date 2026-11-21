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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankySegmentInfoFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::SegmentInfoFormat;
use rucene::error::Result;
use rucene::index::SegmentInfo;
use rucene::store::{Directory, IOContext};

use super::maybe_fail;
use crate::codecs::{CodecFormat, SegmentInfoKind};

/// Port of `CrankySegmentInfoFormat`: `write` fails one time in 100.
pub struct CrankySegmentInfoFormat {
    delegate: CodecFormat<SegmentInfoKind>,
    random: Arc<dyn Random>,
}

impl CrankySegmentInfoFormat {
    pub(crate) fn new(delegate: CodecFormat<SegmentInfoKind>, random: Arc<dyn Random>) -> Self {
        CrankySegmentInfoFormat { delegate, random }
    }
}

impl fmt::Debug for CrankySegmentInfoFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CrankySegmentInfoFormat")
    }
}

impl SegmentInfoFormat for CrankySegmentInfoFormat {
    /// Java's `SegmentInfoFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.delegate.get().name()
    }

    fn read(
        &self,
        directory: Arc<dyn Directory>,
        segment_name: &str,
        segment_id: &[u8],
        context: &dyn IOContext,
    ) -> Result<SegmentInfo> {
        self.delegate
            .get()
            .read(directory, segment_name, segment_id, context)
    }

    fn write(
        &self,
        directory: &dyn Directory,
        info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from SegmentInfoFormat.write()",
        )?;
        self.delegate.get().write(directory, info, context)
    }
}
