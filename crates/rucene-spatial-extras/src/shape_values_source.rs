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

//! The types declared in `ShapeValuesSource.java`, of the Java package
//! `org.apache.lucene.spatial`.

use std::any::Any;
use std::fmt;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::segment_cacheable::SegmentCacheable;

use crate::shape_values::ShapeValues;

/// Equivalent to `org.apache.lucene.spatial.ShapeValuesSource` (`public
/// abstract class ShapeValuesSource implements SegmentCacheable`): produces
/// [`ShapeValues`] per-segment.
///
/// Java's `equals(Object)`, `hashCode()` and `toString()` are
/// [`source_eq`](Self::source_eq), [`source_hash`](Self::source_hash) and the
/// [`fmt::Display`] implementation.
pub trait ShapeValuesSource: SegmentCacheable + fmt::Display + fmt::Debug + Send + Sync + Any {
    /// `getValues(LeafReaderContext ctx)`: get a [`ShapeValues`] instance
    /// for the given leaf reader context.
    ///
    /// # Errors
    ///
    /// The I/O errors of reading the leaf.
    fn get_values(&self, ctx: &LeafReaderContext) -> Result<Box<dyn ShapeValues>>;

    /// `equals(Object o)`.
    fn source_eq(&self, other: &dyn ShapeValuesSource) -> bool;

    /// `hashCode()`.
    fn source_hash(&self) -> i32;

    /// This source as `Any`, for `getClass()` comparisons and casts.
    fn as_any(&self) -> &dyn Any;
}
