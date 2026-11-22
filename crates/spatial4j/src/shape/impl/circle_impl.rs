/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `CircleImpl.java`, of the Java package
//! `org.locationtech.spatial4j.shape.impl`.

use std::fmt;
use std::sync::{Arc, RwLock};

use rucene::error::Result;
use rucene::util::java_fmt::double_to_string;

use crate::context::spatial_context::{SpatialContext, SpatialContextRef};
use crate::jdk::{double_compare_ne, hash_double, AtomicF64};
use crate::shape::base_shape::BaseShape;
use crate::shape::circle::Circle;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::spatial_relation::SpatialRelation;

/// The protected state of `CircleImpl`, which `GeoCircle` shares.
#[derive(Debug)]
pub struct CircleState {
    pub(crate) base: BaseShape,
    /// `point`.
    pub(crate) point: Arc<dyn Point>,
    /// `radiusDEG`.
    pub(crate) radius_deg: AtomicF64,
    /// `enclosingBox`.
    pub(crate) enclosing_box: RwLock<Arc<dyn Rectangle>>,
}

impl CircleState {
    /// The body of `CircleImpl(Point p, double radiusDEG, SpatialContext ctx)`.
    pub(crate) fn new(p: Arc<dyn Point>, radius_deg: f64, ctx: &Arc<SpatialContext>) -> Self {
        let radius_deg = if p.is_empty() { f64::NAN } else { radius_deg };
        let enclosing_box = if p.is_empty() {
            ctx.make_rectangle(f64::NAN, f64::NAN, f64::NAN, f64::NAN)
        } else {
            ctx.get_dist_calc().calc_box_by_dist_from_pt(&*p, radius_deg, ctx, None)
        }
        .unwrap_or_else(|e| panic!("{e}"));
        Self {
            base: BaseShape::new(Some(SpatialContextRef::from(ctx))),
            point: p,
            radius_deg: AtomicF64::new(radius_deg),
            enclosing_box: RwLock::new(enclosing_box),
        }
    }

    pub(crate) fn ctx(&self) -> Arc<SpatialContext> {
        self.base.ctx()
    }

    /// `enclosingBox`.
    pub fn enclosing_box(&self) -> Arc<dyn Rectangle> {
        Arc::clone(&self.enclosing_box.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// `radiusDEG`.
    pub fn radius_deg(&self) -> f64 {
        self.radius_deg.get()
    }

    /// `CircleImpl.reset(double x, double y, double radiusDEG)`.
    pub(crate) fn reset(&self, x: f64, y: f64, radius_deg: f64) {
        debug_assert!(!self.point.is_empty());
        self.point.reset(x, y);
        self.radius_deg.set(radius_deg);
        let ctx = self.ctx();
        let reuse = self.enclosing_box();
        let new_box = ctx
            .get_dist_calc()
            .calc_box_by_dist_from_pt(&*self.point, radius_deg, &ctx, Some(reuse))
            .unwrap_or_else(|e| panic!("{e}"));
        *self.enclosing_box.write().unwrap_or_else(|e| e.into_inner()) = new_box;
    }

    /// `contains(double x, double y)`.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.ctx().get_dist_calc().within(&*self.point, x, y, self.radius_deg.get())
    }
}

/// The methods `CircleImpl` lets subclasses override (`getYAxis`,
/// `getXAxis`, `relateRectanglePhase2`), and the state they act on.
pub trait CircleOverrides: Circle {
    /// The shared `CircleImpl` state.
    fn state(&self) -> &CircleState;

    /// `getYAxis()`.
    fn get_y_axis(&self) -> f64 {
        self.state().point.get_y()
    }

    /// `getXAxis()`.
    fn get_x_axis(&self) -> f64 {
        self.state().point.get_x()
    }

    /// `relateRectanglePhase2(Rectangle r, SpatialRelation bboxSect)`.
    fn relate_rectangle_phase2(&self, r: &dyn Rectangle, bbox_sect: SpatialRelation) -> SpatialRelation {
        CircleImpl::relate_rectangle_phase2_impl(self, r, bbox_sect)
    }
}

/// Equivalent to `org.locationtech.spatial4j.shape.impl.CircleImpl` (`public
/// class CircleImpl extends BaseShape<SpatialContext> implements Circle`): a
/// circle in a flat (Euclidean) space; `GeoCircle` handles the sphere.
#[derive(Debug)]
pub struct CircleImpl {
    state: CircleState,
}

impl CircleImpl {
    /// `CircleImpl(Point p, double radiusDEG, SpatialContext ctx)`.
    pub fn new(p: Arc<dyn Point>, radius_deg: f64, ctx: &Arc<SpatialContext>) -> Arc<CircleImpl> {
        Arc::new(CircleImpl { state: CircleState::new(p, radius_deg, ctx) })
    }

