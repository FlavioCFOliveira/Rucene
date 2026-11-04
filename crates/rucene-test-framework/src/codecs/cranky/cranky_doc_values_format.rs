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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankyDocValuesFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::{
    DocValuesConsumer, DocValuesFormat, DocValuesProducer, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene::index::FieldInfo;

use super::maybe_fail;
use crate::codecs::{CodecFormat, DocValuesKind};

/// Port of `CrankyDocValuesFormat`: impersonates the delegate (its name,
/// its files); `fieldsConsumer` and every consumer call fail one time in 100.
///
/// The consumer does not override `merge`, as Java's does not:
/// `DocValuesConsumer`'s default merge adds every field through the cranky
/// `add…Field` calls.
pub struct CrankyDocValuesFormat {
    delegate: CodecFormat<DocValuesKind>,
    random: Arc<dyn Random>,
}

impl CrankyDocValuesFormat {
    pub(crate) fn new(delegate: CodecFormat<DocValuesKind>, random: Arc<dyn Random>) -> Self {
        CrankyDocValuesFormat { delegate, random }
    }
}

impl fmt::Debug for CrankyDocValuesFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DocValuesFormat(name={})", self.name())
    }
}

impl DocValuesFormat for CrankyDocValuesFormat {
    fn name(&self) -> &str {
        self.delegate.get().name()
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn DocValuesConsumer + 'a>> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from DocValuesFormat.fieldsConsumer()",
        )?;
        Ok(Box::new(CrankyDocValuesConsumer {
            delegate: self.delegate.get().fields_consumer(state)?,
            random: Arc::clone(&self.random),
        }))
    }

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn DocValuesProducer>> {
        self.delegate.get().fields_producer(state)
    }
}

/// Port of `CrankyDocValuesFormat.CrankyDocValuesConsumer`.
struct CrankyDocValuesConsumer<'a> {
    delegate: Box<dyn DocValuesConsumer + 'a>,
    random: Arc<dyn Random>,
}

impl fmt::Debug for CrankyDocValuesConsumer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CrankyDocValuesConsumer({:?})", self.delegate)
    }
}

impl DocValuesConsumer for CrankyDocValuesConsumer<'_> {
    fn add_numeric_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from DocValuesConsumer.addNumericField()",
        )?;
        self.delegate.add_numeric_field(field, values_producer)
    }

    fn add_binary_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from DocValuesConsumer.addBinaryField()",
        )?;
        self.delegate.add_binary_field(field, values_producer)
    }

    fn add_sorted_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from DocValuesConsumer.addSortedField()",
        )?;
        self.delegate.add_sorted_field(field, values_producer)
    }

    fn add_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from DocValuesConsumer.addSortedNumericField()",
        )?;
        self.delegate
            .add_sorted_numeric_field(field, values_producer)
    }

    fn add_sorted_set_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from DocValuesConsumer.addSortedSetField()",
        )?;
        self.delegate.add_sorted_set_field(field, values_producer)
    }

    fn close(&mut self) -> Result<()> {
        self.delegate.close()?;
        maybe_fail(
            &*self.random,
            100,
            "Fake IOException from DocValuesConsumer.close()",
        )
    }
}
