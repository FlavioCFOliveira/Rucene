/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `ShapeFactoryImpl.java`, of the Java package
//! `org.locationtech.spatial4j.shape.impl`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene::util::java_fmt::double_to_string;

use crate::context::spatial_context::{SpatialContext, SpatialContextRef};
use crate::context::spatial_context_factory::SpatialContextFactoryOps;
use crate::distance::distance_utils::DistanceUtils;
use crate::exception::invalid_shape_exception::InvalidShapeException;
use crate::shape::circle::Circle;
use crate::shape::r#impl::buffered_line_string::BufferedLineString;
use crate::shape::r#impl::circle_impl::CircleImpl;
use crate::shape::r#impl::geo_circle::GeoCircle;
use crate::shape::r#impl::point_impl::PointImpl;
use crate::shape::r#impl::rectangle_impl::RectangleImpl;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::shape_collection::ShapeCollection;
use crate::shape::shape_factory::{
    LineStringBuilder, MultiLineStringBuilder, MultiPointBuilder, MultiPolygonBuilder, MultiShapeBuilder,
    PointsBuilder, PolygonBuilder, ShapeFactory,
};

/// Equivalent to `org.locationtech.spatial4j.shape.impl.ShapeFactoryImpl`
/// (`public class ShapeFactoryImpl implements ShapeFactory`): the default
/// shape factory.
///
/// The context owns its factory, so the factory refers back to it weakly;
/// the shapes it makes hold the context strongly.
#[derive(Debug)]
pub struct ShapeFactoryImpl {
    ctx: Weak<SpatialContext>,
    norm_wrap_longitude: bool,
}

impl ShapeFactoryImpl {
    /// `ShapeFactoryImpl(SpatialContext ctx, SpatialContextFactory factory)`.
    pub fn new(ctx: &Arc<SpatialContext>, factory: &dyn SpatialContextFactoryOps) -> Self {
        Self {
            ctx: Arc::downgrade(ctx),
            norm_wrap_longitude: ctx.is_geo() && factory.base().norm_wrap_longitude,
        }
    }

    /// `ctx`.
    pub fn ctx(&self) -> Arc<SpatialContext> {
        self.ctx.upgrade().expect("the SpatialContext of a ShapeFactoryImpl is alive")
    }

    fn ctx_ref(&self) -> Option<SpatialContextRef> {
        Some(SpatialContextRef::from(&self.ctx()))
    }
}

impl ShapeFactory for ShapeFactoryImpl {
    fn get_spatial_context(&self) -> Arc<SpatialContext> {
        self.ctx()
    }

    fn is_norm_wrap_longitude(&self) -> bool {
        self.norm_wrap_longitude
    }

    fn norm_x(&self, x: f64) -> f64 {
        if self.norm_wrap_longitude {
            DistanceUtils::norm_lon_deg(x)
        } else {
            x
        }
    }

    fn norm_y(&self, y: f64) -> f64 {
        y
    }

    fn norm_z(&self, z: f64) -> f64 {
        z
    }

    fn norm_dist(&self, d: f64) -> f64 {
        d
    }

    fn verify_x(&self, x: f64) -> Result<()> {
        let bounds = self.ctx().get_world_bounds();
        if x < bounds.get_min_x() || x > bounds.get_max_x() {
            //NaN will pass
            return Err(InvalidShapeException::error(format!(
                "Bad X value {} is not in boundary {}",
                double_to_string(x),
                bounds
            )));
        }
        Ok(())
    }

    fn verify_y(&self, y: f64) -> Result<()> {
        let bounds = self.ctx().get_world_bounds();
        if y < bounds.get_min_y() || y > bounds.get_max_y() {
            //NaN will pass
            return Err(InvalidShapeException::error(format!(
                "Bad Y value {} is not in boundary {}",
                double_to_string(y),
                bounds
            )));
        }
        Ok(())
    }

    fn verify_z(&self, _z: f64) -> Result<()> {
        // bounds has no 'z' for this simple shapeFactory
        Ok(())
    }

    fn point_xy(&self, x: f64, y: f64) -> Result<Arc<dyn Point>> {
        self.verify_x(x)?;
        self.verify_y(y)?;
        Ok(PointImpl::new(x, y, self.ctx_ref()))
    }

    fn point_xyz(&self, x: f64, y: f64, _z: f64) -> Result<Arc<dyn Point>> {
        self.point_xy(x, y) // or throw?
    }

    fn rect_from_points(&self, lower_left: &dyn Point, upper_right: &dyn Point) -> Result<Arc<dyn Rectangle>> {
        self.rect(lower_left.get_x(), upper_right.get_x(), lower_left.get_y(), upper_right.get_y())
    }

