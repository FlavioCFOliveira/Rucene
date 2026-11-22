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


//! The types declared in `Geo3DDocValuesField.java`, of the Java package
//! `org.apache.lucene.spatial3d`.

use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

use rucene::analysis::{Analyzer, BinaryTokenStream, TokenStream};
use rucene::document::{FieldType, InvertableType, NumericValue, SharedReader, StoredValue};
use rucene::error::{LuceneError, Result};
use rucene::geo::geometry::Polygon;
use rucene::index::{DocValuesType, FieldInfo, IndexableField, IndexableFieldType};
use rucene::util::BytesRef;

use crate::geo3_d_point_outside_sort_field::Geo3DPointOutsideSortField;
use crate::geo3_d_point_sort_field::Geo3DPointSortField;
use crate::geo3_d_util::Geo3DUtil;
use crate::geom::d2s;
use crate::geom::geo_point::GeoPoint;
use crate::geom::planet_model::PlanetModel;

/// An per-document 3D location field.
///
/// Equivalent to `org.apache.lucene.spatial3d.Geo3DDocValuesField`. Java's
/// `extends Field` is this struct's [`IndexableField`] implementation; the
/// mutable `fieldsData` `Long` is an atomic, as `setLocationValue` takes
/// `&self` like the core port's doc-values fields.
///
/// The sort factories return the crate's sort-field types, which hold their
/// `SortField` (see [`Geo3DPointSortField`]).
#[derive(Debug)]
pub struct Geo3DDocValuesField {
    name: String,
    field_type: Arc<FieldType>,
    planet_model: Arc<PlanetModel>,
    fields_data: AtomicI64,
}

