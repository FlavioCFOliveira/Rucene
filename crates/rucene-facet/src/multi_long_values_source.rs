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

//! Port of `MultiLongValuesSource.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{DocValues, LeafReaderContext, SortedNumericDocValues};
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::long_values::LongValues;
use rucene::search::long_values_source::LongValuesSource;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::util::java_collections::java_string_hash_code;

use crate::label_and_value::objects_hash;
use crate::multi_double_values::MultiDoubleValues;
use crate::multi_double_values_source::MultiDoubleValuesSource;
use crate::multi_long_values::MultiLongValues;

/// Base class for producing `MultiLongValues`.
///
/// Equivalent to `org.apache.lucene.facet.MultiLongValuesSource`; its
/// abstract `equals`/`hashCode`/`toString` are
/// [`source_eq`](Self::source_eq), [`source_hash`](Self::source_hash) and
/// [`Display`](fmt::Display).
pub trait MultiLongValuesSource:
    SegmentCacheable + fmt::Display + fmt::Debug + Send + Sync
{
    /// Returns a `MultiLongValues` instance for the passed-in
    /// `LeafReaderContext`.
    ///
    /// Equivalent to `getValues(LeafReaderContext)`.
    fn get_values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn MultiLongValues>>;

    /// Equivalent to `hashCode()`.
    fn source_hash(&self) -> i32;

    /// Equivalent to `equals(Object)`.
    fn source_eq(&self, other: &dyn MultiLongValuesSource) -> bool;

    /// This source as [`Any`], for `equals` and `instanceof`.
    fn as_any(&self) -> &dyn Any;

    /// The `LongValuesSource` a `SingleValuedAsMultiValued` wraps.
    fn single_valued(&self) -> Option<Arc<dyn LongValuesSource>> {
        None
    }
}

impl dyn MultiLongValuesSource {
    /// Creates a `MultiLongValuesSource` that wraps a long-valued field.
    ///
    /// Equivalent to `fromLongField(String)`.
    pub fn from_long_field(field: &str) -> Arc<dyn MultiLongValuesSource> {
        Arc::new(FieldMultiValueSource {
            field: field.to_string(),
        })
    }

    /// Creates a `MultiLongValuesSource` that wraps an int-valued field.
    ///
    /// Equivalent to `fromIntField(String)`.
    pub fn from_int_field(field: &str) -> Arc<dyn MultiLongValuesSource> {
        Self::from_long_field(field)
    }

    /// Creates a `MultiLongValuesSource` that wraps a single-valued
    /// `LongValuesSource`.
    ///
    /// Equivalent to `fromSingleValued(LongValuesSource)`.
    pub fn from_single_valued(
        single_valued: Arc<dyn LongValuesSource>,
    ) -> Arc<dyn MultiLongValuesSource> {
        Arc::new(SingleValuedAsMultiValued {
            input: single_valued,
        })
    }

    /// Returns a single-valued view of the `MultiLongValuesSource` if it was
    /// previously wrapped with `from_single_valued`, or `None`.
    ///
    /// Equivalent to `unwrapSingleton(MultiLongValuesSource)`.
    pub fn unwrap_singleton(
        input: &dyn MultiLongValuesSource,
    ) -> Option<Arc<dyn LongValuesSource>> {
        input.single_valued()
    }
}

/// Convert to a `MultiDoubleValuesSource` by casting long values to
/// doubles.
///
/// Equivalent to `toMultiDoubleValuesSource()`.
pub fn to_multi_double_values_source(
    source: Arc<dyn MultiLongValuesSource>,
) -> Arc<dyn MultiDoubleValuesSource> {
    Arc::new(DoubleLongValuesSources { input: source })
}

/// Equivalent to `org.apache.lucene.facet.MultiLongValuesSource.FieldMultiValueSource`.
#[derive(Debug)]
pub(crate) struct FieldMultiValueSource {
    field: String,
}

/// The anonymous `MultiLongValues` of `FieldMultiValueSource.getValues`.
struct FieldMultiLongValues {
    doc_values: Box<dyn SortedNumericDocValues>,
}

impl MultiLongValues for FieldMultiLongValues {
    fn get_value_count(&self) -> Result<i64> {
        Ok(self.doc_values.doc_value_count()? as i64)
    }

