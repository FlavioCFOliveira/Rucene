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

//! Port of `IndexReaderFunctions.java`, of the Java package
//! `org.apache.lucene.queries.function`.

use std::any::Any;
use std::fmt::Debug;
use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene::index::{IndexReader, LeafReaderContext, PostingsEnum, Term, POSTINGS_ENUM_FREQS};
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::double_values::{DoubleValues, EmptyDoubleValues};
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::long_values::LongValues;
use rucene::search::long_values_source::LongValuesSource;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::util::java_collections::{
    java_double_hash_code, java_identity_hash_code_stand_in, java_long_hash_code,
    java_string_hash_code,
};

use crate::function::value_source::{
    java_double_compare, objects_equals_double, objects_equals_long, objects_hash, same_object,
};

/// Class that exposes index-reader statistics as value sources.
///
/// Equivalent to the final class
/// `org.apache.lucene.queries.function.IndexReaderFunctions`. Every source
/// but [`term_freq`](Self::term_freq) must be rewritten against a searcher
/// before use, and reads the statistic of the searcher's reader then.
#[derive(Debug, Clone, Copy)]
pub struct IndexReaderFunctions;

/// The message of every source that is read before being rewritten.
const MUST_BE_REWRITTEN: &str = "IndexReaderFunction must be rewritten before use";

impl IndexReaderFunctions {
    /// Creates a constant value source returning the `docFreq` of a given
    /// term.
    ///
    /// Equivalent to the static `IndexReaderFunctions.docFreq(Term)`.
    pub fn doc_freq(term: Term) -> Arc<dyn DoubleValuesSource> {
        let description = format!("docFreq({term})");
        Arc::new(IndexReaderDoubleValuesSource::new(
            Arc::new(DocFreqFunction(term)),
            description,
        ))
    }

    /// Creates a constant value source returning the index's `maxDoc`.
    ///
    /// Equivalent to the static `IndexReaderFunctions.maxDoc()`.
    pub fn max_doc() -> Arc<dyn DoubleValuesSource> {
        Arc::new(IndexReaderDoubleValuesSource::new(
            Arc::clone(&MAX_DOC_FUNCTION),
            "maxDoc()".to_string(),
        ))
    }

    /// Creates a constant value source returning the index's `numDocs`.
    ///
    /// Equivalent to the static `IndexReaderFunctions.numDocs()`.
    pub fn num_docs() -> Arc<dyn DoubleValuesSource> {
        Arc::new(IndexReaderDoubleValuesSource::new(
            Arc::clone(&NUM_DOCS_FUNCTION),
            "numDocs()".to_string(),
        ))
    }

    /// Creates a constant value source returning the index's
    /// `numDeletedDocs`.
    ///
    /// Equivalent to the static `IndexReaderFunctions.numDeletedDocs()`.
    pub fn num_deleted_docs() -> Arc<dyn DoubleValuesSource> {
        Arc::new(IndexReaderDoubleValuesSource::new(
            Arc::clone(&NUM_DELETED_DOCS_FUNCTION),
            "numDeletedDocs()".to_string(),
        ))
    }

    /// Creates a constant value source returning the `sumTotalTermFreq` for a
    /// field.
    ///
    /// Equivalent to the static `IndexReaderFunctions.sumTotalTermFreq(String)`.
    pub fn sum_total_term_freq(field: &str) -> Arc<dyn LongValuesSource> {
        Arc::new(SumTotalTermFreqValuesSource::new(field.to_string()))
    }

    /// Creates a value source that returns the term frequency of a given term
    /// for each document.
    ///
    /// Equivalent to the static `IndexReaderFunctions.termFreq(Term)`.
    pub fn term_freq(term: Term) -> Arc<dyn DoubleValuesSource> {
        Arc::new(TermFreqDoubleValuesSource::new(term))
    }

    /// Creates a constant value source returning the `totalTermFreq` for a
    /// given term.
    ///
    /// Equivalent to the static `IndexReaderFunctions.totalTermFreq(Term)`.
    pub fn total_term_freq(term: Term) -> Arc<dyn DoubleValuesSource> {
        let description = format!("totalTermFreq({term})");
        Arc::new(IndexReaderDoubleValuesSource::new(
            Arc::new(TotalTermFreqFunction(term)),
            description,
        ))
    }

    /// Creates a constant value source returning the `sumDocFreq` for a given
    /// field.
    ///
    /// Equivalent to the static `IndexReaderFunctions.sumDocFreq(String)`.
    pub fn sum_doc_freq(field: &str) -> Arc<dyn DoubleValuesSource> {
        Arc::new(IndexReaderDoubleValuesSource::new(
            Arc::new(SumDocFreqFunction(field.to_string())),
            format!("sumDocFreq({field})"),
        ))
    }

