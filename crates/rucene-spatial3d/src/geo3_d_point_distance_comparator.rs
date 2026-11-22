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


//! The types declared in `Geo3DPointDistanceComparator.java`, of the Java package
//! `org.apache.lucene.spatial3d`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{DocValues, LeafReader, LeafReaderContext, SortedNumericDocValues};
use rucene::search::{Scorable, SlotComparator, SortValue};
use rucene::util::java_double_compare;

use crate::geo3_d_doc_values_field::Geo3DDocValuesField;
use crate::geom::distance_style::ARC;
use crate::geom::geo_distance_shape::GeoDistanceShape;
use crate::geom::xyz_bounds::XYZBounds;
use crate::geom::planet_model::PlanetModel;
use crate::geom::{java_min, npe};

fn ub(v: Option<f64>) -> f64 {
    v.unwrap_or_else(|| npe())
}

fn dcmp(a: f64, b: f64) -> i32 {
    match java_double_compare(a, b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Compute path distance for a sort, using the distance of a document from a shape.
///
/// Equivalent to `org.apache.lucene.spatial3d.Geo3DPointDistanceComparator`. As the core port's
/// `LatLonPointDistanceComparator`, Java's `FieldComparator<Double>` that is
/// its own `LeafFieldComparator` is a [`SlotComparator`] whose leaf is the
/// `currentDocs` field.
///
/// Java's class is package-private; it is public here because the public
/// `Geo3DDocValuesField` sort factories return it (see
/// [`Geo3DPointSortField`](crate::geo3_d_point_sort_field::Geo3DPointSortField)).
pub struct Geo3DPointDistanceComparator {
    field: String,
    distance_shape: Arc<dyn GeoDistanceShape>,
    planet_model: Arc<PlanetModel>,
    values: Vec<f64>,
    bottom_distance: f64,
    top_value: f64,
    /// The XYZ bounds of the bottom distance (`priorityQueueBounds`).
    priority_queue_bounds: Option<XYZBounds>,
    set_bottom_counter: i32,
}

impl fmt::Debug for Geo3DPointDistanceComparator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Geo3DPointDistanceComparator").field("field", &self.field).finish_non_exhaustive()
    }
}

impl Geo3DPointDistanceComparator {
    /// `new Geo3DPointDistanceComparator(String field, PlanetModel, GeoDistanceShape, int numHits)`.
    pub fn new(field: &str, planet_model: &Arc<PlanetModel>, distance_shape: Arc<dyn GeoDistanceShape>, num_hits: usize) -> Self {
        Self {
            field: field.to_string(),
            distance_shape,
            planet_model: planet_model.clone(),
            values: vec![0.0; num_hits],
            bottom_distance: 0.0,
            top_value: 0.0,
            priority_queue_bounds: None,
            set_bottom_counter: 0,
        }
    }

    /// `compare(int, int)`.
    pub fn compare_slots(&self, slot1: usize, slot2: usize) -> i32 {
        dcmp(self.values[slot1], self.values[slot2])
    }

    /// `setBottom(int)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the distance bounds computation.
    pub fn set_bottom_slot(&mut self, slot: usize) -> Result<()> {
        self.bottom_distance = self.values[slot];
        if self.set_bottom_counter < 1024 || (self.set_bottom_counter & 0x3F) == 0x3F {
            let mut bounds = XYZBounds::new();
            self.distance_shape.get_distance_bounds(&mut bounds, &ARC, self.bottom_distance)?;
            self.priority_queue_bounds = Some(bounds);
        }
        self.set_bottom_counter += 1;
        Ok(())
    }

    /// `compareBottom(int)`.
    ///
    /// # Errors
    ///
    /// Whatever reading the doc values or the distance raises.
    pub fn compare_bottom_doc(&mut self, current_docs: &mut dyn SortedNumericDocValues, doc: i32) -> Result<i32> {
        if doc > current_docs.doc_id() {
            current_docs.advance(doc)?;
        }
        if doc < current_docs.doc_id() {
            return Ok(dcmp(self.bottom_distance, f64::INFINITY));
        }
        let num_values = current_docs.doc_value_count()?;
        debug_assert!(num_values > 0);
        let mut cmp = -1;
        let encoder = self.planet_model.get_doc_value_encoder();
        for _ in 0..num_values {
            let encoded = current_docs.next_value()?;
            let x = encoder.decode_x_value(encoded);
            let y = encoder.decode_y_value(encoded);
            let z = encoder.decode_z_value(encoded);
            let pq = self.priority_queue_bounds.as_ref().unwrap_or_else(|| npe());
            if x > ub(pq.get_maximum_x())
                || x < ub(pq.get_minimum_x())
                || y > ub(pq.get_maximum_y())
                || y < ub(pq.get_minimum_y())
                || z > ub(pq.get_maximum_z())
                || z < ub(pq.get_minimum_z())
            {
                continue;
            }
            cmp = cmp.max(dcmp(self.bottom_distance, self.distance_shape.compute_distance_xyz(&ARC, x, y, z)?));
        }
        Ok(cmp)
    }

