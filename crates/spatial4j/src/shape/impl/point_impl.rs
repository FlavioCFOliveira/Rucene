/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `PointImpl.java`, of the Java package
//! `org.locationtech.spatial4j.shape.impl`.

use std::fmt;
use std::sync::{Arc, Weak};

use rucene::error::Result;
use rucene::util::java_fmt::double_to_string;

use crate::context::spatial_context::{SpatialContext, SpatialContextRef};
use crate::jdk::{double_compare_ne, hash_double, AtomicF64};
use crate::shape::base_shape::BaseShape;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::spatial_relation::SpatialRelation;

/// Equivalent to `org.locationtech.spatial4j.shape.impl.PointImpl` (`public
/// class PointImpl extends BaseShape<SpatialContext> implements Point`): a
/// basic 2D point.
#[derive(Debug)]
pub struct PointImpl {
    me: Weak<PointImpl>,
    base: BaseShape,
    x: AtomicF64,
    y: AtomicF64,
}

impl PointImpl {
    /// `PointImpl(double x, double y, SpatialContext ctx)`; `None` stands for
    /// a `null` context.
    pub fn new(x: f64, y: f64, ctx: Option<SpatialContextRef>) -> Arc<PointImpl> {
        Arc::new_cyclic(|me| PointImpl {
            me: me.clone(),
            base: BaseShape::new(ctx),
            x: AtomicF64::new(x),
            y: AtomicF64::new(y),
        })
    }

    /// `PointImpl(double x, double y, SpatialContext ctx)` with a non-null
    /// context.
    pub fn with_ctx(x: f64, y: f64, ctx: &Arc<SpatialContext>) -> Arc<PointImpl> {
        Self::new(x, y, Some(SpatialContextRef::from(ctx)))
    }

    /// `equals(Point thiz, Object o)`: the `Point` equality every point
    /// implementation shares.
    pub fn equals_point(thiz: &dyn Point, o: &dyn Shape) -> bool {
        if thiz.as_shape().is_same(o) {
            return true;
        }
        let Some(point) = o.as_point() else {
            return false;
        };
        if double_compare_ne(point.get_x(), thiz.get_x()) {
            return false;
        }
        if double_compare_ne(point.get_y(), thiz.get_y()) {
            return false;
        }
        true
    }

    /// `hashCode(Point thiz)`.
    pub fn hash_code_point(thiz: &dyn Point) -> i32 {
        let mut result = hash_double(thiz.get_x());
        result = 31i32.wrapping_mul(result).wrapping_add(hash_double(thiz.get_y()));
        result
    }

    fn this(&self) -> Arc<PointImpl> {
        self.me.upgrade().expect("a PointImpl lives in an Arc")
    }
}

impl Point for PointImpl {
    fn reset(&self, x: f64, y: f64) {
        debug_assert!(!self.is_empty());
        self.x.set(x);
        self.y.set(y);
    }

    fn get_x(&self) -> f64 {
        self.x.get()
    }

    fn get_y(&self) -> f64 {
        self.y.get()
    }

    fn get_lat(&self) -> f64 {
        self.get_y()
    }

    fn get_lon(&self) -> f64 {
        self.get_x()
    }
}

impl Shape for PointImpl {
    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        if self.is_empty() || other.is_empty() {
            return SpatialRelation::Disjoint;
        }
        if other.as_point().is_some() {
            return if self.equals(other) {
                SpatialRelation::Intersects
            } else {
                SpatialRelation::Disjoint
            };
        }
        other.relate(self).transpose()
    }

    /// `getBoundingBox()`: `ctx.makeRectangle(this, this)`.
    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        self.base
            .ctx()
            .make_rectangle_from_points(self, self)
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn has_area(&self) -> bool {
        false
    }

    fn get_area(&self, _ctx: Option<&SpatialContext>) -> f64 {
        0.0
    }

    /// `getCenter()`: `this`.
    fn get_center(&self) -> Arc<dyn Point> {
        self.this()
    }

    /// `getBuffered(double, SpatialContext)`: `ctx.makeCircle(this, distance)`.
    fn get_buffered(&self, distance: f64, ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        let circle = ctx.make_circle_at(self.this(), distance)?;
        Ok(circle)
    }

    fn is_empty(&self) -> bool {
        self.x.get().is_nan()
    }

    fn equals(&self, other: &dyn Shape) -> bool {
        Self::equals_point(self, other)
    }

    fn hash_code(&self) -> i32 {
        Self::hash_code_point(self)
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        self.base.get_context()
    }

    fn as_point(&self) -> Option<&dyn Point> {
        Some(self)
    }

    crate::spatial4j_shape_boilerplate!("org.locationtech.spatial4j.shape.impl.PointImpl");
}

impl fmt::Display for PointImpl {
    /// `toString()`: `Pt(x=..,y=..)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pt(x={},y={})", double_to_string(self.get_x()), double_to_string(self.get_y()))
    }
}