    /// Creates a constant value source returning the `docCount` for a given
    /// field.
    ///
    /// Equivalent to the static `IndexReaderFunctions.docCount(String)`.
    pub fn doc_count(field: &str) -> Arc<dyn DoubleValuesSource> {
        Arc::new(IndexReaderDoubleValuesSource::new(
            Arc::new(DocCountFunction(field.to_string())),
            format!("docCount({field})"),
        ))
    }
}

/// A source answering `reader.getSumTotalTermFreq(field)` once rewritten.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.IndexReaderFunctions.SumTotalTermFreqValuesSource`.
#[derive(Debug)]
pub(crate) struct SumTotalTermFreqValuesSource {
    field: String,
}

impl SumTotalTermFreqValuesSource {
    /// Equivalent to `SumTotalTermFreqValuesSource(String)`.
    fn new(field: String) -> Self {
        Self { field }
    }
}

impl SegmentCacheable for SumTotalTermFreqValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl LongValuesSource for SumTotalTermFreqValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn LongValues + 'a>> {
        Err(LuceneError::unsupported_operation(MUST_BE_REWRITTEN))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn LongValuesSource>> {
        let value = searcher
            .get_index_reader()
            .get_sum_total_term_freq(&self.field)?;
        Ok(Arc::new(NoCacheConstantLongValuesSource::new(value, self)))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Equivalent to `SumTotalTermFreqValuesSource.equals(Object)`.
    fn source_eq(&self, other: &dyn LongValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<SumTotalTermFreqValuesSource>()
            .is_some_and(|that| self.field == that.field)
    }

    /// `Objects.hash(field)`.
    ///
    /// Equivalent to `SumTotalTermFreqValuesSource.hashCode()`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[java_string_hash_code(&self.field)])
    }

    /// Equivalent to `SumTotalTermFreqValuesSource.toString()`.
    fn to_source_string(&self) -> String {
        format!("sumTotalTermFreq({})", self.field)
    }
}

/// A constant long source that remembers the source it was rewritten from.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.IndexReaderFunctions.NoCacheConstantLongValuesSource`.
#[derive(Debug)]
pub(crate) struct NoCacheConstantLongValuesSource {
    value: i64,
    parent: Arc<dyn LongValuesSource>,
}

impl NoCacheConstantLongValuesSource {
    /// Equivalent to `NoCacheConstantLongValuesSource(long, LongValuesSource)`.
    fn new(value: i64, parent: Arc<dyn LongValuesSource>) -> Self {
        Self { value, parent }
    }
}

/// The anonymous `LongValues` of `NoCacheConstantLongValuesSource.getValues`.
struct ConstantLongValues {
    value: i64,
}

impl LongValues for ConstantLongValues {
    fn long_value(&mut self) -> Result<i64> {
        Ok(self.value)
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }
}

impl SegmentCacheable for NoCacheConstantLongValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl LongValuesSource for NoCacheConstantLongValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn LongValues + 'a>> {
        Ok(Box::new(ConstantLongValues { value: self.value }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn LongValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `o instanceof NoCacheConstantLongValuesSource` with equal value and
    /// parent.
    ///
    /// Equivalent to `NoCacheConstantLongValuesSource.equals(Object)`.
    fn source_eq(&self, other: &dyn LongValuesSource) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<NoCacheConstantLongValuesSource>()
                .is_some_and(|that| {
                    self.value == that.value && objects_equals_long(&self.parent, &that.parent)
                })
    }

    /// `Objects.hash(value, parent)`.
    ///
    /// Equivalent to `NoCacheConstantLongValuesSource.hashCode()`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[java_long_hash_code(self.value), self.parent.source_hash()])
    }

    /// Equivalent to `NoCacheConstantLongValuesSource.toString()`.
    fn to_source_string(&self) -> String {
        self.parent.to_source_string()
    }
}

/// A source answering, for each document, the frequency of a term in it.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.IndexReaderFunctions.TermFreqDoubleValuesSource`.
#[derive(Debug)]
pub(crate) struct TermFreqDoubleValuesSource {
    term: Term,
}

impl TermFreqDoubleValuesSource {
    /// Equivalent to `TermFreqDoubleValuesSource(Term)`.
    fn new(term: Term) -> Self {
        Self { term }
    }
}

/// The anonymous `DoubleValues` of `TermFreqDoubleValuesSource.getValues`.
struct TermFreqDoubleValues {
    pe: Box<dyn PostingsEnum>,
}

impl DoubleValues for TermFreqDoubleValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(f64::from(self.pe.freq()?))
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        if self.pe.doc_id() > doc {
            return Ok(false);
        }
        Ok(self.pe.doc_id() == doc || self.pe.advance(doc)? == doc)
    }
}

