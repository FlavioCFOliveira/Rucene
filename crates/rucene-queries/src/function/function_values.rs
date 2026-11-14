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

//! Port of `FunctionValues.java`, of the Java package
//! `org.apache.lucene.queries.function`.
//!
//! The module also holds the Java number conversions and parsers the function
//! package reads values through (`Float.parseFloat`, `Integer.parseInt`, the
//! narrowing casts), so that every port in the package converts exactly as the
//! JVM does.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::LeafReaderContext;
use rucene::search::similarities::Explanation;
use rucene::util::java_fmt;
use rucene::util::mutable::{MutableValue, MutableValueFloat, MutableValueObject};
use rucene::util::BytesRefBuilder;

use crate::function::value_source_scorer::{ValueSourceScorer, ValueSourceScorerMatcher};

/// `new UnsupportedOperationException()`, which carries no message.
pub(crate) fn unsupported() -> LuceneError {
    LuceneError::unsupported_operation(String::new())
}

/// `new RuntimeException(message, cause)`.
pub(crate) fn runtime_exception(message: String, cause: LuceneError) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Unchecked {
        class: "java.lang.RuntimeException",
        message: Some(message),
        cause: Some(Arc::new(cause)),
    })
}

/// `Float.parseFloat(String)`.
///
/// # Errors
///
/// The `NumberFormatException` Java throws: `empty String` for a value that
/// trims to nothing, `For input string: "<s>"` otherwise.
pub(crate) fn java_parse_float(s: &str) -> Result<f32> {
    if s.trim_matches(|c: char| c <= '\u{20}').is_empty() {
        return Err(LuceneError::number_format("empty String"));
    }
    java_fmt::parse_float(s)
        .ok_or_else(|| LuceneError::number_format(format!("For input string: \"{s}\"")))
}

/// `Double.parseDouble(String)`.
///
/// # Errors
///
/// As [`java_parse_float`].
pub(crate) fn java_parse_double(s: &str) -> Result<f64> {
    if s.trim_matches(|c: char| c <= '\u{20}').is_empty() {
        return Err(LuceneError::number_format("empty String"));
    }
    java_fmt::parse_double(s)
        .ok_or_else(|| LuceneError::number_format(format!("For input string: \"{s}\"")))
}

/// `Integer.parseInt(String)`.
///
/// # Errors
///
/// The `NumberFormatException` of `NumberFormatException.forInputString`.
pub(crate) fn java_parse_int(s: &str) -> Result<i32> {
    rucene::util::extra::java_parse_int(s)
        .ok_or_else(|| LuceneError::number_format_for_input_string(s, 10))
}

/// `Long.parseLong(String)`.
///
/// # Errors
///
/// The `NumberFormatException` of `NumberFormatException.forInputString`.
pub(crate) fn java_parse_long(s: &str) -> Result<i64> {
    rucene::util::extra::java_parse_long(s)
        .ok_or_else(|| LuceneError::number_format_for_input_string(s, 10))
}

