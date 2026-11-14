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

//! Port of `ValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function`.
//!
//! # The `context` map
//!
//! Java threads a `Map<Object, Object> context` through
//! `ValueSource.createWeight` and `ValueSource.getValues`. `newContext` builds
//! it as an `IdentityHashMap` holding the searcher under the key `"searcher"`;
//! value sources store their per-search state under their own identity
//! (`context.put(this, weight)`), and the value-source adapters store a
//! `Scorable` under `"scorer"`. This port keeps the map as [`ContextMap`] — a
//! shared, thread-safe identity map, the object a `FunctionWeight` keeps for
//! the whole search — and passes a [`ValueSourceContext`] view that pairs that
//! map with the `"scorer"` entry.
//!
//! **Divergence.** The `"scorer"` entry is held by the view rather than by the
//! map: it is a `Scorable` reading a borrowed `DoubleValues`, which can neither
//! be shared across threads nor outlive the call that created it, while the map
//! itself lives in `Send + Sync` weights. Only the adapters of this file put a
//! scorer, each into a map it has just created, so no map is ever observed with
//! a scorer by any other code.

use std::any::Any;
use std::cell::RefCell;
use std::fmt::{self, Debug};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::double_values::{DoubleValues, EmptyDoubleValues};
use rucene::search::double_values_source::{self, DoubleValuesSource};
use rucene::search::field_comparator::{
    FieldComparator, SharedFieldComparator, SlotComparator, SortValue,
};
use rucene::search::field_comparator_source::FieldComparatorSource;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::long_values::LongValues;
use rucene::search::long_values_source::LongValuesSource;
use rucene::search::pruning::Pruning;
use rucene::search::scorable::Scorable;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::simple_field_comparator::SimpleFieldComparatorImpl;
use rucene::search::sort::{SortField, SortFieldKind, SortFieldRewriter, SortFieldType};
use rucene::util::java_collections::{java_identity_hash_code_stand_in, JavaHashCode};

use crate::function::function_values::FunctionValues;
use crate::function::valuesource::field_cache_source::FieldCacheSource;

/// The `getClass().hashCode()` of a Java class, an identity hash that is
/// unreproducible in Java itself; see
/// [`java_identity_hash_code_stand_in`].
pub(crate) fn class_hash(java_binary_name: &str) -> i32 {
    java_identity_hash_code_stand_in(java_binary_name)
}

/// `Float.floatToIntBits(float)`: the raw bits, every `NaN` collapsed to the
/// canonical `0x7fc00000`.
pub(crate) fn float_to_int_bits(f: f32) -> i32 {
    if f.is_nan() {
        0x7fc0_0000
    } else {
        f.to_bits() as i32
    }
}

/// `Objects.hash(values...)`: `31 * h + v` folded from `1`.
pub(crate) fn objects_hash(values: &[i32]) -> i32 {
    values
        .iter()
        .fold(1i32, |h, &v| h.wrapping_mul(31).wrapping_add(v))
}

/// `Arrays.hashCode(ValueSource[])`, which is also `List.hashCode()` of a list
/// of value sources.
pub(crate) fn sources_hash(sources: &[Arc<dyn ValueSource>]) -> i32 {
    sources.iter().fold(1i32, |h, s| {
        h.wrapping_mul(31).wrapping_add(s.value_source_hash())
    })
}

/// `Arrays.equals(ValueSource[], ValueSource[])`, which is also
/// `List.equals` of two lists of value sources.
pub(crate) fn sources_eq(a: &[Arc<dyn ValueSource>], b: &[Arc<dyn ValueSource>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.value_source_eq(&**y))
}

/// `ReaderUtil.getTopLevelContext(readerContext)`: the root of the context
/// tree a leaf belongs to; a leaf without a parent is its own top level.
pub(crate) fn top_level_context(
    reader_context: &LeafReaderContext,
) -> Arc<dyn rucene::index::IndexReaderContext> {
    match rucene::index::IndexReaderContext::parent(reader_context).and_then(|p| p.upgrade()) {
        None => Arc::new(reader_context.clone()),
        Some(parent) => rucene::index::get_top_level_context(parent),
    }
}

