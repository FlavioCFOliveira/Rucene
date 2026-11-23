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

//! Port of `MultiDoubleValuesSource.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, LazyLock};

use rucene::error::Result;
use rucene::index::{DocValues, LeafReaderContext, SortedNumericDocValues};
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::util::java_collections::java_string_hash_code;
use rucene::util::NumericUtils;

use crate::label_and_value::objects_hash;
use crate::multi_double_values::MultiDoubleValues;
use crate::multi_long_values::MultiLongValues;
use crate::multi_long_values_source::MultiLongValuesSource;

/// Java's `LongToDoubleFunction`, compared by identity as Java compares
/// lambdas.
pub type LongToDoubleFunction = Arc<dyn Fn(i64) -> f64 + Send + Sync>;

/// `decoder.hashCode()`: Java's identity hash of the lambda object. Distinct
/// decoder objects (`Long::valueOf` and `v -> -1 * v`) have distinct identity
/// hashes in Java, which `Objects.hash(field, decoder)` relies on, so the
/// identity of the shared `Arc` (its address, stable while the object lives)
/// stands in for it rather than a constant.
fn decoder_identity_hash_code(decoder: &LongToDoubleFunction) -> i32 {
    let addr = Arc::as_ptr(decoder) as *const () as usize as u64;
    (addr ^ (addr >> 32)) as i32
}

/// Base class for producing `MultiDoubleValues`.
///
/// Equivalent to `org.apache.lucene.facet.MultiDoubleValuesSource`.
pub trait MultiDoubleValuesSource:
    SegmentCacheable + fmt::Display + fmt::Debug + Send + Sync
{
    /// Returns a `MultiDoubleValues` instance for the passed-in
    /// `LeafReaderContext`.
    ///
    /// Equivalent to `getValues(LeafReaderContext)`.
    fn get_values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn MultiDoubleValues>>;

    /// Equivalent to `hashCode()`.
    fn source_hash(&self) -> i32;

    /// Equivalent to `equals(Object)`.
    fn source_eq(&self, other: &dyn MultiDoubleValuesSource) -> bool;

    /// This source as [`Any`].
    fn as_any(&self) -> &dyn Any;

    /// The `DoubleValuesSource` a `SingleValuedAsMultiValued` wraps.
    fn single_valued(&self) -> Option<Arc<dyn DoubleValuesSource>> {
        None
    }
}

// The method references of the `from*Field` factories are non-capturing, so
// the JVM hands out one instance per call site; one static each keeps that
// identity.
static DOUBLE_DECODER: LazyLock<LongToDoubleFunction> =
    LazyLock::new(|| Arc::new(|v| f64::from_bits(v as u64)));
static FLOAT_DECODER: LazyLock<LongToDoubleFunction> =
    LazyLock::new(|| Arc::new(|v| f32::from_bits(v as i32 as u32) as f64));
static LONG_DECODER: LazyLock<LongToDoubleFunction> = LazyLock::new(|| Arc::new(|v| v as f64));

impl dyn MultiDoubleValuesSource {
    /// Creates a `MultiDoubleValuesSource` that wraps a generic numeric
    /// field.
    ///
    /// Equivalent to `fromField(String, LongToDoubleFunction)`.
    pub fn from_field(
        field: &str,
        decoder: LongToDoubleFunction,
    ) -> Arc<dyn MultiDoubleValuesSource> {
        Arc::new(FieldMultiValuedSource {
            field: field.to_string(),
            decoder,
        })
    }

    /// Creates a `MultiDoubleValuesSource` that wraps a double-valued field.
    ///
    /// Equivalent to `fromDoubleField(String)`.
    pub fn from_double_field(field: &str) -> Arc<dyn MultiDoubleValuesSource> {
        Self::from_field(field, Arc::clone(&DOUBLE_DECODER))
    }

    /// Creates a `MultiDoubleValuesSource` that wraps a float-valued field.
    ///
    /// Equivalent to `fromFloatField(String)`.
    pub fn from_float_field(field: &str) -> Arc<dyn MultiDoubleValuesSource> {
        Self::from_field(field, Arc::clone(&FLOAT_DECODER))
    }

    /// Creates a `MultiDoubleValuesSource` that wraps a long-valued field.
    ///
    /// Equivalent to `fromLongField(String)`.
    pub fn from_long_field(field: &str) -> Arc<dyn MultiDoubleValuesSource> {
        Self::from_field(field, Arc::clone(&LONG_DECODER))
    }

