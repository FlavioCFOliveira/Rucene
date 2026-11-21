//! Port of `lucene/core/src/test/org/apache/lucene/document/TestLatLonShapeEncoding.java`:
//! test case for `LatLonShape` encoding.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::geo::component2d::Component2D;
use rucene::geo::{GeoEncodingUtils, LatLonGeometry, Polygon};
use rucene_test_framework::geo::geo_test_util;

use super::base_shape_encoding_test_case::{base_methods, ShapeEncodingCase};
use crate::core_support::class;

/// `TestLatLonShapeEncoding`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestLatLonShapeEncoding",
        base_methods::<TestLatLonShapeEncoding>(),
    )
}

/// The concrete `BaseShapeEncodingTestCase` over lat/lon encoding.
#[derive(Default)]
pub struct TestLatLonShapeEncoding;

impl ShapeEncodingCase for TestLatLonShapeEncoding {
    type Polygon = Polygon;

    fn encode_x(&self, x: f64) -> Result<i32> {
        GeoEncodingUtils::encode_longitude(x)
    }

    fn encode_y(&self, y: f64) -> Result<i32> {
        GeoEncodingUtils::encode_latitude(y)
    }

    fn decode_x(&self, x_encoded: i32) -> f64 {
        GeoEncodingUtils::decode_longitude(x_encoded)
    }

    fn decode_y(&self, y_encoded: i32) -> f64 {
        GeoEncodingUtils::decode_latitude(y_encoded)
    }

    fn next_x(&self) -> f64 {
        geo_test_util::next_longitude()
    }

    fn next_y(&self) -> f64 {
        geo_test_util::next_latitude()
    }

    fn next_polygon(&self) -> Polygon {
        geo_test_util::next_polygon()
    }

    fn create_polygon_2d(&self, polygon: &Polygon) -> Result<Arc<dyn Component2D>> {
        <dyn LatLonGeometry>::create(&[polygon as &dyn LatLonGeometry])
    }
}