/// `java.lang.Float.compare(float, float)`: `-0.0f` sorts before `0.0f` and
/// every `NaN` equals every other and sorts last.
pub(crate) fn java_float_compare(a: f32, b: f32) -> i32 {
    if a < b {
        return -1;
    }
    if a > b {
        return 1;
    }
    let bits = |v: f32| -> i32 {
        if v.is_nan() {
            0x7fc0_0000
        } else {
            v.to_bits() as i32
        }
    };
    match bits(a).cmp(&bits(b)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// `java.lang.Double.compare(double, double)`.
pub(crate) fn java_double_compare(a: f64, b: f64) -> i32 {
    if a < b {
        return -1;
    }
    if a > b {
        return 1;
    }
    let bits = |v: f64| -> i64 {
        if v.is_nan() {
            0x7ff8_0000_0000_0000_u64 as i64
        } else {
            v.to_bits() as i64
        }
    };
    match bits(a).cmp(&bits(b)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// The views of a [`ValueSource`] that Java's `this` provides for free.
///
/// **This has no counterpart in Lucene 10.5.0.** Java's default methods of
/// `ValueSource` pass `this` where a `ValueSource` is expected
/// (`new WrappedLongValuesSource(this, null)`) and compare classes with
/// `getClass()`. A default method of a Rust trait cannot turn `self` into a
/// trait object, so every value source gets these two views from the blanket
/// implementation below; no implementation writes them by hand.
pub trait ValueSourceDyn {
    /// `this` as a shared `ValueSource`.
    fn as_dyn_value_source(self: Arc<Self>) -> Arc<dyn ValueSource>;

    /// `this` as [`Any`], so that `equals` can recover the concrete class.
    fn as_any(&self) -> &dyn Any;
}

impl<T: ValueSource> ValueSourceDyn for T {
    fn as_dyn_value_source(self: Arc<Self>) -> Arc<dyn ValueSource> {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Instantiates [`FunctionValues`] for a particular reader. Often used when
/// creating a `FunctionQuery`.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.ValueSource`.
///
/// **Divergences.**
/// * `getValues`, `asLongValuesSource`, `asDoubleValuesSource` and
///   `getSortField` take an `Arc<Self>` receiver, as
///   [`DoubleValuesSource`] does: the values and adapters they return keep the
///   source, which is the object graph Java builds with `this`.
/// * `equals(Object)`, `hashCode()` and `toString()` are
///   [`value_source_eq`](Self::value_source_eq),
///   [`value_source_hash`](Self::value_source_hash) and the
///   [`Display`](fmt::Display) of `dyn ValueSource`, which is
///   [`description`](Self::description) as Java's `toString()` is. Where Java
///   hashes a `Class` or an enum constant, the implementation uses
///   [`java_identity_hash_code_stand_in`].
/// * The `context` map is a [`ValueSourceContext`] (see the module
///   documentation).
pub trait ValueSource: ValueSourceDyn + Debug + Send + Sync + 'static {
    /// Gets the values for this reader and the context that was previously
    /// passed to [`create_weight`](Self::create_weight). The values must be
    /// consumed in a forward doc ID manner, and this method must be called
    /// again to iterate through the values again.
    ///
    /// Equivalent to the abstract
    /// `ValueSource.getValues(Map<Object, Object>, LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while opening the values.
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>>;

    /// Equivalent to the abstract `ValueSource.equals(Object)`.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool;

    /// Equivalent to the abstract `ValueSource.hashCode()`.
    fn value_source_hash(&self) -> i32;

    /// Description of the field, used in `explain()`.
    ///
    /// Equivalent to the abstract `ValueSource.description()`.
    fn description(&self) -> String;

    /// Propagates the weight creation to sub-sources, which can optionally
    /// store weight information in the context; the context is passed to
    /// [`get_values`](Self::get_values), where that information can be
    /// retrieved.
    ///
    /// Equivalent to `ValueSource.createWeight(Map<Object, Object>,
    /// IndexSearcher)`, a no-op by default.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while preparing the weight.
    fn create_weight(
        &self,
        _context: &ValueSourceContext<'_>,
        _searcher: &IndexSearcher,
    ) -> Result<()> {
        Ok(())
    }

    /// Exposes this value source as a [`LongValuesSource`].
    ///
    /// Equivalent to `ValueSource.asLongValuesSource()`.
    fn as_long_values_source(self: Arc<Self>) -> Arc<dyn LongValuesSource> {
        Arc::new(WrappedLongValuesSource::new(
            self.as_dyn_value_source(),
            None,
        ))
    }

    /// Exposes this value source as a [`DoubleValuesSource`].
    ///
    /// Equivalent to `ValueSource.asDoubleValuesSource()`.
    fn as_double_values_source(self: Arc<Self>) -> Arc<dyn DoubleValuesSource> {
        Arc::new(WrappedDoubleValuesSource::new(
            self.as_dyn_value_source(),
            None,
        ))
    }

    /// Gets the [`SortField`] for this value source, populated through
    /// [`get_values`](Self::get_values).
    ///
    /// Equivalent to the experimental `ValueSource.getSortField(boolean)`,
    /// which returns a `ValueSourceSortField`.
    ///
    /// # Errors
    ///
    /// Propagates the [`SortField`] construction error.
    fn get_sort_field(self: Arc<Self>, reverse: bool) -> Result<SortField> {
        ValueSourceSortField::new(self.as_dyn_value_source(), reverse)
    }

    /// Returns this source as a [`FieldCacheSource`] when its Java class
    /// extends `FieldCacheSource`, and `None` otherwise.
    ///
    /// **This has no counterpart in Lucene 10.5.0.** `FieldCacheSource.equals`
    /// tests `o instanceof FieldCacheSource` (`FieldCacheSource.java:43-47`),
    /// which Rust cannot ask of a `dyn ValueSource`; the test is declared
    /// here, `None` by default — Java's `false` branch — and every subclass of
    /// `FieldCacheSource` answers `self`.
    fn as_field_cache_source(&self) -> Option<&dyn FieldCacheSource> {
        None
    }
}

impl fmt::Display for dyn ValueSource {
    /// Equivalent to `ValueSource.toString()`, which returns `description()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description())
    }
}

/// A hashable, comparable handle on a [`ValueSource`], for the collections
/// that Java keys on `ValueSource.equals`/`hashCode`.
#[derive(Debug, Clone)]
pub struct ValueSourceKey(pub Arc<dyn ValueSource>);

impl PartialEq for ValueSourceKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.value_source_eq(&*other.0)
    }
}

impl Eq for ValueSourceKey {}

impl JavaHashCode for ValueSourceKey {
    fn java_hash_code(&self) -> i32 {
        self.0.value_source_hash()
    }
}

/// The identity of a value source, which is what keys Java's
/// `IdentityHashMap` context: the address of the source object.
fn identity(vs: &dyn ValueSource) -> usize {
    std::ptr::from_ref(vs).cast::<()>() as usize
}

/// The context map Java creates with `ValueSource.newContext(IndexSearcher)`:
/// the searcher under `"searcher"`, and whatever value sources store under
/// their own identity.
///
/// Equivalent to the `IdentityHashMap<Object, Object>` that
/// `ValueSource.newContext` returns. It is shared between the weight that
/// created it and every [`ValueSourceContext`] view of it, as the Java map is;
/// Java documents the map as not thread-safe, and this one serialises its
/// accesses so that a weight can share it between threads.
#[derive(Default)]
pub struct ContextMap {
    inner: Mutex<ContextInner>,
}

#[derive(Default)]
struct ContextInner {
    /// The `"searcher"` entry; `None` for an absent key and for `null`, which
    /// `context.get("searcher")` does not tell apart.
    searcher: Option<IndexSearcher>,
    /// The entries keyed by a value source's identity.
    entries: Vec<(usize, Arc<dyn Any + Send + Sync>)>,
}

impl Debug for ContextMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.lock();
        f.debug_struct("ContextMap")
            .field("searcher", &inner.searcher.is_some())
            .field("entries", &inner.entries.len())
            .finish()
    }
}