    fn rect(&self, mut min_x: f64, mut max_x: f64, min_y: f64, max_y: f64) -> Result<Arc<dyn Rectangle>> {
        let ctx = self.ctx();
        let bounds = ctx.get_world_bounds();
        // Y
        if min_y < bounds.get_min_y() || max_y > bounds.get_max_y() {
            //NaN will pass
            return Err(InvalidShapeException::error(format!(
                "Y values [{} to {}] not in boundary {}",
                double_to_string(min_y),
                double_to_string(max_y),
                bounds
            )));
        }
        if min_y > max_y {
            return Err(InvalidShapeException::error(format!(
                "maxY must be >= minY: {} to {}",
                double_to_string(min_y),
                double_to_string(max_y)
            )));
        }
        // X
        if ctx.is_geo() {
            self.verify_x(min_x)?;
            self.verify_x(max_x)?;
            //TODO consider removing this logic so that there is no normalization here
            //if (minX != maxX) {   USUALLY TRUE, inline check below
            //If an edge coincides with the dateline then don't make this rect cross it
            if min_x == 180.0 && min_x != max_x {
                min_x = -180.0;
            } else if max_x == -180.0 && min_x != max_x {
                max_x = 180.0;
            }
        } else {
            if min_x < bounds.get_min_x() || max_x > bounds.get_max_x() {
                //NaN will pass
                return Err(InvalidShapeException::error(format!(
                    "X values [{} to {}] not in boundary {}",
                    double_to_string(min_x),
                    double_to_string(max_x),
                    bounds
                )));
            }
            if min_x > max_x {
                return Err(InvalidShapeException::error(format!(
                    "maxX must be >= minX: {} to {}",
                    double_to_string(min_x),
                    double_to_string(max_x)
                )));
            }
        }
        Ok(RectangleImpl::new(min_x, max_x, min_y, max_y, self.ctx_ref()))
    }

    fn circle(&self, x: f64, y: f64, distance: f64) -> Result<Arc<dyn Circle>> {
        let p = self.point_xy(x, y)?;
        self.circle_at(p, distance)
    }

    fn circle_at(&self, point: Arc<dyn Point>, mut distance: f64) -> Result<Arc<dyn Circle>> {
        if distance < 0.0 {
            return Err(InvalidShapeException::error(format!(
                "distance must be >= 0; got {}",
                double_to_string(distance)
            )));
        }
        let ctx = self.ctx();
        if ctx.is_geo() {
            if distance > 180.0 {
                // (it's debatable whether to error or not)
                //throw new InvalidShapeException("distance must be <= 180; got " + distance);
                distance = 180.0;
            }
            Ok(GeoCircle::new(point, distance, &ctx))
        } else {
            //(it's debatable whether to error or not)
            //  double maxDist = ctx.getMaxDistance();
            //  if (maxDist != 0 && distance > maxDist)
            //    throw ...
            Ok(CircleImpl::new(point, distance, &ctx))
        }
    }

    fn line_string_from(&self, points: Vec<Arc<dyn Point>>, buf: f64) -> Result<Arc<dyn Shape>> {
        let ctx = self.ctx();
        Ok(BufferedLineString::with_skew(&points, buf, ctx.is_geo(), &ctx)?)
    }

    fn multi_shape_from(&self, coll: Vec<Arc<dyn Shape>>) -> Result<Arc<ShapeCollection>> {
        ShapeCollection::new(coll, &self.ctx())
    }

    fn line_string(&self) -> Box<dyn LineStringBuilder> {
        Box::new(ImplLineStringBuilder { factory: self.ctx(), points: Vec::new(), buffer_distance: 0.0 })
    }

    fn polygon(&self) -> Result<Box<dyn PolygonBuilder>> {
        Err(LuceneError::unsupported_operation(
            "Unsupported shape of this SpatialContext. Try JTS or Geo3D.",
        ))
    }

    fn multi_shape(&self) -> Box<dyn MultiShapeBuilder> {
        Box::new(GeneralShapeMultiShapeBuilder::new(self.ctx()))
    }

    fn multi_point(&self) -> Box<dyn MultiPointBuilder> {
        Box::new(GeneralShapeMultiShapeBuilder::new(self.ctx()))
    }

    fn multi_line_string(&self) -> Box<dyn MultiLineStringBuilder> {
        Box::new(GeneralShapeMultiShapeBuilder::new(self.ctx()))
    }

