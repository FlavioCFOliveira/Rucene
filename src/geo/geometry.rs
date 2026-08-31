//! Geometry shapes ported from `org.apache.lucene.geo`.
//!
//! The shapes a spatial query is built from, in both the geographic
//! (latitude/longitude) and the cartesian (x/y) coordinate systems.

use crate::error::{LuceneError, Result};
use crate::geo::component2d::Component2D;
use crate::geo::encoding::{next_up_f32, GeoUtils, WindingOrder, XYEncodingUtils};
use crate::geo::simple_geojson_polygon_parser::SimpleGeoJSONPolygonParser;
use crate::geo::{Circle2D, Line2D, ParseException, Point2D, Polygon2D, Rectangle2D};
use crate::util::SloppyMath;
use std::f64::consts::PI;
use std::sync::Arc;

/// A single point on the globe.
///
/// Equivalent to `org.apache.lucene.geo.Point`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    lat: f64,
    lon: f64,
}

impl Point {
    /// Creates the point, checking both coordinates.
    pub fn new(lat: f64, lon: f64) -> Result<Self> {
        GeoUtils::check_latitude(lat)?;
        GeoUtils::check_longitude(lon)?;
        Ok(Self { lat, lon })
    }

    /// Returns the latitude.
    pub fn lat(&self) -> f64 {
        self.lat
    }

    /// Returns the longitude.
    pub fn lon(&self) -> f64 {
        self.lon
    }
}

/// An axis-aligned box on the globe.
///
/// Equivalent to `org.apache.lucene.geo.Rectangle`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rectangle {
    min_lat: f64,
    max_lat: f64,
    min_lon: f64,
    max_lon: f64,
}

impl Rectangle {
    /// Creates the box. A box whose minimum longitude exceeds its maximum
    /// crosses the dateline, which Lucene allows.
    pub fn new(min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) -> Result<Self> {
        GeoUtils::check_latitude(min_lat)?;
        GeoUtils::check_latitude(max_lat)?;
        GeoUtils::check_longitude(min_lon)?;
        GeoUtils::check_longitude(max_lon)?;
        if min_lat > max_lat {
            return Err(LuceneError::IllegalArgument(format!(
                "minLat must be lower than maxLat, got {min_lat} > {max_lat}"
            )));
        }
        Ok(Self {
            min_lat,
            max_lat,
            min_lon,
            max_lon,
        })
    }

    /// Returns the smallest latitude.
    pub fn min_lat(&self) -> f64 {
        self.min_lat
    }

    /// Returns the largest latitude.
    pub fn max_lat(&self) -> f64 {
        self.max_lat
    }

    /// Returns the smallest longitude.
    pub fn min_lon(&self) -> f64 {
        self.min_lon
    }

    /// Returns the largest longitude.
    pub fn max_lon(&self) -> f64 {
        self.max_lon
    }

    /// Returns whether the box spans the dateline.
    ///
    /// Equivalent to `Rectangle.crossesDateline()`.
    pub fn crosses_dateline(&self) -> bool {
        self.max_lon < self.min_lon
    }

    /// Returns whether `(lat, lon)` falls inside the box.
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        if lat < self.min_lat || lat > self.max_lat {
            return false;
        }
        if self.crosses_dateline() {
            lon >= self.min_lon || lon <= self.max_lon
        } else {
            lon >= self.min_lon && lon <= self.max_lon
        }
    }
}

/// A connected sequence of points on the globe.
///
/// Equivalent to `org.apache.lucene.geo.Line`.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    lats: Vec<f64>,
    lons: Vec<f64>,
    min_lat: f64,
    max_lat: f64,
    min_lon: f64,
    max_lon: f64,
}

impl Line {
    /// Creates the line from parallel coordinate arrays.
    pub fn new(lats: Vec<f64>, lons: Vec<f64>) -> Result<Self> {
        if lats.len() != lons.len() {
            return Err(LuceneError::IllegalArgument(
                "lats and lons must be equal length".to_string(),
            ));
        }
        if lats.len() < 2 {
            return Err(LuceneError::IllegalArgument(
                "at least 2 line points required".to_string(),
            ));
        }
        // compute bounding box
        let mut min_lat = lats[0];
        let mut min_lon = lons[0];
        let mut max_lat = lats[0];
        let mut max_lon = lons[0];
        for i in 0..lats.len() {
            GeoUtils::check_latitude(lats[i])?;
            GeoUtils::check_longitude(lons[i])?;
            min_lat = lats[i].min(min_lat);
            min_lon = lons[i].min(min_lon);
            max_lat = lats[i].max(max_lat);
            max_lon = lons[i].max(max_lon);
        }
        Ok(Self {
            lats,
            lons,
            min_lat,
            max_lat,
            min_lon,
            max_lon,
        })
    }

