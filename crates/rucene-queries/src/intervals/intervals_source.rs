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

//! Port of `IntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::any::Any;
use std::fmt::{self, Debug};
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::java_collections::{JavaHashCode, JavaObjectHashSet};
use rucene::util::BytesRef;

use crate::intervals::interval_iterator::IntervalIterator;
use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;

/// The views of an [`IntervalsSource`] that Java's `this` provides for free.
///
/// **This has no counterpart in Lucene 10.5.0.** Java passes `this` where a
/// source is expected (`Collections.singleton(this)`,
/// `new IntervalQuery(field, this)`) and compares classes with `getClass()`;
/// every source gets these views from the blanket implementation below.
pub trait IntervalsSourceDyn {
    /// `this` as a shared source.
    fn as_dyn_intervals_source(self: Arc<Self>) -> Arc<dyn IntervalsSource>;

    /// `this` as [`Any`], so that `equals` can recover the concrete class.
    fn as_any(&self) -> &dyn Any;
}

impl<T: IntervalsSource> IntervalsSourceDyn for T {
    fn as_dyn_intervals_source(self: Arc<Self>) -> Arc<dyn IntervalsSource> {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A helper for `IntervalQuery` that provides an [`IntervalIterator`] for a
/// given field and segment.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.IntervalsSource`. Static constructor
/// functions for the different sources are in
/// [`Intervals`](crate::intervals::intervals::Intervals).
///
/// **Divergences.**
/// * `visit` and `pullUpDisjunctions` take an `Arc<Self>` receiver, because
///   they hand `this` on (`new IntervalQuery(field, this)`,
///   `Collections.singleton(this)`).
/// * `pullUpDisjunctions` returns the collection's elements in its iteration
///   order; the sources whose Java collection is a `HashSet` build it with
///   [`JavaObjectHashSet`], which iterates in the same order.
/// * `hashCode()`, `equals(Object)` and `toString()` are
///   [`source_hash`](Self::source_hash), [`source_eq`](Self::source_eq) and
///   [`to_source_string`](Self::to_source_string), also the
///   [`Display`](fmt::Display) of `dyn IntervalsSource`.
pub trait IntervalsSource: IntervalsSourceDyn + Debug + Send + Sync + 'static {
    /// Creates an [`IntervalIterator`] exposing the minimum intervals defined
    /// by this source, or `None` if no intervals for this field exist in this
    /// segment.
    ///
    /// Equivalent to the abstract
    /// `IntervalsSource.intervals(String, LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the index.
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>>;

    /// Returns a matches iterator over the intervals defined by this source
    /// for a given document and field, or `None` if no intervals exist in the
    /// given document and field.
    ///
    /// Equivalent to the abstract
    /// `IntervalsSource.matches(String, LeafReaderContext, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the index.
    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>>;

    /// Expert: visits the tree of sources.
    ///
    /// Equivalent to the abstract `IntervalsSource.visit(String,
    /// QueryVisitor)`.
    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor);

    /// Returns the minimum possible width of an interval returned by this
    /// source.
    ///
    /// Equivalent to the abstract `IntervalsSource.minExtent()`.
    fn min_extent(&self) -> i32;

    /// Expert: returns the set of disjunctions that make up this source.
    ///
    /// Equivalent to the abstract `IntervalsSource.pullUpDisjunctions()`;
    /// most implementations return `Collections.singleton(this)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of a rewrite into too many
    /// disjunctions.
    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>>;

    /// Equivalent to the abstract `IntervalsSource.hashCode()`.
    fn source_hash(&self) -> i32;

    /// Equivalent to the abstract `IntervalsSource.equals(Object)`.
    fn source_eq(&self, other: &dyn IntervalsSource) -> bool;

    /// Equivalent to the abstract `IntervalsSource.toString()`.
    fn to_source_string(&self) -> String;
}

impl fmt::Display for dyn IntervalsSource {
    /// Equivalent to `IntervalsSource.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_source_string())
    }
}

/// A hashable, comparable handle on an [`IntervalsSource`], for the
/// collections Java keys on `IntervalsSource.equals`/`hashCode`.
#[derive(Debug, Clone)]
pub struct IntervalsSourceKey(pub Arc<dyn IntervalsSource>);

impl PartialEq for IntervalsSourceKey {
    /// The key comparison of Java's hash collections (`HashMap.getNode`):
    /// `k == key || key.equals(k)`, the identity first.
    fn eq(&self, other: &Self) -> bool {
        objects_equals(&self.0, &other.0)
    }
}

impl Eq for IntervalsSourceKey {}

impl JavaHashCode for IntervalsSourceKey {
    fn java_hash_code(&self) -> i32 {
        self.0.source_hash()
    }
}

/// `List.hashCode()` of a list of sources: `31 * h + e.hashCode()` from `1`.
pub(crate) fn sources_list_hash(sources: &[Arc<dyn IntervalsSource>]) -> i32 {
    sources.iter().fold(1i32, |h, s| {
        h.wrapping_mul(31).wrapping_add(s.source_hash())
    })
}

/// `List.equals` (or `Arrays.equals`) of two lists of sources: each pair of
/// elements compared with `Objects.equals`, the identity first.
pub(crate) fn sources_list_eq(
    a: &[Arc<dyn IntervalsSource>],
    b: &[Arc<dyn IntervalsSource>],
) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| objects_equals(x, y))
}

/// `Objects.equals(a, b)` over two sources: `a == b || a.equals(b)`, the
/// identity first, so that a source whose `equals` is not reflexive still
/// equals itself.
pub(crate) fn objects_equals(a: &Arc<dyn IntervalsSource>, b: &Arc<dyn IntervalsSource>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b)) || a.source_eq(&**b)
}

/// `this == o`: whether `other` is the very object `this`.
pub(crate) fn same_object<T>(this: &T, other: &dyn IntervalsSource) -> bool {
    std::ptr::addr_eq(this as *const T, other as *const dyn IntervalsSource)
}

/// `Objects.hash(values...)`: `31 * h + v` folded from `1`.
pub(crate) fn objects_hash(values: &[i32]) -> i32 {
    values
        .iter()
        .fold(1i32, |h, &v| h.wrapping_mul(31).wrapping_add(v))
}

/// `BytesRef.utf8ToString()` in a rendering that cannot fail.
///
/// **Divergence.** Java's `utf8ToString()` throws
/// `ArrayIndexOutOfBoundsException` for a multi-byte sequence that runs past
/// the bytes; the `toString()` methods that call it cannot return an error,
/// so that exception is a panic here.
pub(crate) fn utf8_to_string(bytes: &BytesRef) -> String {
    match bytes.utf8_to_string() {
        Ok(s) => s,
        Err(e) => panic!("{e}"),
    }
}

/// `Collectors.toSet()` of a stream of sources: a `java.util.HashSet`,
/// returned in its iteration order.
pub(crate) fn to_java_hash_set(
    sources: impl IntoIterator<Item = Arc<dyn IntervalsSource>>,
) -> Vec<Arc<dyn IntervalsSource>> {
    let mut set = JavaObjectHashSet::new();
    for source in sources {
        set.insert(IntervalsSourceKey(source));
    }
    set.into_elements().map(|key| key.0).collect()
}
