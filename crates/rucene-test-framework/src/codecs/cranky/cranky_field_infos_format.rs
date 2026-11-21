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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankyFieldInfosFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::FieldInfosFormat;
use rucene::error::Result;
use rucene::index::{FieldInfos, SegmentInfo};
use rucene::store::{Directory, IOContext};

use super::maybe_fail;
use crate::codecs::{CodecFormat, FieldInfosKind};

/// Port of `CrankyFieldInfosFormat`: `write` fails one time in 100.
pub struct CrankyFieldInfosFormat {
    delegate: CodecFormat<FieldInfosKind>,
    random: Arc<dyn Random>,
}

impl CrankyFieldInfosFormat {
    pub(crate) fn new(delegate: CodecFormat<FieldInfosKind>, random: Arc<dyn Random>) -> Self {
        CrankyFieldInfosFormat { delegate, random }
    }
}

impl fmt::Debug for CrankyFieldInfosFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CrankyFieldInfosFormat")
    }
}

impl FieldInfosFormat for CrankyFieldInfosFormat {
    /// Java's `FieldInfosFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.delegate.get().name()
    }

    fn read(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        context: &dyn IOContext,
    ) -> Result<FieldInfos> {
        self.delegate
            .get()
            .read(directory, segment_info, segment_suffix, context)
    }

    fn write(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from FieldInfosFormat.getFieldInfosWriter()",
        )?;
        self.delegate
            .get()
            .write(directory, segment_info, segment_suffix, infos, context)
    }
}
