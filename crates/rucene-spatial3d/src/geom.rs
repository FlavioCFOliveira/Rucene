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

//! The Java package `org.apache.lucene.spatial3d.geom`.

// ---------------------------------------------------------------------------
// The shared implementation of the package's abstract classes, as macros that
// implement the interfaces of each concrete class. They are defined before
// the modules so that every module can use them.
// ---------------------------------------------------------------------------

/// Implements `SerializableObject` (the `Object` behaviour and the casts) for
/// a concrete class with inherent `write_impl`, `equals_impl(&Self)` and
/// `hash_code_impl`.
macro_rules! impl_serializable_object {
    ($t:ty, $name:literal $(, $cast:ident)*) => {
        impl $crate::geom::serializable_object::SerializableObject for $t {
            fn write(&self, output: &mut dyn std::io::Write) -> rucene::error::Result<()> {
                self.write_impl(output)
            }
            fn class_name(&self) -> &'static str {
                $name
            }
            fn java_equals(&self, o: &dyn std::any::Any) -> bool {
                o.downcast_ref::<$t>().is_some_and(|other| self.equals_impl(other))
            }
            fn java_hash_code(&self) -> i32 {
                self.hash_code_impl()
            }
            $( impl_serializable_object!(@cast $cast); )*
        }
    };
    (@cast planet_object) => {
        fn into_planet_object(
            self: std::sync::Arc<Self>,
        ) -> Option<std::sync::Arc<dyn $crate::geom::planet_object::PlanetObject>> {
            Some(self)
        }
    };
    (@cast geo_polygon) => {
        fn into_geo_polygon(
            self: std::sync::Arc<Self>,
        ) -> Option<std::sync::Arc<dyn $crate::geom::geo_polygon::GeoPolygon>> {
            Some(self)
        }
    };
    (@cast geo_membership_shape) => {
        fn into_geo_membership_shape(
            self: std::sync::Arc<Self>,
        ) -> Option<std::sync::Arc<dyn $crate::geom::geo_membership_shape::GeoMembershipShape>> {
            Some(self)
        }
    };
    (@cast geo_b_box) => {
        fn into_geo_b_box(
            self: std::sync::Arc<Self>,
        ) -> Option<std::sync::Arc<dyn $crate::geom::geo_b_box::GeoBBox>> {
            Some(self)
        }
    };
    (@cast geo_circle) => {
        fn into_geo_circle(
            self: std::sync::Arc<Self>,
        ) -> Option<std::sync::Arc<dyn $crate::geom::geo_circle::GeoCircle>> {
            Some(self)
        }
    };
    (@cast geo_area_shape) => {
        fn into_geo_area_shape(
            self: std::sync::Arc<Self>,
        ) -> Option<std::sync::Arc<dyn $crate::geom::geo_area_shape::GeoAreaShape>> {
            Some(self)
        }
    };
}

/// `BasePlanetObject`: `getPlanetModel()` over the `planet_model` field.
macro_rules! impl_planet_object {
    ($t:ty) => {
        impl $crate::geom::planet_object::PlanetObject for $t {
            fn get_planet_model(&self) -> &std::sync::Arc<$crate::geom::planet_model::PlanetModel> {
                &self.planet_model
            }
        }
    };
}

/// `GeoBaseBounds implements GeoBounds`.
macro_rules! impl_geo_bounds {
    ($t:ty) => {
        impl $crate::geom::geo_bounds::GeoBounds for $t {}
    };
}

/// `GeoBaseMembershipShape`: `computeOutsideDistance` over the inherent
/// `outside_distance`, and the `GeoMembershipShape` marker.
macro_rules! impl_geo_base_membership_shape {
    ($t:ty) => {
        impl $crate::geom::geo_outside_distance::GeoOutsideDistance for $t {
            fn compute_outside_distance_xyz(
                &self,
                distance_style: &dyn $crate::geom::distance_style::DistanceStyle,
                x: f64,
                y: f64,
                z: f64,
            ) -> rucene::error::Result<f64> {
                if $crate::geom::membership::Membership::is_within_xyz(self, x, y, z) {
                    return Ok(0.0);
                }
                self.outside_distance(distance_style, x, y, z)
            }
        }
        impl $crate::geom::geo_membership_shape::GeoMembershipShape for $t {}
    };
}

/// `GeoBaseAreaShape.getRelationship(GeoShape)`.
macro_rules! impl_geo_base_area_relationship {
    ($t:ty) => {
        impl $crate::geom::geo_area::GeoArea for $t {
            fn get_relationship(
                &self,
                shape: &dyn $crate::geom::geo_shape::GeoShape,
            ) -> rucene::error::Result<i32> {
                $crate::geom::geo_base_area_shape::GeoBaseAreaShape::get_relationship(self, shape)
            }
        }
    };
}

