//! Geospatial primitives ported from `org.apache.lucene.geo`.
//!
//! Holds the coordinate encodings the geo fields index with, the constants that
//! bound valid coordinates, and the geometry shapes a spatial query is built
//! from.

pub mod encoding;
pub mod geometry;

pub use encoding::{GeoEncodingUtils, GeoUtils, XYEncodingUtils};
pub use geometry::{
    Circle, Line, Point, Polygon, Rectangle, XYCircle, XYLine, XYPoint, XYPolygon, XYRectangle,
};