    fn next_value(&mut self) -> Result<i64> {
        self.doc_values.next_value()
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.doc_values.advance_exact(doc)
    }
}

impl SegmentCacheable for FieldMultiValueSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        DocValues::is_cacheable(ctx, &[&self.field])
    }
}

impl fmt::Display for FieldMultiValueSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "multi-long({})", self.field)
    }
}

impl MultiLongValuesSource for FieldMultiValueSource {
    fn get_values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn MultiLongValues>> {
        let doc_values = DocValues::get_sorted_numeric(&*ctx.leaf_reader(), &self.field)?;
        Ok(Box::new(FieldMultiLongValues { doc_values }))
    }

    fn source_hash(&self) -> i32 {
        objects_hash(&[java_string_hash_code(&self.field)])
    }

    fn source_eq(&self, other: &dyn MultiLongValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<FieldMultiValueSource>()
            .is_some_and(|that| self.field == that.field)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Equivalent to `org.apache.lucene.facet.MultiLongValuesSource.SingleValuedAsMultiValued`.
#[derive(Debug)]
pub(crate) struct SingleValuedAsMultiValued {
    input: Arc<dyn LongValuesSource>,
}

/// The anonymous `MultiLongValues` of `SingleValuedAsMultiValued.getValues`.
struct SingleLongAsMulti {
    single_valued: Box<dyn LongValues>,
}

impl MultiLongValues for SingleLongAsMulti {
    fn get_value_count(&self) -> Result<i64> {
        Ok(1)
    }

    fn next_value(&mut self) -> Result<i64> {
        self.single_valued.long_value()
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.single_valued.advance_exact(doc)
    }
}

impl SegmentCacheable for SingleValuedAsMultiValued {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.input.is_cacheable(ctx)
    }
}

impl fmt::Display for SingleValuedAsMultiValued {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "multi-long({})", self.input.to_source_string())
    }
}

impl MultiLongValuesSource for SingleValuedAsMultiValued {
    fn get_values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn MultiLongValues>> {
        let single_valued = Arc::clone(&self.input).get_values(ctx, None)?;
        Ok(Box::new(SingleLongAsMulti { single_valued }))
    }

    fn source_hash(&self) -> i32 {
        objects_hash(&[self.input.source_hash()])
    }

    fn source_eq(&self, other: &dyn MultiLongValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<SingleValuedAsMultiValued>()
            .is_some_and(|that| self.input.source_eq(&*that.input))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn single_valued(&self) -> Option<Arc<dyn LongValuesSource>> {
        Some(Arc::clone(&self.input))
    }
}

/// Equivalent to `org.apache.lucene.facet.MultiLongValuesSource.DoubleLongValuesSources`.
#[derive(Debug)]
pub(crate) struct DoubleLongValuesSources {
    input: Arc<dyn MultiLongValuesSource>,
}

/// The anonymous `MultiDoubleValues` of `DoubleLongValuesSources.getValues`.
struct LongAsDouble {
    long_values: Box<dyn MultiLongValues>,
}

impl MultiDoubleValues for LongAsDouble {
    fn get_value_count(&self) -> Result<i64> {
        self.long_values.get_value_count()
    }

    fn next_value(&mut self) -> Result<f64> {
        Ok(self.long_values.next_value()? as f64)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.long_values.advance_exact(doc)
    }
}

impl SegmentCacheable for DoubleLongValuesSources {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.input.is_cacheable(ctx)
    }
}

impl fmt::Display for DoubleLongValuesSources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "multi-long({})", self.input)
    }
}

impl MultiDoubleValuesSource for DoubleLongValuesSources {
    fn get_values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn MultiDoubleValues>> {
        let long_values = self.input.get_values(ctx)?;
        Ok(Box::new(LongAsDouble { long_values }))
    }

    fn source_hash(&self) -> i32 {
        objects_hash(&[self.input.source_hash()])
    }

    fn source_eq(&self, other: &dyn MultiDoubleValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<DoubleLongValuesSources>()
            .is_some_and(|that| self.input.source_eq(&*that.input))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[allow(dead_code)]
fn _double_values_source(_: &dyn DoubleValuesSource) {}
