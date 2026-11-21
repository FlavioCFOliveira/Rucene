//! Port of the tests of `lucene/core/src/test/org/apache/lucene/geo/`
//! (sprint 9, task #374): `TestCircle`, `TestCircle2D`, `TestGeoEncodingUtils`, `TestGeoUtils`, `TestLine2D`, `TestPoint`, `TestPoint2D`, `TestPolygon`, `TestPolygon2D`, `TestRectangle2D`, `TestSimpleWKTShapeParsing`, `TestTessellator`, `TestXYCircle`, `TestXYLine`, `TestXYPoint`, `TestXYPolygon`, `TestXYRectangle`.
//! Each class runs as a suite under `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop, clippy::explicit_counter_loop)]

#[path = "core_geo/component_checks.rs"]
mod component_checks;
mod core_support;
#[path = "core_geo/test_circle.rs"]
mod test_circle;
#[path = "core_geo/test_circle2d.rs"]
mod test_circle2d;
#[path = "core_geo/test_geo_encoding_utils.rs"]
mod test_geo_encoding_utils;
#[path = "core_geo/test_geo_utils.rs"]
mod test_geo_utils;
#[path = "core_geo/test_line2d.rs"]
mod test_line2d;
#[path = "core_geo/test_point.rs"]
mod test_point;
#[path = "core_geo/test_point2d.rs"]
mod test_point2d;
#[path = "core_geo/test_polygon.rs"]
mod test_polygon;
#[path = "core_geo/test_polygon2d.rs"]
mod test_polygon2d;
#[path = "core_geo/test_rectangle2d.rs"]
mod test_rectangle2d;
#[path = "core_geo/test_simple_wkt_shape_parsing.rs"]
mod test_simple_wkt_shape_parsing;
#[path = "core_geo/test_tessellator.rs"]
mod test_tessellator;
#[path = "core_geo/test_tessellator_data.rs"]
mod test_tessellator_data;
#[path = "core_geo/test_xy_circle.rs"]
mod test_xy_circle;
#[path = "core_geo/test_xy_line.rs"]
mod test_xy_line;
#[path = "core_geo/test_xy_point.rs"]
mod test_xy_point;
#[path = "core_geo/test_xy_polygon.rs"]
mod test_xy_polygon;
#[path = "core_geo/test_xy_rectangle.rs"]
mod test_xy_rectangle;

use core_support::run;

#[test]
fn test_circle() {
    run(test_circle::class_def);
}

#[test]
fn test_circle2d() {
    run(test_circle2d::class_def);
}

#[test]
fn test_geo_encoding_utils() {
    run(test_geo_encoding_utils::class_def);
}

#[test]
fn test_geo_utils() {
    run(test_geo_utils::class_def);
}

#[test]
fn test_line2d() {
    run(test_line2d::class_def);
}

#[test]
fn test_point() {
    run(test_point::class_def);
}

#[test]
fn test_point2d() {
    run(test_point2d::class_def);
}

#[test]
fn test_polygon() {
    run(test_polygon::class_def);
}

#[test]
fn test_polygon2d() {
    run(test_polygon2d::class_def);
}

#[test]
fn test_rectangle2d() {
    run(test_rectangle2d::class_def);
}

#[test]
fn test_simple_wkt_shape_parsing() {
    run(test_simple_wkt_shape_parsing::class_def);
}

#[test]
fn test_tessellator() {
    run(test_tessellator::class_def);
}

#[test]
fn test_xy_circle() {
    run(test_xy_circle::class_def);
}

#[test]
fn test_xy_line() {
    run(test_xy_line::class_def);
}

#[test]
fn test_xy_point() {
    run(test_xy_point::class_def);
}

#[test]
fn test_xy_polygon() {
    run(test_xy_polygon::class_def);
}

#[test]
fn test_xy_rectangle() {
    run(test_xy_rectangle::class_def);
}
