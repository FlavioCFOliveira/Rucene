/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `SpatialContext.java`, of the Java package
//! `org.locationtech.spatial4j.context`.

use std::fmt;
use std::sync::{Arc, LazyLock, OnceLock, Weak};

use rucene::error::{LuceneError, Result};

use crate::context::spatial_context_factory::{SpatialContextFactory, SpatialContextFactoryOps};
use crate::distance::cartesian_dist_calc::CartesianDistCalc;
use crate::distance::distance_calculator::DistanceCalculator;
use crate::distance::geodesic_sphere_dist_calc::Haversine;
use crate::io::binary_codec::BinaryCodec;
use crate::io::legacy_shape_writer::LegacyShapeWriter;
use crate::io::supported_formats::SupportedFormats;
use crate::io::wkt_reader::WKTReader;
use crate::shape::circle::Circle;
use crate::shape::point::Point;
use crate::shape::r#impl::rectangle_impl::RectangleImpl;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::shape_collection::ShapeCollection;
use crate::shape::shape_factory::ShapeFactory;

/// How a shape refers to its [`SpatialContext`]: strongly, or weakly for the
/// objects the context itself owns (its world bounds), which would otherwise
/// keep it alive forever through a reference cycle.
#[derive(Clone, Debug)]
pub enum SpatialContextRef {
    /// A strong reference.
    Strong(Arc<SpatialContext>),
    /// A weak reference, from an object the context owns.
    Weak(Weak<SpatialContext>),
}

impl SpatialContextRef {
    /// The context.
    ///
    /// # Panics
    ///
    /// When a weak reference outlives its context.
    pub fn get(&self) -> Arc<SpatialContext> {
        match self {
            SpatialContextRef::Strong(ctx) => Arc::clone(ctx),
            SpatialContextRef::Weak(ctx) => ctx.upgrade().expect("the SpatialContext is alive"),
        }
    }
}

impl From<&Arc<SpatialContext>> for SpatialContextRef {
    fn from(ctx: &Arc<SpatialContext>) -> Self {
        SpatialContextRef::Strong(Arc::clone(ctx))
    }
}

/// Equivalent to `org.locationtech.spatial4j.context.SpatialContext`
/// (`public class`): the configuration — geo or not, the world bounds, the
/// distance calculator, the shape factory, the formats and the binary codec
/// — that shapes are made and related in. Shared as `Arc<SpatialContext>`.
///
/// Java's constructor hands `this` to the factory methods that make the
/// shape factory, the codec and the formats while the object is still
/// being built; here the context is allocated first and those members are
/// filled once, in the Java order, through [`OnceLock`]s.
pub struct SpatialContext {
    me: Weak<SpatialContext>,
    geo: bool,
    shape_factory: OnceLock<Arc<dyn ShapeFactory>>,
    calculator: OnceLock<Arc<dyn DistanceCalculator>>,
    world_bounds: OnceLock<Arc<RectangleImpl>>,
    binary_codec: OnceLock<Arc<dyn BinaryCodec>>,
    formats: OnceLock<SupportedFormats>,
}

/// `SpatialContext.GEO`: a geo context with the default settings.
static GEO: LazyLock<Arc<SpatialContext>> = LazyLock::new(|| {
    SpatialContext::new(&mut SpatialContextFactory::new())
        .expect("the default SpatialContext builds")
});

impl fmt::Debug for SpatialContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpatialContext")
            .field("geo", &self.geo)
            .finish_non_exhaustive()
    }
}

impl SpatialContext {
    /// `SpatialContext.GEO`.
    pub fn geo() -> Arc<SpatialContext> {
        Arc::clone(&GEO)
    }

    /// `SpatialContext(boolean geo, DistanceCalculator calculator, Rectangle
    /// worldBounds)` (deprecated).
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn new_legacy(
        geo: bool,
        calculator: Option<Arc<dyn DistanceCalculator>>,
        world_bounds: Option<Arc<dyn Rectangle>>,
    ) -> Result<Arc<SpatialContext>> {
        Self::new(&mut Self::init_from_legacy_constructor(
            geo,
            calculator,
            world_bounds,
        ))
    }

    fn init_from_legacy_constructor(
        geo: bool,
        calculator: Option<Arc<dyn DistanceCalculator>>,
        world_bounds: Option<Arc<dyn Rectangle>>,
    ) -> SpatialContextFactory {
        let mut factory = SpatialContextFactory::new();
        factory.geo = geo;
        factory.dist_calc = calculator;
        factory.world_bounds = world_bounds;
        factory
    }

    /// `SpatialContext(boolean geo)` (deprecated).
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn new_geo(geo: bool) -> Result<Arc<SpatialContext>> {
        Self::new(&mut Self::init_from_legacy_constructor(geo, None, None))
    }

