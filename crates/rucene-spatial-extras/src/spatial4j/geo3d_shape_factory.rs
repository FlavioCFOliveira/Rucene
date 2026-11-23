/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The types declared in `Geo3dShapeFactory.java`, of the Java package
//! `org.apache.lucene.spatial.spatial4j`.

use std::any::{Any, TypeId};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene_spatial3d::geom::geo_area_shape::GeoAreaShape;
use rucene_spatial3d::geom::geo_b_box_factory::GeoBBoxFactory;
use rucene_spatial3d::geom::geo_circle::GeoCircle;
use rucene_spatial3d::geom::geo_circle_factory::GeoCircleFactory;
use rucene_spatial3d::geom::geo_composite_area_shape::GeoCompositeAreaShape;
use rucene_spatial3d::geom::geo_path_factory::GeoPathFactory;
use rucene_spatial3d::geom::geo_point::GeoPoint;
use rucene_spatial3d::geom::geo_point_shape_factory::GeoPointShapeFactory;
use rucene_spatial3d::geom::geo_polygon_factory::{GeoPolygonFactory, PolygonDescription};
use rucene_spatial3d::geom::geo_s2_shape_factory::GeoS2ShapeFactory;
use rucene_spatial3d::geom::planet_model::PlanetModel;
use s2_geometry::s2_cell::S2Cell;
use s2_geometry::s2_cell_id::S2CellId;
use s2_geometry::s2_point::S2Point;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::SpatialContextFactoryOps;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::exception::invalid_shape_exception::InvalidShapeException;
use spatial4j::shape::circle::Circle;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::shape_collection::ShapeCollection;
use spatial4j::shape::shape_factory::{
    HoleBuilder, LineStringBuilder, MultiLineStringBuilder, MultiPointBuilder, MultiPolygonBuilder, MultiShapeBuilder,
    PointsBuilder, PolygonBuilder, ShapeFactory,
};

use crate::jdk::unsupported;
use crate::prefix::tree::s2_shape_factory::S2ShapeFactory;
use crate::spatial4j::geo3d_circle_shape::Geo3dCircleShape;
use crate::spatial4j::geo3d_point_shape::Geo3dPointShape;
use crate::spatial4j::geo3d_rectangle_shape::Geo3dRectangleShape;
use crate::spatial4j::geo3d_shape::{geo3d_shape_of, Geo3dShape};
use crate::spatial4j::geo3d_spatial_context_factory::geo3d_factory;

/// `DEFAULT_CIRCLE_ACCURACY`.
const DEFAULT_CIRCLE_ACCURACY: f64 = 1e-4;

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dShapeFactory`
/// (`public class Geo3dShapeFactory implements S2ShapeFactory`): geo3d
/// implementation of `S2ShapeFactory`.
///
/// The context owns its shape factory, so the factory refers back to it
/// weakly (as spatial4j's `ShapeFactoryImpl` does).
///
/// **Divergence.** Java's multi-line-string and multi-polygon builders keep
/// the builders `add` receives and build them in `build()`
/// (`Geo3dShapeFactory.java:404-447`); spatial4j's port lends `add` the
/// builder only for the call, so it is built when added, as the spatial4j
/// port's own builders do. The result differs only for a builder changed
/// after it was added. Likewise the multi-shape builder hands its composite
/// to the shape `build()` returns and starts a new one, where Java's shape
/// keeps sharing the composite with the builder.
#[derive(Debug)]
pub struct Geo3dShapeFactory {
    norm_wrap_longitude: bool,
    context: Weak<SpatialContext>,
    planet_model: Arc<PlanetModel>,
    circle_accuracy: AtomicU64,
}

impl Geo3dShapeFactory {
    /// `Geo3dShapeFactory(SpatialContext context, SpatialContextFactory
    /// factory)`.
    ///
    /// # Errors
    ///
    /// `ClassCastException` when `factory` is not a
    /// `Geo3dSpatialContextFactory`.
    pub fn new(context: &Arc<SpatialContext>, factory: &dyn SpatialContextFactoryOps) -> Result<Self> {
        Ok(Self {
            context: Arc::downgrade(context),
            planet_model: geo3d_factory(factory)?.planet_model(),
            norm_wrap_longitude: context.is_geo() && factory.base().norm_wrap_longitude,
            circle_accuracy: AtomicU64::new(DEFAULT_CIRCLE_ACCURACY.to_bits()),
        })
    }

    /// `setCircleAccuracy(double circleAccuracy)`: set the accuracy for
    /// geo3d circles.
    pub fn set_circle_accuracy(&self, circle_accuracy: f64) {
        self.circle_accuracy.store(circle_accuracy.to_bits(), Ordering::Relaxed);
    }

    fn circle_accuracy(&self) -> f64 {
        f64::from_bits(self.circle_accuracy.load(Ordering::Relaxed))
    }

    fn ctx(&self) -> Arc<SpatialContext> {
        self.context.upgrade().expect("the SpatialContext of a Geo3dShapeFactory is alive")
    }

    /// `getGeoPoint(S2Point point)`.
    fn get_geo_point(&self, point: &S2Point) -> GeoPoint {
        self.planet_model.create_surface_point_xyz(point.get(0), point.get(1), point.get(2))
    }

    fn builder(&self) -> Geo3dPointBuilder {
        Geo3dPointBuilder {
            planet_model: Arc::clone(&self.planet_model),
            context: self.ctx(),
            points: Vec::new(),
        }
    }
}

