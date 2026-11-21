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

//! Port of `IntervalScoreFunction.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::similarities::{Explanation, SimScorer};
use rucene::util::java_collections::java_float_hash_code;
use rucene::util::java_fmt::float_to_string;

/// `java.lang.Float.compare(float, float)`.
fn float_compare(a: f32, b: f32) -> std::cmp::Ordering {
    let bits = |v: f32| -> i32 {
        if v.is_nan() {
            0x7fc0_0000
        } else {
            v.to_bits() as i32
        }
    };
    if a < b {
        std::cmp::Ordering::Less
    } else if a > b {
        std::cmp::Ordering::Greater
    } else {
        bits(a).cmp(&bits(b))
    }
}

/// The function that turns the sloppy frequency of an interval query into a
/// score.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.IntervalScoreFunction`, with its two
/// subclasses [`SaturationFunction`] and [`SigmoidFunction`].
pub(crate) trait IntervalScoreFunction: Debug + Send + Sync + Any {
    /// Equivalent to the abstract `IntervalScoreFunction.scorer(float)`.
    fn scorer(&self, weight: f32) -> Box<dyn SimScorer + Send + Sync>;

    /// Equivalent to the abstract
    /// `IntervalScoreFunction.explain(String, float, float)`.
    fn explain(&self, interval: &str, weight: f32, sloppy_freq: f32) -> Explanation;

    /// Equivalent to the abstract `IntervalScoreFunction.equals(Object)`.
    fn function_eq(&self, other: &dyn IntervalScoreFunction) -> bool;

    /// Equivalent to the abstract `IntervalScoreFunction.hashCode()`.
    fn function_hash(&self) -> i32;

    /// Equivalent to the abstract `IntervalScoreFunction.toString()`.
    fn function_to_string(&self) -> String;

    /// The function as [`Any`], for `equals`.
    fn as_any(&self) -> &dyn Any;
}

/// Equivalent to the package-private static
/// `IntervalScoreFunction.saturationFunction(float)`.
///
/// # Errors
///
/// `IllegalArgumentException` unless `pivot` is finite and positive.
pub(crate) fn saturation_function(pivot: f32) -> Result<Arc<dyn IntervalScoreFunction>> {
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if pivot <= 0.0 || !pivot.is_finite() {
        return Err(LuceneError::illegal_argument(format!(
            "pivot must be > 0, got: {}",
            float_to_string(pivot)
        )));
    }
    Ok(Arc::new(SaturationFunction::new(pivot)))
}

/// Equivalent to the package-private static
/// `IntervalScoreFunction.sigmoidFunction(float, float)`.
///
/// # Errors
///
/// `IllegalArgumentException` unless `pivot` and `exp` are finite and
/// positive.
pub(crate) fn sigmoid_function(pivot: f32, exp: f32) -> Result<Arc<dyn IntervalScoreFunction>> {
    if pivot <= 0.0 || !pivot.is_finite() {
        return Err(LuceneError::illegal_argument(format!(
            "pivot must be > 0, got: {}",
            float_to_string(pivot)
        )));
    }
    if exp <= 0.0 || !exp.is_finite() {
        return Err(LuceneError::illegal_argument(format!(
            "exp must be > 0, got: {}",
            float_to_string(exp)
        )));
    }
    Ok(Arc::new(SigmoidFunction::new(pivot, exp)))
}

/// `w * S / (S + k)`, rewritten so as not to decrease with `S` in spite of
/// rounding.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.intervals.IntervalScoreFunction.SaturationFunction`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SaturationFunction {
    pivot: f32,
}

impl SaturationFunction {
    /// Equivalent to the private `SaturationFunction(float)`.
    pub(crate) fn new(pivot: f32) -> Self {
        Self { pivot }
    }
}

/// The anonymous `SimScorer` of `SaturationFunction.scorer(float)`.
#[derive(Debug, Clone, Copy)]
struct SaturationScorer {
    weight: f32,
    pivot: f32,
}

impl SimScorer for SaturationScorer {
    fn score(&self, freq: f32, _norm: i64) -> f32 {
        // should be f / (f + k) but we rewrite it to
        // 1 - k / (f + k) to make sure it doesn't decrease
        // with f in spite of rounding
        self.weight * (1.0f32 - self.pivot / (self.pivot + freq))
    }
}

impl IntervalScoreFunction for SaturationFunction {
    fn scorer(&self, weight: f32) -> Box<dyn SimScorer + Send + Sync> {
        Box::new(SaturationScorer {
            weight,
            pivot: self.pivot,
        })
    }

    fn explain(&self, interval: &str, weight: f32, sloppy_freq: f32) -> Explanation {
        let score = self.scorer(weight).score(sloppy_freq, 1);
        Explanation::matched(
            score,
            "Saturation function on interval frequency, computed as w * S / (S + k) from:",
            vec![
                Explanation::matched(weight, "w, weight of this function", Vec::new()),
                Explanation::matched(
                    self.pivot,
                    "k, pivot feature value that would give a score contribution equal to w/2",
                    Vec::new(),
                ),
                Explanation::matched(
                    sloppy_freq,
                    format!("S, the sloppy frequency of the interval query {interval}"),
                    Vec::new(),
                ),
            ],
        )
    }

    /// Equivalent to `SaturationFunction.equals(Object)`: same class and
    /// `Float.compare(that.pivot, pivot) == 0`.
    fn function_eq(&self, other: &dyn IntervalScoreFunction) -> bool {
        other
            .as_any()
            .downcast_ref::<SaturationFunction>()
            .is_some_and(|that| float_compare(that.pivot, self.pivot).is_eq())
    }