impl ContextMap {
    /// A map holding `searcher` under `"searcher"`.
    pub fn new(searcher: Option<IndexSearcher>) -> Self {
        Self {
            inner: Mutex::new(ContextInner {
                searcher,
                entries: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ContextInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `(IndexSearcher) context.get("searcher")`.
    pub fn searcher(&self) -> Option<IndexSearcher> {
        self.lock().searcher.clone()
    }

    /// `context.put("searcher", searcher)`.
    pub fn put_searcher(&self, searcher: &IndexSearcher) {
        self.lock().searcher = Some(searcher.clone());
    }

    /// `context.get(vs)`, keyed by the identity of `vs`.
    pub fn get(&self, vs: &dyn ValueSource) -> Option<Arc<dyn Any + Send + Sync>> {
        let key = identity(vs);
        self.lock()
            .entries
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| Arc::clone(v))
    }

    /// `context.put(vs, value)`, keyed by the identity of `vs`.
    pub fn put(&self, vs: &dyn ValueSource, value: Arc<dyn Any + Send + Sync>) {
        let key = identity(vs);
        let mut inner = self.lock();
        if let Some(entry) = inner.entries.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = value;
        } else {
            inner.entries.push((key, value));
        }
    }
}

/// A shared [`Scorable`], the value Java stores under `"scorer"`.
pub type SharedScorable<'a> = Rc<RefCell<dyn Scorable + 'a>>;

/// The `Map<Object, Object> context` argument of
/// [`ValueSource::get_values`] and [`ValueSource::create_weight`]: the shared
/// [`ContextMap`] and its `"scorer"` entry (see the module documentation).
pub struct ValueSourceContext<'a> {
    map: Arc<ContextMap>,
    scorer: Option<SharedScorable<'a>>,
}

impl Debug for ValueSourceContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValueSourceContext")
            .field("map", &self.map)
            .field("scorer", &self.scorer.is_some())
            .finish()
    }
}

