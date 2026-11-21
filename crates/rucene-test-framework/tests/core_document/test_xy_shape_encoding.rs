//! Port of `lucene/core/src/test/org/apache/lucene/document/TestXYShapeEncoding.java`:
//! tests `XYShape` encoding.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::geo::component2d::Component2D;
use rucene::geo::{XYEncodingUtils, XYGeometry, XYPolygon};
use rucene_test_framework::geo::shape_test_util;

use super::base_shape_encoding_test_case::{base_methods, verify_encoding, ShapeEncodingCase};
use crate::core_support::{class, random, test};

/// `TestXYShapeEncoding`.
pub fn class_def() -> Arc<Class> {
    let mut methods = base_methods::<TestXYShapeEncoding>();
    methods.push(test(
        "testRotationChangesOrientation",
        test_rotation_changes_orientation,
    ));
    class("org.apache.lucene.document.TestXYShapeEncoding", methods)
}

/// The concrete `BaseShapeEncodingTestCase` over XY encoding.
#[derive(Default)]
pub struct TestXYShapeEncoding;

impl ShapeEncodingCase for TestXYShapeEncoding {
    type Polygon = XYPolygon;

    fn encode_x(&self, x: f64) -> Result<i32> {
        XYEncodingUtils::encode(x as f32)
    }

    fn encode_y(&self, y: f64) -> Result<i32> {
        XYEncodingUtils::encode(y as f32)
    }

    fn decode_x(&self, x_encoded: i32) -> f64 {
        XYEncodingUtils::decode(x_encoded) as f64
    }

    fn decode_y(&self, y_encoded: i32) -> f64 {
        XYEncodingUtils::decode(y_encoded) as f64
    }

    fn next_x(&self) -> f64 {
        shape_test_util::next_float(&*random()) as f64
    }

    fn next_y(&self) -> f64 {
        shape_test_util::next_float(&*random()) as f64
    }

    fn next_polygon(&self) -> XYPolygon {
        shape_test_util::next_polygon()
    }

    fn create_polygon_2d(&self, polygon: &XYPolygon) -> Result<Arc<dyn Component2D>> {
        <dyn XYGeometry>::create(&[polygon as &dyn XYGeometry])
    }
}

/// Port of `testRotationChangesOrientation`.
fn test_rotation_changes_orientation() -> Result<()> {
    let ay = -3.4028218437925203E38;
    let ax = 3.4028220466166163E38;
    let by = 3.4028218437925203E38;
    let bx = -3.4028218437925203E38;
    let cy = 3.4028230607370965E38;
    let cx = -3.4028230607370965E38;
    verify_encoding(&TestXYShapeEncoding, ay, ax, by, bx, cy, cx)
}