/// `GeoBaseDistanceShape`: `computeDistance`, `computeDeltaDistance` and
/// `getDistanceBounds` over the inherent `distance`, `delta_distance` and
/// `distance_bounds`.
macro_rules! impl_geo_base_distance_shape {
    ($t:ty) => {
        impl $crate::geom::geo_distance::GeoDistance for $t {
            fn compute_distance_xyz(
                &self,
                distance_style: &dyn $crate::geom::distance_style::DistanceStyle,
                x: f64,
                y: f64,
                z: f64,
            ) -> rucene::error::Result<f64> {
                if !$crate::geom::membership::Membership::is_within_xyz(self, x, y, z) {
                    return Ok(f64::INFINITY);
                }
                self.distance(distance_style, x, y, z)
            }
            fn compute_delta_distance_xyz(
                &self,
                distance_style: &dyn $crate::geom::distance_style::DistanceStyle,
                x: f64,
                y: f64,
                z: f64,
            ) -> rucene::error::Result<f64> {
                if !$crate::geom::membership::Membership::is_within_xyz(self, x, y, z) {
                    return Ok(f64::INFINITY);
                }
                self.delta_distance(distance_style, x, y, z)
            }
        }
        impl $crate::geom::geo_distance_shape::GeoDistanceShape for $t {
            fn get_distance_bounds(
                &self,
                bounds: &mut dyn $crate::geom::bounds::Bounds,
                distance_style: &dyn $crate::geom::distance_style::DistanceStyle,
                distance_value: f64,
            ) -> rucene::error::Result<()> {
                if distance_value == f64::INFINITY {
                    $crate::geom::bounded::Bounded::get_bounds(self, bounds);
                    return Ok(());
                }
                self.distance_bounds(bounds, distance_style, distance_value)
            }
        }
    };
}

/// The interfaces every `GeoBaseAreaShape` subclass implements the same
/// way: `SerializableObject` (with the `PlanetObject`, `GeoMembershipShape`
/// and `GeoAreaShape` casts plus the extra ones listed), `PlanetObject`,
/// `GeoBounds` and `GeoBaseMembershipShape`'s `GeoOutsideDistance`.
macro_rules! impl_geo_base_area_shape_common {
    ($t:ty, $name:literal $(, $cast:ident)*) => {
        impl_serializable_object!($t, $name, planet_object, geo_membership_shape, geo_area_shape $(, $cast)*);
        impl_planet_object!($t);
        impl_geo_bounds!($t);
        impl_geo_base_membership_shape!($t);
    };
}

pub mod arc_distance;
pub mod base_planet_object;
pub mod base_xyz_solid;
pub mod bounded;
pub mod bounds;
pub mod d_x_yd_z_solid;
pub mod d_xd_yd_z_solid;
pub mod d_xd_yz_solid;
pub mod d_xyz_solid;
pub mod distance_style;
pub mod geo_area;
pub mod geo_area_factory;
pub mod geo_area_shape;
pub mod geo_b_box;
pub mod geo_b_box_factory;
pub mod geo_base_area_shape;
pub mod geo_base_b_box;
pub mod geo_base_bounds;
pub mod geo_base_circle;
pub mod geo_base_composite_area_shape;
pub mod geo_base_composite_membership_shape;
pub mod geo_base_composite_shape;
pub mod geo_base_distance_shape;
pub mod geo_base_membership_shape;
pub mod geo_base_path;
pub mod geo_base_polygon;
pub mod geo_base_shape;
pub mod geo_bounds;
pub mod geo_circle;
pub mod geo_circle_factory;
pub mod geo_complex_polygon;
pub mod geo_composite_area_shape;
pub mod geo_composite_membership_shape;
pub mod geo_composite_polygon;
pub mod geo_concave_polygon;
pub mod geo_convex_polygon;
pub mod geo_degenerate_horizontal_line;
pub mod geo_degenerate_latitude_zone;
pub mod geo_degenerate_longitude_slice;
pub mod geo_degenerate_path;
pub mod geo_degenerate_point;
pub mod geo_degenerate_vertical_line;
pub mod geo_distance;
pub mod geo_distance_shape;
pub mod geo_exact_circle;
pub mod geo_latitude_zone;
pub mod geo_longitude_slice;
pub mod geo_membership_shape;
pub mod geo_north_latitude_zone;
pub mod geo_north_rectangle;
pub mod geo_outside_distance;
pub mod geo_path;
pub mod geo_path_factory;
pub mod geo_point;
pub mod geo_point_shape;
pub mod geo_point_shape_factory;
pub mod geo_polygon;
pub mod geo_polygon_factory;
pub mod geo_rectangle;
pub mod geo_s2_shape;
pub mod geo_s2_shape_factory;
pub mod geo_shape;
pub mod geo_sizeable;
pub mod geo_south_latitude_zone;
pub mod geo_south_rectangle;
pub mod geo_standard_circle;
pub mod geo_standard_path;
pub mod geo_wide_degenerate_horizontal_line;
pub mod geo_wide_longitude_slice;
pub mod geo_wide_north_rectangle;
pub mod geo_wide_rectangle;
pub mod geo_wide_south_rectangle;
pub mod geo_world;
pub mod lat_lon_bounds;
pub mod linear_distance;
pub mod linear_squared_distance;
pub mod membership;
pub mod normal_distance;
pub mod normal_squared_distance;
pub mod plane;
pub mod planet_model;
pub mod planet_object;
pub mod serializable_object;
pub mod sided_plane;
pub mod standard_objects;
pub mod standard_xyz_solid;
pub mod tools;
pub mod vector;
pub mod x_yd_z_solid;
pub mod xd_yd_z_solid;
pub mod xd_yz_solid;
pub mod xyz_bounds;
pub mod xyz_solid;
pub mod xyz_solid_factory;