impl Geo3DDocValuesField {
    /// `Geo3DDocValuesField.TYPE`: `SORTED_NUMERIC` doc values.
    pub fn type_() -> &'static Arc<FieldType> {
        static TYPE: OnceLock<Arc<FieldType>> = OnceLock::new();
        TYPE.get_or_init(|| {
            let ft = FieldType::new();
            ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
                .expect("INVARIANT: a new type is mutable");
            ft.freeze();
            Arc::new(ft)
        })
    }

    fn with_name(name: &str, planet_model: &Arc<PlanetModel>) -> Self {
        Self {
            name: name.to_string(),
            field_type: Self::type_().clone(),
            planet_model: planet_model.clone(),
            fields_data: AtomicI64::new(0),
        }
    }

    /// `new Geo3DDocValuesField(String name, GeoPoint point, PlanetModel)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a point out of the model.
    pub fn new_point(name: &str, point: &GeoPoint, planet_model: &Arc<PlanetModel>) -> Result<Self> {
        let s = Self::with_name(name, planet_model);
        s.set_location_value(point)?;
        Ok(s)
    }

    /// `new Geo3DDocValuesField(String name, double x, double y, double z,
    /// PlanetModel)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a value out of the model.
    pub fn new_xyz(name: &str, x: f64, y: f64, z: f64, planet_model: &Arc<PlanetModel>) -> Result<Self> {
        let s = Self::with_name(name, planet_model);
        s.set_location_value_xyz(x, y, z)?;
        Ok(s)
    }

    /// `setLocationValue(GeoPoint)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a point out of the model.
    pub fn set_location_value(&self, point: &GeoPoint) -> Result<()> {
        let v = self.planet_model.get_doc_value_encoder().encode_point(point)?;
        self.fields_data.store(v, Ordering::Relaxed);
        Ok(())
    }

    /// `setLocationValue(double, double, double)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a value out of the model.
    pub fn set_location_value_xyz(&self, x: f64, y: f64, z: f64) -> Result<()> {
        let v = self.planet_model.get_doc_value_encoder().encode_point_xyz(x, y, z)?;
        self.fields_data.store(v, Ordering::Relaxed);
        Ok(())
    }

    /// `checkCompatible(FieldInfo)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the field was indexed with another
    /// doc-values type.
    pub(crate) fn check_compatible(field_info: &FieldInfo) -> Result<()> {
        let dvt = field_info.get_doc_values_type();
        if dvt != DocValuesType::NONE && dvt != DocValuesType::SORTED_NUMERIC {
            return Err(LuceneError::illegal_argument(format!(
                "field=\"{}\" was indexed with docValuesType={:?} but this type has docValuesType={:?}, is the field really a Geo3DDocValuesField?",
                field_info.name,
                dvt,
                DocValuesType::SORTED_NUMERIC
            )));
        }
        Ok(())
    }

    /// `newDistanceSort(String, double, double, double, PlanetModel)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid arguments.
    pub fn new_distance_sort(
        field: &str,
        latitude: f64,
        longitude: f64,
        max_radius_meters: f64,
        planet_model: &Arc<PlanetModel>,
    ) -> Result<Geo3DPointSortField> {
        let shape = Geo3DUtil::from_distance(planet_model, latitude, longitude, max_radius_meters)?;
        Geo3DPointSortField::new(field, planet_model, shape)
    }

    /// `newPathSort(String, double[], double[], double, PlanetModel)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid arguments.
    pub fn new_path_sort(
        field: &str,
        path_latitudes: &[f64],
        path_longitudes: &[f64],
        path_width_meters: f64,
        planet_model: &Arc<PlanetModel>,
    ) -> Result<Geo3DPointSortField> {
        let shape = Geo3DUtil::from_path(planet_model, path_latitudes, path_longitudes, path_width_meters)?;
        Geo3DPointSortField::new(field, planet_model, shape)
    }

    /// `newOutsideDistanceSort(String, double, double, double, PlanetModel)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid arguments.
    pub fn new_outside_distance_sort(
        field: &str,
        latitude: f64,
        longitude: f64,
        max_radius_meters: f64,
        planet_model: &Arc<PlanetModel>,
    ) -> Result<Geo3DPointOutsideSortField> {
        let shape = Geo3DUtil::from_distance(planet_model, latitude, longitude, max_radius_meters)?;
        Geo3DPointOutsideSortField::new(field, planet_model, shape)
    }

    /// `newOutsideBoxSort(String, double, double, double, double, PlanetModel)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid arguments.
    pub fn new_outside_box_sort(
        field: &str,
        min_latitude: f64,
        max_latitude: f64,
        min_longitude: f64,
        max_longitude: f64,
        planet_model: &Arc<PlanetModel>,
    ) -> Result<Geo3DPointOutsideSortField> {
        let shape = Geo3DUtil::from_box(planet_model, min_latitude, max_latitude, min_longitude, max_longitude)?;
        Geo3DPointOutsideSortField::new(field, planet_model, shape)
    }

    /// `newOutsidePolygonSort(String, PlanetModel, Polygon...)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid polygons.
    pub fn new_outside_polygon_sort(
        field: &str,
        planet_model: &Arc<PlanetModel>,
        polygons: &[Polygon],
    ) -> Result<Geo3DPointOutsideSortField> {
        let shape = Geo3DUtil::from_polygon(planet_model, polygons)?;
        Geo3DPointOutsideSortField::new(field, planet_model, shape)
    }

    /// `newOutsideLargePolygonSort(String, PlanetModel, Polygon...)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid polygons.
    pub fn new_outside_large_polygon_sort(
        field: &str,
        planet_model: &Arc<PlanetModel>,
        polygons: &[Polygon],
    ) -> Result<Geo3DPointOutsideSortField> {
        let shape = Geo3DUtil::from_large_polygon(planet_model, polygons)?;
        Geo3DPointOutsideSortField::new(field, planet_model, shape)
    }

    /// `newOutsidePathSort(String, double[], double[], double, PlanetModel)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid arguments.
    pub fn new_outside_path_sort(
        field: &str,
        path_latitudes: &[f64],
        path_longitudes: &[f64],
        path_width_meters: f64,
        planet_model: &Arc<PlanetModel>,
    ) -> Result<Geo3DPointOutsideSortField> {
        let shape = Geo3DUtil::from_path(planet_model, path_latitudes, path_longitudes, path_width_meters)?;
        Geo3DPointOutsideSortField::new(field, planet_model, shape)
    }

    /// The field's name (`Field.name()`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The encoded value (`fieldsData`).
    pub fn numeric_value(&self) -> i64 {
        self.fields_data.load(Ordering::Relaxed)
    }
}

impl IndexableField for Geo3DDocValuesField {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(BinaryTokenStream::new(BytesRef::new(Vec::new()))?))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.fields_data.load(Ordering::Relaxed)))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

impl fmt::Display for Geo3DDocValuesField {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let current_value = self.fields_data.load(Ordering::Relaxed);
        let encoder = self.planet_model.get_doc_value_encoder();
        write!(
            f,
            "Geo3DDocValuesField <{}:{},{},{}>",
            self.name,
            d2s(encoder.decode_x_value(current_value)),
            d2s(encoder.decode_y_value(current_value)),
            d2s(encoder.decode_z_value(current_value))
        )
    }
}