    fn multi_polygon(&self) -> Box<dyn MultiPolygonBuilder> {
        Box::new(GeneralShapeMultiShapeBuilder::new(self.ctx()))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The anonymous `LineStringBuilder` of `ShapeFactoryImpl.lineString()`.
struct ImplLineStringBuilder {
    /// The enclosing factory, reached through its context.
    factory: Arc<SpatialContext>,
    points: Vec<Arc<dyn Point>>,
    buffer_distance: f64,
}

impl PointsBuilder for ImplLineStringBuilder {
    fn point_xy(&mut self, x: f64, y: f64) -> Result<()> {
        self.points.push(self.factory.get_shape_factory().point_xy(x, y)?);
        Ok(())
    }

    fn point_xyz(&mut self, x: f64, y: f64, z: f64) -> Result<()> {
        self.points.push(self.factory.get_shape_factory().point_xyz(x, y, z)?);
        Ok(())
    }

    fn point_lat_lon(&mut self, latitude: f64, longitude: f64) -> Result<()> {
        self.points.push(self.factory.get_shape_factory().point_lat_lon(latitude, longitude)?);
        Ok(())
    }
}

impl LineStringBuilder for ImplLineStringBuilder {
    fn buffer(&mut self, distance: f64) {
        self.buffer_distance = distance;
    }

    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        Ok(BufferedLineString::with_skew(&self.points, self.buffer_distance, false, &self.factory)?)
    }
}

/// Equivalent to
/// `org.locationtech.spatial4j.shape.impl.ShapeFactoryImpl.GeneralShapeMultiShapeBuilder`
/// (`protected class GeneralShapeMultiShapeBuilder<T extends Shape>
/// implements MultiShapeBuilder<T>, MultiPointBuilder,
/// MultiLineStringBuilder, MultiPolygonBuilder`): collects shapes into a
/// [`ShapeCollection`].
pub struct GeneralShapeMultiShapeBuilder {
    /// The context of the enclosing `ShapeFactoryImpl`.
    ctx: Arc<SpatialContext>,
    /// `shapes`.
    pub(crate) shapes: Vec<Arc<dyn Shape>>,
}

impl GeneralShapeMultiShapeBuilder {
    /// `new GeneralShapeMultiShapeBuilder<>()` of the factory of `ctx`.
    pub fn new(ctx: Arc<SpatialContext>) -> Self {
        Self { ctx, shapes: Vec::new() }
    }

    fn factory(&self) -> Arc<dyn ShapeFactory> {
        self.ctx.get_shape_factory()
    }

    fn build_collection(&mut self) -> Result<Arc<dyn Shape>> {
        Ok(ShapeCollection::new(self.shapes.clone(), &self.ctx)?)
    }
}

impl MultiShapeBuilder for GeneralShapeMultiShapeBuilder {
    fn add(&mut self, shape: Arc<dyn Shape>) {
        self.shapes.push(shape);
    }

    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        self.build_collection()
    }
}

impl PointsBuilder for GeneralShapeMultiShapeBuilder {
    fn point_xy(&mut self, x: f64, y: f64) -> Result<()> {
        let p = self.factory().point_xy(x, y)?;
        self.shapes.push(p);
        Ok(())
    }

    fn point_xyz(&mut self, x: f64, y: f64, z: f64) -> Result<()> {
        let p = self.factory().point_xyz(x, y, z)?;
        self.shapes.push(p);
        Ok(())
    }

    fn point_lat_lon(&mut self, latitude: f64, longitude: f64) -> Result<()> {
        let p = self.factory().point_lat_lon(latitude, longitude)?;
        self.shapes.push(p);
        Ok(())
    }
}

impl MultiPointBuilder for GeneralShapeMultiShapeBuilder {
    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        self.build_collection()
    }
}

impl MultiLineStringBuilder for GeneralShapeMultiShapeBuilder {
    fn line_string(&mut self) -> Box<dyn LineStringBuilder> {
        self.factory().line_string()
    }

    fn add(&mut self, line_string_builder: &mut dyn LineStringBuilder) -> Result<()> {
        let s = line_string_builder.build()?;
        self.shapes.push(s);
        Ok(())
    }

    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        self.build_collection()
    }
}

impl MultiPolygonBuilder for GeneralShapeMultiShapeBuilder {
    fn polygon(&mut self) -> Result<Box<dyn PolygonBuilder>> {
        self.factory().polygon()
    }

    fn add(&mut self, polygon_builder: &mut dyn PolygonBuilder) -> Result<()> {
        let s = polygon_builder.build()?;
        self.shapes.push(s);
        Ok(())
    }

    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        self.build_collection()
    }
}