    /// Returns the smallest latitude of this line's bounding box.
    ///
    /// Equivalent to the public field `Line.minLat`.
    pub fn min_lat(&self) -> f64 {
        self.min_lat
    }

    /// Returns the largest latitude of this line's bounding box.
    ///
    /// Equivalent to the public field `Line.maxLat`.
    pub fn max_lat(&self) -> f64 {
        self.max_lat
    }

    /// Returns the smallest longitude of this line's bounding box.
    ///
    /// Equivalent to the public field `Line.minLon`.
    pub fn min_lon(&self) -> f64 {
        self.min_lon
    }

    /// Returns the largest longitude of this line's bounding box.
    ///
    /// Equivalent to the public field `Line.maxLon`.
    pub fn max_lon(&self) -> f64 {
        self.max_lon
    }

    /// Returns the latitude at `vertex`.
    ///
    /// Equivalent to `Line.getLat(int)`.
    pub fn get_lat(&self, vertex: usize) -> f64 {
        self.lats[vertex]
    }

    /// Returns the longitude at `vertex`.
    ///
    /// Equivalent to `Line.getLon(int)`.
    pub fn get_lon(&self, vertex: usize) -> f64 {
        self.lons[vertex]
    }

    /// Renders the line's vertices as GeoJSON.
    ///
    /// Equivalent to `Line.toGeoJSON()`.
    pub fn to_geojson(&self) -> String {
        format!("[{}]", Polygon::vertices_to_geojson(&self.lats, &self.lons))
    }

    /// Returns how many vertices the line has.
    pub fn num_points(&self) -> usize {
        self.lats.len()
    }

    /// Returns the latitudes.
    pub fn lats(&self) -> &[f64] {
        &self.lats
    }

    /// Returns the longitudes.
    pub fn lons(&self) -> &[f64] {
        &self.lons
    }
}

/// A closed ring on the globe, optionally with holes.
///
/// Equivalent to `org.apache.lucene.geo.Polygon`.
#[derive(Clone, Debug, PartialEq)]
pub struct Polygon {
    lats: Vec<f64>,
    lons: Vec<f64>,
    holes: Vec<Polygon>,
    min_lat: f64,
    max_lat: f64,
    min_lon: f64,
    max_lon: f64,
    winding_order: WindingOrder,
}

impl Polygon {
    /// Creates the polygon. The ring must be closed — the first and last
    /// vertices equal — as Lucene requires.
    pub fn new(lats: Vec<f64>, lons: Vec<f64>, holes: Vec<Polygon>) -> Result<Self> {
        if lats.len() != lons.len() {
            return Err(LuceneError::IllegalArgument(
                "lats and lons must be equal length".to_string(),
            ));
        }
        if lats.len() < 4 {
            return Err(LuceneError::IllegalArgument(
                "at least 4 polygon points required".to_string(),
            ));
        }
        if lats[0] != lats[lats.len() - 1] {
            return Err(LuceneError::IllegalArgument(
                "first and last points of the polygon must be the same (it must close itself): \
                 lats[0]!=lats[lats.length-1]"
                    .to_string(),
            ));
        }
        if lons[0] != lons[lons.len() - 1] {
            return Err(LuceneError::IllegalArgument(
                "first and last points of the polygon must be the same (it must close itself): \
                 lons[0]!=lons[lons.length-1]"
                    .to_string(),
            ));
        }
        for i in 0..lats.len() {
            GeoUtils::check_latitude(lats[i])?;
            GeoUtils::check_longitude(lons[i])?;
        }
        for inner in &holes {
            if !inner.holes.is_empty() {
                return Err(LuceneError::IllegalArgument(
                    "holes may not contain holes: polygons may not nest.".to_string(),
                ));
            }
        }

        // compute bounding box
        let mut min_lat = lats[0];
        let mut max_lat = lats[0];
        let mut min_lon = lons[0];
        let mut max_lon = lons[0];

        let mut winding_sum = 0.0f64;
        let num_pts = lats.len() - 1;
        let mut j = 0usize;
        for i in 1..num_pts {
            min_lat = lats[i].min(min_lat);
            max_lat = lats[i].max(max_lat);
            min_lon = lons[i].min(min_lon);
            max_lon = lons[i].max(max_lon);
            // compute signed area
            winding_sum += (lons[j] - lons[num_pts]) * (lats[i] - lats[num_pts])
                - (lats[j] - lats[num_pts]) * (lons[i] - lons[num_pts]);
            j = i;
        }
        let winding_order = if winding_sum < 0.0 {
            WindingOrder::CCW
        } else {
            WindingOrder::CW
        };

        Ok(Self {
            lats,
            lons,
            holes,
            min_lat,
            max_lat,
            min_lon,
            max_lon,
            winding_order,
        })
    }