impl<'a> ValueSourceContext<'a> {
    /// A view of `map` with no `"scorer"` entry.
    pub fn new(map: Arc<ContextMap>) -> Self {
        Self { map, scorer: None }
    }

    /// A view of `map` whose `"scorer"` entry is `scorer`.
    pub fn with_scorer(map: Arc<ContextMap>, scorer: SharedScorable<'a>) -> Self {
        Self {
            map,
            scorer: Some(scorer),
        }
    }

    /// The shared map, which a weight keeps across leaves.
    pub fn map(&self) -> &Arc<ContextMap> {
        &self.map
    }

    /// `(IndexSearcher) context.get("searcher")`.
    pub fn searcher(&self) -> Option<IndexSearcher> {
        self.map.searcher()
    }

    /// `context.put("searcher", searcher)`.
    pub fn put_searcher(&self, searcher: &IndexSearcher) {
        self.map.put_searcher(searcher);
    }

    /// `(Scorable) context.get("scorer")`.
    pub fn scorer(&self) -> Option<&SharedScorable<'a>> {
        self.scorer.as_ref()
    }

    /// `context.get(vs)`.
    pub fn get(&self, vs: &dyn ValueSource) -> Option<Arc<dyn Any + Send + Sync>> {
        self.map.get(vs)
    }

    /// `context.put(vs, value)`.
    pub fn put(&self, vs: &dyn ValueSource, value: Arc<dyn Any + Send + Sync>) {
        self.map.put(vs, value);
    }
}

/// Returns a new non-thread-safe context map.
///
/// Equivalent to the static `ValueSource.newContext(IndexSearcher)`; Java's
/// `null` searcher is `None`.
pub fn new_context(searcher: Option<&IndexSearcher>) -> ValueSourceContext<'static> {
    ValueSourceContext::new(Arc::new(ContextMap::new(searcher.cloned())))
}

/// A [`Scorable`] over a [`DoubleValues`] that computes the score of the
/// current document at most once.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.ValueSource.ScorableView`.
pub(crate) struct ScorableView<'a> {
    /// `scores == null ? DoubleValues.EMPTY : scores`.
    scores: Box<dyn DoubleValues + 'a>,
    /// The `docId` field.
    pub(crate) doc_id: i32,
    /// The `scoresDocId` field.
    scores_doc_id: i32,
    /// The `score` field.
    score: f32,
}

impl<'a> ScorableView<'a> {
    /// Equivalent to `ScorableView(int, float)`.
    pub(crate) fn with_score(doc_id: i32, score: f32) -> Self {
        let mut view = Self::new(None);
        view.doc_id = doc_id;
        view.scores_doc_id = doc_id;
        view.score = score;
        view
    }