/// `Math.max(float, float)`: `NaN` if either value is `NaN`, and `0.0f` above
/// `-0.0f`.
pub(crate) fn java_max_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == (-0.0f32).to_bits() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// `Math.min(float, float)`: `NaN` if either value is `NaN`, and `-0.0f`
/// below `0.0f`.
pub(crate) fn java_min_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.to_bits() == (-0.0f32).to_bits() {
        return b;
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// `(byte) f`: the JVM narrows a `float` to `int` (saturating, `NaN` to `0`),
/// then the `int` to its low 8 bits.
pub(crate) fn f32_to_byte(f: f32) -> i8 {
    (f as i32) as i8
}

/// `(short) f`, through `int` as [`f32_to_byte`].
pub(crate) fn f32_to_short(f: f32) -> i16 {
    (f as i32) as i16
}

/// `(byte) d`, through `int` as [`f32_to_byte`].
pub(crate) fn f64_to_byte(d: f64) -> i8 {
    (d as i32) as i8
}

/// `(short) d`, through `int` as [`f32_to_byte`].
pub(crate) fn f64_to_short(d: f64) -> i16 {
    (d as i32) as i16
}

/// Represents field values as different types. Normally created via a
/// [`ValueSource`](crate::function::value_source::ValueSource) for a
/// particular field and reader.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.FunctionValues`.
///
/// `FunctionValues` is distinct from `ValueSource` because there needs to be
/// an object created at query evaluation time that is not referenced by the
/// query itself: query objects should be thread-safe, and are often used as
/// cache keys.
///
/// Every method keeps Java's default: the typed accessors throw
/// `UnsupportedOperationException` unless overridden, [`exists`](Self::exists)
/// answers `true` and [`cost`](Self::cost) `100`. The accessors take
/// `&mut self` because the values read forward-only doc-values iterators.
///
/// **Divergences.**
/// * Java's overloads `byteVal(int, byte[])` and the like, which fill an array
///   with the values of a multi-valued function, are the `*_vals` methods.
/// * `Object objectVal(int)` answers the boxed value as a
///   [`MutableValueObject`], the closed set of boxes Lucene's functions
///   return; `None` stands for `null`.
/// * `getRangeScorer` and `getScorer` (`FunctionValues.java:213-299`) build a
///   [`ValueSourceScorer`] that holds `this`. A default trait method cannot
///   move `self` into a returned value, so the overridable part — the
///   `ValueSourceScorer` subclass each override instantiates, which decides
///   what matches — is the hook [`get_range_matcher`](Self::get_range_matcher)
///   (and [`get_scorer_matcher`](Self::get_scorer_matcher)), and the free
///   functions [`get_range_scorer`] and [`get_scorer`] assemble the scorer
///   from the values and that matcher, exactly as every Java override does.
pub trait FunctionValues {
    /// Equivalent to `FunctionValues.byteVal(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn byte_val(&mut self, _doc: i32) -> Result<i8> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.shortVal(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn short_val(&mut self, _doc: i32) -> Result<i16> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.floatVal(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn float_val(&mut self, _doc: i32) -> Result<f32> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.intVal(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn int_val(&mut self, _doc: i32) -> Result<i32> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.longVal(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn long_val(&mut self, _doc: i32) -> Result<i64> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.doubleVal(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn double_val(&mut self, _doc: i32) -> Result<f64> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.strVal(int)`; `None` stands for `null`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn str_val(&mut self, _doc: i32) -> Result<Option<String>> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.boolVal(int)`, `intVal(doc) != 0` by
    /// default.
    ///
    /// # Errors
    ///
    /// Propagates the error of [`int_val`](Self::int_val).
    fn bool_val(&mut self, doc: i32) -> Result<bool> {
        Ok(self.int_val(doc)? != 0)
    }

    /// Equivalent to `FunctionValues.floatVectorVal(int)`; `None` stands for
    /// `null`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn float_vector_val(&mut self, _doc: i32) -> Result<Option<Vec<f32>>> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.byteVectorVal(int)`; `None` stands for
    /// `null`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn byte_vector_val(&mut self, _doc: i32) -> Result<Option<Vec<u8>>> {
        Err(unsupported())
    }

    /// Returns the bytes representation of the string value.
    ///
    /// Equivalent to `FunctionValues.bytesVal(int, BytesRefBuilder)`, which
    /// clears `target` and answers `false` when [`str_val`](Self::str_val) is
    /// `null`, and copies its characters otherwise.
    ///
    /// # Errors
    ///
    /// Propagates the error of [`str_val`](Self::str_val).
    fn bytes_val(&mut self, doc: i32, target: &mut BytesRefBuilder) -> Result<bool> {
        match self.str_val(doc)? {
            None => {
                target.clear();
                Ok(false)
            }
            Some(s) => {
                target.copy_chars(&s);
                Ok(true)
            }
        }
    }

    /// Native Java object representation of the value.
    ///
    /// Equivalent to `FunctionValues.objectVal(int)`, which returns
    /// `floatVal(doc)` boxed by default, since most functions are float
    /// functions.
    ///
    /// # Errors
    ///
    /// Propagates the error of [`float_val`](Self::float_val).
    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        Ok(Some(MutableValueObject::Float(self.float_val(doc)?)))
    }

    /// Returns true if there is a value for this document.
    ///
    /// Equivalent to `FunctionValues.exists(int)`, `true` by default.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error an override raises.
    fn exists(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }

    /// Returns the sort ordinal for the specified doc.
    ///
    /// Equivalent to `FunctionValues.ordVal(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn ord_val(&mut self, _doc: i32) -> Result<i32> {
        Err(unsupported())
    }

    /// Returns the number of unique sort ordinals this instance has.
    ///
    /// Equivalent to `FunctionValues.numOrd()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn num_ord(&self) -> Result<i32> {
        Err(unsupported())
    }

    /// An estimate of the expected cost to return a value for a document, in
    /// simple operations; used by `TwoPhaseIterator::match_cost`
    /// implementations.
    ///
    /// Equivalent to `FunctionValues.cost()`, `100` by default.
    fn cost(&self) -> f32 {
        100.0
    }

    /// Renders the value of `doc`.
    ///
    /// Equivalent to the abstract `FunctionValues.toString(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the value.
    fn to_doc_string(&mut self, doc: i32) -> Result<String>;

    /// Returns the logic that fills the value of a document into a reusable
    /// [`MutableValue`].
    ///
    /// Equivalent to the experimental `FunctionValues.getValueFiller()`,
    /// whose default fills a `MutableValueFloat` with `floatVal(doc)`.
    fn get_value_filler(&self) -> Box<dyn ValueFiller> {
        Box::new(FloatValuesFiller::new(false))
    }

    /// Equivalent to `FunctionValues.byteVal(int, byte[])`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn byte_vals(&mut self, _doc: i32, _vals: &mut [i8]) -> Result<()> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.shortVal(int, short[])`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn short_vals(&mut self, _doc: i32, _vals: &mut [i16]) -> Result<()> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.floatVal(int, float[])`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn float_vals(&mut self, _doc: i32, _vals: &mut [f32]) -> Result<()> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.intVal(int, int[])`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn int_vals(&mut self, _doc: i32, _vals: &mut [i32]) -> Result<()> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.longVal(int, long[])`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn long_vals(&mut self, _doc: i32, _vals: &mut [i64]) -> Result<()> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.doubleVal(int, double[])`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn double_vals(&mut self, _doc: i32, _vals: &mut [f64]) -> Result<()> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.strVal(int, String[])`; `None` entries
    /// stand for `null`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn str_vals(&mut self, _doc: i32, _vals: &mut [Option<String>]) -> Result<()> {
        Err(unsupported())
    }

    /// Equivalent to `FunctionValues.explain(int)`:
    /// `Explanation.match(floatVal(doc), toString(doc))`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the value.
    fn explain(&mut self, doc: i32) -> Result<Explanation> {
        let value = self.float_val(doc)?;
        Ok(Explanation::matched(
            value,
            self.to_doc_string(doc)?,
            Vec::new(),
        ))
    }

    /// The matching logic of the scorer [`get_scorer`] builds: every document
    /// matches, at a match cost of `0`.
    ///
    /// Equivalent to the anonymous `ValueSourceScorer` of
    /// `FunctionValues.getScorer(LeafReaderContext)`; an override of
    /// `getScorer` overrides this hook.
    fn get_scorer_matcher(&mut self) -> Box<dyn ValueSourceScorerMatcher> {
        Box::new(MatchAllMatcher)
    }

    /// The matching logic of the scorer [`get_range_scorer`] builds: documents
    /// whose `floatVal` falls in the range, the bounds parsed with
    /// `Float.parseFloat` and `null` bounds open.
    ///
    /// Equivalent to the four anonymous `ValueSourceScorer`s of
    /// `FunctionValues.getRangeScorer(LeafReaderContext, String, String,
    /// boolean, boolean)` (`FunctionValues.java:234-299`); an override of
    /// `getRangeScorer` overrides this hook.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of a bound that does not parse.
    fn get_range_matcher(
        &mut self,
        lower_val: Option<&str>,
        upper_val: Option<&str>,
        include_lower: bool,
        include_upper: bool,
    ) -> Result<Box<dyn ValueSourceScorerMatcher>> {
        let lower = match lower_val {
            None => f32::NEG_INFINITY,
            Some(s) => java_parse_float(s)?,
        };
        let upper = match upper_val {
            None => f32::INFINITY,
            Some(s) => java_parse_float(s)?,
        };
        Ok(Box::new(FloatRangeMatcher {
            l: lower,
            u: upper,
            include_lower,
            include_upper,
        }))
    }
}