    /// Creates a `MultiDoubleValuesSource` that wraps an int-valued field.
    ///
    /// Equivalent to `fromIntField(String)`.
    pub fn from_int_field(field: &str) -> Arc<dyn MultiDoubleValuesSource> {
        Self::from_long_field(field)
    }

    /// Creates a `MultiDoubleValuesSource` that wraps a single-valued
    /// `DoubleValuesSource`.
    ///
    /// Equivalent to `fromSingleValued(DoubleValuesSource)`.
    pub fn from_single_valued(
        single_valued: Arc<dyn DoubleValuesSource>,
    ) -> Arc<dyn MultiDoubleValuesSource> {
        Arc::new(SingleValuedAsMultiValued {
            input: single_valued,
        })
    }

    /// Returns a single-valued view of the `MultiDoubleValuesSource` if it
    /// was previously wrapped with `from_single_valued`, or `None`.
    ///
    /// Equivalent to `unwrapSingleton(MultiDoubleValuesSource)`.
    pub fn unwrap_singleton(
        input: &dyn MultiDoubleValuesSource,
    ) -> Option<Arc<dyn DoubleValuesSource>> {
        input.single_valued()
    }
}

/// Convert to a `MultiLongValuesSource` by casting the double values to
/// longs.
///
/// Equivalent to `toMultiLongValuesSource()`.
pub fn to_multi_long_values_source(
    source: Arc<dyn MultiDoubleValuesSource>,
) -> Arc<dyn MultiLongValuesSource> {
    Arc::new(LongDoubleValuesSource { input: source })
}

/// Convert to a `MultiLongValuesSource` using
/// `NumericUtils.doubleToSortableLong`.
///
/// Equivalent to `toSortableMultiLongValuesSource()`.
pub fn to_sortable_multi_long_values_source(
    source: Arc<dyn MultiDoubleValuesSource>,
) -> Arc<dyn MultiLongValuesSource> {
    Arc::new(SortableMultiLongValuesSource { inner: source })
}

/// Equivalent to `org.apache.lucene.facet.MultiDoubleValuesSource.SortableMultiLongValuesSource`.
#[derive(Debug)]
pub(crate) struct SortableMultiLongValuesSource {
    inner: Arc<dyn MultiDoubleValuesSource>,
}

/// The anonymous `MultiLongValues` of `SortableMultiLongValuesSource.getValues`.
struct SortableLongs {
    double_values: Box<dyn MultiDoubleValues>,
}

impl MultiLongValues for SortableLongs {
    fn get_value_count(&self) -> Result<i64> {
        self.double_values.get_value_count()
    }

    fn next_value(&mut self) -> Result<i64> {
        Ok(NumericUtils::double_to_sortable_long(
            self.double_values.next_value()?,
        ))
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.double_values.advance_exact(doc)
    }
}

impl SegmentCacheable for SortableMultiLongValuesSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }
}

impl fmt::Display for SortableMultiLongValuesSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sortableMultiLong({})", self.inner)
    }
}

impl MultiLongValuesSource for SortableMultiLongValuesSource {
    fn get_values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn MultiLongValues>> {
        let double_values = self.inner.get_values(ctx)?;
        Ok(Box::new(SortableLongs { double_values }))
    }

    fn source_hash(&self) -> i32 {
        self.inner.source_hash()
    }

    fn source_eq(&self, other: &dyn MultiLongValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<SortableMultiLongValuesSource>()
            .is_some_and(|that| self.inner.source_eq(&*that.inner))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Equivalent to `org.apache.lucene.facet.MultiDoubleValuesSource.FieldMultiValuedSource`.
pub(crate) struct FieldMultiValuedSource {
    field: String,
    decoder: LongToDoubleFunction,
}

impl fmt::Debug for FieldMultiValuedSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldMultiValuedSource")
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

/// The anonymous `MultiDoubleValues` of `FieldMultiValuedSource.getValues`.
struct FieldDoubles {
    doc_values: Box<dyn SortedNumericDocValues>,
    decoder: LongToDoubleFunction,
}

impl MultiDoubleValues for FieldDoubles {
    fn get_value_count(&self) -> Result<i64> {
        Ok(self.doc_values.doc_value_count()? as i64)
    }

    fn next_value(&mut self) -> Result<f64> {
        Ok((self.decoder)(self.doc_values.next_value()?))
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.doc_values.advance_exact(doc)
    }
}

impl SegmentCacheable for FieldMultiValuedSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        DocValues::is_cacheable(ctx, &[&self.field])
    }
}