impl ShapeFactory for Geo3dShapeFactory {
    fn get_spatial_context(&self) -> Arc<SpatialContext> {
        self.ctx()
    }

    fn is_norm_wrap_longitude(&self) -> bool {
        self.norm_wrap_longitude
    }

    fn norm_x(&self, mut x: f64) -> f64 {
        if self.norm_wrap_longitude {
            x = DistanceUtils::norm_lon_deg(x);
        }
        x
    }

    fn norm_y(&self, y: f64) -> f64 {
        y
    }

    fn norm_z(&self, z: f64) -> f64 {
        z
    }

    fn norm_dist(&self, distance: f64) -> f64 {
        distance
    }

    fn verify_x(&self, x: f64) -> Result<()> {
        let bounds = self.ctx().get_world_bounds();
        if x < bounds.get_min_x() || x > bounds.get_max_x() {
            return Err(InvalidShapeException::error(format!(
                "Bad X value {} is not in boundary {bounds}",
                crate::jdk::d2s(x)
            )));
        }
        Ok(())
    }

    fn verify_y(&self, y: f64) -> Result<()> {
        let bounds = self.ctx().get_world_bounds();
        if y < bounds.get_min_y() || y > bounds.get_max_y() {
            return Err(InvalidShapeException::error(format!(
                "Bad Y value {} is not in boundary {bounds}",
                crate::jdk::d2s(y)
            )));
        }
        Ok(())
    }

    fn verify_z(&self, _v: f64) -> Result<()> {
        Ok(())
    }

    fn point_xy(&self, x: f64, y: f64) -> Result<Arc<dyn Point>> {
        let point = GeoPointShapeFactory::make_geo_point_shape(
            &self.planet_model,
            y * DistanceUtils::DEGREES_TO_RADIANS,
            x * DistanceUtils::DEGREES_TO_RADIANS,
        )?;
        Ok(Geo3dPointShape::new(point, self.ctx()))
    }

    fn point_xyz(&self, x: f64, y: f64, z: f64) -> Result<Arc<dyn Point>> {
        let point = GeoPoint::new(x, y, z);
        let point_shape =
            GeoPointShapeFactory::make_geo_point_shape(&self.planet_model, point.get_latitude(), point.get_longitude())?;
        Ok(Geo3dPointShape::new(point_shape, self.ctx()))
        // throw new UnsupportedOperationException();
    }

    fn rect_from_points(&self, point: &dyn Point, point1: &dyn Point) -> Result<Arc<dyn Rectangle>> {
        self.rect(point.get_x(), point1.get_x(), point.get_y(), point1.get_y())
    }