    /// `SpatialContext(SpatialContextFactory factory)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid world bounds; the errors of
    /// the factory's `make*` methods.
    pub fn new(factory: &mut dyn SpatialContextFactoryOps) -> Result<Arc<SpatialContext>> {
        let ctx = Arc::new_cyclic(|me| SpatialContext {
            me: me.clone(),
            geo: factory.base().geo,
            shape_factory: OnceLock::new(),
            calculator: OnceLock::new(),
            world_bounds: OnceLock::new(),
            binary_codec: OnceLock::new(),
            formats: OnceLock::new(),
        });
        let shape_factory = factory.make_shape_factory(&ctx)?;
        let _ = ctx.shape_factory.set(shape_factory);

        let calculator: Arc<dyn DistanceCalculator> = match &factory.base().dist_calc {
            None => {
                if ctx.is_geo() {
                    Arc::new(Haversine)
                } else {
                    Arc::new(CartesianDistCalc::new())
                }
            }
            Some(calc) => Arc::clone(calc),
        };
        let _ = ctx.calculator.set(calculator);

        //TODO remove worldBounds from Spatial4j: see Issue #55
        let weak = Some(SpatialContextRef::Weak(Arc::downgrade(&ctx)));
        let world_bounds = match &factory.base().world_bounds {
            None => {
                if ctx.is_geo() {
                    RectangleImpl::new(-180.0, 180.0, -90.0, 90.0, weak)
                } else {
                    RectangleImpl::new(-f64::MAX, f64::MAX, -f64::MAX, f64::MAX, weak)
                }
            }
            Some(bounds) => {
                if ctx.is_geo() {
                    let geo_bounds = RectangleImpl::new(-180.0, 180.0, -90.0, 90.0, weak.clone());
                    if !bounds.equals(geo_bounds.as_shape()) {
                        return Err(LuceneError::illegal_argument(format!(
                            "for geo (lat/lon), bounds must be {}",
                            Self::geo().get_world_bounds()
                        )));
                    }
                }
                if bounds.get_min_x() > bounds.get_max_x() {
                    return Err(LuceneError::illegal_argument(format!(
                        "worldBounds minX should be <= maxX: {bounds}"
                    )));
                }
                if bounds.get_min_y() > bounds.get_max_y() {
                    return Err(LuceneError::illegal_argument(format!(
                        "worldBounds minY should be <= maxY: {bounds}"
                    )));
                }
                //hopefully worldBounds' rect implementation is compatible
                RectangleImpl::from_rect(&**bounds, weak)
            }
        };
        let _ = ctx.world_bounds.set(world_bounds);

        let binary_codec = factory.make_binary_codec(&ctx)?;
        let _ = ctx.binary_codec.set(binary_codec);

        factory.check_default_formats();
        let formats = factory.make_formats(&ctx)?;
        let _ = ctx.formats.set(formats);
        Ok(ctx)
    }

    /// This context as an `Arc`.
    pub fn self_arc(&self) -> Arc<SpatialContext> {
        self.me.upgrade().expect("a SpatialContext lives in an Arc")
    }

    /// `getShapeFactory()`.
    pub fn get_shape_factory(&self) -> Arc<dyn ShapeFactory> {
        Arc::clone(
            self.shape_factory
                .get()
                .expect("the shape factory is made in the constructor"),
        )
    }

    /// `getFormats()`.
    pub fn get_formats(&self) -> &SupportedFormats {
        self.formats
            .get()
            .expect("the formats are made in the constructor")
    }

    /// `getDistCalc()`.
    pub fn get_dist_calc(&self) -> Arc<dyn DistanceCalculator> {
        Arc::clone(
            self.calculator
                .get()
                .expect("the calculator is set in the constructor"),
        )
    }

    /// `calcDistance(Point p, double x2, double y2)`.
    pub fn calc_distance_xy(&self, p: &dyn Point, x2: f64, y2: f64) -> f64 {
        self.get_dist_calc().distance_xy(p, x2, y2)
    }

    /// `calcDistance(Point p, Point p2)`.
    pub fn calc_distance(&self, p: &dyn Point, p2: &dyn Point) -> f64 {
        self.get_dist_calc().distance(p, p2)
    }

    /// `getWorldBounds()`.
    pub fn get_world_bounds(&self) -> Arc<dyn Rectangle> {
        let bounds: Arc<RectangleImpl> = Arc::clone(
            self.world_bounds
                .get()
                .expect("the world bounds are set in the constructor"),
        );
        bounds
    }

    /// `isNormWrapLongitude()` (deprecated).
    pub fn is_norm_wrap_longitude(&self) -> bool {
        self.get_shape_factory().is_norm_wrap_longitude()
    }

    /// `isGeo()`.
    pub fn is_geo(&self) -> bool {
        self.geo
    }

    /// `normX(double)` (deprecated).
    pub fn norm_x(&self, x: f64) -> f64 {
        self.get_shape_factory().norm_x(x)
    }

    /// `normY(double)` (deprecated).
    pub fn norm_y(&self, y: f64) -> f64 {
        self.get_shape_factory().norm_y(y)
    }

    /// `verifyX(double)` (deprecated).
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for a value outside the world bounds.
    pub fn verify_x(&self, x: f64) -> Result<()> {
        self.get_shape_factory().verify_x(x)
    }

