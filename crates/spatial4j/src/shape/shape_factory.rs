/*******************************************************************************
 * Copyright (c) 2016 David Smiley
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `ShapeFactory.java`, of the Java package
//! `org.locationtech.spatial4j.shape`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

use crate::context::spatial_context::SpatialContext;
use crate::shape::circle::Circle;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::shape_collection::ShapeCollection;

/// Equivalent to `org.locationtech.spatial4j.shape.ShapeFactory` (`public
/// interface`): the factory of a context's shapes.
///
/// The `InvalidShapeException`s Java's factory methods throw for invalid
/// coordinates are the `Err` values.
pub trait ShapeFactory: Send + Sync + Any {
    /// `getSpatialContext()`.
    fn get_spatial_context(&self) -> Arc<SpatialContext>;

    /// `isNormWrapLongitude()`.
    fn is_norm_wrap_longitude(&self) -> bool;

    /// `normX(double)`.
    fn norm_x(&self, x: f64) -> f64;

    /// `normY(double)`.
    fn norm_y(&self, y: f64) -> f64;

    /// `normZ(double)`.
    fn norm_z(&self, z: f64) -> f64;

    /// `normDist(double)`.
    fn norm_dist(&self, d: f64) -> f64;

    /// `verifyX(double)`.
    fn verify_x(&self, x: f64) -> Result<()>;

    /// `verifyY(double)`.
    fn verify_y(&self, y: f64) -> Result<()>;

    /// `verifyZ(double)`.
    fn verify_z(&self, z: f64) -> Result<()>;

    /// `pointXY(double x, double y)`.
    fn point_xy(&self, x: f64, y: f64) -> Result<Arc<dyn Point>>;

    /// `pointLatLon(double latitude, double longitude)`: `pointXY(longitude,
    /// latitude)` by default.
    fn point_lat_lon(&self, latitude: f64, longitude: f64) -> Result<Arc<dyn Point>> {
        self.point_xy(longitude, latitude)
    }

    /// `pointXYZ(double x, double y, double z)`.
    fn point_xyz(&self, x: f64, y: f64, z: f64) -> Result<Arc<dyn Point>>;

    /// `rect(Point lowerLeft, Point upperRight)`.
    fn rect_from_points(&self, lower_left: &dyn Point, upper_right: &dyn Point) -> Result<Arc<dyn Rectangle>>;

    /// `rect(double minX, double maxX, double minY, double maxY)`.
    fn rect(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Result<Arc<dyn Rectangle>>;

    /// `circle(double x, double y, double distance)`.
    fn circle(&self, x: f64, y: f64, distance: f64) -> Result<Arc<dyn Circle>>;

    /// `circle(Point point, double distance)`.
    fn circle_at(&self, point: Arc<dyn Point>, distance: f64) -> Result<Arc<dyn Circle>>;

    /// `lineString(List<Point> points, double buf)` (deprecated in Java).
    fn line_string_from(&self, points: Vec<Arc<dyn Point>>, buf: f64) -> Result<Arc<dyn Shape>>;

    /// `multiShape(List<S> coll)` (deprecated in Java).
    fn multi_shape_from(&self, coll: Vec<Arc<dyn Shape>>) -> Result<Arc<ShapeCollection>>;

    /// `lineString()`: a line string builder.
    fn line_string(&self) -> Box<dyn LineStringBuilder>;

    /// `polygon()`: a polygon builder.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` from a factory without polygons.
    fn polygon(&self) -> Result<Box<dyn PolygonBuilder>>;

    /// `multiShape(Class<T> shapeClass)`: a builder of a collection of
    /// shapes (Java's type token only narrows `add`'s parameter type).
    fn multi_shape(&self) -> Box<dyn MultiShapeBuilder>;

    /// `multiPoint()`.
    fn multi_point(&self) -> Box<dyn MultiPointBuilder>;

    /// `multiLineString()`.
    fn multi_line_string(&self) -> Box<dyn MultiLineStringBuilder>;

    /// `multiPolygon()`.
    fn multi_polygon(&self) -> Box<dyn MultiPolygonBuilder>;

    /// This factory as `Any`, for `instanceof` and casts.
    fn as_any(&self) -> &dyn Any;
}

/// Equivalent to `org.locationtech.spatial4j.shape.ShapeFactory.PointsBuilder`
/// (`interface PointsBuilder<T>`): a builder that accepts points. Java's
/// methods return the builder for chaining; these mutate it in place.
pub trait PointsBuilder {
    /// `pointXY(double x, double y)`.
    fn point_xy(&mut self, x: f64, y: f64) -> Result<()>;

    /// `pointXYZ(double x, double y, double z)`.
    fn point_xyz(&mut self, x: f64, y: f64, z: f64) -> Result<()>;

    /// `pointLatLon(double latitude, double longitude)`: `pointXY(longitude,
    /// latitude)` by default.
    fn point_lat_lon(&mut self, latitude: f64, longitude: f64) -> Result<()> {
        self.point_xy(longitude, latitude)
    }
}

/// Equivalent to
/// `org.locationtech.spatial4j.shape.ShapeFactory.LineStringBuilder`
/// (`interface LineStringBuilder extends PointsBuilder<LineStringBuilder>`).
pub trait LineStringBuilder: PointsBuilder {
    /// `buffer(double distance)`.
    fn buffer(&mut self, distance: f64);

    /// `build()`.
    fn build(&mut self) -> Result<Arc<dyn Shape>>;
}

/// Equivalent to `org.locationtech.spatial4j.shape.ShapeFactory.PolygonBuilder`
/// (`interface PolygonBuilder extends PointsBuilder<PolygonBuilder>`).
pub trait PolygonBuilder: PointsBuilder {
    /// `hole()`: a builder of a hole, which `endHole()` adds to this
    /// polygon.
    fn hole(&mut self) -> Box<dyn HoleBuilder + '_>;

    /// `build()`: never a rectangle.
    fn build(&mut self) -> Result<Arc<dyn Shape>>;

    /// `buildOrRect()`.
    fn build_or_rect(&mut self) -> Result<Arc<dyn Shape>>;
}

/// Equivalent to
/// `org.locationtech.spatial4j.shape.ShapeFactory.PolygonBuilder.HoleBuilder`
/// (`interface HoleBuilder extends PointsBuilder<HoleBuilder>`).
pub trait HoleBuilder: PointsBuilder {
    /// `endHole()`: adds the hole to the polygon builder that made this one.
    fn end_hole(self: Box<Self>) -> Result<()>;
}

/// Equivalent to
/// `org.locationtech.spatial4j.shape.ShapeFactory.MultiShapeBuilder`
/// (`interface MultiShapeBuilder<T extends Shape>`).
pub trait MultiShapeBuilder {
    /// `add(T shape)`.
    fn add(&mut self, shape: Arc<dyn Shape>);

    /// `build()`.
    fn build(&mut self) -> Result<Arc<dyn Shape>>;
}

/// Equivalent to
/// `org.locationtech.spatial4j.shape.ShapeFactory.MultiPointBuilder`
/// (`interface MultiPointBuilder extends PointsBuilder<MultiPointBuilder>`).
pub trait MultiPointBuilder: PointsBuilder {
    /// `build()`.
    fn build(&mut self) -> Result<Arc<dyn Shape>>;
}

/// Equivalent to
/// `org.locationtech.spatial4j.shape.ShapeFactory.MultiLineStringBuilder`
/// (`interface MultiLineStringBuilder`).
pub trait MultiLineStringBuilder {
    /// `lineString()`: a new line string builder, to `add` back.
    fn line_string(&mut self) -> Box<dyn LineStringBuilder>;

    /// `add(LineStringBuilder)`: builds it and adds the result.
    fn add(&mut self, line_string_builder: &mut dyn LineStringBuilder) -> Result<()>;

    /// `build()`.
    fn build(&mut self) -> Result<Arc<dyn Shape>>;
}

/// Equivalent to
/// `org.locationtech.spatial4j.shape.ShapeFactory.MultiPolygonBuilder`
/// (`interface MultiPolygonBuilder`).
pub trait MultiPolygonBuilder {
    /// `polygon()`: a new polygon builder, to `add` back.
    fn polygon(&mut self) -> Result<Box<dyn PolygonBuilder>>;

    /// `add(PolygonBuilder)`: builds it and adds the result.
    fn add(&mut self, polygon_builder: &mut dyn PolygonBuilder) -> Result<()>;

    /// `build()`.
    fn build(&mut self) -> Result<Arc<dyn Shape>>;
}