    /// Returns the smallest latitude of this polygon's bounding box.
    ///
    /// Equivalent to the public field `Polygon.minLat`.
    pub fn min_lat(&self) -> f64 {
        self.min_lat
    }

    /// Returns the largest latitude of this polygon's bounding box.
    ///
    /// Equivalent to the public field `Polygon.maxLat`.
    pub fn max_lat(&self) -> f64 {
        self.max_lat
    }

    /// Returns the smallest longitude of this polygon's bounding box.
    ///
    /// Equivalent to the public field `Polygon.minLon`.
    pub fn min_lon(&self) -> f64 {
        self.min_lon
    }

    /// Returns the largest longitude of this polygon's bounding box.
    ///
    /// Equivalent to the public field `Polygon.maxLon`.
    pub fn max_lon(&self) -> f64 {
        self.max_lon
    }

    /// Returns the winding order of the polygon shell.
    ///
    /// Equivalent to `Polygon.getWindingOrder()`.
    pub fn get_winding_order(&self) -> WindingOrder {
        self.winding_order
    }

    /// Returns the number of holes.
    ///
    /// Equivalent to `Polygon.numHoles()`.
    pub fn num_holes(&self) -> usize {
        self.holes.len()
    }

    /// Returns the hole at `i`.
    ///
    /// Equivalent to the package-private `Polygon.getHole(int)`.
    pub fn get_hole(&self, i: usize) -> &Polygon {
        &self.holes[i]
    }

    /// Returns the latitude at `vertex`.
    ///
    /// Equivalent to `Polygon.getPolyLat(int)`.
    pub fn get_polylat(&self, vertex: usize) -> f64 {
        self.lats[vertex]
    }

    /// Returns the longitude at `vertex`.
    ///
    /// Equivalent to `Polygon.getPolyLon(int)`.
    pub fn get_polylon(&self, vertex: usize) -> f64 {
        self.lons[vertex]
    }

    /// Renders a vertex ring as a GeoJSON coordinate array.
    ///
    /// Equivalent to `Polygon.verticesToGeoJSON(double[], double[])`.
    pub fn vertices_to_geojson(lats: &[f64], lons: &[f64]) -> String {
        let mut sb = String::from("[");
        for i in 0..lats.len() {
            sb.push_str(&format!("[{}, {}]", lons[i], lats[i]));
            if i != lats.len() - 1 {
                sb.push_str(", ");
            }
        }
        sb.push(']');
        sb
    }

    /// Renders the polygon and its holes as GeoJSON.
    ///
    /// Equivalent to `Polygon.toGeoJSON()`.
    pub fn to_geojson(&self) -> String {
        let mut sb = String::from("[");
        sb.push_str(&Self::vertices_to_geojson(&self.lats, &self.lons));
        for hole in &self.holes {
            sb.push(',');
            sb.push_str(&Self::vertices_to_geojson(&hole.lats, &hole.lons));
        }
        sb.push(']');
        sb
    }

    /// Parses a standard GeoJSON polygon string.
    ///
    /// The type of the incoming GeoJSON object must be `Polygon` or
    /// `MultiPolygon`, optionally embedded under a `type: Feature`. A `Polygon`
    /// returns a length-1 vector, a `MultiPolygon` one or more.
    ///
    /// Equivalent to `Polygon.fromGeoJSON(String)`.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseException`] describing the offending character offset,
    /// exactly as Java's `java.text.ParseException` does.
    pub fn from_geojson(geojson: &str) -> std::result::Result<Vec<Polygon>, ParseException> {
        SimpleGeoJSONPolygonParser::new(geojson).parse()
    }

    /// Returns how many vertices the outer ring has.
    pub fn num_points(&self) -> usize {
        self.lats.len()
    }

    /// Returns the outer ring's latitudes.
    pub fn get_polylats(&self) -> &[f64] {
        &self.lats
    }

    /// Returns the outer ring's longitudes.
    pub fn get_polylons(&self) -> &[f64] {
        &self.lons
    }

    /// Returns the holes.
    pub fn get_holes(&self) -> &[Polygon] {
        &self.holes
    }

    /// Returns the bounding box of the outer ring.
    pub fn bounding_box(&self) -> Result<Rectangle> {
        let min_lat = self.lats.iter().copied().fold(f64::MAX, f64::min);
        let max_lat = self.lats.iter().copied().fold(f64::MIN, f64::max);
        let min_lon = self.lons.iter().copied().fold(f64::MAX, f64::min);
        let max_lon = self.lons.iter().copied().fold(f64::MIN, f64::max);
        Rectangle::new(min_lat, max_lat, min_lon, max_lon)
    }
}

