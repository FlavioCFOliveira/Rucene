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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankyNormsFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::{
    NormsConsumer, NormsFormat, NormsProducer, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene::index::FieldInfo;

use super::maybe_fail;
use crate::codecs::{CodecFormat, NormsKind};

/// Port of `CrankyNormsFormat`: `normsConsumer` and every consumer call fail
/// one time in 100.
///
/// The consumer does not override `merge`, as Java's does not:
/// `NormsConsumer`'s default merge adds every field through the cranky
/// `addNormsField`.
pub struct CrankyNormsFormat {
    delegate: CodecFormat<NormsKind>,
    random: Arc<dyn Random>,
}

impl CrankyNormsFormat {
    pub(crate) fn new(delegate: CodecFormat<NormsKind>, random: Arc<dyn Random>) -> Self {
        CrankyNormsFormat { delegate, random }
    }
}

impl fmt::Debug for CrankyNormsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CrankyNormsFormat")
    }
}

impl NormsFormat for CrankyNormsFormat {
    /// Java's `NormsFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.delegate.get().name()
    }

    fn norms_consumer(&self, state: &SegmentWriteState) -> Result<Box<dyn NormsConsumer>> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from NormsFormat.normsConsumer()",
        )?;
        Ok(Box::new(CrankyNormsConsumer {
            delegate: self.delegate.get().norms_consumer(state)?,
            random: Arc::clone(&self.random),
        }))
    }

    fn norms_producer(&self, state: &SegmentReadState) -> Result<Box<dyn NormsProducer>> {
        self.delegate.get().norms_producer(state)
    }
}

/// Port of `CrankyNormsFormat.CrankyNormsConsumer`.
struct CrankyNormsConsumer {
    delegate: Box<dyn NormsConsumer>,
    random: Arc<dyn Random>,
}

impl fmt::Debug for CrankyNormsConsumer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CrankyNormsConsumer({:?})", self.delegate)
    }
}

impl NormsConsumer for CrankyNormsConsumer {
    fn add_norms_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn NormsProducer,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from NormsConsumer.addNormsField()",
        )?;
        self.delegate.add_norms_field(field, values_producer)
    }

    fn close(&mut self) -> Result<()> {
        self.delegate.close()?;
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from NormsConsumer.close()",
        )
    }
}