// ---------------------------------------------------------------------------
// JDK behaviours the package relies on, shared by its modules.
// ---------------------------------------------------------------------------

use std::fmt::Write as _;

/// `Math.min(double, double)`: `NaN` wins and `-0.0` is smaller than `0.0`.
#[inline]
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

/// `Math.max(double, double)`: `NaN` wins and `0.0` is larger than `-0.0`.
#[inline]
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

/// `Math.signum(double)`: zero and `NaN` are returned unchanged.
#[inline]
pub(crate) fn java_signum(d: f64) -> f64 {
    if d == 0.0 || d.is_nan() {
        d
    } else if d > 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// `Double.toString(double)`, the form Java string concatenation uses.
#[inline]
pub(crate) fn d2s(d: f64) -> String {
    rucene::util::java_fmt::double_to_string(d)
}

/// `String.valueOf(Double)` over a nullable boxed double.
#[inline]
pub(crate) fn od2s(d: Option<f64>) -> String {
    match d {
        Some(v) => d2s(v),
        None => "null".to_string(),
    }
}

/// `Double.hashCode(double)`, also the `(int) (temp ^ (temp >>> 32))` idiom
/// over `Double.doubleToLongBits`.
#[inline]
pub(crate) fn dhash(d: f64) -> i32 {
    rucene::util::java_collections::java_double_hash_code(d)
}

/// `Double.compare(a, b) == 0`.
#[inline]
pub(crate) fn double_compare_eq(a: f64, b: f64) -> bool {
    rucene::util::java_double_compare(a, b) == std::cmp::Ordering::Equal
}

/// `new IllegalArgumentException(message)`.
#[inline]
pub(crate) fn iae(message: impl Into<String>) -> rucene::error::LuceneError {
    rucene::error::LuceneError::illegal_argument(message)
}

/// `new IllegalStateException(message)`.
#[inline]
pub(crate) fn ise(message: impl Into<String>) -> rucene::error::LuceneError {
    rucene::error::LuceneError::illegal_state(message)
}

/// `new IOException(message)`.
#[inline]
pub(crate) fn ioe(message: impl Into<String>) -> rucene::error::LuceneError {
    rucene::error::LuceneError::from(std::io::Error::other(message.into()))
}

/// `IllegalArgumentException.getMessage()`, `None` for another error.
pub(crate) fn iae_message(error: &rucene::error::LuceneError) -> Option<String> {
    match error.kind() {
        rucene::error::LuceneErrorKind::IllegalArgument { message, .. } => Some(message.clone()),
        _ => None,
    }
}

/// `new RuntimeException(message)`.
#[inline]
pub(crate) fn rte(message: impl Into<String>) -> rucene::error::LuceneError {
    rucene::error::LuceneError::unchecked("java.lang.RuntimeException", message)
}

/// The `NullPointerException` Java raises when it dereferences a `null` the
/// port carries as `None`.
#[track_caller]
pub(crate) fn npe() -> ! {
    panic!("java.lang.NullPointerException")
}

/// The unchecked exception Java lets escape from a method whose Rust
/// counterpart returns no `Result`: the JVM ends the calling thread's
/// operation with it, the port panics with the same class and message.
///
/// The panic message is `java.lang.<Class>: <message>`, the form the test
/// framework turns back into the Java throwable.
#[track_caller]
pub(crate) fn unchecked_panic(error: rucene::error::LuceneError) -> ! {
    use rucene::error::LuceneErrorKind;
    match error.kind() {
        LuceneErrorKind::IllegalArgument { message, .. } => {
            panic!("java.lang.IllegalArgumentException: {}", message)
        }
        LuceneErrorKind::IllegalState { message, .. } => {
            panic!("java.lang.IllegalStateException: {}", message)
        }
        LuceneErrorKind::Unchecked {
            class,
            message: Some(message),
            ..
        } => panic!("{}: {}", class, message),
        _ => panic!("{}", error),
    }
}

/// `Arrays.toString(Object[])` / `AbstractCollection.toString()` over the
/// `toString()` of each element.
pub(crate) fn list_to_string<T: std::fmt::Display>(items: &[T]) -> String {
    let mut s = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        let _ = write!(s, "{}", item);
    }
    s.push(']');
    s
}