/// A circle on the globe, given by a centre and a radius in metres.
///
/// Equivalent to `org.apache.lucene.geo.Circle`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    lat: f64,
    lon: f64,
    radius_meters: f64,
}

impl Circle {
    /// Creates the circle.
    pub fn new(lat: f64, lon: f64, radius_meters: f64) -> Result<Self> {
        GeoUtils::check_latitude(lat)?;
        GeoUtils::check_longitude(lon)?;
        if radius_meters <= 0.0 || !radius_meters.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "radiusMeters: '{radius_meters}' is invalid"
            )));
        }
        Ok(Self {
            lat,
            lon,
            radius_meters,
        })
    }

    /// Returns the centre's latitude.
    pub fn get_lat(&self) -> f64 {
        self.lat
    }

    /// Returns the centre's longitude.
    pub fn get_lon(&self) -> f64 {
        self.lon
    }

    /// Returns the radius, in metres.
    pub fn get_radius(&self) -> f64 {
        self.radius_meters
    }
}

/// A single point in the cartesian plane.
///
/// Equivalent to `org.apache.lucene.geo.XYPoint`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XYPoint {
    x: f32,
    y: f32,
}

impl XYPoint {
    /// Creates the point, checking both coordinates are finite.
    pub fn new(x: f32, y: f32) -> Result<Self> {
        Ok(Self {
            x: XYEncodingUtils::check_val(x)?,
            y: XYEncodingUtils::check_val(y)?,
        })
    }

    /// Returns the x coordinate.
    pub fn get_x(&self) -> f32 {
        self.x
    }

    /// Returns the y coordinate.
    pub fn get_y(&self) -> f32 {
        self.y
    }
}

/// An axis-aligned box in the cartesian plane.
///
/// Equivalent to `org.apache.lucene.geo.XYRectangle`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XYRectangle {
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

impl XYRectangle {
    /// Creates the box.
    pub fn new(min_x: f32, max_x: f32, min_y: f32, max_y: f32) -> Result<Self> {
        let min_x = XYEncodingUtils::check_val(min_x)?;
        let max_x = XYEncodingUtils::check_val(max_x)?;
        let min_y = XYEncodingUtils::check_val(min_y)?;
        let max_y = XYEncodingUtils::check_val(max_y)?;
        if min_x > max_x {
            return Err(LuceneError::IllegalArgument(
                "minX must be lower than maxX".to_string(),
            ));
        }
        if min_y > max_y {
            return Err(LuceneError::IllegalArgument(
                "minY must be lower than maxY".to_string(),
            ));
        }
        Ok(Self {
            min_x,
            max_x,
            min_y,
            max_y,
        })
    }

    /// Returns the smallest x.
    pub fn min_x(&self) -> f32 {
        self.min_x
    }

    /// Returns the largest x.
    pub fn max_x(&self) -> f32 {
        self.max_x
    }

    /// Returns the smallest y.
    pub fn min_y(&self) -> f32 {
        self.min_y
    }

    /// Returns the largest y.
    pub fn max_y(&self) -> f32 {
        self.max_y
    }

    /// Returns whether `(x, y)` falls inside the box.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }
}

/// A connected sequence of points in the cartesian plane.
///
/// Equivalent to `org.apache.lucene.geo.XYLine`.
#[derive(Clone, Debug, PartialEq)]
pub struct XYLine {
    xs: Vec<f32>,
    ys: Vec<f32>,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

impl XYLine {
    /// Creates the line from parallel coordinate arrays.
    pub fn new(xs: Vec<f32>, ys: Vec<f32>) -> Result<Self> {
        if xs.len() != ys.len() {
            return Err(LuceneError::IllegalArgument(
                "xs and ys must be equal length".to_string(),
            ));
        }
        if xs.len() < 2 {
            return Err(LuceneError::IllegalArgument(
                "at least 2 line points required".to_string(),
            ));
        }
        // compute bounding box
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = -f32::MAX;
        let mut max_y = -f32::MAX;
        for i in 0..xs.len() {
            min_x = XYEncodingUtils::check_val(xs[i])?.min(min_x);
            min_y = XYEncodingUtils::check_val(ys[i])?.min(min_y);
            max_x = xs[i].max(max_x);
            max_y = ys[i].max(max_y);
        }
        Ok(Self {
            xs,
            ys,
            min_x,
            max_x,
            min_y,
            max_y,
        })
    }

    /// Returns the smallest x of this line's bounding box.
    ///
    /// Equivalent to the public field `XYLine.minX`.
    pub fn min_x(&self) -> f32 {
        self.min_x
    }

    /// Returns the largest x of this line's bounding box.
    ///
    /// Equivalent to the public field `XYLine.maxX`.
    pub fn max_x(&self) -> f32 {
        self.max_x
    }