/// Yields a scorer that matches all documents, and that produces scores equal
/// to `floatVal(doc)`.
///
/// Equivalent to `FunctionValues.getScorer(LeafReaderContext)`, assembled from
/// [`FunctionValues::get_scorer_matcher`].
///
/// # Errors
///
/// Propagates the error of building the all-documents approximation.
pub fn get_scorer<'a>(
    mut values: Box<dyn FunctionValues + 'a>,
    reader_context: &LeafReaderContext,
) -> Result<ValueSourceScorer<'a>> {
    let matcher = values.get_scorer_matcher();
    ValueSourceScorer::new(reader_context, values, matcher)
}

/// Yields a scorer that matches documents with values between the specified
/// range, and that produces scores equal to `floatVal(doc)`.
///
/// Equivalent to `FunctionValues.getRangeScorer(LeafReaderContext, String,
/// String, boolean, boolean)`, assembled from
/// [`FunctionValues::get_range_matcher`]; `None` bounds stand for `null`.
///
/// # Errors
///
/// The `NumberFormatException` of a bound that does not parse, and the error
/// of building the all-documents approximation.
pub fn get_range_scorer<'a>(
    mut values: Box<dyn FunctionValues + 'a>,
    reader_context: &LeafReaderContext,
    lower_val: Option<&str>,
    upper_val: Option<&str>,
    include_lower: bool,
    include_upper: bool,
) -> Result<ValueSourceScorer<'a>> {
    let matcher = values.get_range_matcher(lower_val, upper_val, include_lower, include_upper)?;
    ValueSourceScorer::new(reader_context, values, matcher)
}