    /// Equivalent to `ScorableView(DoubleValues)`; `None` stands for `null`.
    pub(crate) fn new(scores: Option<Box<dyn DoubleValues + 'a>>) -> Self {
        Self {
            scores: scores.unwrap_or_else(|| Box::new(EmptyDoubleValues)),
            doc_id: -1,
            scores_doc_id: -1,
            score: 0.0,
        }
    }
}

impl Scorable for ScorableView<'_> {
    /// Equivalent to `ScorableView.score()`.
    fn score(&mut self) -> Result<f32> {
        // ensure we calculate the score at most once
        if self.scores_doc_id != self.doc_id {
            self.scores_doc_id = self.doc_id;
            if self.scores.advance_exact(self.doc_id)? {
                self.score = self.scores.double_value()? as f32;
            } else {
                self.score = 0.0;
            }
        }
        Ok(self.score)
    }
}

/// Builds the context of an adapter's `getValues`: `newContext(searcher)` with
/// `context.put("scorer", scorer)`.
fn adapter_context<'a>(
    searcher: Option<&IndexSearcher>,
    scorer: Rc<RefCell<ScorableView<'a>>>,
) -> ValueSourceContext<'a> {
    let shared: SharedScorable<'a> = scorer;
    ValueSourceContext::with_scorer(Arc::new(ContextMap::new(searcher.cloned())), shared)
}

/// A [`LongValuesSource`] over a [`ValueSource`].
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.ValueSource.WrappedLongValuesSource`.
#[derive(Debug)]
pub(crate) struct WrappedLongValuesSource {
    in_: Arc<dyn ValueSource>,
    searcher: Option<IndexSearcher>,
}

impl WrappedLongValuesSource {
    /// Equivalent to `WrappedLongValuesSource(ValueSource, IndexSearcher)`.
    fn new(in_: Arc<dyn ValueSource>, searcher: Option<IndexSearcher>) -> Self {
        Self { in_, searcher }
    }
}

/// The anonymous `LongValues` of `WrappedLongValuesSource.getValues`.
struct WrappedLongValues<'a> {
    fv: Box<dyn FunctionValues + 'a>,
    scorer: Rc<RefCell<ScorableView<'a>>>,
}

impl LongValues for WrappedLongValues<'_> {
    fn long_value(&mut self) -> Result<i64> {
        let doc = self.scorer.borrow().doc_id;
        self.fv.long_val(doc)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.scorer.borrow_mut().doc_id = doc;
        self.fv.exists(doc)
    }
}

impl SegmentCacheable for WrappedLongValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl LongValuesSource for WrappedLongValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn LongValues + 'a>> {
        let scorer = Rc::new(RefCell::new(ScorableView::new(scores)));
        let context = adapter_context(self.searcher.as_ref(), Rc::clone(&scorer));
        let fv = Arc::clone(&self.in_).get_values(&context, ctx)?;
        Ok(Box::new(WrappedLongValues { fv, scorer }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn LongValuesSource>> {
        Ok(Arc::new(WrappedLongValuesSource::new(
            Arc::clone(&self.in_),
            Some(searcher.clone()),
        )))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn LongValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<WrappedLongValuesSource>()
            .is_some_and(|that| self.in_.value_source_eq(&*that.in_))
    }

    /// `Objects.hash(in)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[self.in_.value_source_hash()])
    }

    fn to_source_string(&self) -> String {
        self.in_.description()
    }
}

/// A [`DoubleValuesSource`] over a [`ValueSource`].
///
/// Equivalent to the package-private static class
/// `org.apache.lucene.queries.function.ValueSource.WrappedDoubleValuesSource`.
#[derive(Debug)]
pub(crate) struct WrappedDoubleValuesSource {
    /// The `in` field.
    pub(crate) in_: Arc<dyn ValueSource>,
    /// The `searcher` field.
    searcher: Option<IndexSearcher>,
}

impl WrappedDoubleValuesSource {
    /// Equivalent to `WrappedDoubleValuesSource(ValueSource, IndexSearcher)`.
    fn new(in_: Arc<dyn ValueSource>, searcher: Option<IndexSearcher>) -> Self {
        Self { in_, searcher }
    }
}

