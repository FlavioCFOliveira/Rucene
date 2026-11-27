//! The Java package `org.locationtech.spatial4j.shape`.

pub mod base_shape;
pub mod circle;
pub mod r#impl;
pub mod point;
#[cfg(feature = "test-helpers")]
pub mod randomized_shape_test;
#[cfg(feature = "test-helpers")]
pub mod rect_intersection_test_helper;
pub mod rectangle;
// `Shape.java` of the package `shape`: the module tree mirrors the Java one.
#[allow(clippy::module_inception)]
pub mod shape;
pub mod shape_collection;
pub mod shape_factory;
pub mod spatial_relation;