    /// `contains(double x, double y)`.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.state.contains(x, y)
    }

    /// `relate(Shape)` of `CircleImpl`, dispatching to the overridable
    /// phases of `circle`.
    pub fn relate_impl<C: CircleOverrides + ?Sized>(circle: &C, other: &dyn Shape) -> SpatialRelation {
        if circle.is_empty() || other.is_empty() {
            return SpatialRelation::Disjoint;
        }
        if let Some(point) = other.as_point() {
            return Self::relate_point_impl(circle, point);
        }
        if let Some(rect) = other.as_rectangle() {
            return Self::relate_rect_impl(circle, rect);
        }
        if let Some(c) = other.as_circle() {
            return Self::relate_circle_impl(circle, c);
        }
        other.relate(circle.as_shape()).transpose()
    }

    /// `relate(Point point)`.
    pub fn relate_point_impl<C: CircleOverrides + ?Sized>(circle: &C, point: &dyn Point) -> SpatialRelation {
        if circle.state().contains(point.get_x(), point.get_y()) {
            SpatialRelation::Contains
        } else {
            SpatialRelation::Disjoint
        }
    }

    /// `relate(Rectangle r)`.
    pub fn relate_rect_impl<C: CircleOverrides + ?Sized>(circle: &C, r: &dyn Rectangle) -> SpatialRelation {
        let enclosing_box = circle.state().enclosing_box();
        let bbox_sect = enclosing_box.relate(r.as_shape());
        if bbox_sect == SpatialRelation::Disjoint || bbox_sect == SpatialRelation::Within {
            return bbox_sect;
        } else if bbox_sect == SpatialRelation::Contains && enclosing_box.equals(r.as_shape()) {
            //nasty identity edge-case
            return SpatialRelation::Within;
        }
        circle.relate_rectangle_phase2(r, bbox_sect)
    }

    /// `CircleImpl.relateRectanglePhase2(Rectangle, SpatialRelation)`, the
    /// implementation subclasses reach with `super`.
    pub fn relate_rectangle_phase2_impl<C: CircleOverrides + ?Sized>(
        circle: &C,
        r: &dyn Rectangle,
        bbox_sect: SpatialRelation,
    ) -> SpatialRelation {
        let state = circle.state();
        let x_axis = circle.get_x_axis();
        let (closest_x, farthest_x) = if x_axis < r.get_min_x() {
            (r.get_min_x(), r.get_max_x())
        } else if x_axis > r.get_max_x() {
            (r.get_max_x(), r.get_min_x())
        } else {
            //we don't really use this value but to check this condition
            (
                x_axis,
                if r.get_max_x() - x_axis > x_axis - r.get_min_x() {
                    r.get_max_x()
                } else {
                    r.get_min_x()
                },
            )
        };
        let y_axis = circle.get_y_axis();
        let (closest_y, farthest_y) = if y_axis < r.get_min_y() {
            (r.get_min_y(), r.get_max_y())
        } else if y_axis > r.get_max_y() {
            (r.get_max_y(), r.get_min_y())
        } else {
            (
                y_axis,
                if r.get_max_y() - y_axis > y_axis - r.get_min_y() {
                    r.get_max_y()
                } else {
                    r.get_min_y()
                },
            )
        };
        if x_axis != closest_x && y_axis != closest_y && !state.contains(closest_x, closest_y) {
            return SpatialRelation::Disjoint;
        } // else CAN'T be disjoint if spans axis because earlier bbox check ruled that out
        if bbox_sect != SpatialRelation::Contains {
            return SpatialRelation::Intersects;
        }
        if !state.contains(farthest_x, farthest_y) {
            return SpatialRelation::Intersects;
        }
        if state.point.get_y() != circle.get_y_axis() && y_axis == closest_y {
            //geodetic; r crosses north to south over x axis (confusing)
            let other_y = if farthest_y == r.get_max_y() { r.get_min_y() } else { r.get_max_y() };
            if !state.contains(farthest_x, other_y) {
                return SpatialRelation::Intersects;
            }
        }
        SpatialRelation::Contains
    }