    /// Returns the smallest y of this line's bounding box.
    ///
    /// Equivalent to the public field `XYLine.minY`.
    pub fn min_y(&self) -> f32 {
        self.min_y
    }

    /// Returns the largest y of this line's bounding box.
    ///
    /// Equivalent to the public field `XYLine.maxY`.
    pub fn max_y(&self) -> f32 {
        self.max_y
    }

    /// Returns the x coordinate at `vertex`.
    ///
    /// Equivalent to `XYLine.getX(int)`.
    pub fn get_x_at(&self, vertex: usize) -> f32 {
        self.xs[vertex]
    }

    /// Returns the y coordinate at `vertex`.
    ///
    /// Equivalent to `XYLine.getY(int)`.
    pub fn get_y_at(&self, vertex: usize) -> f32 {
        self.ys[vertex]
    }

    /// Returns how many vertices the line has.
    pub fn num_points(&self) -> usize {
        self.xs.len()
    }

    /// Returns the x coordinates.
    pub fn get_x(&self) -> &[f32] {
        &self.xs
    }

    /// Returns the y coordinates.
    pub fn get_y(&self) -> &[f32] {
        &self.ys
    }
}

/// A closed ring in the cartesian plane, optionally with holes.
///
/// Equivalent to `org.apache.lucene.geo.XYPolygon`.
#[derive(Clone, Debug, PartialEq)]
pub struct XYPolygon {
    xs: Vec<f32>,
    ys: Vec<f32>,
    holes: Vec<XYPolygon>,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
    winding_order: WindingOrder,
}

impl XYPolygon {
    /// Creates the polygon. The ring must close itself.
    pub fn new(xs: Vec<f32>, ys: Vec<f32>, holes: Vec<XYPolygon>) -> Result<Self> {
        if xs.len() != ys.len() {
            return Err(LuceneError::IllegalArgument(
                "xs and ys must be equal length".to_string(),
            ));
        }
        if xs.len() < 4 {
            return Err(LuceneError::IllegalArgument(
                "at least 4 polygon points required".to_string(),
            ));
        }
        if xs[0] != xs[xs.len() - 1] || ys[0] != ys[ys.len() - 1] {
            return Err(LuceneError::IllegalArgument(
                "first and last points of the polygon must be the same (it must close itself)"
                    .to_string(),
            ));
        }
        for inner in &holes {
            if !inner.holes.is_empty() {
                return Err(LuceneError::IllegalArgument(
                    "holes may not contain holes: polygons may not nest.".to_string(),
                ));
            }
        }

        // compute bounding box
        let mut min_x = XYEncodingUtils::check_val(xs[0])?;
        let mut max_x = xs[0];
        let mut min_y = XYEncodingUtils::check_val(ys[0])?;
        let mut max_y = ys[0];

        let mut winding_sum = 0.0f64;
        let num_pts = xs.len() - 1;
        let mut j = 0usize;
        for i in 1..num_pts {
            min_x = XYEncodingUtils::check_val(xs[i])?.min(min_x);
            max_x = xs[i].max(max_x);
            min_y = XYEncodingUtils::check_val(ys[i])?.min(min_y);
            max_y = ys[i].max(max_y);
            // compute signed area
            winding_sum += f64::from(
                (xs[j] - xs[num_pts]) * (ys[i] - ys[num_pts])
                    - (ys[j] - ys[num_pts]) * (xs[i] - xs[num_pts]),
            );
            j = i;
        }
        // Java only validates the vertices the bounding-box loop visits; the
        // closing vertex equals the first one, which was validated above.
        let winding_order = if winding_sum < 0.0 {
            WindingOrder::CCW
        } else {
            WindingOrder::CW
        };

        Ok(Self {
            xs,
            ys,
            holes,
            min_x,
            max_x,
            min_y,
            max_y,
            winding_order,
        })
    }

    /// Returns the smallest x of this polygon's bounding box.
    ///
    /// Equivalent to the public field `XYPolygon.minX`.
    pub fn min_x(&self) -> f32 {
        self.min_x
    }

    /// Returns the largest x of this polygon's bounding box.
    ///
    /// Equivalent to the public field `XYPolygon.maxX`.
    pub fn max_x(&self) -> f32 {
        self.max_x
    }

    /// Returns the smallest y of this polygon's bounding box.
    ///
    /// Equivalent to the public field `XYPolygon.minY`.
    pub fn min_y(&self) -> f32 {
        self.min_y
    }

    /// Returns the largest y of this polygon's bounding box.
    ///
    /// Equivalent to the public field `XYPolygon.maxY`.
    pub fn max_y(&self) -> f32 {
        self.max_y
    }

