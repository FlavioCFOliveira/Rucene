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

//! Port of `org.apache.lucene.tests.codecs.asserting.AssertingDocValuesFormat`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::thread::Thread;
use rucene::codecs::{
    DocValuesConsumer, DocValuesFormat, DocValuesProducer, SegmentReadState, SegmentWriteState,
};
use rucene::error::{LuceneError, Result};
use rucene::index::{
    BinaryDocValues, DocValuesSkipIndexType, DocValuesSkipper, DocValuesType, NumericDocValues,
    SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
};
use rucene::index::{FieldInfo, FieldInfos};
use rucene::search::NO_MORE_DOCS;
use rucene::util::{BytesRef, FixedBitSet, LongBitSet};

use super::AssertingCodec;
use crate::index::asserting_leaf_reader::{
    assert_valid, npe, AssertingBinaryDocValues, AssertingDocValuesSkipper,
    AssertingNumericDocValues, AssertingSortedDocValues, AssertingSortedNumericDocValues,
    AssertingSortedSetDocValues,
};
use crate::util::TestUtil;

/// The helpful `NullPointerException` of the first call on a `null`
/// `values` local in an `add*Field` method of `AssertingDocValuesConsumer`
/// (`AssertingDocValuesFormat.java:79-176`); `method` is the called
/// `org.apache.lucene.index` method.
fn null_values(method: &str) -> LuceneError {
    LuceneError::null_pointer_with_message(format!(
        "Cannot invoke \"org.apache.lucene.index.{method}\" because \"values\" is null"
    ))
}

/// Just like the default doc-values format but with additional asserts.
///
/// Port of `AssertingDocValuesFormat` (named `Asserting`, over
/// `TestUtil.getDefaultDocValuesFormat()`).
pub struct AssertingDocValuesFormat {
    r#in: Arc<dyn DocValuesFormat>,
}

impl AssertingDocValuesFormat {
    /// `new AssertingDocValuesFormat()`.
    pub fn new() -> Self {
        AssertingDocValuesFormat {
            r#in: TestUtil::get_default_doc_values_format(),
        }
    }
}

impl Default for AssertingDocValuesFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AssertingDocValuesFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DocValuesFormat(name=Asserting)")
    }
}

impl DocValuesFormat for AssertingDocValuesFormat {
    fn name(&self) -> &str {
        "Asserting"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn DocValuesConsumer + 'a>> {
        let consumer = self.r#in.fields_consumer(state)?;
        Ok(Box::new(AssertingDocValuesConsumer {
            r#in: consumer,
            max_doc: state.segment_info.max_doc()?,
        }))
    }

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn DocValuesProducer>> {
        java_assert!(state.field_infos.has_doc_values());
        let producer = self.r#in.fields_producer(state)?;
        Ok(Box::new(AssertingDocValuesProducer::new(
            producer,
            state.field_infos.clone(),
            state.segment_info.max_doc()?,
            false,
        )))
    }
}

/// Port of `AssertingDocValuesFormat.AssertingDocValuesConsumer`: walks every
/// field's values before handing them to the delegate.
struct AssertingDocValuesConsumer<'a> {
    r#in: Box<dyn DocValuesConsumer + 'a>,
    max_doc: i32,
}

impl fmt::Debug for AssertingDocValuesConsumer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertingDocValuesConsumer({:?})", self.r#in)
    }
}