    /// `verifyY(double)` (deprecated).
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for a value outside the world bounds.
    pub fn verify_y(&self, y: f64) -> Result<()> {
        self.get_shape_factory().verify_y(y)
    }

    /// `makePoint(double x, double y)` (deprecated).
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for invalid coordinates.
    pub fn make_point(&self, x: f64, y: f64) -> Result<Arc<dyn Point>> {
        self.get_shape_factory().point_xy(x, y)
    }

    /// `makeRectangle(Point lowerLeft, Point upperRight)` (deprecated).
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for invalid coordinates.
    pub fn make_rectangle_from_points(
        &self,
        lower_left: &dyn Point,
        upper_right: &dyn Point,
    ) -> Result<Arc<dyn Rectangle>> {
        self.get_shape_factory()
            .rect_from_points(lower_left, upper_right)
    }

    /// `makeRectangle(double minX, double maxX, double minY, double maxY)`
    /// (deprecated).
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for invalid coordinates.
    pub fn make_rectangle(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
    ) -> Result<Arc<dyn Rectangle>> {
        self.get_shape_factory().rect(min_x, max_x, min_y, max_y)
    }

    /// `makeCircle(double x, double y, double distance)` (deprecated).
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for invalid coordinates or distance.
    pub fn make_circle(&self, x: f64, y: f64, distance: f64) -> Result<Arc<dyn Circle>> {
        self.get_shape_factory().circle(x, y, distance)
    }

    /// `makeCircle(Point point, double distance)` (deprecated).
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for an invalid distance.
    pub fn make_circle_at(&self, point: Arc<dyn Point>, distance: f64) -> Result<Arc<dyn Circle>> {
        self.get_shape_factory().circle_at(point, distance)
    }

    /// `makeLineString(List<Point> points)` (deprecated).
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for invalid points.
    pub fn make_line_string(&self, points: Vec<Arc<dyn Point>>) -> Result<Arc<dyn Shape>> {
        self.get_shape_factory().line_string_from(points, 0.0)
    }

    /// `makeBufferedLineString(List<Point> points, double buf)` (deprecated).
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for invalid points.
    pub fn make_buffered_line_string(
        &self,
        points: Vec<Arc<dyn Point>>,
        buf: f64,
    ) -> Result<Arc<dyn Shape>> {
        self.get_shape_factory().line_string_from(points, buf)
    }

    /// `makeCollection(List<S> coll)` (deprecated).
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for the collection's bounding box.
    pub fn make_collection(&self, coll: Vec<Arc<dyn Shape>>) -> Result<Arc<ShapeCollection>> {
        self.get_shape_factory().multi_shape_from(coll)
    }

    /// `getWktShapeParser()` (deprecated): the WKT reader cast to
    /// `WKTReader`.
    ///
    /// # Errors
    ///
    /// `ClassCastException` when the WKT reader is not a `WKTReader`.
    pub fn get_wkt_shape_parser(&self) -> Result<Arc<WKTReader>> {
        let reader = self.get_formats().get_wkt_reader();
        let Some(reader) = reader else {
            return Err(LuceneError::null_pointer());
        };
        Arc::clone(&reader)
            .as_any_arc()
            .downcast::<WKTReader>()
            .map_err(|_| {
                LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "the WKT reader is not a WKTReader",
                )
            })
    }

    /// `readShapeFromWkt(String wkt)` (deprecated).
    ///
    /// # Errors
    ///
    /// `ParseException` and `InvalidShapeException` of the WKT reader.
    pub fn read_shape_from_wkt(&self, wkt: &str) -> Result<Arc<dyn Shape>> {
        self.get_wkt_shape_parser()?.parse(wkt)
    }

    /// `getBinaryCodec()`.
    pub fn get_binary_codec(&self) -> Arc<dyn BinaryCodec> {
        Arc::clone(
            self.binary_codec
                .get()
                .expect("the binary codec is made in the constructor"),
        )
    }

    /// `readShape(String value)` (deprecated): the first format that reads
    /// it; `None` (Java `null`) when none does.
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` of a reader.
    pub fn read_shape(&self, value: &str) -> Result<Option<Arc<dyn Shape>>> {
        self.get_formats().read(value)
    }

    /// `toString(Shape shape)` (deprecated): the legacy format.
    pub fn to_string_shape(&self, shape: &dyn Shape) -> String {
        LegacyShapeWriter::write_shape(shape)
    }
}

impl fmt::Display for SpatialContext {
    /// `toString()`: `SpatialContext.GEO` for the `GEO` instance, else the
    /// simple class name with `geo`, `calculator` and `worldBounds`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if std::ptr::eq(self, &**GEO) {
            f.write_str("SpatialContext.GEO")
        } else {
            write!(
                f,
                "SpatialContext{{geo={}, calculator={}, worldBounds={}}}",
                self.geo,
                self.get_dist_calc(),
                self.get_world_bounds()
            )
        }
    }
}