    /// Returns the winding order of the polygon shell.
    ///
    /// Equivalent to `XYPolygon.getWindingOrder()`.
    pub fn get_winding_order(&self) -> WindingOrder {
        self.winding_order
    }

    /// Returns the number of holes.
    ///
    /// Equivalent to `XYPolygon.numHoles()`.
    pub fn num_holes(&self) -> usize {
        self.holes.len()
    }

    /// Returns the hole at `i`.
    ///
    /// Equivalent to the package-private `XYPolygon.getHole(int)`.
    pub fn get_hole(&self, i: usize) -> &XYPolygon {
        &self.holes[i]
    }

    /// Returns the x coordinate at `vertex`.
    ///
    /// Equivalent to `XYPolygon.getPolyX(int)`.
    pub fn get_polyx_at(&self, vertex: usize) -> f32 {
        self.xs[vertex]
    }

    /// Returns the y coordinate at `vertex`.
    ///
    /// Equivalent to `XYPolygon.getPolyY(int)`.
    pub fn get_polyy_at(&self, vertex: usize) -> f32 {
        self.ys[vertex]
    }

    /// Renders a vertex ring as a GeoJSON coordinate array.
    ///
    /// Equivalent to `XYPolygon.verticesToGeoJSON(float[], float[])`.
    pub fn vertices_to_geojson(xs: &[f32], ys: &[f32]) -> String {
        let mut sb = String::from("[");
        for i in 0..xs.len() {
            sb.push_str(&format!("[{}, {}]", xs[i], ys[i]));
            if i != xs.len() - 1 {
                sb.push_str(", ");
            }
        }
        sb.push(']');
        sb
    }

    /// Returns how many vertices the outer ring has.
    pub fn num_points(&self) -> usize {
        self.xs.len()
    }

    /// Returns the outer ring's x coordinates.
    pub fn get_polyx(&self) -> &[f32] {
        &self.xs
    }

    /// Returns the outer ring's y coordinates.
    pub fn get_polyy(&self) -> &[f32] {
        &self.ys
    }

    /// Returns the holes.
    pub fn get_holes(&self) -> &[XYPolygon] {
        &self.holes
    }
}

/// A circle in the cartesian plane.
///
/// Equivalent to `org.apache.lucene.geo.XYCircle`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XYCircle {
    x: f32,
    y: f32,
    radius: f32,
}

impl XYCircle {
    /// Creates the circle.
    pub fn new(x: f32, y: f32, radius: f32) -> Result<Self> {
        let x = XYEncodingUtils::check_val(x)?;
        let y = XYEncodingUtils::check_val(y)?;
        if radius <= 0.0 || !radius.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "radius: '{radius}' is invalid"
            )));
        }
        Ok(Self { x, y, radius })
    }

    /// Returns the centre's x coordinate.
    pub fn get_x(&self) -> f32 {
        self.x
    }

    /// Returns the centre's y coordinate.
    pub fn get_y(&self) -> f32 {
        self.y
    }

    /// Returns the radius.
    pub fn get_radius(&self) -> f32 {
        self.radius
    }
}

// -----------------------------------------------------------------------------
// Rectangle: static helpers
// -----------------------------------------------------------------------------

impl Rectangle {
    /// Maximum error of [`Rectangle::axis_lat`]; callers must be prepared to
    /// handle it.
    ///
    /// Equivalent to `Rectangle.AXISLAT_ERROR`.
    pub const AXISLAT_ERROR: f64 = (0.1 / GeoUtils::EARTH_MEAN_RADIUS_METERS) * (180.0 / PI);

    /// Returns whether the rectangle given by the four bounds contains the
    /// `(lat, lon)` point.
    ///
    /// Equivalent to `Rectangle.containsPoint(...)`. Note this static helper,
    /// unlike [`Rectangle::contains`], does not handle dateline crossing.
    pub fn contains_point(
        lat: f64,
        lon: f64,
        min_lat: f64,
        max_lat: f64,
        min_lon: f64,
        max_lon: f64,
    ) -> bool {
        lat >= min_lat && lat <= max_lat && lon >= min_lon && lon <= max_lon
    }