    /// `relate(Circle circle)`.
    pub fn relate_circle_impl<C: CircleOverrides + ?Sized>(this: &C, circle: &dyn Circle) -> SpatialRelation {
        let state = this.state();
        let cross_dist = state.ctx().get_dist_calc().distance(&*state.point, &*circle.get_center());
        let a_dist = state.radius_deg.get();
        let b_dist = circle.get_radius();
        if cross_dist > a_dist + b_dist {
            return SpatialRelation::Disjoint;
        }
        if cross_dist < a_dist && cross_dist + b_dist <= a_dist {
            return SpatialRelation::Contains;
        }
        if cross_dist < b_dist && cross_dist + a_dist <= b_dist {
            return SpatialRelation::Within;
        }
        SpatialRelation::Intersects
    }

    /// `equals(Circle thiz, Object o)`.
    pub fn equals_circle(thiz: &dyn Circle, o: &dyn Shape) -> bool {
        if thiz.as_shape().is_same(o) {
            return true;
        }
        let Some(circle) = o.as_circle() else {
            return false;
        };
        if !thiz.get_center().equals(circle.get_center().as_shape()) {
            return false;
        }
        if double_compare_ne(circle.get_radius(), thiz.get_radius()) {
            return false;
        }
        true
    }

    /// `hashCode(Circle thiz)`.
    pub fn hash_code_circle(thiz: &dyn Circle) -> i32 {
        let result = thiz.get_center().hash_code();
        31i32.wrapping_mul(result).wrapping_add(hash_double(thiz.get_radius()))
    }

    /// `getArea(SpatialContext)` of `CircleImpl`.
    pub(crate) fn get_area_impl(circle: &dyn Circle, radius_deg: f64, ctx: Option<&SpatialContext>) -> f64 {
        match ctx {
            None => std::f64::consts::PI * radius_deg * radius_deg,
            Some(ctx) => ctx.get_dist_calc().area_circle(circle),
        }
    }
}

impl CircleOverrides for CircleImpl {
    fn state(&self) -> &CircleState {
        &self.state
    }
}

impl Circle for CircleImpl {
    fn reset(&self, x: f64, y: f64, radius_deg: f64) {
        self.state.reset(x, y, radius_deg);
    }

    fn get_radius(&self) -> f64 {
        self.state.radius_deg.get()
    }
}

impl Shape for CircleImpl {
    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        Self::relate_impl(self, other)
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        self.state.enclosing_box()
    }

    fn has_area(&self) -> bool {
        self.state.radius_deg.get() > 0.0
    }

    fn get_area(&self, ctx: Option<&SpatialContext>) -> f64 {
        Self::get_area_impl(self, self.state.radius_deg.get(), ctx)
    }

    /// `getCenter()`: the `point` field.
    fn get_center(&self) -> Arc<dyn Point> {
        Arc::clone(&self.state.point)
    }

    /// `getBuffered(double, SpatialContext)`: `ctx.makeCircle(point, distance
    /// + radiusDEG)`.
    fn get_buffered(&self, distance: f64, ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        let c = ctx.make_circle_at(Arc::clone(&self.state.point), distance + self.state.radius_deg.get())?;
        Ok(c)
    }

    fn is_empty(&self) -> bool {
        self.state.point.is_empty()
    }

    fn equals(&self, other: &dyn Shape) -> bool {
        Self::equals_circle(self, other)
    }

    fn hash_code(&self) -> i32 {
        Self::hash_code_circle(self)
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        self.state.base.get_context()
    }

    fn as_circle(&self) -> Option<&dyn Circle> {
        Some(self)
    }

    crate::spatial4j_shape_boilerplate!("org.locationtech.spatial4j.shape.impl.CircleImpl");
}

impl fmt::Display for CircleImpl {
    /// `toString()`: `Circle(<point>, d=<radius>°)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Circle({}, d={}\u{00B0})",
            self.state.point,
            double_to_string(self.state.radius_deg.get())
        )
    }
}
