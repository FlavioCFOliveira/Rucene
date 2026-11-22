/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `Shape.java`, of the Java package
//! `org.locationtech.spatial4j.shape`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::Result;

use crate::context::spatial_context::SpatialContext;
use crate::shape::circle::Circle;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape_collection::ShapeCollection;
use crate::shape::spatial_relation::SpatialRelation;

/// Equivalent to `org.locationtech.spatial4j.shape.Shape` (`public
/// interface`): a geometric shape, shared as `Arc<dyn Shape>`.
///
/// Java's `equals(Object)` and `hashCode()` are [`equals`](Self::equals) and
/// [`hash_code`](Self::hash_code); `toString()` is the [`fmt::Display`]
/// implementation; `instanceof Point`, `instanceof Rectangle`, `instanceof
/// Circle` and `instanceof ShapeCollection` are the `as_*` views, and
/// `getClass()` is [`as_any`](Self::as_any) (its `TypeId`) with
/// [`java_class_name`](Self::java_class_name).
pub trait Shape: fmt::Display + fmt::Debug + Send + Sync + Any {
    /// `relate(Shape other)`.
    fn relate(&self, other: &dyn Shape) -> SpatialRelation;

    /// `getBoundingBox()`.
    fn get_bounding_box(&self) -> Arc<dyn Rectangle>;

    /// `hasArea()`.
    fn has_area(&self) -> bool;

    /// `getArea(SpatialContext ctx)`; `None` stands for a `null` context.
    fn get_area(&self, ctx: Option<&SpatialContext>) -> f64;

    /// `getCenter()`.
    fn get_center(&self) -> Arc<dyn Point>;

    /// `getBuffered(double distance, SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` the shape factory throws for the result.
    fn get_buffered(&self, distance: f64, ctx: &SpatialContext) -> Result<Arc<dyn Shape>>;

    /// `isEmpty()`.
    fn is_empty(&self) -> bool;

    /// `equals(Object other)`.
    fn equals(&self, other: &dyn Shape) -> bool;

    /// `hashCode()`.
    fn hash_code(&self) -> i32;

    /// `getContext()`; `None` for a `null` context.
    fn get_context(&self) -> Option<Arc<SpatialContext>>;

    /// `getClass().getName()`.
    fn java_class_name(&self) -> &'static str;

    /// This shape as `Any`, for `getClass()` comparisons and downcasts.
    fn as_any(&self) -> &dyn Any;

    /// This shape as a `&dyn Shape`.
    fn as_shape(&self) -> &dyn Shape;

    /// `this instanceof Point ? (Point) this : null`.
    fn as_point(&self) -> Option<&dyn Point> {
        None
    }

    /// `this instanceof Rectangle ? (Rectangle) this : null`.
    fn as_rectangle(&self) -> Option<&dyn Rectangle> {
        None
    }

    /// `this instanceof Circle ? (Circle) this : null`.
    fn as_circle(&self) -> Option<&dyn Circle> {
        None
    }

    /// `this instanceof ShapeCollection ? (ShapeCollection) this : null`.
    fn as_shape_collection(&self) -> Option<&ShapeCollection> {
        None
    }
}

impl dyn Shape {
    /// `getClass() == other.getClass()`.
    pub fn same_class(&self, other: &dyn Shape) -> bool {
        self.as_any().type_id() == other.as_any().type_id()
    }

    /// `this == other`: reference identity.
    pub fn is_same(&self, other: &dyn Shape) -> bool {
        std::ptr::addr_eq(self as *const dyn Shape, other as *const dyn Shape)
    }
}

/// Implements the [`Shape::as_any`], [`Shape::as_shape`] and
/// [`Shape::java_class_name`] boilerplate for a shape type.
#[macro_export]
macro_rules! spatial4j_shape_boilerplate {
    ($java_class:expr) => {
        fn java_class_name(&self) -> &'static str {
            $java_class
        }

        fn as_any(&self) -> &dyn ::std::any::Any {
            self
        }

        fn as_shape(&self) -> &dyn $crate::shape::shape::Shape {
            self
        }
    };
}