    /// Computes the bounding box of a circle using WGS-84 parameters.
    ///
    /// Equivalent to `Rectangle.fromPointDistance(double, double, double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the centre coordinates are
    /// out of range.
    pub fn from_point_distance(
        center_lat: f64,
        center_lon: f64,
        radius_meters: f64,
    ) -> Result<Rectangle> {
        GeoUtils::check_latitude(center_lat)?;
        GeoUtils::check_longitude(center_lon)?;
        let rad_lat = center_lat.to_radians();
        let rad_lon = center_lon.to_radians();
        // LUCENE-7143
        let rad_distance = (radius_meters + 7E-2) / GeoUtils::EARTH_MEAN_RADIUS_METERS;
        let mut min_lat = rad_lat - rad_distance;
        let mut max_lat = rad_lat + rad_distance;
        let min_lon;
        let max_lon;

        if min_lat > GeoUtils::min_lat_radians() && max_lat < GeoUtils::max_lat_radians() {
            let delta_lon =
                SloppyMath::asin(SloppyMath::sin(rad_distance) / SloppyMath::cos(rad_lat));
            let mut lo = rad_lon - delta_lon;
            if lo < GeoUtils::min_lon_radians() {
                lo += 2.0 * PI;
            }
            let mut hi = rad_lon + delta_lon;
            if hi > GeoUtils::max_lon_radians() {
                hi -= 2.0 * PI;
            }
            min_lon = lo;
            max_lon = hi;
        } else {
            // a pole is within the distance
            min_lat = min_lat.max(GeoUtils::min_lat_radians());
            max_lat = max_lat.min(GeoUtils::max_lat_radians());
            min_lon = GeoUtils::min_lon_radians();
            max_lon = GeoUtils::max_lon_radians();
        }

        Rectangle::new(
            min_lat.to_degrees(),
            max_lat.to_degrees(),
            min_lon.to_degrees(),
            max_lon.to_degrees(),
        )
    }

    /// Calculates the latitude at which a circle touches the meridians of its
    /// bounding box.
    ///
    /// Equivalent to `Rectangle.axisLat(double, double)`. The returned value is
    /// within [`Rectangle::AXISLAT_ERROR`] of the exact one.
    pub fn axis_lat(center_lat: f64, radius_meters: f64) -> f64 {
        // A spherical triangle with:
        // r is the radius of the circle in radians
        // l1 is the latitude of the circle center
        // l2 is the latitude of the point at which the circle intersects its bbox longitudes
        // We know r is tangent to the bbox meridians at l2, therefore it is a right angle, and
        // from the law of cosines cos(l1) = cos(r) * cos(l2), so l2 = acos(cos(l1) / cos(r)).
        const PIO2: f64 = PI / 2.0;
        let mut l1 = center_lat.to_radians();
        let r = (radius_meters + 7E-2) / GeoUtils::EARTH_MEAN_RADIUS_METERS;

        // if we are within radius range of a pole, the lat is the pole itself
        if l1.abs() + r >= GeoUtils::max_lat_radians() {
            return if center_lat >= 0.0 {
                GeoUtils::MAX_LAT_INCL
            } else {
                GeoUtils::MIN_LAT_INCL
            };
        }

        // adjust l1 as distance from closest pole, to form a right triangle with bbox meridians
        // and ensure it is in the range (0, PI/2]
        l1 = if center_lat >= 0.0 {
            PIO2 - l1
        } else {
            l1 + PIO2
        };

        let mut l2 = (l1.cos() / r.cos()).acos();

        // now adjust back to range [-pi/2, pi/2], ie latitude in radians
        l2 = if center_lat >= 0.0 {
            PIO2 - l2
        } else {
            l2 - PIO2
        };

        l2.to_degrees()
    }

    /// Returns the bounding box over an array of polygons.
    ///
    /// Equivalent to `Rectangle.fromPolygon(Polygon[])`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the resulting bounds are
    /// not a valid rectangle, which for an empty input is always the case.
    pub fn from_polygon(polygons: &[Polygon]) -> Result<Rectangle> {
        // compute bounding box
        let mut min_lat = f64::INFINITY;
        let mut max_lat = f64::NEG_INFINITY;
        let mut min_lon = f64::INFINITY;
        let mut max_lon = f64::NEG_INFINITY;

        for p in polygons {
            min_lat = p.min_lat().min(min_lat);
            max_lat = p.max_lat().max(max_lat);
            min_lon = p.min_lon().min(min_lon);
            max_lon = p.max_lon().max(max_lon);
        }

        Rectangle::new(min_lat, max_lat, min_lon, max_lon)
    }
}

impl std::fmt::Display for Rectangle {
    /// Equivalent to `Rectangle.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Rectangle(lat={} TO {} lon={} TO {}",
            self.min_lat, self.max_lat, self.min_lon, self.max_lon
        )?;
        if self.max_lon < self.min_lon {
            write!(f, " [crosses dateline!]")?;
        }
        write!(f, ")")
    }
}

// -----------------------------------------------------------------------------
// XYRectangle: static helpers
// -----------------------------------------------------------------------------

