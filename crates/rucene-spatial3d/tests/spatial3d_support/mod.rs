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

//! Helpers shared by the ported spatial3d test classes: the [`prelude`] the
//! class modules glob-import (the Java package's types, the suite plumbing of
//! `core_support`, the JUnit assertions) and the Java semantics the ported
//! bodies need (auto-unboxing, `String.valueOf(double)`, `Math.toRadians`).

#![allow(dead_code, unused_imports, unused_macros)]

/// Java's `assert condition [: message];` under `-ea` in a method that
/// returns [`rucene::error::Result`]: a failed assertion returns
/// `Err(AssertionError(message))`.
///
/// **Divergence.** Java evaluates `assert` only with `-ea` (as Lucene's
/// builds run tests); the port evaluates it when `debug_assertions` are on.
macro_rules! java_assert {
    ($cond:expr) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(rucene::error::LuceneError::from(
                rucene::error::LuceneErrorKind::AssertionError(String::new()),
            ));
        }
    };
    ($cond:expr, $($msg:tt)+) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(rucene::error::LuceneError::from(
                rucene::error::LuceneErrorKind::AssertionError(format!($($msg)+)),
            ));
        }
    };
}

use rucene::error::{LuceneError, Result};

/// A Java reference that the test dereferences: its value, or Java's
/// `NullPointerException` for `null`.
pub fn npe<T>(value: Option<T>) -> Result<T> {
    value.ok_or_else(LuceneError::null_pointer)
}

/// Auto-unboxing of a `java.lang.Double`: its value, or Java's
/// `NullPointerException` for `null`.
pub fn unbox(value: Option<f64>) -> f64 {
    value.unwrap_or_else(|| {
        rucene_test_framework::util::java_exceptions::rethrow_java(LuceneError::null_pointer())
    })
}

/// `assertNull(object)` / `assertEquals(null, object)`.
pub fn assert_null<T>(object: &Option<T>) {
    if object.is_some() {
        randomizedtesting::junit::Assert::fail("expected null, but was:<not null>");
    }
}

/// `Double.toString(double)`, the form Java string concatenation uses.
pub fn d2s(d: f64) -> String {
    rucene::util::java_fmt::double_to_string(d)
}

/// `String.valueOf(Double)` over a nullable boxed double.
pub fn od2s(d: Option<f64>) -> String {
    d.map_or_else(|| "null".to_string(), d2s)
}

/// `AbstractCollection.toString()` of a list: `[e1, e2, ...]`.
pub fn list_to_string<T: std::fmt::Display>(items: &[T]) -> String {
    let parts: Vec<String> = items.iter().map(|i| i.to_string()).collect();
    format!("[{}]", parts.join(", "))
}

/// `Math.toRadians(double)`: `angdeg * DEGREES_TO_RADIANS` (JDK 25
/// `Math.java`), whose constant is `PI / 180.0`.
pub fn to_radians(angdeg: f64) -> f64 {
    angdeg * (std::f64::consts::PI / 180.0)
}

/// `Math.nextUp(double)`.
pub fn java_next_up(d: f64) -> f64 {
    if d.is_nan() || d == f64::INFINITY {
        d
    } else if d == 0.0 {
        f64::from_bits(1)
    } else if d > 0.0 {
        f64::from_bits(d.to_bits() + 1)
    } else {
        f64::from_bits(d.to_bits() - 1)
    }
}

/// `Math.nextDown(double)`.
pub fn java_next_down(d: f64) -> f64 {
    if d.is_nan() || d == f64::NEG_INFINITY {
        d
    } else if d == 0.0 {
        -f64::from_bits(1)
    } else {
        -java_next_up(-d)
    }
}

/// The names the ported test classes use.
pub mod prelude {
    pub use super::{
        assert_equals_object, assert_equals_object_msg, assert_not_null, assert_null, class_cast,
        d2s, java_next_down, java_next_up, list_to_string, npe, od2s, to_radians, unbox,
    };
    pub use crate::core_support::{class, random, test, verbose};
    pub use crate::geom::geo3_d_util::Geo3DUtil;
    pub use randomizedtesting::annotations::Repeat;
    pub use randomizedtesting::jdk::random::Random;
    pub use randomizedtesting::jdk::reflect::Class;
    pub use randomizedtesting::junit::{Assert, Test};
    pub use randomizedtesting::RandomizedTest;
    pub use rucene::error::{LuceneError, Result};
    pub use rucene_test_framework::geo::geo_test_util as GeoTestUtil;
    pub use rucene_test_framework::util::java_exceptions::expect_throws;
    pub use rucene_test_framework::util::LuceneTestCase;
    pub use std::f64::consts::PI;
    pub use std::sync::{Arc, LazyLock};

