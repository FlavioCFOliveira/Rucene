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


//! The types declared in `Geo3DPointOutsideSortField.java`, of the Java package
//! `org.apache.lucene.spatial3d`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::{MissingValue, Pruning, SortField, SortFieldKind, SortFieldType};
use rucene::util::java_collections::{java_identity_hash_code_stand_in, java_string_hash_code};

use crate::geo3_d_point_outside_distance_comparator::Geo3DPointOutsideDistanceComparator;
use crate::geom::geo_membership_shape::GeoMembershipShape;
use crate::geom::planet_model::PlanetModel;
use crate::geom::{d2s, dhash};

/// Sorts by distance from a shape, outside of it.
///
/// Equivalent to `org.apache.lucene.spatial3d.Geo3DPointOutsideSortField`. As the core port's
/// `LatLonPointSortField`, the subclass of `SortField` holds a [`SortField`]
/// (`CUSTOM`, ascending, missing values last) and hands out its comparator
/// through [`get_comparator`](Self::get_comparator).
///
/// Java types the shape as `GeoOutsideDistance`, an interface without
/// `equals`/`hashCode`/`toString` of its own; every shape `Geo3DDocValuesField`
/// passes is a `GeoMembershipShape`, whose Rust trait carries them.
///
/// Java's class is package-private; it is public here because the public
/// `Geo3DDocValuesField` sort factories return it (see
/// [`Geo3DPointSortField`](crate::geo3_d_point_sort_field::Geo3DPointSortField)).
pub struct Geo3DPointOutsideSortField {
    sort_field: SortField,
    distance_shape: Arc<dyn GeoMembershipShape>,
    planet_model: Arc<PlanetModel>,
}

impl Geo3DPointOutsideSortField {
    /// `new Geo3DPointOutsideSortField(String field, PlanetModel, GeoOutsideDistance)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the `SortField` constructor.
    pub fn new(field: &str, planet_model: &Arc<PlanetModel>, distance_shape: Arc<dyn GeoMembershipShape>) -> Result<Self> {
        Ok(Self {
            sort_field: SortField::with_kind(
                Some(field.to_string()),
                SortFieldType::Custom,
                false,
                Some(MissingValue::Double(f64::INFINITY)),
                SortFieldKind::Plain,
            )?,
            distance_shape,
            planet_model: planet_model.clone(),
        })
    }

    /// The `SortField` this subclass is.
    pub fn sort_field(&self) -> &SortField {
        &self.sort_field
    }

    /// `getField()`.
    pub fn get_field(&self) -> Option<&str> {
        self.sort_field.field()
    }

    /// `getComparator(int numHits, Pruning pruning)`.
    pub fn get_comparator(&self, num_hits: usize, _pruning: Pruning) -> Geo3DPointOutsideDistanceComparator {
        Geo3DPointOutsideDistanceComparator::new(
            self.get_field().unwrap_or_else(|| crate::geom::npe()),
            &self.planet_model,
            self.distance_shape.clone(),
            num_hits,
        )
    }

    /// `getMissingValue()`.
    pub fn get_missing_value(&self) -> Option<f64> {
        match self.sort_field.missing_value() {
            Some(MissingValue::Double(v)) => Some(v),
            _ => None,
        }
    }

    /// `setMissingValue(Object)`: only `Double.POSITIVE_INFINITY` is accepted.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for any other value.
    pub fn set_missing_value(&mut self, missing_value: Option<MissingValue>) -> Result<()> {
        if missing_value != Some(MissingValue::Double(f64::INFINITY)) {
            return Err(LuceneError::illegal_argument(format!(
                "Missing value can only be Double.POSITIVE_INFINITY (missing values last), but got {}",
                missing_value_to_string(missing_value)
            )));
        }
        self.sort_field = SortField::with_kind(
            self.sort_field.field().map(str::to_string),
            SortFieldType::Custom,
            self.sort_field.reverse(),
            missing_value,
            SortFieldKind::Plain,
        )?;
        Ok(())
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        // `SortField.hashCode()`: `Objects.hash(field, type, reverse,
        // comparatorSource, missingValue)`; `type` is the enum constant
        // `CUSTOM`, whose identity hash has the stable stand-in.
        let mut super_hash: i32 = 1;
        super_hash = super_hash
            .wrapping_mul(31)
            .wrapping_add(self.sort_field.field().map_or(0, java_string_hash_code));
        super_hash = super_hash
            .wrapping_mul(31)
            .wrapping_add(java_identity_hash_code_stand_in("org.apache.lucene.search.SortField$Type.CUSTOM"));
        super_hash = super_hash
            .wrapping_mul(31)
            .wrapping_add(if self.sort_field.reverse() { 1231 } else { 1237 });
        super_hash = super_hash.wrapping_mul(31);
        super_hash = super_hash
            .wrapping_mul(31)
            .wrapping_add(missing_value_hash(self.sort_field.missing_value()));
        let mut result = super_hash;
        let temp = self.distance_shape.java_hash_code() as i64;
        result = result
            .wrapping_mul(31)
            .wrapping_add((temp ^ ((temp as u64) >> 32) as i64) as i32);
        result
    }

    /// `equals(Object)` against another sort field of this class.
    pub fn java_equals(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
            || (self.sort_field == other.sort_field
                && self.distance_shape.java_equals(other.distance_shape.as_any()))
    }
}

/// `String.valueOf(Object)` of a missing value.
fn missing_value_to_string(missing_value: Option<MissingValue>) -> String {
    match missing_value {
        None => "null".to_string(),
        Some(MissingValue::Double(v)) => d2s(v),
        Some(MissingValue::Float(v)) => rucene::util::java_fmt::float_to_string(v),
        Some(MissingValue::Int(v)) => v.to_string(),
        Some(MissingValue::Long(v)) => v.to_string(),
        Some(MissingValue::StringFirst) => "SortField.STRING_FIRST".to_string(),
        Some(MissingValue::StringLast) => "SortField.STRING_LAST".to_string(),
    }
}

/// `Object.hashCode()` of a missing value.
fn missing_value_hash(missing_value: Option<MissingValue>) -> i32 {
    match missing_value {
        None => 0,
        Some(MissingValue::Double(v)) => dhash(v),
        Some(MissingValue::Float(v)) => {
            if v.is_nan() {
                0x7fc0_0000
            } else {
                v.to_bits() as i32
            }
        }
        Some(MissingValue::Int(v)) => v,
        Some(MissingValue::Long(v)) => (v ^ ((v as u64) >> 32) as i64) as i32,
        Some(MissingValue::StringFirst) => {
            java_identity_hash_code_stand_in("org.apache.lucene.search.SortField.STRING_FIRST")
        }
        Some(MissingValue::StringLast) => {
            java_identity_hash_code_stand_in("org.apache.lucene.search.SortField.STRING_LAST")
        }
    }
}

impl fmt::Display for Geo3DPointOutsideSortField {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<outsideDistanceShape:\"{}\" shape={}",
            self.get_field().unwrap_or("null"),
            self.distance_shape
        )?;
        let missing = self.get_missing_value();
        if missing != Some(f64::INFINITY) {
            write!(f, " missingValue={}", missing_value_to_string(self.sort_field.missing_value()))?;
        }
        f.write_str(">")
    }
}