/// The anonymous `ValueSourceScorer` of `FunctionValues.getScorer`.
#[derive(Debug, Clone, Copy)]
struct MatchAllMatcher;

impl ValueSourceScorerMatcher for MatchAllMatcher {
    fn matches(&mut self, _values: &mut dyn FunctionValues, _doc: i32) -> Result<bool> {
        Ok(true)
    }

    fn match_cost(&self, _values: &dyn FunctionValues) -> f32 {
        0.0
    }
}

/// The four anonymous `ValueSourceScorer`s of
/// `FunctionValues.getRangeScorer`, one per pair of inclusion flags; the flags
/// select the comparison exactly as Java's four classes do.
#[derive(Debug, Clone, Copy)]
struct FloatRangeMatcher {
    l: f32,
    u: f32,
    include_lower: bool,
    include_upper: bool,
}

impl ValueSourceScorerMatcher for FloatRangeMatcher {
    fn matches(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<bool> {
        if !values.exists(doc)? {
            return Ok(false);
        }
        let doc_val = values.float_val(doc)?;
        Ok(match (self.include_lower, self.include_upper) {
            (true, true) => doc_val >= self.l && doc_val <= self.u,
            (true, false) => doc_val >= self.l && doc_val < self.u,
            (false, true) => doc_val > self.l && doc_val <= self.u,
            (false, false) => doc_val > self.l && doc_val < self.u,
        })
    }
}

/// Abstraction of the logic required to fill the value of a specified doc into
/// a reusable [`MutableValue`].
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.FunctionValues.ValueFiller`.
///
/// **Divergence.** Java's fillers are anonymous inner classes of the
/// `FunctionValues` that created them and read its accessors through the
/// captured outer instance. A Rust value cannot keep a borrow of the
/// `FunctionValues` it was obtained from while that one stays usable, so
/// [`fill_value`](Self::fill_value) receives those values — the same object
/// Java's filler captures — as a parameter.
pub trait ValueFiller {
    /// The value, reused across calls.
    ///
    /// Equivalent to `ValueFiller.getValue()`.
    fn get_value(&self) -> &dyn MutableValue;

    /// Fills the value of `doc`, read from `values`.
    ///
    /// Equivalent to `ValueFiller.fillValue(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the value.
    fn fill_value(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<()>;
}

/// The filler of `FunctionValues.getValueFiller()` and
/// `FloatDocValues.getValueFiller()`: a `MutableValueFloat` set to
/// `floatVal(doc)` and, for the latter, its `exists` flag to `exists(doc)`.
#[derive(Debug, Clone)]
pub(crate) struct FloatValuesFiller {
    mval: MutableValueFloat,
    with_exists: bool,
}

impl FloatValuesFiller {
    /// A filler that also sets `exists` when `with_exists` is true.
    pub(crate) fn new(with_exists: bool) -> Self {
        Self {
            mval: MutableValueFloat::new(),
            with_exists,
        }
    }
}

impl ValueFiller for FloatValuesFiller {
    fn get_value(&self) -> &dyn MutableValue {
        &self.mval
    }

    fn fill_value(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<()> {
        self.mval.value = values.float_val(doc)?;
        if self.with_exists {
            self.mval.exists = values.exists(doc)?;
        }
        Ok(())
    }
}