    pub use rucene_spatial3d::geom::bounded::Bounded;
    pub use rucene_spatial3d::geom::distance_style::{
        DistanceStyle, ARC, LINEAR, LINEAR_SQUARED, NORMAL, NORMAL_SQUARED,
    };
    pub use rucene_spatial3d::geom::geo_area::{GeoArea, CONTAINS, DISJOINT, OVERLAPS, WITHIN};
    pub use rucene_spatial3d::geom::geo_area_factory::GeoAreaFactory;
    pub use rucene_spatial3d::geom::geo_area_shape::GeoAreaShape;
    pub use rucene_spatial3d::geom::geo_b_box::GeoBBox;
    pub use rucene_spatial3d::geom::geo_b_box_factory::GeoBBoxFactory;
    pub use rucene_spatial3d::geom::geo_circle::GeoCircle;
    pub use rucene_spatial3d::geom::geo_circle_factory::GeoCircleFactory;
    pub use rucene_spatial3d::geom::geo_complex_polygon::GeoComplexPolygon;
    pub use rucene_spatial3d::geom::geo_composite_membership_shape::GeoCompositeMembershipShape;
    pub use rucene_spatial3d::geom::geo_composite_polygon::GeoCompositePolygon;
    pub use rucene_spatial3d::geom::geo_concave_polygon::GeoConcavePolygon;
    pub use rucene_spatial3d::geom::geo_convex_polygon::GeoConvexPolygon;
    pub use rucene_spatial3d::geom::geo_distance::GeoDistance;
    pub use rucene_spatial3d::geom::geo_distance_shape::GeoDistanceShape;
    pub use rucene_spatial3d::geom::geo_exact_circle::GeoExactCircle;
    pub use rucene_spatial3d::geom::geo_membership_shape::GeoMembershipShape;
    pub use rucene_spatial3d::geom::geo_north_rectangle::GeoNorthRectangle;
    pub use rucene_spatial3d::geom::geo_outside_distance::GeoOutsideDistance;
    pub use rucene_spatial3d::geom::geo_path::GeoPath;
    pub use rucene_spatial3d::geom::geo_path_factory::GeoPathFactory;
    pub use rucene_spatial3d::geom::geo_point::GeoPoint;
    pub use rucene_spatial3d::geom::geo_polygon::GeoPolygon;
    pub use rucene_spatial3d::geom::geo_polygon_factory::{
        GeoPolygonFactory, MutableBoolean, PolygonDescription,
    };
    pub use rucene_spatial3d::geom::geo_rectangle::GeoRectangle;
    pub use rucene_spatial3d::geom::geo_shape::GeoShape;
    pub use rucene_spatial3d::geom::geo_sizeable::GeoSizeable;
    pub use rucene_spatial3d::geom::geo_standard_circle::GeoStandardCircle;
    pub use rucene_spatial3d::geom::geo_standard_path::GeoStandardPath;
    pub use rucene_spatial3d::geom::geo_world::GeoWorld;
    pub use rucene_spatial3d::geom::lat_lon_bounds::LatLonBounds;
    pub use rucene_spatial3d::geom::membership::Membership;
    pub use rucene_spatial3d::geom::plane::Plane;
    pub use rucene_spatial3d::geom::planet_model::{PlanetModel, CLARKE_1866, SPHERE, WGS84};
    pub use rucene_spatial3d::geom::planet_object::PlanetObject;
    pub use rucene_spatial3d::geom::serializable_object::{self, SerializableObject};
    pub use rucene_spatial3d::geom::sided_plane::SidedPlane;
    pub use rucene_spatial3d::geom::standard_xyz_solid::StandardXYZSolid;
    pub use rucene_spatial3d::geom::vector::Vector;
    pub use rucene_spatial3d::geom::xyz_bounds::XYZBounds;
    pub use rucene_spatial3d::geom::xyz_solid::XYZSolid;
    pub use rucene_spatial3d::geom::xyz_solid_factory::XYZSolidFactory;
    pub use rucene_spatial3d::geom::JavaBitSet;
    pub use rucene_spatial_test_fixtures::random_geo3d_shape_generator::RandomGeo3dShapeGenerator;
}

/// `(T) object`: the object as the concrete class `T`, or Java's
/// `ClassCastException` (`class X cannot be cast to class Y`).
pub fn class_cast<'a, T: 'static>(
    object: &'a dyn prelude::SerializableObject,
    to: &str,
) -> Result<&'a T> {
    object.as_any().downcast_ref::<T>().ok_or_else(|| {
        LuceneError::unchecked(
            "java.lang.ClassCastException",
            format!(
                "class {} cannot be cast to class {}",
                object.class_name(),
                to
            ),
        )
    })
}

/// `assertNotNull(object)` over a Rust value, which is never `null`: the
/// assertion holds by construction.
pub fn assert_not_null<T: ?Sized>(_object: &T) {}

/// `assertEquals(expected, actual)` over two geo3d objects: Java's
/// `expected.equals(actual)`, failing with JUnit's
/// `expected:<...> but was:<...>` message.
pub fn assert_equals_object(
    expected: &dyn prelude::SerializableObject,
    actual: &dyn prelude::SerializableObject,
) {
    assert_equals_object_msg("", expected, actual);
}

/// `assertEquals(message, expected, actual)` over two geo3d objects: JUnit's
/// `failNotEquals` message, the message (when not empty) then
/// ` expected:<...> but was:<...>`.
pub fn assert_equals_object_msg(
    message: &str,
    expected: &dyn prelude::SerializableObject,
    actual: &dyn prelude::SerializableObject,
) {
    if !expected.java_equals(actual.as_any()) {
        let prefix = if message.is_empty() {
            String::new()
        } else {
            format!("{message} ")
        };
        randomizedtesting::junit::Assert::fail(&format!(
            "{prefix}expected:<{expected}> but was:<{actual}>"
        ));
    }
}
