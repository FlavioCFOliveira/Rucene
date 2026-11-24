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

//! Port of the test classes of Lucene 10.5.0 `lucene/spatial3d/src/test`
//! (rmp #631), each run as a suite by the ported runner. The Java package
//! `org.apache.lucene.spatial3d` is the module [`spatial3d`] and
//! `org.apache.lucene.spatial3d.geom` the module [`geom`].

// The ports keep Java's statements line by line: indexed loops, float
// literals, list building by `add` after `new ArrayList<>()`, late-initialized
// locals, `List` arguments built by `Collections.singletonList`, helper
// methods with many parameters, and `return` of a call that may throw.
#![allow(
    clippy::needless_range_loop,
    clippy::excessive_precision,
    clippy::approx_constant,
    clippy::vec_init_then_push,
    clippy::useless_vec,
    clippy::needless_late_init,
    clippy::too_many_arguments,
    clippy::needless_question_mark
)]

#[path = "../../rucene-test-framework/tests/core_support/mod.rs"]
mod core_support;
#[macro_use]
mod spatial3d_support;

mod geom {
    pub mod geo3_d_util;
    pub mod test_composite_geo_polygon_relationships;
    pub mod test_geo_b_box;
    pub mod test_geo_circle;
    pub mod test_geo_convex_polygon;
    pub mod test_geo_exact_circle;
    pub mod test_geo_model;
    pub mod test_geo_path;
    pub mod test_geo_point;
    pub mod test_geo_polygon;
    pub mod test_plane;
    pub mod test_random_binary_codec;
    pub mod test_random_geo_polygon;
    pub mod test_random_geo_shape_relationship;
    pub mod test_random_plane;
    pub mod test_serializable_object_class_filter;
    pub mod test_simple_geo_polygon_relationships;
    pub mod test_xyz_solid;
}
mod spatial3d {
    pub mod test_geo3_d_doc_values;
    pub mod test_geo3_d_point;
}

use core_support::run;

#[test]
fn test_composite_geo_polygon_relationships() {
    run(geom::test_composite_geo_polygon_relationships::class_def);
}

#[test]
fn test_geo_b_box() {
    run(geom::test_geo_b_box::class_def);
}

#[test]
fn test_geo_circle() {
    run(geom::test_geo_circle::class_def);
}

#[test]
fn test_geo_convex_polygon() {
    run(geom::test_geo_convex_polygon::class_def);
}

#[test]
fn test_geo_exact_circle() {
    run(geom::test_geo_exact_circle::class_def);
}

#[test]
fn test_geo_model() {
    run(geom::test_geo_model::class_def);
}

#[test]
fn test_geo_path() {
    run(geom::test_geo_path::class_def);
}

#[test]
fn test_geo_point() {
    run(geom::test_geo_point::class_def);
}

#[test]
fn test_geo_polygon() {
    run(geom::test_geo_polygon::class_def);
}

#[test]
fn test_plane() {
    run(geom::test_plane::class_def);
}

#[test]
fn test_random_binary_codec() {
    run(geom::test_random_binary_codec::class_def);
}

#[test]
fn test_random_geo_polygon() {
    run(geom::test_random_geo_polygon::class_def);
}

#[test]
fn test_random_geo_shape_relationship() {
    run(geom::test_random_geo_shape_relationship::class_def);
}

#[test]
fn test_random_plane() {
    run(geom::test_random_plane::class_def);
}

#[test]
fn test_serializable_object_class_filter() {
    run(geom::test_serializable_object_class_filter::class_def);
}

#[test]
fn test_simple_geo_polygon_relationships() {
    run(geom::test_simple_geo_polygon_relationships::class_def);
}

#[test]
fn test_xyz_solid() {
    run(geom::test_xyz_solid::class_def);
}

#[test]
fn test_geo3_d_doc_values() {
    run(spatial3d::test_geo3_d_doc_values::class_def);
}

#[test]
fn test_geo3_d_point() {
    run(spatial3d::test_geo3_d_point::class_def);
}
