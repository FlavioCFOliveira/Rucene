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

//! Port of `SumTotalTermFreqValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::terms::get_terms;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_collections::java_string_hash_code;

use crate::function::docvalues::long_doc_values::{long_doc_values_members, LongDocValues};
use crate::function::function_values::{FunctionValues, ValueFiller};
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};
use crate::function::value_source_scorer::ValueSourceScorerMatcher;
use rucene::search::similarities::Explanation;
use rucene::util::mutable::MutableValueObject;
use rucene::util::BytesRefBuilder;

/// The constant values a statistics source stores in the context: the
/// anonymous `LongDocValues` whose `longVal` is the statistic.
pub(crate) struct ConstLongDocValues {
    vs: Arc<dyn ValueSource>,
    ttf: i64,
}

impl ConstLongDocValues {
    /// The values of `vs`, answering `ttf`.
    pub(crate) fn new(vs: Arc<dyn ValueSource>, ttf: i64) -> Self {
        Self { vs, ttf }
    }
}

impl FunctionValues for ConstLongDocValues {
    fn long_val(&mut self, _doc: i32) -> Result<i64> {
        Ok(self.ttf)
    }

    long_doc_values_members!(
        byte_val,
        short_val,
        float_val,
        int_val,
        double_val,
        bool_val,
        str_val,
        object_val,
        to_doc_string,
        get_range_matcher,
        get_value_filler
    );
}

impl LongDocValues for ConstLongDocValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.vs
    }
}

/// `(FunctionValues) context.get(this)` of a statistics source: the statistic
/// `createWeight` stored, as the constant values Java stored.
///
/// When `createWeight` was not called, Java's map answers `null` and
/// `getValues` returns it (`TotalTermFreqValueSource.java:62-65`,
/// `SumTotalTermFreqValueSource.java:53-57`); the caller fails only when it
/// uses that reference. That `null` is [`NullFunctionValues`] here.
///
/// The context map is shared across threads here, so it stores the
/// statistic and every call builds the same constant values around it, where
/// Java returns the one `LongDocValues` it stored.
///
/// # Errors
///
/// None in practice; the signature is the trait's.
pub(crate) fn stored_values(
    context: &ValueSourceContext<'_>,
    vs: Arc<dyn ValueSource>,
) -> Result<Box<dyn FunctionValues>> {
    match context
        .get(&*vs)
        .and_then(|value| value.downcast_ref::<i64>().copied())
    {
        Some(ttf) => Ok(Box::new(ConstLongDocValues::new(vs, ttf))),
        None => Ok(Box::new(NullFunctionValues)),
    }
}

/// The `null` reference a statistics source's `getValues` returns before
/// `createWeight`: every use of it is Java's `NullPointerException`.
///
/// A method answering through `Result` returns that exception; a method
/// whose signature carries no error (`cost`, `getValueFiller`,
/// `getScorer`'s matcher) raises it as a panic payload, as Java's unchecked
/// exception propagates from the call.
#[derive(Debug)]
pub(crate) struct NullFunctionValues;

fn null_dereference<T>() -> T {
    std::panic::panic_any(LuceneError::null_pointer())
}