impl DocValuesConsumer for AssertingDocValuesConsumer<'_> {
    fn add_numeric_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        let mut values = values_producer
            .get_numeric(field)?
            .ok_or_else(|| null_values("NumericDocValues.nextDoc()"))?;
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
        self.r#in.add_numeric_field(field, values_producer)
    }

    fn add_binary_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        let mut values = values_producer
            .get_binary(field)?
            .ok_or_else(|| null_values("BinaryDocValues.nextDoc()"))?;
        let mut last_doc_id = -1;
        loop {
            let doc_id = values.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            java_assert!(doc_id >= 0 && doc_id < self.max_doc);
            java_assert!(doc_id > last_doc_id);
            last_doc_id = doc_id;
            let value = values.binary_value()?;
            assert_valid(&value)?;
        }
        self.r#in.add_binary_field(field, values_producer)
    }

    fn add_sorted_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        let mut values = values_producer
            .get_sorted(field)?
            .ok_or_else(|| null_values("SortedDocValues.getValueCount()"))?;

        let value_count = values.get_value_count()?;
        java_assert!(value_count <= self.max_doc);
        let mut last_value: Option<BytesRef> = None;
        for ord in 0..value_count {
            let b = values.lookup_ord(ord)?;
            assert_valid(b)?;
            if ord > 0 {
                java_assert!(last_value.as_ref().is_some_and(|last| b > last));
            }
            last_value = Some(BytesRef::deep_copy_of(b));
        }

        let mut seen_ords = FixedBitSet::new(value_count.max(0) as usize);

        let mut last_doc_id = -1;
        loop {
            let doc_id = values.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            java_assert!(doc_id >= 0 && doc_id < self.max_doc);
            java_assert!(doc_id > last_doc_id);
            last_doc_id = doc_id;
            let ord = values.ord_value()?;
            java_assert!(ord >= 0 && ord < value_count);
            seen_ords.set(ord as usize);
        }

        java_assert!(seen_ords.cardinality() as i64 == i64::from(value_count));
        self.r#in.add_sorted_field(field, values_producer)
    }

    fn add_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        let mut values = values_producer
            .get_sorted_numeric(field)?
            .ok_or_else(|| null_values("SortedNumericDocValues.nextDoc()"))?;

        let mut last_doc_id = -1;
        loop {
            let doc_id = values.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            java_assert!(values.doc_id() > last_doc_id);
            last_doc_id = values.doc_id();
            let count = values.doc_value_count()?;
            java_assert!(count > 0);
            let mut previous = i64::MIN;
            for _ in 0..count {
                let next_value = values.next_value()?;
                java_assert!(next_value >= previous);
                previous = next_value;
            }
        }
        self.r#in.add_sorted_numeric_field(field, values_producer)
    }

    fn add_sorted_set_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        let mut values = values_producer
            .get_sorted_set(field)?
            .ok_or_else(|| null_values("SortedSetDocValues.getValueCount()"))?;

        let value_count = values.get_value_count()?;
        let mut last_value: Option<BytesRef> = None;
        for i in 0..value_count {
            let b = values.lookup_ord(i)?;
            assert_valid(b)?;
            if i > 0 {
                java_assert!(last_value.as_ref().is_some_and(|last| b > last));
            }
            last_value = Some(BytesRef::deep_copy_of(b));
        }

        let mut seen_ords = LongBitSet::new(value_count)?;
        loop {
            let doc_id = values.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }

            let mut last_ord = -1i64;
            let mut i = 0;
            while i < values.doc_value_count()? {
                let ord = values.next_ord()?;
                java_assert!(
                    ord >= 0 && ord < value_count,
                    "ord={} is not in bounds 0 ..{}",
                    ord,
                    value_count - 1
                );
                java_assert!(ord > last_ord, "ord={},lastOrd={}", ord, last_ord);
                seen_ords.set(ord);
                last_ord = ord;
                i += 1;
            }
        }

        java_assert!(seen_ords.cardinality() == value_count);
        self.r#in.add_sorted_set_field(field, values_producer)
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}

/// Port of `AssertingDocValuesFormat.AssertingDocValuesProducer`.
struct AssertingDocValuesProducer {
    r#in: Box<dyn DocValuesProducer>,
    field_infos: FieldInfos,
    max_doc: i32,
    merging: bool,
    creation_thread: Thread,
}

impl AssertingDocValuesProducer {
    fn new(
        r#in: Box<dyn DocValuesProducer>,
        field_infos: FieldInfos,
        max_doc: i32,
        merging: bool,
    ) -> Self {
        // Java's `assert toString() != null` cannot fail in Rust.
        AssertingDocValuesProducer {
            r#in,
            field_infos,
            max_doc,
            merging,
            creation_thread: Thread::current(),
        }
    }

    /// The checks every getter opens with: the field is this segment's, and,
    /// for a merge instance, the thread is the one that created it.
    fn check_field(&self, field: &FieldInfo, check_thread: bool) -> Result<()> {
        java_assert!(
            self.field_infos
                .field_info(&field.name)
                .ok_or_else(|| LuceneError::null_pointer_with_message(
                    "Cannot read field \"number\" because the return value of \
                     \"org.apache.lucene.index.FieldInfos.fieldInfo(String)\" is null"
                ))?
                .number
                == field.number
        );
        if check_thread && self.merging {
            AssertingCodec::assert_thread("DocValuesProducer", &self.creation_thread)?;
        }
        Ok(())
    }
}

impl fmt::Debug for AssertingDocValuesProducer {
    /// `getClass().getSimpleName() + "(" + in + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertingDocValuesProducer({:?})", self.r#in)
    }
}