impl XYRectangle {
    /// Computes the bounding box of a circle in cartesian geometry.
    ///
    /// Equivalent to `XYRectangle.fromPointDistance(float, float, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the centre is not finite,
    /// or when the radius is negative or not finite.
    pub fn from_point_distance(x: f32, y: f32, radius: f32) -> Result<XYRectangle> {
        XYEncodingUtils::check_val(x)?;
        XYEncodingUtils::check_val(y)?;
        if radius < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "radius must be bigger than 0, got {radius}"
            )));
        }
        if !radius.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "radius must be finite, got {radius}"
            )));
        }
        // LUCENE-9243: We round up the bounding box to avoid numerical errors.
        let distance_box = next_up_f32(radius);
        let min_x = (-f32::MAX).max(x - distance_box);
        let max_x = f32::MAX.min(x + distance_box);
        let min_y = (-f32::MAX).max(y - distance_box);
        let max_y = f32::MAX.min(y + distance_box);
        XYRectangle::new(min_x, max_x, min_y, max_y)
    }
}

// -----------------------------------------------------------------------------
// Geometry / LatLonGeometry / XYGeometry
// -----------------------------------------------------------------------------

/// Base behaviour shared by [`LatLonGeometry`] and [`XYGeometry`].
///
/// Equivalent to `org.apache.lucene.geo.Geometry`. Java declares
/// `toComponent2D()` `protected`; Rust has no `protected`, so the method is
/// part of the public trait surface.
pub trait Geometry {
    /// Returns the [`Component2D`] this geometry relates against.
    ///
    /// Equivalent to `Geometry.toComponent2D()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the geometry cannot be
    /// turned into a component, mirroring the `IllegalArgumentException` the
    /// Java factories throw.
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>>;
}

/// A geometry expressed in latitude/longitude degrees.
///
/// Equivalent to `org.apache.lucene.geo.LatLonGeometry`.
pub trait LatLonGeometry: Geometry {}

impl dyn LatLonGeometry + '_ {
    /// Creates a [`Component2D`] from the provided geometries.
    ///
    /// Equivalent to `LatLonGeometry.create(LatLonGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `geometries` is empty, as
    /// Java throws `IllegalArgumentException`. Java also rejects `null`
    /// entries; Rust's type system rules those out.
    pub fn create(geometries: &[&dyn LatLonGeometry]) -> Result<Arc<dyn Component2D>> {
        if geometries.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "geometries must not be empty".to_string(),
            ));
        }
        if geometries.len() == 1 {
            return geometries[0].to_component2d();
        }
        let mut components = Vec::with_capacity(geometries.len());
        for g in geometries {
            components.push(g.to_component2d()?);
        }
        Ok(crate::geo::ComponentTree::create(components))
    }
}

/// A geometry expressed in cartesian coordinates.
///
/// Equivalent to `org.apache.lucene.geo.XYGeometry`.
pub trait XYGeometry: Geometry {}

impl dyn XYGeometry + '_ {
    /// Creates a [`Component2D`] from the provided geometries.
    ///
    /// Equivalent to `XYGeometry.create(XYGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `geometries` is empty, as
    /// Java throws `IllegalArgumentException`.
    pub fn create(geometries: &[&dyn XYGeometry]) -> Result<Arc<dyn Component2D>> {
        if geometries.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "geometries must not be empty".to_string(),
            ));
        }
        if geometries.len() == 1 {
            return geometries[0].to_component2d();
        }
        let mut components = Vec::with_capacity(geometries.len());
        for g in geometries {
            components.push(g.to_component2d()?);
        }
        Ok(crate::geo::ComponentTree::create(components))
    }
}

impl Geometry for Point {
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        Point2D::create_from_point(self)
    }
}
impl LatLonGeometry for Point {}

impl Geometry for Rectangle {
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        Rectangle2D::create_from_rectangle(self)
    }
}
impl LatLonGeometry for Rectangle {}

impl Geometry for Line {
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        Ok(Line2D::create_from_line(self))
    }
}
impl LatLonGeometry for Line {}

impl Geometry for Polygon {
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        Polygon2D::create_from_polygon(self)
    }
}
impl LatLonGeometry for Polygon {}

impl Geometry for Circle {
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        Circle2D::create_from_circle(self)
    }
}
impl LatLonGeometry for Circle {}

impl Geometry for XYPoint {
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        Ok(Point2D::create_from_xy_point(self))
    }
}
impl XYGeometry for XYPoint {}

impl Geometry for XYRectangle {
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        Ok(Rectangle2D::create_from_xy_rectangle(self))
    }
}
impl XYGeometry for XYRectangle {}

impl Geometry for XYLine {
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        Ok(Line2D::create_from_xy_line(self))
    }
}
impl XYGeometry for XYLine {}

impl Geometry for XYPolygon {
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        Polygon2D::create_from_xy_polygon(self)
    }
}
impl XYGeometry for XYPolygon {}

impl Geometry for XYCircle {
    fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        Ok(Circle2D::create_from_xy_circle(self))
    }
}
impl XYGeometry for XYCircle {}