    fn rect(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Result<Arc<dyn Rectangle>> {
        let b_box = GeoBBoxFactory::make_geo_bbox(
            &self.planet_model,
            max_y * DistanceUtils::DEGREES_TO_RADIANS,
            min_y * DistanceUtils::DEGREES_TO_RADIANS,
            min_x * DistanceUtils::DEGREES_TO_RADIANS,
            max_x * DistanceUtils::DEGREES_TO_RADIANS,
        )?;
        Ok(Geo3dRectangleShape::with_bounds(b_box, self.ctx(), min_x, max_x, min_y, max_y))
    }

    fn circle(&self, x: f64, y: f64, distance: f64) -> Result<Arc<dyn Circle>> {
        let circle: Arc<dyn GeoCircle> = if self.planet_model.is_sphere() {
            GeoCircleFactory::make_geo_circle(
                &self.planet_model,
                y * DistanceUtils::DEGREES_TO_RADIANS,
                x * DistanceUtils::DEGREES_TO_RADIANS,
                distance * DistanceUtils::DEGREES_TO_RADIANS,
            )?
        } else {
            // accuracy is defined as a linear distance in this class. At tiny distances, linear distance
            // can be approximated to surface distance in radians.
            GeoCircleFactory::make_exact_geo_circle(
                &self.planet_model,
                y * DistanceUtils::DEGREES_TO_RADIANS,
                x * DistanceUtils::DEGREES_TO_RADIANS,
                distance * DistanceUtils::DEGREES_TO_RADIANS,
                self.circle_accuracy() * DistanceUtils::DEGREES_TO_RADIANS,
            )?
        };
        Ok(Geo3dCircleShape::new(circle, self.ctx()))
    }

    fn circle_at(&self, point: Arc<dyn Point>, distance: f64) -> Result<Arc<dyn Circle>> {
        self.circle(point.get_x(), point.get_y(), distance)
    }

    fn line_string_from(&self, list: Vec<Arc<dyn Point>>, distance: f64) -> Result<Arc<dyn Shape>> {
        let mut builder = self.line_string();
        for point in &list {
            builder.point_xy(point.get_x(), point.get_y())?;
        }
        builder.buffer(distance);
        builder.build()
    }

    fn multi_shape_from(&self, _list: Vec<Arc<dyn Shape>>) -> Result<Arc<ShapeCollection>> {
        Err(unsupported())
    }

    fn line_string(&self) -> Box<dyn LineStringBuilder> {
        Box::new(Geo3dLineStringBuilder {
            points: self.builder(),
            distance: 0.0,
        })
    }

    fn polygon(&self) -> Result<Box<dyn PolygonBuilder>> {
        Ok(Box::new(Geo3dPolygonBuilder {
            points: self.builder(),
            poly_holes: Vec::new(),
        }))
    }

    fn multi_shape(&self) -> Box<dyn MultiShapeBuilder> {
        Box::new(Geo3dMultiShapeBuilder {
            composite: GeoCompositeAreaShape::new(&self.planet_model),
            context: self.ctx(),
        })
    }

    fn multi_point(&self) -> Box<dyn MultiPointBuilder> {
        Box::new(Geo3dMultiPointBuilder { points: self.builder() })
    }

    fn multi_line_string(&self) -> Box<dyn MultiLineStringBuilder> {
        Box::new(Geo3dMultiLineBuilder {
            factory: self.builder(),
            shapes: Vec::new(),
        })
    }

    fn multi_polygon(&self) -> Box<dyn MultiPolygonBuilder> {
        Box::new(Geo3dMultiPolygonBuilder {
            factory: self.builder(),
            shapes: Vec::new(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_interface(self: Arc<Self>, interface: TypeId) -> Option<Box<dyn Any + Send + Sync>> {
        (interface == TypeId::of::<dyn S2ShapeFactory>())
            .then(|| Box::new(self as Arc<dyn S2ShapeFactory>) as Box<dyn Any + Send + Sync>)
    }
}

impl S2ShapeFactory for Geo3dShapeFactory {
    fn get_s2_cell_shape(&self, cell_id: S2CellId) -> Arc<dyn Shape> {
        let cell = S2Cell::new(cell_id);
        let point1 = self.get_geo_point(&cell.get_vertex_raw(0));
        let point2 = self.get_geo_point(&cell.get_vertex_raw(1));
        let point3 = self.get_geo_point(&cell.get_vertex_raw(2));
        let point4 = self.get_geo_point(&cell.get_vertex_raw(3));
        let shape: Arc<dyn GeoAreaShape> =
            GeoS2ShapeFactory::make_geo_s2_shape(&self.planet_model, point1, point2, point3, point4)
                .unwrap_or_else(|e| panic!("{e}"));
        Arc::new(Geo3dShape::new(shape, self.ctx()))
    }
}

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dShapeFactory.Geo3dPointBuilder`
/// (`private class Geo3dPointBuilder<T> implements PointsBuilder<T>`):
/// geo3d implementation of `PointsBuilder` interface to generate
/// `GeoPoint`.
pub(crate) struct Geo3dPointBuilder {
    planet_model: Arc<PlanetModel>,
    context: Arc<SpatialContext>,
    points: Vec<GeoPoint>,
}

impl PointsBuilder for Geo3dPointBuilder {
    fn point_xy(&mut self, x: f64, y: f64) -> Result<()> {
        let point = GeoPoint::new_lat_lon(
            &self.planet_model,
            y * DistanceUtils::DEGREES_TO_RADIANS,
            x * DistanceUtils::DEGREES_TO_RADIANS,
        )?;
        self.points.push(point);
        Ok(())
    }

    fn point_xyz(&mut self, x: f64, y: f64, z: f64) -> Result<()> {
        let point = GeoPoint::new(x, y, z);
        if !self.points.contains(&point) {
            self.points.push(point);
        }
        Ok(())
    }
}

/// Implements `PointsBuilder` over the embedded `Geo3dPointBuilder` of a
/// subclass.
macro_rules! delegate_points_builder {
    ($t:ty) => {
        impl PointsBuilder for $t {
            fn point_xy(&mut self, x: f64, y: f64) -> Result<()> {
                self.points.point_xy(x, y)
            }

            fn point_xyz(&mut self, x: f64, y: f64, z: f64) -> Result<()> {
                self.points.point_xyz(x, y, z)
            }
        }
    };
}

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dShapeFactory.Geo3dLineStringBuilder`
/// (`private class Geo3dLineStringBuilder extends
/// Geo3dPointBuilder<LineStringBuilder> implements LineStringBuilder`):
/// geo3d implementation of `LineStringBuilder` to generate line strings.
pub(crate) struct Geo3dLineStringBuilder {
    points: Geo3dPointBuilder,
    distance: f64,
}

delegate_points_builder!(Geo3dLineStringBuilder);

impl LineStringBuilder for Geo3dLineStringBuilder {
    fn buffer(&mut self, distance: f64) {
        self.distance = distance;
    }

    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        let path = GeoPathFactory::make_geo_path(&self.points.planet_model, self.distance, &self.points.points)?;
        let shape: Arc<dyn GeoAreaShape> = path;
        Ok(Arc::new(Geo3dShape::new(shape, Arc::clone(&self.points.context))))
    }
}

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dShapeFactory.Geo3dPolygonBuilder`
/// (`private class Geo3dPolygonBuilder extends Geo3dPointBuilder<PolygonBuilder>
/// implements PolygonBuilder`): geo3d implementation of `PolygonBuilder` to
/// generate polygons.
pub(crate) struct Geo3dPolygonBuilder {
    points: Geo3dPointBuilder,
    poly_holes: Vec<PolygonDescription>,
}

delegate_points_builder!(Geo3dPolygonBuilder);

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dShapeFactory.Geo3dPolygonBuilder.Geo3dHoleBuilder`
/// (`class Geo3dHoleBuilder extends Geo3dPointBuilder<PolygonBuilder.HoleBuilder>
/// implements PolygonBuilder.HoleBuilder`): geo3d implementation of
/// `HoleBuilder` to generate polygon holes.
pub(crate) struct Geo3dHoleBuilder<'a> {
    points: Geo3dPointBuilder,
    polygon: &'a mut Geo3dPolygonBuilder,
}

impl PointsBuilder for Geo3dHoleBuilder<'_> {
    fn point_xy(&mut self, x: f64, y: f64) -> Result<()> {
        self.points.point_xy(x, y)
    }

    fn point_xyz(&mut self, x: f64, y: f64, z: f64) -> Result<()> {
        self.points.point_xyz(x, y, z)
    }
}

impl HoleBuilder for Geo3dHoleBuilder<'_> {
    fn end_hole(self: Box<Self>) -> Result<()> {
        let this = *self;
        this.polygon.poly_holes.push(PolygonDescription::new(this.points.points));
        Ok(())
    }
}

impl PolygonBuilder for Geo3dPolygonBuilder {
    fn hole(&mut self) -> Box<dyn HoleBuilder + '_> {
        let points = Geo3dPointBuilder {
            planet_model: Arc::clone(&self.points.planet_model),
            context: Arc::clone(&self.points.context),
            points: Vec::new(),
        };
        Box::new(Geo3dHoleBuilder { points, polygon: self })
    }

    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        let description = PolygonDescription::with_holes(self.points.points.clone(), self.poly_holes.clone());
        let polygon = GeoPolygonFactory::make_geo_polygon_from_description(&self.points.planet_model, &description)?;
        let Some(polygon) = polygon else {
            return Err(InvalidShapeException::error("Invalid polygon, all points are coplanar"));
        };
        let shape: Arc<dyn GeoAreaShape> = polygon;
        Ok(Arc::new(Geo3dShape::new(shape, Arc::clone(&self.points.context))))
    }

    fn build_or_rect(&mut self) -> Result<Arc<dyn Shape>> {
        self.build()
    }
}

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dShapeFactory.Geo3dMultiPointBuilder`
/// (`private class Geo3dMultiPointBuilder extends
/// Geo3dPointBuilder<MultiPointBuilder> implements MultiPointBuilder`):
/// geo3d implementation of `MultiPointBuilder` to generate multi-points.
pub(crate) struct Geo3dMultiPointBuilder {
    points: Geo3dPointBuilder,
}