impl SegmentCacheable for TermFreqDoubleValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl DoubleValuesSource for TermFreqDoubleValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let terms = rucene::index::terms::get_terms(ctx.leaf_reader().as_ref(), self.term.field())?;
        let mut te = terms.iterator()?;
        if !te.seek_exact(self.term.bytes())? {
            return Ok(Box::new(EmptyDoubleValues));
        }
        let pe = te.postings(None, POSTINGS_ENUM_FREQS)?;
        debug_assert!(pe.is_some());
        let pe = pe.ok_or_else(LuceneError::null_pointer)?;
        Ok(Box::new(TermFreqDoubleValues { pe }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<TermFreqDoubleValuesSource>()
            .is_some_and(|that| self.term == that.term)
    }

    /// `Objects.hash(term)`.
    ///
    /// Equivalent to `TermFreqDoubleValuesSource.hashCode()`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[self.term.hash_code()])
    }

    fn to_source_string(&self) -> String {
        format!("termFreq({})", self.term)
    }
}

/// A statistic of an index reader.
///
/// Equivalent to the private functional interface
/// `org.apache.lucene.queries.function.IndexReaderFunctions.ReaderFunction`.
///
/// **Divergence.** Java implements it with lambdas and compares them as
/// objects (`IndexReaderDoubleValuesSource.java:350`), that is by identity: the
/// non-capturing method references of `maxDoc()`, `numDocs()` and
/// `numDeletedDocs()` are one shared instance each (HotSpot caches a
/// non-capturing lambda per call site), while the capturing lambdas of the
/// other factories are a new instance per call. This port shares one instance
/// of the three non-capturing functions and creates a new one per call for the
/// others, and compares them by [`Arc::ptr_eq`]; the identity hash Java mixes
/// in is the stand-in of [`java_identity_hash_code_stand_in`].
pub(crate) trait ReaderFunction: Debug + Send + Sync {
    /// Equivalent to `ReaderFunction.apply(IndexReader)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistic.
    fn apply(&self, reader: &dyn IndexReader) -> Result<f64>;

    /// The stand-in of the function's identity hash.
    fn identity_hash(&self) -> i32;
}

/// `r -> (double) r.docFreq(term)`.
#[derive(Debug)]
struct DocFreqFunction(Term);

impl ReaderFunction for DocFreqFunction {
    fn apply(&self, reader: &dyn IndexReader) -> Result<f64> {
        Ok(f64::from(reader.doc_freq(&self.0)?))
    }

    fn identity_hash(&self) -> i32 {
        java_identity_hash_code_stand_in("IndexReaderFunctions$docFreq$lambda")
    }
}

/// `IndexReader::maxDoc`.
#[derive(Debug)]
struct MaxDocFunction;

impl ReaderFunction for MaxDocFunction {
    fn apply(&self, reader: &dyn IndexReader) -> Result<f64> {
        Ok(f64::from(reader.max_doc()))
    }

    fn identity_hash(&self) -> i32 {
        java_identity_hash_code_stand_in("IndexReaderFunctions$maxDoc$lambda")
    }
}

/// `IndexReader::numDocs`.
#[derive(Debug)]
struct NumDocsFunction;

impl ReaderFunction for NumDocsFunction {
    fn apply(&self, reader: &dyn IndexReader) -> Result<f64> {
        Ok(f64::from(reader.num_docs()))
    }

    fn identity_hash(&self) -> i32 {
        java_identity_hash_code_stand_in("IndexReaderFunctions$numDocs$lambda")
    }
}

/// `IndexReader::numDeletedDocs`.
#[derive(Debug)]
struct NumDeletedDocsFunction;

impl ReaderFunction for NumDeletedDocsFunction {
    fn apply(&self, reader: &dyn IndexReader) -> Result<f64> {
        Ok(f64::from(reader.num_deleted_docs()))
    }

    fn identity_hash(&self) -> i32 {
        java_identity_hash_code_stand_in("IndexReaderFunctions$numDeletedDocs$lambda")
    }
}

/// `r -> r.totalTermFreq(term)`.
#[derive(Debug)]
struct TotalTermFreqFunction(Term);

impl ReaderFunction for TotalTermFreqFunction {
    fn apply(&self, reader: &dyn IndexReader) -> Result<f64> {
        Ok(reader.total_term_freq(&self.0)? as f64)
    }

    fn identity_hash(&self) -> i32 {
        java_identity_hash_code_stand_in("IndexReaderFunctions$totalTermFreq$lambda")
    }
}

/// `r -> r.getSumDocFreq(field)`.
#[derive(Debug)]
struct SumDocFreqFunction(String);