/// The anonymous `DoubleValues` of `WrappedDoubleValuesSource.getValues`.
struct WrappedDoubleValues<'a> {
    fv: Box<dyn FunctionValues + 'a>,
    scorer: Rc<RefCell<ScorableView<'a>>>,
}

impl DoubleValues for WrappedDoubleValues<'_> {
    fn double_value(&mut self) -> Result<f64> {
        let doc = self.scorer.borrow().doc_id;
        self.fv.double_val(doc)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.scorer.borrow_mut().doc_id = doc;
        // ValueSource will return values even if exists() is false, generally a
        // default of some kind. To preserve this behaviour with the iterator,
        // we need to always return 'true' here.
        Ok(true)
    }
}

impl SegmentCacheable for WrappedDoubleValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl DoubleValuesSource for WrappedDoubleValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let scorer = Rc::new(RefCell::new(ScorableView::new(scores)));
        let context = adapter_context(self.searcher.as_ref(), Rc::clone(&scorer));
        let fv = Arc::clone(&self.in_).get_values(&context, ctx)?;
        Ok(Box::new(WrappedDoubleValues { fv, scorer }))
    }

    fn needs_scores(&self) -> bool {
        true // be on the safe side
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(Arc::new(WrappedDoubleValuesSource::new(
            Arc::clone(&self.in_),
            Some(searcher.clone()),
        )))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<WrappedDoubleValuesSource>()
            .is_some_and(|that| self.in_.value_source_eq(&*that.in_))
    }

    /// `Objects.hash(in)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[self.in_.value_source_hash()])
    }

    fn to_source_string(&self) -> String {
        self.in_.description()
    }

    /// Equivalent to `WrappedDoubleValuesSource.explain(LeafReaderContext,
    /// int, Explanation)`: the values' own explanation, read with a scorer that
    /// answers the explained score.
    fn explain(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        doc_id: i32,
        score_explanation: &Explanation,
    ) -> Result<Explanation> {
        let scorer = Rc::new(RefCell::new(ScorableView::with_score(
            doc_id,
            score_explanation.value().float_value(),
        )));
        let context = adapter_context(self.searcher.as_ref(), scorer);
        let mut fv = Arc::clone(&self.in_).get_values(&context, ctx)?;
        fv.explain(doc_id)
    }
}

/// Wraps a [`DoubleValuesSource`] as a [`ValueSource`]; a source that is
/// itself a wrapped value source is unwrapped.
///
/// Equivalent to the static
/// `ValueSource.fromDoubleValuesSource(DoubleValuesSource)`.
pub fn from_double_values_source(in_: Arc<dyn DoubleValuesSource>) -> Arc<dyn ValueSource> {
    if let Some(wrapped) = in_.as_any().downcast_ref::<WrappedDoubleValuesSource>() {
        return Arc::clone(&wrapped.in_);
    }
    Arc::new(FromDoubleValuesSource { in_ })
}

