/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `ShapeCollection.java`, of the Java package
//! `org.locationtech.spatial4j.shape`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;

use crate::context::spatial_context::{SpatialContext, SpatialContextRef};
use crate::shape::point::Point;
use crate::shape::r#impl::b_box_calculator::BBoxCalculator;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::spatial_relation::SpatialRelation;

/// Equivalent to `org.locationtech.spatial4j.shape.ShapeCollection` (`public
/// class ShapeCollection<S extends Shape> extends AbstractList<S> implements
/// Shape`): a collection of shapes, a shape itself (a "geometry collection").
///
/// **Divergence.** Java's type parameter `S` only narrows what `get(int)` and
/// `getShapes()` return; the shapes are held here as `Arc<dyn Shape>` and a
/// caller that knows their class downcasts them through
/// [`Shape::as_any`]. Java also rejects a list that is not `RandomAccess`
/// (`ShapeCollection.java:57-58`); a `Vec` always is, so that check has no
/// failing case here.
#[derive(Debug)]
pub struct ShapeCollection {
    /// `ctx`.
    ctx: Option<SpatialContextRef>,
    /// `shapes`.
    shapes: Vec<Arc<dyn Shape>>,
    /// `bbox`.
    bbox: Arc<dyn Rectangle>,
}

impl ShapeCollection {
    /// `ShapeCollection(List<S> shapes, SpatialContext ctx)`.
    pub fn new(
        shapes: Vec<Arc<dyn Shape>>,
        ctx: &Arc<SpatialContext>,
    ) -> Result<Arc<ShapeCollection>> {
        let bbox = Self::compute_bounding_box(&shapes, ctx)?;
        Ok(Arc::new(ShapeCollection {
            ctx: Some(SpatialContextRef::from(ctx)),
            shapes,
            bbox,
        }))
    }

    /// `computeBoundingBox(Collection<? extends Shape>, SpatialContext)`.
    pub fn compute_bounding_box(
        shapes: &[Arc<dyn Shape>],
        ctx: &Arc<SpatialContext>,
    ) -> Result<Arc<dyn Rectangle>> {
        if shapes.is_empty() {
            return ctx.make_rectangle(f64::NAN, f64::NAN, f64::NAN, f64::NAN);
        }
        let mut bbox_calc = BBoxCalculator::new(Arc::clone(ctx));
        for geom in shapes {
            bbox_calc.expand_range(&*geom.get_bounding_box());
        }
        bbox_calc.get_boundary()
    }

    /// `getShapes()`.
    pub fn get_shapes(&self) -> &[Arc<dyn Shape>] {
        &self.shapes
    }

    /// `get(int index)`.
    pub fn get(&self, index: usize) -> &Arc<dyn Shape> {
        &self.shapes[index]
    }

    /// `size()`.
    pub fn size(&self) -> usize {
        self.shapes.len()
    }

    /// `relateContainsShortCircuits()`.
    pub fn relate_contains_short_circuits(&self) -> bool {
        true
    }

    /// `computeMutualDisjoint(List<? extends Shape>)`.
    pub fn compute_mutual_disjoint(shapes: &[Arc<dyn Shape>]) -> bool {
        for i in 1..shapes.len() {
            let shape_i = &shapes[i];
            for shape_j in &shapes[..i] {
                if shape_j.relate(shape_i.as_shape()).intersects() {
                    return false;
                }
            }
        }
        true
    }

    /// `AbstractList.isEmpty()`: no shapes. Java's `ShapeCollection` inherits
    /// both `List.isEmpty()` and `Shape.isEmpty()` from this one method.
    pub fn is_empty_list(&self) -> bool {
        self.shapes.is_empty()
    }

    /// `AbstractList.hashCode()` over the shapes.
    fn list_hash_code(&self) -> i32 {
        let mut hash: i32 = 1;
        for s in &self.shapes {
            hash = 31i32.wrapping_mul(hash).wrapping_add(s.hash_code());
        }
        hash
    }
}

impl Shape for ShapeCollection {
    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        let bbox_sect = self.bbox.relate(other);
        if bbox_sect == SpatialRelation::Disjoint || bbox_sect == SpatialRelation::Within {
            return bbox_sect;
        }
        let contains_will_short_circuit =
            other.as_point().is_some() || self.relate_contains_short_circuits();
        let mut sect: Option<SpatialRelation> = None;
        for shape in &self.shapes {
            let next_sect = shape.relate(other);
            let s = match sect {
                None => next_sect, //first pass
                Some(s) => s.combine(Some(next_sect)),
            };
            sect = Some(s);
            if s == SpatialRelation::Intersects {
                return SpatialRelation::Intersects;
            }
            if s == SpatialRelation::Contains && contains_will_short_circuit {
                return SpatialRelation::Contains;
            }
        }
        // Java returns `sect`, `null` for an empty collection; an empty
        // collection's bounding box is empty and relates DISJOINT above, so
        // the loop always ran here.
        sect.expect("a non-empty ShapeCollection")
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        Arc::clone(&self.bbox)
    }

    fn has_area(&self) -> bool {
        self.shapes.iter().any(|geom| geom.has_area())
    }

    fn get_area(&self, ctx: Option<&SpatialContext>) -> f64 {
        let max_area = self.bbox.get_area(ctx);
        let mut sum = 0.0;
        for geom in &self.shapes {
            sum += geom.get_area(ctx);
            if sum >= max_area {
                return max_area;
            }
        }
        sum
    }

    /// `getCenter()`: `bbox.getCenter()`.
    fn get_center(&self) -> Arc<dyn Point> {
        self.bbox.get_center()
    }

    fn get_buffered(&self, distance: f64, ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        let mut buf_coll: Vec<Arc<dyn Shape>> = Vec::with_capacity(self.size());
        for shape in &self.shapes {
            buf_coll.push(shape.get_buffered(distance, ctx)?);
        }
        let coll = ctx.make_collection(buf_coll)?;
        Ok(coll)
    }

    /// `isEmpty()`: the `AbstractList` method, no shapes.
    fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    fn equals(&self, o: &dyn Shape) -> bool {
        if self.as_shape().is_same(o) {
            return true;
        }
        if !self.as_shape().same_class(o) {
            return false;
        }
        let Some(that) = o.as_any().downcast_ref::<ShapeCollection>() else {
            return false;
        };
        // `List.equals`: same size, pairwise `equals`.
        self.shapes.len() == that.shapes.len()
            && self
                .shapes
                .iter()
                .zip(&that.shapes)
                .all(|(a, b)| a.equals(b.as_shape()))
    }

    fn hash_code(&self) -> i32 {
        self.list_hash_code()
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        self.ctx.as_ref().map(SpatialContextRef::get)
    }

    fn as_shape_collection(&self) -> Option<&ShapeCollection> {
        Some(self)
    }

    crate::spatial4j_shape_boilerplate!("org.locationtech.spatial4j.shape.ShapeCollection");
}

impl fmt::Display for ShapeCollection {
    /// `toString()`: at most about 150 characters of the shapes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = String::with_capacity(100);
        buf.push_str("ShapeCollection(");
        for (i, shape) in self.shapes.iter().enumerate() {
            if i > 0 {
                buf.push_str(", ");
            }
            buf.push_str(&shape.to_string());
            if buf.encode_utf16().count() > 150 {
                buf.push_str(" ...");
                buf.push_str(&self.shapes.len().to_string());
                break;
            }
        }
        buf.push(')');
        f.write_str(&buf)
    }
}