impl fmt::Display for FieldMultiValuedSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "multi-double({})", self.field)
    }
}

impl MultiDoubleValuesSource for FieldMultiValuedSource {
    fn get_values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn MultiDoubleValues>> {
        let doc_values = DocValues::get_sorted_numeric(&*ctx.leaf_reader(), &self.field)?;
        Ok(Box::new(FieldDoubles {
            doc_values,
            decoder: Arc::clone(&self.decoder),
        }))
    }

    fn source_hash(&self) -> i32 {
        objects_hash(&[
            java_string_hash_code(&self.field),
            decoder_identity_hash_code(&self.decoder),
        ])
    }

    fn source_eq(&self, other: &dyn MultiDoubleValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<FieldMultiValuedSource>()
            .is_some_and(|that| {
                self.field == that.field && Arc::ptr_eq(&self.decoder, &that.decoder)
            })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Equivalent to `org.apache.lucene.facet.MultiDoubleValuesSource.SingleValuedAsMultiValued`.
#[derive(Debug)]
pub(crate) struct SingleValuedAsMultiValued {
    input: Arc<dyn DoubleValuesSource>,
}

/// The anonymous `MultiDoubleValues` of `SingleValuedAsMultiValued.getValues`.
struct SingleDoubleAsMulti {
    single_values: Box<dyn DoubleValues>,
}

impl MultiDoubleValues for SingleDoubleAsMulti {
    fn get_value_count(&self) -> Result<i64> {
        Ok(1)
    }

    fn next_value(&mut self) -> Result<f64> {
        self.single_values.double_value()
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.single_values.advance_exact(doc)
    }
}

impl SegmentCacheable for SingleValuedAsMultiValued {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.input.is_cacheable(ctx)
    }
}

impl fmt::Display for SingleValuedAsMultiValued {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "multi-double({})", self.input.to_source_string())
    }
}

impl MultiDoubleValuesSource for SingleValuedAsMultiValued {
    fn get_values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn MultiDoubleValues>> {
        let single_values = Arc::clone(&self.input).get_values(ctx, None)?;
        Ok(Box::new(SingleDoubleAsMulti { single_values }))
    }

    fn source_hash(&self) -> i32 {
        objects_hash(&[self.input.source_hash()])
    }

    fn source_eq(&self, other: &dyn MultiDoubleValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<SingleValuedAsMultiValued>()
            .is_some_and(|that| self.input.source_eq(&*that.input))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn single_valued(&self) -> Option<Arc<dyn DoubleValuesSource>> {
        Some(Arc::clone(&self.input))
    }
}

/// Equivalent to `org.apache.lucene.facet.MultiDoubleValuesSource.LongDoubleValuesSource`.
#[derive(Debug)]
pub(crate) struct LongDoubleValuesSource {
    input: Arc<dyn MultiDoubleValuesSource>,
}

/// The anonymous `MultiLongValues` of `LongDoubleValuesSource.getValues`.
struct DoubleAsLong {
    double_values: Box<dyn MultiDoubleValues>,
}

impl MultiLongValues for DoubleAsLong {
    fn get_value_count(&self) -> Result<i64> {
        self.double_values.get_value_count()
    }

    fn next_value(&mut self) -> Result<i64> {
        Ok(self.double_values.next_value()? as i64)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.double_values.advance_exact(doc)
    }
}

impl SegmentCacheable for LongDoubleValuesSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.input.is_cacheable(ctx)
    }
}

impl fmt::Display for LongDoubleValuesSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "multi-double({})", self.input)
    }
}

impl MultiLongValuesSource for LongDoubleValuesSource {
    fn get_values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn MultiLongValues>> {
        let double_values = self.input.get_values(ctx)?;
        Ok(Box::new(DoubleAsLong { double_values }))
    }

    fn source_hash(&self) -> i32 {
        objects_hash(&[self.input.source_hash()])
    }

    fn source_eq(&self, other: &dyn MultiLongValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<LongDoubleValuesSource>()
            .is_some_and(|that| self.input.source_eq(&*that.input))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
