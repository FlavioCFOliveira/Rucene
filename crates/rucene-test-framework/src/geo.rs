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

//! Port of `org.apache.lucene.tests.geo` (rmp #358): [`geo_test_util`]
//! (`GeoTestUtil`), [`shape_test_util`] (`ShapeTestUtil`),
//! [`earth_debugger`] (`EarthDebugger`), [`base_geo_point_test_case`]
//! (`BaseGeoPointTestCase`), [`base_xy_point_test_case`]
//! (`BaseXYPointTestCase`).
//!
//! The shape resources of the package
//! (`lucene/test-framework/src/resources/org/apache/lucene/tests/geo/*.gz`,
//! Apache License 2.0, Copyright The Apache Software Foundation, see the
//! Lucene `NOTICE.txt`) are copied unchanged under `src/geo/resources/` and
//! read by [`geo_test_util::read_shape`].

pub mod base_geo_point_test_case;
pub mod base_xy_point_test_case;
pub mod earth_debugger;
pub mod geo_test_util;
pub mod shape_test_util;

pub use earth_debugger::EarthDebugger;

/// `Math.min(double, double)` (= `StrictMath.min`): `NaN` when either is
/// `NaN`, and `-0.0` is below `0.0`; `f64::min` returns the other operand for
/// a `NaN` and either zero for `±0.0`.
pub(crate) fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.to_bits() == (-0.0f64).to_bits() {
        return b;
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// `Math.max(double, double)` (= `StrictMath.max`): `NaN` when either is
/// `NaN`, and `0.0` is above `-0.0`.
pub(crate) fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == (-0.0f64).to_bits() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// `StrictMath.min(float, float)`: as [`java_min`] over floats.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `Math.min`/`Math.max` order the zeros and propagate `NaN`.
    #[test]
    fn min_and_max_follow_java_math() {
        assert_eq!(java_min(0.0, -0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(java_min(-0.0, 0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(java_max(-0.0, 0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(java_max(0.0, -0.0).to_bits(), 0.0f64.to_bits());
        assert!(java_min(1.0, f64::NAN).is_nan());
        assert!(java_max(f64::NAN, 1.0).is_nan());
        assert_eq!(java_min(-90.0, 3.0), -90.0);
        assert_eq!(java_max(-90.0, 3.0), 3.0);
        assert_eq!(java_min_f32(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
        assert!(java_min_f32(f32::NAN, 1.0).is_nan());
    }
}
