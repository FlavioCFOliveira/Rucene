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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankyPostingsFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::{
    FieldsConsumer, FieldsProducer, NormsProducer, PostingsFormat, SegmentReadState,
    SegmentWriteState,
};
use rucene::error::Result;
use rucene::index::Fields;

use super::maybe_fail;
use crate::codecs::{CodecFormat, PostingsKind};

/// Port of `CrankyPostingsFormat`: impersonates the delegate (its name, its
/// files); `fieldsConsumer`, `write` and `close` fail one time in 100.
///
/// The consumer does not override `merge`, as Java's does not:
/// `FieldsConsumer`'s default merge writes through the cranky `write`.
pub struct CrankyPostingsFormat {
    delegate: CodecFormat<PostingsKind>,
    random: Arc<dyn Random>,
}

impl CrankyPostingsFormat {
    pub(crate) fn new(delegate: CodecFormat<PostingsKind>, random: Arc<dyn Random>) -> Self {
        CrankyPostingsFormat { delegate, random }
    }
}

impl fmt::Debug for CrankyPostingsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PostingsFormat(name={})", self.name())
    }
}

impl PostingsFormat for CrankyPostingsFormat {
    fn name(&self) -> &str {
        self.delegate.get().name()
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from PostingsFormat.fieldsConsumer()",
        )?;
        Ok(Box::new(CrankyFieldsConsumer {
            delegate: self.delegate.get().fields_consumer(state)?,
            random: Arc::clone(&self.random),
        }))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        self.delegate.get().fields_producer(state)
    }
}

/// Port of `CrankyPostingsFormat.CrankyFieldsConsumer`.
struct CrankyFieldsConsumer<'a> {
    delegate: Box<dyn FieldsConsumer + 'a>,
    random: Arc<dyn Random>,
}

impl FieldsConsumer for CrankyFieldsConsumer<'_> {
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from FieldsConsumer.write()",
        )?;
        self.delegate.write(fields, norms)
    }

    fn close(&mut self) -> Result<()> {
        self.delegate.close()?;
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from FieldsConsumer.close()",
        )
    }
}
