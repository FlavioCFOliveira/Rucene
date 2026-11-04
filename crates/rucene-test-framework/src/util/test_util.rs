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

//! The members of `org.apache.lucene.tests.util.TestUtil` that
//! `LuceneTestCase`'s static API calls, ported in full under the sprint's
//! cycle rule; task #342 ports the rest of the class.

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::Random;

/// Port of `org.apache.lucene.tests.util.TestUtil` (the members
/// `LuceneTestCase` calls).
pub struct TestUtil;

impl TestUtil {
    /// `nextInt(r, start, end)`: a number in `[start, end]`,
    /// `RandomNumbers.randomIntBetween(r, start, end)`.
    ///
    /// # Panics
    ///
    /// As `RandomNumbers.randomIntBetween`.
    pub fn next_int<R: Random + ?Sized>(r: &R, start: i32, end: i32) -> i32 {
        RandomNumbers::random_int_between(r, start, end)
    }

    /// `floatUlpEquals(x, y, maxUlps)`: whether `x` and `y` are within
    /// `maxUlps` units in the last place, over their raw bits.
    pub fn float_ulp_equals(x: f32, y: f32, max_ulps: i16) -> bool {
        let x_int = x.to_bits() as i32;
        let y_int = y.to_bits() as i32;
        let max_ulps = i32::from(max_ulps);
        if (x_int ^ y_int) < 0 {
            // Numbers have opposite signs, take care of overflow. Remove the
            // sign bit to obtain the absolute ULP above zero.
            let delta_plus = x_int & i32::MAX;
            let delta_minus = y_int & i32::MAX;
            // If either value is NaN, the exponent bits are set to (255 << 23)
            // and the distance above 0.0 is always above a short ULP error.
            // Avoid possible overflow from adding the deltas by splitting the
            // comparison.
            return delta_plus <= max_ulps && delta_minus <= (max_ulps - delta_plus);
        }
        // Numbers have same sign, there is no risk of overflow.
        x_int.wrapping_sub(y_int).wrapping_abs() <= max_ulps && !x.is_nan() && !y.is_nan()
    }

    /// `doubleUlpEquals(x, y, maxUlps)`.
    pub fn double_ulp_equals(x: f64, y: f64, max_ulps: i32) -> bool {
        let x_int = x.to_bits() as i64;
        let y_int = y.to_bits() as i64;
        let max_ulps = i64::from(max_ulps);
        if (x_int ^ y_int) < 0 {
            let delta_plus = x_int & i64::MAX;
            let delta_minus = y_int & i64::MAX;
            return delta_plus <= max_ulps && delta_minus <= (max_ulps - delta_plus);
        }
        x_int.wrapping_sub(y_int).wrapping_abs() <= max_ulps && !x.is_nan() && !y.is_nan()
    }
}