delegate_points_builder!(Geo3dMultiPointBuilder);

impl MultiPointBuilder for Geo3dMultiPointBuilder {
    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        let planet_model = &self.points.planet_model;
        let mut area_shape = GeoCompositeAreaShape::new(planet_model);
        for point in &self.points.points {
            let point_shape =
                GeoPointShapeFactory::make_geo_point_shape(planet_model, point.get_latitude(), point.get_longitude())?;
            let shape: Arc<dyn GeoAreaShape> = point_shape;
            area_shape.add_shape(shape)?;
        }
        Ok(Arc::new(Geo3dShape::new(Arc::new(area_shape), Arc::clone(&self.points.context))))
    }
}

/// `(Geo3dShape<?>) shape`'s wrapped shape.
fn wrapped(shape: &dyn Shape) -> Result<Arc<dyn GeoAreaShape>> {
    geo3d_shape_of(shape).map(Geo3dShape::shape).ok_or_else(|| {
        LuceneError::unchecked(
            "java.lang.ClassCastException",
            format!(
                "class {} cannot be cast to class org.apache.lucene.spatial.spatial4j.Geo3dShape",
                shape.java_class_name()
            ),
        )
    })
}

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dShapeFactory.Geo3dMultiLineBuilder`
/// (`private class Geo3dMultiLineBuilder implements MultiLineStringBuilder`):
/// geo3d implementation of `MultiLineStringBuilder` to generate
/// multi-lines.
pub(crate) struct Geo3dMultiLineBuilder {
    factory: Geo3dPointBuilder,
    /// The shapes of the builders added (see the divergence on
    /// [`Geo3dShapeFactory`]).
    shapes: Vec<Arc<dyn GeoAreaShape>>,
}

impl MultiLineStringBuilder for Geo3dMultiLineBuilder {
    fn line_string(&mut self) -> Box<dyn LineStringBuilder> {
        Box::new(Geo3dLineStringBuilder {
            points: Geo3dPointBuilder {
                planet_model: Arc::clone(&self.factory.planet_model),
                context: Arc::clone(&self.factory.context),
                points: Vec::new(),
            },
            distance: 0.0,
        })
    }

    fn add(&mut self, line_string_builder: &mut dyn LineStringBuilder) -> Result<()> {
        let shape = line_string_builder.build()?;
        self.shapes.push(wrapped(&*shape)?);
        Ok(())
    }

    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        let mut area_shape = GeoCompositeAreaShape::new(&self.factory.planet_model);
        for shape in &self.shapes {
            area_shape.add_shape(Arc::clone(shape))?;
        }
        Ok(Arc::new(Geo3dShape::new(Arc::new(area_shape), Arc::clone(&self.factory.context))))
    }
}

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dShapeFactory.Geo3dMultiPolygonBuilder`
/// (`private class Geo3dMultiPolygonBuilder implements MultiPolygonBuilder`):
/// geo3d implementation of `MultiPolygonBuilder` to generate
/// multi-polygons. We have chosen to use a composite shape but it might be
/// possible to use `GeoComplexPolygon`.
pub(crate) struct Geo3dMultiPolygonBuilder {
    factory: Geo3dPointBuilder,
    /// The shapes of the builders added (see the divergence on
    /// [`Geo3dShapeFactory`]).
    shapes: Vec<Arc<dyn GeoAreaShape>>,
}

