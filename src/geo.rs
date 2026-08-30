//! Geospatial primitives ported from `org.apache.lucene.geo`.
//!
//! Holds the coordinate encodings the geo fields index with, the constants that
//! bound valid coordinates, the geometry shapes a spatial query is built from,
//! the `Component2D` spatial-relation tree those shapes compile down to, and the
//! ear-clipping tessellator that turns a polygon into the triangles a shape
//! field indexes.

pub mod circle2d;
pub mod component2d;
pub mod component_tree;
pub mod edge_tree;
pub mod encoding;
pub mod geometry;
pub mod line2d;
pub mod point2d;
pub mod polygon2d;
pub mod rectangle2d;
pub mod simple_geojson_polygon_parser;
pub mod simple_wkt_shape_parser;
pub mod tessellator;

pub use circle2d::Circle2D;
pub use component2d::{Component2D, WithinRelation};
pub use component_tree::ComponentTree;
pub use edge_tree::EdgeTree;
pub use encoding::{GeoEncodingUtils, GeoUtils, XYEncodingUtils};
pub use geometry::{
    Circle, Geometry, LatLonGeometry, Line, Point, Polygon, Rectangle, XYCircle, XYGeometry,
    XYLine, XYPoint, XYPolygon, XYRectangle,
};
pub use line2d::Line2D;
pub use point2d::Point2D;
pub use polygon2d::Polygon2D;
pub use rectangle2d::Rectangle2D;
pub use simple_geojson_polygon_parser::{JsonValue, ParseException};
pub use simple_wkt_shape_parser::{ShapeType, SimpleWKTShapeParser, WktGeometry};
pub use tessellator::{Monitor, Tessellator, Triangle};
