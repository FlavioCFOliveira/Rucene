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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankyLiveDocsFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::LiveDocsFormat;
use rucene::error::Result;
use rucene::index::SegmentCommitInfo;
use rucene::store::{Directory, IOContext};
use rucene::util::Bits;

use super::maybe_fail;
use crate::codecs::{CodecFormat, LiveDocsKind};

/// Port of `CrankyLiveDocsFormat`: `writeLiveDocs` fails one time in 100.
pub struct CrankyLiveDocsFormat {
    delegate: CodecFormat<LiveDocsKind>,
    random: Arc<dyn Random>,
}

impl CrankyLiveDocsFormat {
    pub(crate) fn new(delegate: CodecFormat<LiveDocsKind>, random: Arc<dyn Random>) -> Self {
        CrankyLiveDocsFormat { delegate, random }
    }
}

impl fmt::Debug for CrankyLiveDocsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CrankyLiveDocsFormat")
    }
}

impl LiveDocsFormat for CrankyLiveDocsFormat {
    /// Java's `LiveDocsFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.delegate.get().name()
    }

    fn read_live_docs(
        &self,
        dir: &dyn Directory,
        info: &SegmentCommitInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn Bits>> {
        self.delegate.get().read_live_docs(dir, info, context)
    }

    fn write_live_docs(
        &self,
        bits: &dyn Bits,
        dir: &dyn Directory,
        info: &SegmentCommitInfo,
        new_del_count: i32,
        context: &dyn IOContext,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from LiveDocsFormat.writeLiveDocs()",
        )?;
        self.delegate
            .get()
            .write_live_docs(bits, dir, info, new_del_count, context)
    }

    fn files(&self, info: &SegmentCommitInfo, files: &mut Vec<String>) -> Result<()> {
        // TODO: is this called only from write? if so we should throw exception!
        self.delegate.get().files(info, files)
    }
}