impl DocValuesProducer for AssertingDocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.check_field(field, true)?;
        java_assert!(field.doc_values_type == DocValuesType::NUMERIC);
        let values = self.r#in.get_numeric(field)?;
        java_assert!(values.is_some());
        let values = values.ok_or_else(npe)?;
        Ok(Some(Box::new(AssertingNumericDocValues::new(
            values,
            self.max_doc,
        )?)))
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.check_field(field, true)?;
        java_assert!(field.doc_values_type == DocValuesType::BINARY);
        let values = self.r#in.get_binary(field)?;
        java_assert!(values.is_some());
        let values = values.ok_or_else(npe)?;
        Ok(Some(Box::new(AssertingBinaryDocValues::new(
            values,
            self.max_doc,
        )?)))
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.check_field(field, true)?;
        java_assert!(field.doc_values_type == DocValuesType::SORTED);
        let values = self.r#in.get_sorted(field)?;
        java_assert!(values.is_some());
        // Assertions disabled: the constructor's `in.getValueCount()`
        // (`AssertingLeafReader.java:988`) dereferences the `null`.
        let values = values.ok_or_else(|| {
            LuceneError::null_pointer_with_message(
                "Cannot invoke \"org.apache.lucene.index.SortedDocValues.getValueCount()\" \
                 because \"in\" is null",
            )
        })?;
        Ok(Some(Box::new(AssertingSortedDocValues::new(
            values,
            self.max_doc,
        )?)))
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.check_field(field, true)?;
        java_assert!(field.doc_values_type == DocValuesType::SORTED_NUMERIC);
        let values = self.r#in.get_sorted_numeric(field)?;
        java_assert!(values.is_some());
        let values = values.ok_or_else(npe)?;
        Ok(Some(AssertingSortedNumericDocValues::create(
            values,
            self.max_doc,
        )?))
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.check_field(field, true)?;
        java_assert!(field.doc_values_type == DocValuesType::SORTED_SET);
        let values = self.r#in.get_sorted_set(field)?;
        java_assert!(values.is_some());
        let values = values.ok_or_else(npe)?;
        Ok(Some(AssertingSortedSetDocValues::create(
            values,
            self.max_doc,
        )?))
    }

    /// Java's `getSkipper` makes no thread check.
    fn get_skipper(&self, field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        self.check_field(field, false)?;
        java_assert!(field.doc_values_skip_index_type() != DocValuesSkipIndexType::NONE);
        let skipper = self.r#in.get_skipper(field)?;
        java_assert!(skipper.is_some());
        let skipper = skipper.ok_or_else(npe)?;
        Ok(Some(Box::new(AssertingDocValuesSkipper::new(skipper)?)))
    }

    fn check_integrity(&self) -> Result<()> {
        self.r#in.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(AssertingDocValuesProducer::new(
            self.r#in.get_merge_instance()?,
            self.field_infos.clone(),
            self.max_doc,
            true,
        )))
    }

    fn close(&self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::java_exceptions::java_throwable;

    /// A producer whose every field is `null`.
    #[derive(Debug)]
    struct NullProducer;

    impl DocValuesProducer for NullProducer {
        fn get_numeric(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }
        fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
            Ok(None)
        }
        fn get_sorted(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
            Ok(None)
        }
        fn get_sorted_numeric(
            &self,
            _field: &FieldInfo,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            Ok(None)
        }
        fn get_sorted_set(
            &self,
            _field: &FieldInfo,
        ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            Ok(None)
        }
        fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            Ok(None)
        }
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
            Ok(Box::new(NullProducer))
        }
        fn close(&self) -> Result<()> {
            Ok(())
        }
    }

    /// A consumer the asserting consumer never reaches in these tests.
    #[derive(Debug)]
    struct Unreached;

    impl DocValuesConsumer for Unreached {
        fn add_numeric_field(&mut self, _: &FieldInfo, _: &dyn DocValuesProducer) -> Result<()> {
            unreachable!()
        }
        fn add_binary_field(&mut self, _: &FieldInfo, _: &dyn DocValuesProducer) -> Result<()> {
            unreachable!()
        }
        fn add_sorted_field(&mut self, _: &FieldInfo, _: &dyn DocValuesProducer) -> Result<()> {
            unreachable!()
        }
        fn add_sorted_numeric_field(
            &mut self,
            _: &FieldInfo,
            _: &dyn DocValuesProducer,
        ) -> Result<()> {
            unreachable!()
        }
        fn add_sorted_set_field(&mut self, _: &FieldInfo, _: &dyn DocValuesProducer) -> Result<()> {
            unreachable!()
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// A `null` `values` local NPEs at its first call with the JDK's helpful
    /// message, as `AssertingDocValuesConsumer.add*Field` does
    /// (`AssertingDocValuesFormat.java:79-176`; local names from the 10.5.0
    /// jar's `LocalVariableTable`).
    #[test]
    fn null_values_are_helpful_null_pointer_exceptions() {
        let mut consumer = AssertingDocValuesConsumer {
            r#in: Box::new(Unreached),
            max_doc: 1,
        };
        let field = FieldInfo::new("f", 0);
        let message = |r: Result<()>| {
            let t = java_throwable(&r.unwrap_err());
            assert_eq!(t.class_name(), "java.lang.NullPointerException");
            t.message().unwrap().to_string()
        };
        let expected = |method: &str| {
            format!("Cannot invoke \"org.apache.lucene.index.{method}\" because \"values\" is null")
        };
        assert_eq!(
            message(consumer.add_numeric_field(&field, &NullProducer)),
            expected("NumericDocValues.nextDoc()")
        );
        assert_eq!(
            message(consumer.add_binary_field(&field, &NullProducer)),
            expected("BinaryDocValues.nextDoc()")
        );
        assert_eq!(
            message(consumer.add_sorted_field(&field, &NullProducer)),
            expected("SortedDocValues.getValueCount()")
        );
        assert_eq!(
            message(consumer.add_sorted_numeric_field(&field, &NullProducer)),
            expected("SortedNumericDocValues.nextDoc()")
        );
        assert_eq!(
            message(consumer.add_sorted_set_field(&field, &NullProducer)),
            expected("SortedSetDocValues.getValueCount()")
        );
    }
}