impl MultiPolygonBuilder for Geo3dMultiPolygonBuilder {
    fn polygon(&mut self) -> Result<Box<dyn PolygonBuilder>> {
        Ok(Box::new(Geo3dPolygonBuilder {
            points: Geo3dPointBuilder {
                planet_model: Arc::clone(&self.factory.planet_model),
                context: Arc::clone(&self.factory.context),
                points: Vec::new(),
            },
            poly_holes: Vec::new(),
        }))
    }

    fn add(&mut self, polygon_builder: &mut dyn PolygonBuilder) -> Result<()> {
        let shape = polygon_builder.build()?;
        self.shapes.push(wrapped(&*shape)?);
        Ok(())
    }

    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        let mut area_shape = GeoCompositeAreaShape::new(&self.factory.planet_model);
        for shape in &self.shapes {
            area_shape.add_shape(Arc::clone(shape))?;
        }
        Ok(Arc::new(Geo3dShape::new(Arc::new(area_shape), Arc::clone(&self.factory.context))))
    }
}

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dShapeFactory.Geo3dMultiShapeBuilder`
/// (`private class Geo3dMultiShapeBuilder<T extends Shape> implements
/// MultiShapeBuilder<T>`): geo3d implementation of `MultiShapeBuilder` to
/// generate geometry collections.
pub(crate) struct Geo3dMultiShapeBuilder {
    composite: GeoCompositeAreaShape,
    context: Arc<SpatialContext>,
}

impl MultiShapeBuilder for Geo3dMultiShapeBuilder {
    fn add(&mut self, shape: Arc<dyn Shape>) {
        let area_shape = wrapped(&*shape).unwrap_or_else(|e| panic!("{e}"));
        self.composite.add_shape(area_shape).unwrap_or_else(|e| panic!("{e}"));
    }

    fn build(&mut self) -> Result<Arc<dyn Shape>> {
        let planet_model = Arc::clone(self.planet_model());
        let composite = std::mem::replace(&mut self.composite, GeoCompositeAreaShape::new(&planet_model));
        Ok(Arc::new(Geo3dShape::new(Arc::new(composite), Arc::clone(&self.context))))
    }
}

impl Geo3dMultiShapeBuilder {
    fn planet_model(&self) -> &Arc<PlanetModel> {
        use rucene_spatial3d::geom::planet_object::PlanetObject;
        self.composite.get_planet_model()
    }
}