impl ReaderFunction for SumDocFreqFunction {
    fn apply(&self, reader: &dyn IndexReader) -> Result<f64> {
        Ok(reader.get_sum_doc_freq(&self.0)? as f64)
    }

    fn identity_hash(&self) -> i32 {
        java_identity_hash_code_stand_in("IndexReaderFunctions$sumDocFreq$lambda")
    }
}

/// `r -> r.getDocCount(field)`.
#[derive(Debug)]
struct DocCountFunction(String);

impl ReaderFunction for DocCountFunction {
    fn apply(&self, reader: &dyn IndexReader) -> Result<f64> {
        Ok(f64::from(reader.get_doc_count(&self.0)?))
    }

    fn identity_hash(&self) -> i32 {
        java_identity_hash_code_stand_in("IndexReaderFunctions$docCount$lambda")
    }
}

/// The shared instance of `IndexReader::maxDoc`.
static MAX_DOC_FUNCTION: LazyLock<Arc<dyn ReaderFunction>> =
    LazyLock::new(|| Arc::new(MaxDocFunction));

/// The shared instance of `IndexReader::numDocs`.
static NUM_DOCS_FUNCTION: LazyLock<Arc<dyn ReaderFunction>> =
    LazyLock::new(|| Arc::new(NumDocsFunction));

/// The shared instance of `IndexReader::numDeletedDocs`.
static NUM_DELETED_DOCS_FUNCTION: LazyLock<Arc<dyn ReaderFunction>> =
    LazyLock::new(|| Arc::new(NumDeletedDocsFunction));

/// A source answering a statistic of the searcher's reader once rewritten.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.IndexReaderFunctions.IndexReaderDoubleValuesSource`.
#[derive(Debug)]
pub(crate) struct IndexReaderDoubleValuesSource {
    func: Arc<dyn ReaderFunction>,
    description: String,
}

impl IndexReaderDoubleValuesSource {
    /// Equivalent to `IndexReaderDoubleValuesSource(ReaderFunction, String)`.
    fn new(func: Arc<dyn ReaderFunction>, description: String) -> Self {
        Self { func, description }
    }
}

impl SegmentCacheable for IndexReaderDoubleValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl DoubleValuesSource for IndexReaderDoubleValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        Err(LuceneError::unsupported_operation(MUST_BE_REWRITTEN))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        let value = self.func.apply(searcher.get_index_reader().as_ref())?;
        Ok(Arc::new(NoCacheConstantDoubleValuesSource::new(
            value, self,
        )))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        other
            .as_any()
            .downcast_ref::<IndexReaderDoubleValuesSource>()
            .is_some_and(|that| {
                self.description == that.description && Arc::ptr_eq(&self.func, &that.func)
            })
    }

    /// `Objects.hash(description, func)`.
    ///
    /// Equivalent to `IndexReaderDoubleValuesSource.hashCode()`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[
            java_string_hash_code(&self.description),
            self.func.identity_hash(),
        ])
    }

    fn to_source_string(&self) -> String {
        self.description.clone()
    }
}

/// A constant double source that remembers the source it was rewritten from.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.IndexReaderFunctions.NoCacheConstantDoubleValuesSource`.
#[derive(Debug)]
pub(crate) struct NoCacheConstantDoubleValuesSource {
    value: f64,
    parent: Arc<dyn DoubleValuesSource>,
}

impl NoCacheConstantDoubleValuesSource {
    /// Equivalent to
    /// `NoCacheConstantDoubleValuesSource(double, DoubleValuesSource)`.
    fn new(value: f64, parent: Arc<dyn DoubleValuesSource>) -> Self {
        Self { value, parent }
    }
}

/// The anonymous `DoubleValues` of
/// `NoCacheConstantDoubleValuesSource.getValues`.
struct ConstantDoubleValues {
    value: f64,
}

impl DoubleValues for ConstantDoubleValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(self.value)
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }
}

impl SegmentCacheable for NoCacheConstantDoubleValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl DoubleValuesSource for NoCacheConstantDoubleValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        Ok(Box::new(ConstantDoubleValues { value: self.value }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `o instanceof NoCacheConstantDoubleValuesSource`,
    /// `Double.compare(that.value, value) == 0` and equal parents.
    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<NoCacheConstantDoubleValuesSource>()
                .is_some_and(|that| {
                    java_double_compare(that.value, self.value) == 0
                        && objects_equals_double(&self.parent, &that.parent)
                })
    }

    /// `Objects.hash(value, parent)`.
    ///
    /// Equivalent to `NoCacheConstantDoubleValuesSource.hashCode()`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[java_double_hash_code(self.value), self.parent.source_hash()])
    }

    fn to_source_string(&self) -> String {
        self.parent.to_source_string()
    }
}