/// `List.hashCode()`: `31 * h + e.hashCode()` from `1`.
pub(crate) fn list_hash_code<T>(items: &[T], hash: impl Fn(&T) -> i32) -> i32 {
    let mut h: i32 = 1;
    for item in items {
        h = h.wrapping_mul(31).wrapping_add(hash(item));
    }
    h
}

/// The subset of `java.util.BitSet` the package uses: `get`, `set`,
/// `toByteArray`, `valueOf(byte[])`, `equals` and `hashCode`.
///
/// It is public because the public constructors `GeoConvexPolygon(PlanetModel,
/// List<GeoPoint>, BitSet, boolean)` and their `GeoConcavePolygon`
/// counterparts take a `java.util.BitSet`, as the package's tests do (hidden
/// from the documentation).
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct JavaBitSet {
    words: Vec<u64>,
}

impl JavaBitSet {
    /// `new BitSet()`.
    pub fn new() -> Self {
        Self { words: Vec::new() }
    }

    fn words_in_use(&self) -> usize {
        let mut n = self.words.len();
        while n > 0 && self.words[n - 1] == 0 {
            n -= 1;
        }
        n
    }

    /// `BitSet.get(int)`.
    pub fn get(&self, index: usize) -> bool {
        let w = index >> 6;
        w < self.words.len() && (self.words[w] & (1u64 << (index & 63))) != 0
    }

    /// `BitSet.set(int)`.
    pub fn set(&mut self, index: usize) {
        let w = index >> 6;
        if w >= self.words.len() {
            self.words.resize(w + 1, 0);
        }
        self.words[w] |= 1u64 << (index & 63);
    }

    /// `BitSet.set(int, boolean)`.
    pub fn set_value(&mut self, index: usize, value: bool) {
        if value {
            self.set(index);
        } else {
            let w = index >> 6;
            if w < self.words.len() {
                self.words[w] &= !(1u64 << (index & 63));
            }
        }
    }

    /// `BitSet.toByteArray()`: little-endian, without trailing zero bytes.
    pub(crate) fn to_byte_array(&self) -> Vec<u8> {
        let n = self.words_in_use();
        let mut bytes = Vec::with_capacity(n * 8);
        for w in &self.words[..n] {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        bytes
    }

    /// `BitSet.valueOf(byte[])`.
    pub(crate) fn value_of(bytes: &[u8]) -> Self {
        let mut words = Vec::with_capacity(bytes.len().div_ceil(8));
        for chunk in bytes.chunks(8) {
            let mut b = [0u8; 8];
            b[..chunk.len()].copy_from_slice(chunk);
            words.push(u64::from_le_bytes(b));
        }
        let mut s = Self { words };
        let n = s.words_in_use();
        s.words.truncate(n);
        s
    }

    /// `BitSet.equals(Object)`.
    pub(crate) fn java_equals(&self, other: &JavaBitSet) -> bool {
        let n = self.words_in_use();
        n == other.words_in_use() && self.words[..n] == other.words[..n]
    }
}

impl std::fmt::Display for JavaBitSet {
    /// `BitSet.toString()`: the indexes of the set bits, `{1, 3}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("{")?;
        let mut first = true;
        for (wi, w) in self.words.iter().enumerate() {
            for b in 0..64 {
                if w & (1u64 << b) != 0 {
                    if !first {
                        f.write_str(", ")?;
                    }
                    first = false;
                    write!(f, "{}", wi * 64 + b)?;
                }
            }
        }
        f.write_str("}")
    }
}
