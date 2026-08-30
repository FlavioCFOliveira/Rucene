//! Geometry shapes ported from `org.apache.lucene.geo`.
//!
//! The shapes a spatial query is built from, in both the geographic
//! (latitude/longitude) and the cartesian (x/y) coordinate systems.

use crate::error::{LuceneError, Result};
use crate::geo::encoding::{GeoUtils, XYEncodingUtils};

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
        for i in 0..lats.len() {
            GeoUtils::check_latitude(lats[i])?;
            GeoUtils::check_longitude(lons[i])?;
        }
        Ok(Self { lats, lons })
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
        Ok(Self { lats, lons, holes })
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
        for i in 0..xs.len() {
            XYEncodingUtils::check_val(xs[i])?;
            XYEncodingUtils::check_val(ys[i])?;
        }
        Ok(Self { xs, ys })
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
        for i in 0..xs.len() {
            XYEncodingUtils::check_val(xs[i])?;
            XYEncodingUtils::check_val(ys[i])?;
        }
        Ok(Self { xs, ys, holes })
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
