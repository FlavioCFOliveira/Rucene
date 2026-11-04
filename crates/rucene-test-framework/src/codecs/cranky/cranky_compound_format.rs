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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankyCompoundFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::{CompoundDirectory, CompoundFormat};
use rucene::error::Result;
use rucene::index::SegmentInfo;
use rucene::store::{Directory, IOContext};

use super::maybe_fail;
use crate::codecs::{CodecFormat, CompoundKind};

/// Port of `CrankyCompoundFormat`: `write` fails one time in 100.
pub struct CrankyCompoundFormat {
    delegate: CodecFormat<CompoundKind>,
    random: Arc<dyn Random>,
}

impl CrankyCompoundFormat {
    pub(crate) fn new(delegate: CodecFormat<CompoundKind>, random: Arc<dyn Random>) -> Self {
        CrankyCompoundFormat { delegate, random }
    }
}

impl fmt::Debug for CrankyCompoundFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CrankyCompoundFormat")
    }
}

impl CompoundFormat for CrankyCompoundFormat {
    /// Java's `CompoundFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.delegate.get().name()
    }

    fn get_compound_reader(
        &self,
        dir: &dyn Directory,
        segment_info: &SegmentInfo,
    ) -> Result<Box<dyn CompoundDirectory>> {
        self.delegate.get().get_compound_reader(dir, segment_info)
    }

    fn write(
        &self,
        dir: &dyn Directory,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from CompoundFormat.write()",
        )?;
        self.delegate.get().write(dir, segment_info, context)
    }
}
