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


//! The types declared in `PointInShapeIntersectVisitor.java`, of the Java package
//! `org.apache.lucene.spatial3d`.

use rucene::error::Result;
use rucene::index::point_values::{IntersectVisitor, Relation};
use rucene::search::DocIdSetIterator;
use rucene::util::doc_id_set::DocIdSetBuilder;
use rucene::util::{IntsRef, NumericUtils};

use crate::geo3_d_point::Geo3DPoint;
use crate::geo3_d_util::Geo3DUtil;
use crate::geom::geo_area;
use crate::geom::geo_area_factory::GeoAreaFactory;
use crate::geom::geo_shape::GeoShape;
use crate::geom::xyz_bounds::XYZBounds;

/// The intersect visitor of [`PointInGeo3DShapeQuery`](crate::point_in_geo3_d_shape_query::PointInGeo3DShapeQuery).
///
/// Equivalent to `org.apache.lucene.spatial3d.PointInShapeIntersectVisitor`.
///
/// Java keeps the `DocIdSetBuilder.BulkAdder` that `grow(int)` returns; the
/// builder's adder borrows it mutably, so the visitor re-obtains it with
/// `grow(0)` for each addition, as the core port's point query visitors do
/// (`point_queries.rs`, `XYPointIntersectVisitor`).
pub(crate) struct PointInShapeIntersectVisitor<'a> {
    hits: &'a mut DocIdSetBuilder,
    shape: &'a dyn GeoShape,
    minimum_x: f64,
    maximum_x: f64,
    minimum_y: f64,
    maximum_y: f64,
    minimum_z: f64,
    maximum_z: f64,
}

fn ub(v: Option<f64>) -> f64 {
    v.unwrap_or_else(|| crate::geom::npe())
}

impl<'a> PointInShapeIntersectVisitor<'a> {
    /// `new PointInShapeIntersectVisitor(DocIdSetBuilder, GeoShape, XYZBounds)`.
    pub(crate) fn new(hits: &'a mut DocIdSetBuilder, shape: &'a dyn GeoShape, bounds: &XYZBounds) -> Self {
        let doc_value_encoder = shape.get_planet_model().get_doc_value_encoder();
        Self {
            minimum_x: doc_value_encoder.round_down_x(ub(bounds.get_minimum_x())),
            maximum_x: doc_value_encoder.round_up_x(ub(bounds.get_maximum_x())),
            minimum_y: doc_value_encoder.round_down_y(ub(bounds.get_minimum_y())),
            maximum_y: doc_value_encoder.round_up_y(ub(bounds.get_maximum_y())),
            minimum_z: doc_value_encoder.round_down_z(ub(bounds.get_minimum_z())),
            maximum_z: doc_value_encoder.round_up_z(ub(bounds.get_maximum_z())),
            hits,
            shape,
        }
    }
}

impl IntersectVisitor for PointInShapeIntersectVisitor<'_> {
    fn java_to_string(&self) -> String {
        rucene::index::index_reader::java_object_to_string_stand_in(
            "org.apache.lucene.spatial3d.PointInShapeIntersectVisitor",
        )
    }

    fn grow(&mut self, count: i32) -> Result<()> {
        self.hits.grow(count);
        Ok(())
    }

    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.hits.grow(0).add(doc_id);
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        self.hits.grow(0).add_iterator(iterator)
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        self.hits.grow(0).add_ints(ints_ref);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        debug_assert!(packed_value.len() == 12);
        let pm = self.shape.get_planet_model();
        let x = Geo3DPoint::decode_dimension(packed_value, 0, pm);
        let y = Geo3DPoint::decode_dimension(packed_value, 4, pm);
        let z = Geo3DPoint::decode_dimension(packed_value, 2 * 4, pm);
        if x >= self.minimum_x
            && x <= self.maximum_x
            && y >= self.minimum_y
            && y <= self.maximum_y
            && z >= self.minimum_z
            && z <= self.maximum_z
            && self.shape.is_within_xyz(x, y, z)
        {
            self.hits.grow(0).add(doc_id);
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        let pm = self.shape.get_planet_model();
        let x_min = Geo3DUtil::decode_value_floor(NumericUtils::sortable_bytes_to_int(min_packed_value, 0), pm);
        let x_max = Geo3DUtil::decode_value_ceil(NumericUtils::sortable_bytes_to_int(max_packed_value, 0), pm);
        let y_min = Geo3DUtil::decode_value_floor(NumericUtils::sortable_bytes_to_int(min_packed_value, 4), pm);
        let y_max = Geo3DUtil::decode_value_ceil(NumericUtils::sortable_bytes_to_int(max_packed_value, 4), pm);
        let z_min = Geo3DUtil::decode_value_floor(NumericUtils::sortable_bytes_to_int(min_packed_value, 2 * 4), pm);
        let z_max = Geo3DUtil::decode_value_ceil(NumericUtils::sortable_bytes_to_int(max_packed_value, 2 * 4), pm);
        debug_assert!(x_min <= x_max);
        debug_assert!(y_min <= y_max);
        debug_assert!(z_min <= z_max);
        if self.minimum_x >= x_min
            && self.maximum_x <= x_max
            && self.minimum_y >= y_min
            && self.maximum_y <= y_max
            && self.minimum_z >= z_min
            && self.maximum_z <= z_max
        {
            return Ok(Relation::CellCrossesQuery);
        }
        let xyz_solid = GeoAreaFactory::make_geo_area_xyz(pm, x_min, x_max, y_min, y_max, z_min, z_max)?;
        Ok(match xyz_solid.get_relationship(self.shape)? {
            geo_area::CONTAINS => Relation::CellInsideQuery,
            geo_area::OVERLAPS => Relation::CellCrossesQuery,
            geo_area::WITHIN => Relation::CellCrossesQuery,
            geo_area::DISJOINT => Relation::CellOutsideQuery,
            _ => {
                debug_assert!(false);
                Relation::CellCrossesQuery
            }
        })
    }
}