/// A [`ValueSource`] over a [`DoubleValuesSource`].
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.ValueSource.FromDoubleValuesSource`.
#[derive(Debug)]
pub(crate) struct FromDoubleValuesSource {
    in_: Arc<dyn DoubleValuesSource>,
}

/// `DoubleValuesSource.fromScorer(scorer)` over the context's shared
/// `"scorer"`: `advanceExact` always accepts and `doubleValue` reads
/// `scorer.score()`.
struct ContextScorerValues<'a> {
    scorer: SharedScorable<'a>,
}

impl DoubleValues for ContextScorerValues<'_> {
    fn double_value(&mut self) -> Result<f64> {
        Ok(f64::from(self.scorer.borrow_mut().score()?))
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }
}

/// The anonymous `FunctionValues` of `FromDoubleValuesSource.getValues`.
struct FromDoubleValuesFunctionValues<'a> {
    in_: Arc<dyn DoubleValuesSource>,
    inner: Box<dyn DoubleValues + 'a>,
}

impl FunctionValues for FromDoubleValuesFunctionValues<'_> {
    fn to_doc_string(&mut self, _doc: i32) -> Result<String> {
        Ok(self.in_.to_source_string())
    }

    fn float_val(&mut self, doc: i32) -> Result<f32> {
        if !self.inner.advance_exact(doc)? {
            return Ok(0.0);
        }
        Ok(self.inner.double_value()? as f32)
    }

    fn double_val(&mut self, doc: i32) -> Result<f64> {
        if !self.inner.advance_exact(doc)? {
            return Ok(0.0);
        }
        self.inner.double_value()
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        self.inner.advance_exact(doc)
    }
}

impl ValueSource for FromDoubleValuesSource {
    /// Returns the original [`DoubleValuesSource`].
    fn as_double_values_source(self: Arc<Self>) -> Arc<dyn DoubleValuesSource> {
        Arc::clone(&self.in_)
    }

    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let scores: Option<Box<dyn DoubleValues + 'a>> = context.scorer().map(|scorer| {
            Box::new(ContextScorerValues {
                scorer: Rc::clone(scorer),
            }) as Box<dyn DoubleValues + 'a>
        });
        let inner = match context.searcher() {
            Some(searcher) => Arc::clone(&self.in_)
                .rewrite(&searcher)?
                .get_values(reader_context, scores)?,
            None => Arc::clone(&self.in_).get_values(reader_context, scores)?,
        };
        Ok(Box::new(FromDoubleValuesFunctionValues {
            in_: Arc::clone(&self.in_),
            inner,
        }))
    }

    /// Avoids indirection and supports a better `needsScores()`.
    fn get_sort_field(self: Arc<Self>, reverse: bool) -> Result<SortField> {
        double_values_source::get_sort_field(Arc::clone(&self.in_), reverse)
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<FromDoubleValuesSource>()
            .is_some_and(|that| self.in_.source_eq(&*that.in_))
    }

    /// `Objects.hash(in)`.
    fn value_source_hash(&self) -> i32 {
        objects_hash(&[self.in_.source_hash()])
    }

    fn description(&self) -> String {
        self.in_.to_source_string()
    }
}

/// The sort field [`ValueSource::get_sort_field`] returns: a
/// `REWRITEABLE` sort named after the source's description, which rewrites to
/// a custom sort over a [`ValueSourceComparatorSource`].
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.function.ValueSource.ValueSourceSortField`.
///
/// **Divergence.** Java subclasses `SortField` and overrides
/// `rewrite(IndexSearcher)`. The core models `SortField` subclasses as a
/// [`SortFieldKind`]; this sort field is a [`SortField`] of kind
/// [`SortFieldKind::Rewriteable`] whose [`SortFieldRewriter`] is this class's
/// `rewrite`. Like the Java subclass, it takes no part in equality: two such
/// sort fields compare as `SortField.equals` compares them.
pub(crate) struct ValueSourceSortField;

impl ValueSourceSortField {
    /// Equivalent to `ValueSourceSortField(boolean)`:
    /// `super(description(), SortField.Type.REWRITEABLE, reverse)`.
    ///
    /// The subclass instance is the core [`SortField`] it configures.
    ///
    /// # Errors
    ///
    /// Propagates the [`SortField`] construction error.
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(vs: Arc<dyn ValueSource>, reverse: bool) -> Result<SortField> {
        let description = vs.description();
        let rewriter = SortFieldRewriter(Arc::new(
            move |sort_field: &SortField, searcher: &IndexSearcher| {
                ValueSourceSortField::rewrite(&vs, sort_field, searcher)
            },
        ));
        SortField::with_kind(
            Some(description),
            SortFieldType::Rewriteable,
            reverse,
            None,
            SortFieldKind::Rewriteable { rewriter },
        )
    }

    /// Equivalent to `ValueSourceSortField.rewrite(IndexSearcher)`.
    fn rewrite(
        vs: &Arc<dyn ValueSource>,
        sort_field: &SortField,
        searcher: &IndexSearcher,
    ) -> Result<SortField> {
        let context = new_context(Some(searcher));
        vs.create_weight(&context, searcher)?;
        SortField::new_custom(
            sort_field.field().map(str::to_string),
            Arc::new(ValueSourceComparatorSource::new(
                Arc::clone(vs),
                Arc::clone(context.map()),
            )),
            sort_field.reverse(),
        )
    }
}