    /// `copy(int, int)`.
    ///
    /// # Errors
    ///
    /// Whatever reading the doc values or the distance raises.
    pub fn copy_doc(&mut self, current_docs: &mut dyn SortedNumericDocValues, slot: usize, doc: i32) -> Result<()> {
        self.values[slot] = self.compute_minimum_distance(current_docs, doc)?;
        Ok(())
    }

    /// `getLeafComparator(LeafReaderContext)`: returns `currentDocs`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an incompatible field; reader errors.
    pub fn get_leaf_comparator(&mut self, reader: &dyn LeafReader) -> Result<Box<dyn SortedNumericDocValues>> {
        let field_infos = reader.get_field_infos();
        if let Some(info) = field_infos.field_info(&self.field) {
            Geo3DDocValuesField::check_compatible(info)?;
        }
        DocValues::get_sorted_numeric(reader, &self.field)
    }

    /// `value(int)`: the distance in meters.
    pub fn value_slot(&self, slot: usize) -> f64 {
        self.values[slot] * self.planet_model.get_mean_radius()
    }

    /// `compareTop(int)`.
    ///
    /// # Errors
    ///
    /// Whatever reading the doc values or the distance raises.
    pub fn compare_top_doc(&mut self, current_docs: &mut dyn SortedNumericDocValues, doc: i32) -> Result<i32> {
        Ok(dcmp(self.top_value, self.compute_minimum_distance(current_docs, doc)?))
    }

    /// `computeMinimumDistance(int)`.
    fn compute_minimum_distance(&self, current_docs: &mut dyn SortedNumericDocValues, doc: i32) -> Result<f64> {
        if doc > current_docs.doc_id() {
            current_docs.advance(doc)?;
        }
        let mut min_value = f64::INFINITY;
        if doc == current_docs.doc_id() {
            let num_values = current_docs.doc_value_count()?;
            let encoder = self.planet_model.get_doc_value_encoder();
            for _ in 0..num_values {
                let encoded = current_docs.next_value()?;
                let distance = self.distance_shape.compute_distance_xyz(
                    &ARC,
                    encoder.decode_x_value(encoded),
                    encoder.decode_y_value(encoded),
                    encoder.decode_z_value(encoded),
                )?;
                min_value = java_min(min_value, distance);
            }
        }
        Ok(min_value)
    }
}

impl SlotComparator for Geo3DPointDistanceComparator {
    type Leaf = Box<dyn SortedNumericDocValues>;

    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        self.compare_slots(slot1 as usize, slot2 as usize)
    }

    /// `setTopValue(Double)`; a value of another type is unrepresentable in
    /// Java's signature and ignored.
    fn set_top_value(&mut self, value: SortValue) {
        if let SortValue::Double(value) = value {
            self.top_value = value;
        }
    }

    fn value(&self, slot: i32) -> SortValue {
        SortValue::Double(self.value_slot(slot as usize))
    }

    fn new_leaf(&mut self, context: &LeafReaderContext) -> Result<Self::Leaf> {
        self.get_leaf_comparator(context.leaf_reader().as_ref())
    }

    fn set_bottom(&mut self, _leaf: &mut Self::Leaf, slot: i32) -> Result<()> {
        self.set_bottom_slot(slot as usize)
    }

    fn compare_bottom(&mut self, leaf: &mut Self::Leaf, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        self.compare_bottom_doc(leaf.as_mut(), doc)
    }

    fn compare_top(&mut self, leaf: &mut Self::Leaf, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        self.compare_top_doc(leaf.as_mut(), doc)
    }

    fn copy(&mut self, leaf: &mut Self::Leaf, slot: i32, doc: i32, _scorer: &mut dyn Scorable) -> Result<()> {
        self.copy_doc(leaf.as_mut(), slot as usize, doc)
    }

    /// `setScorer(Scorable)`: empty.
    fn set_scorer(&mut self, _leaf: &mut Self::Leaf, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }
}