impl FunctionValues for NullFunctionValues {
    fn byte_val(&mut self, _doc: i32) -> Result<i8> {
        Err(LuceneError::null_pointer())
    }
    fn short_val(&mut self, _doc: i32) -> Result<i16> {
        Err(LuceneError::null_pointer())
    }
    fn float_val(&mut self, _doc: i32) -> Result<f32> {
        Err(LuceneError::null_pointer())
    }
    fn int_val(&mut self, _doc: i32) -> Result<i32> {
        Err(LuceneError::null_pointer())
    }
    fn long_val(&mut self, _doc: i32) -> Result<i64> {
        Err(LuceneError::null_pointer())
    }
    fn double_val(&mut self, _doc: i32) -> Result<f64> {
        Err(LuceneError::null_pointer())
    }
    fn str_val(&mut self, _doc: i32) -> Result<Option<String>> {
        Err(LuceneError::null_pointer())
    }
    fn bool_val(&mut self, _doc: i32) -> Result<bool> {
        Err(LuceneError::null_pointer())
    }
    fn float_vector_val(&mut self, _doc: i32) -> Result<Option<Vec<f32>>> {
        Err(LuceneError::null_pointer())
    }
    fn byte_vector_val(&mut self, _doc: i32) -> Result<Option<Vec<u8>>> {
        Err(LuceneError::null_pointer())
    }
    fn bytes_val(&mut self, _doc: i32, _target: &mut BytesRefBuilder) -> Result<bool> {
        Err(LuceneError::null_pointer())
    }
    fn object_val(&mut self, _doc: i32) -> Result<Option<MutableValueObject>> {
        Err(LuceneError::null_pointer())
    }
    fn exists(&mut self, _doc: i32) -> Result<bool> {
        Err(LuceneError::null_pointer())
    }
    fn ord_val(&mut self, _doc: i32) -> Result<i32> {
        Err(LuceneError::null_pointer())
    }
    fn num_ord(&self) -> Result<i32> {
        Err(LuceneError::null_pointer())
    }
    fn cost(&self) -> f32 {
        null_dereference()
    }
    fn to_doc_string(&mut self, _doc: i32) -> Result<String> {
        Err(LuceneError::null_pointer())
    }
    fn get_value_filler(&self) -> Box<dyn ValueFiller> {
        null_dereference()
    }
    fn byte_vals(&mut self, _doc: i32, _vals: &mut [i8]) -> Result<()> {
        Err(LuceneError::null_pointer())
    }
    fn short_vals(&mut self, _doc: i32, _vals: &mut [i16]) -> Result<()> {
        Err(LuceneError::null_pointer())
    }
    fn float_vals(&mut self, _doc: i32, _vals: &mut [f32]) -> Result<()> {
        Err(LuceneError::null_pointer())
    }
    fn int_vals(&mut self, _doc: i32, _vals: &mut [i32]) -> Result<()> {
        Err(LuceneError::null_pointer())
    }
    fn long_vals(&mut self, _doc: i32, _vals: &mut [i64]) -> Result<()> {
        Err(LuceneError::null_pointer())
    }
    fn double_vals(&mut self, _doc: i32, _vals: &mut [f64]) -> Result<()> {
        Err(LuceneError::null_pointer())
    }
    fn str_vals(&mut self, _doc: i32, _vals: &mut [Option<String>]) -> Result<()> {
        Err(LuceneError::null_pointer())
    }
    fn explain(&mut self, _doc: i32) -> Result<Explanation> {
        Err(LuceneError::null_pointer())
    }
    fn get_scorer_matcher(&mut self) -> Box<dyn ValueSourceScorerMatcher> {
        null_dereference()
    }
    fn get_range_matcher(
        &mut self,
        _lower_val: Option<&str>,
        _upper_val: Option<&str>,
        _include_lower: bool,
        _include_upper: bool,
    ) -> Result<Box<dyn ValueSourceScorerMatcher>> {
        Err(LuceneError::null_pointer())
    }
}

/// `SumTotalTermFreqValueSource` returns the number of tokens (sum of term
/// frequencies) across all documents in the index.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.SumTotalTermFreqValueSource`.
/// This value source requires that term frequencies are indexed.
#[derive(Debug, Clone)]
pub struct SumTotalTermFreqValueSource {
    /// The protected `indexedField` field.
    indexed_field: String,
}

impl SumTotalTermFreqValueSource {
    /// Equivalent to `SumTotalTermFreqValueSource(String)`.
    pub fn new(indexed_field: impl Into<String>) -> Self {
        Self {
            indexed_field: indexed_field.into(),
        }
    }

    /// Equivalent to `SumTotalTermFreqValueSource.name()`.
    pub fn name(&self) -> String {
        "sumtotaltermfreq".to_string()
    }
}

impl ValueSource for SumTotalTermFreqValueSource {
    fn description(&self) -> String {
        format!("{}({})", self.name(), self.indexed_field)
    }

    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        stored_values(context, self)
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        let mut sum_total_term_freq = 0i64;
        for reader_context in Arc::clone(searcher.get_top_reader_context()).leaves() {
            let terms = get_terms(reader_context.leaf_reader().as_ref(), &self.indexed_field)?;
            let v = terms.sum_total_term_freq()?;
            debug_assert!(v != -1);
            sum_total_term_freq = sum_total_term_freq.wrapping_add(v);
        }
        let ttf = sum_total_term_freq;
        context.put(self, Arc::new(ttf));
        Ok(())
    }

    /// `getClass().hashCode() + indexedField.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        class_hash("org.apache.lucene.queries.function.valuesource.SumTotalTermFreqValueSource")
            .wrapping_add(java_string_hash_code(&self.indexed_field))
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<SumTotalTermFreqValueSource>()
            .is_some_and(|other| self.indexed_field == other.indexed_field)
    }
}