    /// `Objects.hash(pivot)`.
    ///
    /// Equivalent to `SaturationFunction.hashCode()`.
    fn function_hash(&self) -> i32 {
        31i32.wrapping_add(java_float_hash_code(self.pivot))
    }

    /// Equivalent to `SaturationFunction.toString()`:
    /// `"SaturationFunction(pivot=" + pivot + ")"`.
    fn function_to_string(&self) -> String {
        format!("SaturationFunction(pivot={})", float_to_string(self.pivot))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `w * S^a / (S^a + k^a)`, rewritten so as not to decrease with `S` in spite
/// of rounding.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.intervals.IntervalScoreFunction.SigmoidFunction`.
///
/// `Math.pow` is computed with [`f64::powf`], as the core's feature functions
/// do; both are allowed one ulp of error, so a JVM and this port can differ in
/// the last bit of a `double` power, as two JVMs on different hosts can.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SigmoidFunction {
    pivot: f32,
    a: f32,
    pivot_pa: f64,
}

impl SigmoidFunction {
    /// Equivalent to the private `SigmoidFunction(float, float)`.
    pub(crate) fn new(pivot: f32, a: f32) -> Self {
        Self {
            pivot,
            a,
            pivot_pa: f64::from(pivot).powf(f64::from(a)),
        }
    }
}

/// The anonymous `SimScorer` of `SigmoidFunction.scorer(float)`.
#[derive(Debug, Clone, Copy)]
struct SigmoidScorer {
    weight: f32,
    a: f32,
    pivot_pa: f64,
}

impl SimScorer for SigmoidScorer {
    fn score(&self, freq: f32, _norm: i64) -> f32 {
        // should be f^a / (f^a + k^a) but we rewrite it to
        // 1 - k^a / (f + k^a) to make sure it doesn't decrease
        // with f in spite of rounding
        (f64::from(self.weight)
            * (1.0f64 - self.pivot_pa / (f64::from(freq).powf(f64::from(self.a)) + self.pivot_pa)))
            as f32
    }
}

impl IntervalScoreFunction for SigmoidFunction {
    fn scorer(&self, weight: f32) -> Box<dyn SimScorer + Send + Sync> {
        Box::new(SigmoidScorer {
            weight,
            a: self.a,
            pivot_pa: self.pivot_pa,
        })
    }

    fn explain(&self, interval: &str, weight: f32, sloppy_freq: f32) -> Explanation {
        let score = self.scorer(weight).score(sloppy_freq, 1);
        Explanation::matched(
            score,
            "Sigmoid function on interval frequency, computed as w * S^a / (S^a + k^a) from:",
            vec![
                Explanation::matched(weight, "w, weight of this function", Vec::new()),
                Explanation::matched(
                    self.pivot,
                    "k, pivot feature value that would give a score contribution equal to w/2",
                    Vec::new(),
                ),
                Explanation::matched(
                    self.a,
                    "a, exponent, higher values make the function grow slower before k and faster after k",
                    Vec::new(),
                ),
                Explanation::matched(
                    sloppy_freq,
                    format!("S, the sloppy frequency of the interval query {interval}"),
                    Vec::new(),
                ),
            ],
        )
    }

    /// Equivalent to `SigmoidFunction.equals(Object)`: same class,
    /// `Float.compare(that.pivot, pivot) == 0` and
    /// `Float.compare(that.a, a) == 0`.
    fn function_eq(&self, other: &dyn IntervalScoreFunction) -> bool {
        other
            .as_any()
            .downcast_ref::<SigmoidFunction>()
            .is_some_and(|that| {
                float_compare(that.pivot, self.pivot).is_eq()
                    && float_compare(that.a, self.a).is_eq()
            })
    }

    /// `Objects.hash(pivot, a)`.
    ///
    /// Equivalent to `SigmoidFunction.hashCode()`.
    fn function_hash(&self) -> i32 {
        (31i32.wrapping_add(java_float_hash_code(self.pivot)))
            .wrapping_mul(31)
            .wrapping_add(java_float_hash_code(self.a))
    }

    /// Equivalent to `SigmoidFunction.toString()`:
    /// `"SigmoidFunction(pivot=" + pivot + ", a=" + a + ")"`.
    fn function_to_string(&self) -> String {
        format!(
            "SigmoidFunction(pivot={}, a={})",
            float_to_string(self.pivot),
            float_to_string(self.a)
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod function_member_tests {
    use super::*;

    /// #596: `SaturationFunction`/`SigmoidFunction` `equals` (same class,
    /// `Float.compare` of each parameter) and `toString`
    /// (`IntervalScoreFunction.java:91-106, 150-165`).
    #[test]
    fn saturation_and_sigmoid_equals_and_to_string_match_java() {
        let saturation = saturation_function(2.5).unwrap();
        assert_eq!(
            saturation.function_to_string(),
            "SaturationFunction(pivot=2.5)"
        );
        assert!(saturation.function_eq(saturation_function(2.5).unwrap().as_ref()));
        assert!(!saturation.function_eq(saturation_function(3.0).unwrap().as_ref()));

        let sigmoid = sigmoid_function(1.0, 0.5).unwrap();
        assert_eq!(
            sigmoid.function_to_string(),
            "SigmoidFunction(pivot=1.0, a=0.5)"
        );
        assert!(sigmoid.function_eq(sigmoid_function(1.0, 0.5).unwrap().as_ref()));
        assert!(!sigmoid.function_eq(sigmoid_function(1.0, 0.75).unwrap().as_ref()));
        assert!(!sigmoid.function_eq(sigmoid_function(2.0, 0.5).unwrap().as_ref()));
        assert!(!sigmoid.function_eq(saturation.as_ref()));
        assert!(!saturation.function_eq(sigmoid.as_ref()));
    }
}