/// The comparator source of the rewritten [`ValueSourceSortField`].
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.function.ValueSource.ValueSourceComparatorSource`.
#[derive(Debug)]
pub(crate) struct ValueSourceComparatorSource {
    /// `ValueSource.this`.
    vs: Arc<dyn ValueSource>,
    /// The `context` field.
    context: Arc<ContextMap>,
}

impl ValueSourceComparatorSource {
    /// Equivalent to `ValueSourceComparatorSource(Map<Object, Object>)`.
    fn new(vs: Arc<dyn ValueSource>, context: Arc<ContextMap>) -> Self {
        Self { vs, context }
    }
}

impl FieldComparatorSource for ValueSourceComparatorSource {
    /// Equivalent to `ValueSourceComparatorSource.newComparator(String, int,
    /// Pruning, boolean)`.
    fn new_comparator(
        &self,
        _fieldname: &str,
        num_hits: usize,
        _pruning: Pruning,
        _reversed: bool,
    ) -> Result<Box<dyn FieldComparator>> {
        Ok(Box::new(SharedFieldComparator::new(
            ValueSourceComparator::new(Arc::clone(&self.vs), Arc::clone(&self.context), num_hits),
        )))
    }
}

/// A comparator that works off the [`FunctionValues`] of a value source
/// instead of the normal comparators that work off a field.
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.function.ValueSource.ValueSourceComparator`, a
/// `SimpleFieldComparator<Double>`; its `docVals` field is the leaf state.
#[derive(Debug)]
pub(crate) struct ValueSourceComparator {
    /// `ValueSource.this`.
    vs: Arc<dyn ValueSource>,
    values: Vec<f64>,
    bottom: f64,
    fcontext: Arc<ContextMap>,
    top_value: f64,
}

impl ValueSourceComparator {
    /// Equivalent to `ValueSourceComparator(Map<Object, Object>, int)`.
    fn new(vs: Arc<dyn ValueSource>, fcontext: Arc<ContextMap>, num_hits: usize) -> Self {
        Self {
            vs,
            values: vec![0.0; num_hits],
            bottom: 0.0,
            fcontext,
            top_value: 0.0,
        }
    }

    /// The `docVals` field, `null` until `doSetNextReader`.
    fn doc_vals(
        leaf: &mut Option<Box<dyn FunctionValues>>,
    ) -> Result<&mut Box<dyn FunctionValues>> {
        leaf.as_mut().ok_or_else(LuceneError::null_pointer)
    }
}

impl SlotComparator for ValueSourceComparator {
    /// The `docVals` field `doSetNextReader` sets.
    type Leaf = Option<Box<dyn FunctionValues>>;

    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        java_double_compare(self.values[slot1 as usize], self.values[slot2 as usize])
    }

    fn set_top_value(&mut self, value: SortValue) {
        if let SortValue::Double(v) = value {
            self.top_value = v;
        }
    }

    fn value(&self, slot: i32) -> SortValue {
        SortValue::Double(self.values[slot as usize])
    }

    fn new_leaf(&mut self, context: &LeafReaderContext) -> Result<Self::Leaf> {
        self.do_set_next_reader(context)
    }

    fn set_bottom(&mut self, _leaf: &mut Self::Leaf, bottom: i32) -> Result<()> {
        self.bottom = self.values[bottom as usize];
        Ok(())
    }

    fn compare_bottom(
        &mut self,
        leaf: &mut Self::Leaf,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        let value = Self::doc_vals(leaf)?.double_val(doc)?;
        Ok(java_double_compare(self.bottom, value))
    }

    fn compare_top(
        &mut self,
        leaf: &mut Self::Leaf,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        let doc_value = Self::doc_vals(leaf)?.double_val(doc)?;
        Ok(java_double_compare(self.top_value, doc_value))
    }

    fn copy(
        &mut self,
        leaf: &mut Self::Leaf,
        slot: i32,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<()> {
        self.values[slot as usize] = Self::doc_vals(leaf)?.double_val(doc)?;
        Ok(())
    }

    /// `SimpleFieldComparator.setScorer`, which does nothing.
    fn set_scorer(&mut self, _leaf: &mut Self::Leaf, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }
}

impl SimpleFieldComparatorImpl for ValueSourceComparator {
    /// Equivalent to `ValueSourceComparator.doSetNextReader(LeafReaderContext)`.
    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<Self::Leaf> {
        let view = ValueSourceContext::new(Arc::clone(&self.fcontext));
        Ok(Some(Arc::clone(&self.vs).get_values(&view, context)?))
    }
}
