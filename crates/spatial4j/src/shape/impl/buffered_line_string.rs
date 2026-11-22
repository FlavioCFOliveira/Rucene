/*******************************************************************************
 * Copyright (c) 2015 MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `BufferedLineString.java`, of the Java package
//! `org.locationtech.spatial4j.shape.impl`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_fmt::double_to_string;

use crate::context::spatial_context::{SpatialContext, SpatialContextRef};
use crate::jdk::{double_compare_ne, hash_double};
use crate::shape::base_shape::BaseShape;
use crate::shape::r#impl::buffered_line::BufferedLine;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::shape_collection::ShapeCollection;
use crate::shape::spatial_relation::SpatialRelation;

/// Equivalent to `org.locationtech.spatial4j.shape.impl.BufferedLineString`
/// (`public class BufferedLineString extends BaseShape<SpatialContext>`): a
/// line string of [`BufferedLine`] segments, buffered by a distance.
#[derive(Debug)]
pub struct BufferedLineString {
    base: BaseShape,
    /// `segments`, a `ShapeCollection<BufferedLine>`.
    segments: Arc<ShapeCollection>,
    /// The same segments as `BufferedLine`s, the element type Java's
    /// `ShapeCollection<BufferedLine>` carries.
    lines: Vec<Arc<BufferedLine>>,
    buf: f64,
}

impl BufferedLineString {
    /// `BufferedLineString(List<Point> points, double buf, SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of a segment's bounding box.
    pub fn new(points: &[Arc<dyn Point>], buf: f64, ctx: &Arc<SpatialContext>) -> Result<Arc<BufferedLineString>> {
        Self::with_skew(points, buf, false, ctx)
    }

    /// `BufferedLineString(List<Point> points, double buf, boolean
    /// expandBufForLongitudeSkew, SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of a segment's bounding box.
    pub fn with_skew(
        points: &[Arc<dyn Point>],
        buf: f64,
        expand_buf_for_longitude_skew: bool,
        ctx: &Arc<SpatialContext>,
    ) -> Result<Arc<BufferedLineString>> {
        let mut lines: Vec<Arc<BufferedLine>> = Vec::new();
        if !points.is_empty() {
            let mut prev_point: Option<&Arc<dyn Point>> = None;
            for point in points {
                if let Some(prev) = prev_point {
                    let mut seg_buf = buf;
                    if expand_buf_for_longitude_skew {
                        seg_buf = BufferedLine::expand_buf_for_longitude_skew(&**prev, &**point, buf);
                    }
                    lines.push(BufferedLine::new(Arc::clone(prev), Arc::clone(point), seg_buf, ctx)?);
                }
                prev_point = Some(point);
            }
            if lines.is_empty() {
                //TODO throw exception instead?
                let prev = prev_point.expect("points is not empty");
                lines.push(BufferedLine::new(Arc::clone(prev), Arc::clone(prev), buf, ctx)?);
            }
        }
        let shapes: Vec<Arc<dyn Shape>> = lines.iter().map(|l| Arc::clone(l) as Arc<dyn Shape>).collect();
        let segments = ctx.make_collection(shapes)?;
        Ok(Arc::new(BufferedLineString {
            base: BaseShape::new(Some(SpatialContextRef::from(ctx))),
            segments,
            lines,
            buf,
        }))
    }

    /// `getSegments()`.
    pub fn get_segments(&self) -> &Arc<ShapeCollection> {
        &self.segments
    }

    /// The segments as their `BufferedLine` element type.
    pub fn get_segment_lines(&self) -> &[Arc<BufferedLine>] {
        &self.lines
    }

    /// `getBuf()`.
    pub fn get_buf(&self) -> f64 {
        self.buf
    }

    /// `getPoints()`: the first segment's `A`, then every segment's `B`.
    pub fn get_points(&self) -> Vec<Arc<dyn Point>> {
        if self.segments.is_empty_list() {
            return Vec::new();
        }
        let mut points = Vec::with_capacity(self.lines.len() + 1);
        points.push(Arc::clone(self.lines[0].get_a()));
        for line in &self.lines {
            points.push(Arc::clone(line.get_b()));
        }
        points
    }
}

impl Shape for BufferedLineString {
    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        self.segments.relate(other)
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        self.segments.get_bounding_box()
    }

    fn has_area(&self) -> bool {
        self.segments.has_area()
    }

    fn get_area(&self, ctx: Option<&SpatialContext>) -> f64 {
        self.segments.get_area(ctx)
    }

    fn get_center(&self) -> Arc<dyn Point> {
        self.segments.get_center()
    }

    /// `getBuffered(double, SpatialContext)`:
    /// `ctx.makeBufferedLineString(getPoints(), buf + distance)`.
    fn get_buffered(&self, distance: f64, ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        ctx.make_buffered_line_string(self.get_points(), self.buf + distance)
    }

    fn is_empty(&self) -> bool {
        self.segments.is_empty_list()
    }

    fn equals(&self, o: &dyn Shape) -> bool {
        if self.as_shape().is_same(o) {
            return true;
        }
        if !self.as_shape().same_class(o) {
            return false;
        }
        let Some(that) = o.as_any().downcast_ref::<BufferedLineString>() else {
            return false;
        };
        if double_compare_ne(that.buf, self.buf) {
            return false;
        }
        self.segments.equals(that.segments.as_shape())
    }

    fn hash_code(&self) -> i32 {
        let result = self.segments.hash_code();
        31i32.wrapping_mul(result).wrapping_add(hash_double(self.buf))
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        self.base.get_context()
    }

    crate::spatial4j_shape_boilerplate!("org.locationtech.spatial4j.shape.impl.BufferedLineString");
}

impl fmt::Display for BufferedLineString {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::with_capacity(100);
        s.push_str("BufferedLineString(buf=");
        s.push_str(&double_to_string(self.buf));
        s.push_str(" pts=");
        for (i, point) in self.get_points().iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&double_to_string(point.get_x()));
            s.push(' ');
            s.push_str(&double_to_string(point.get_y()));
        }
        s.push(')');
        f.write_str(&s)
    }
}
