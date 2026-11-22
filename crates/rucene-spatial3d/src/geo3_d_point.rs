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


//! The types declared in `Geo3DPoint.java`, of the Java package
//! `org.apache.lucene.spatial3d`.

use std::fmt;
use std::sync::{Arc, OnceLock};

use rucene::analysis::{Analyzer, BinaryTokenStream, TokenStream};
use rucene::document::{FieldType, InvertableType, NumericValue, SharedReader, StoredValue};
use rucene::error::Result;
use rucene::geo::encoding::GeoUtils;
use rucene::geo::geometry::Polygon;
use rucene::index::{IndexableField, IndexableFieldType};
use rucene::search::Query;
use rucene::util::{BytesRef, NumericUtils};

use crate::geo3_d_util::Geo3DUtil;
use crate::geom::geo_point::GeoPoint;
use crate::geom::geo_shape::GeoShape;
use crate::geom::planet_model::{PlanetModel, WGS84};
use crate::geom::d2s;
use crate::point_in_geo3_d_shape_query::PointInGeo3DShapeQuery;

/// Add this to a document to index lat/lon or x/y/z point, indexed as a 3D
/// point. Multiple values are allowed: just add multiple Geo3DPoint to the
/// document with the same field name.
///
/// Equivalent to `org.apache.lucene.spatial3d.Geo3DPoint`. Java's `extends
/// Field` is this struct's [`IndexableField`] implementation over the name,
/// the shared [`type_`](Self::type_) and the packed `fieldsData`.
#[derive(Debug)]
pub struct Geo3DPoint {
    name: String,
    field_type: Arc<FieldType>,
    /// Planet Model for this Geo3DPoint.
    planet_model: Arc<PlanetModel>,
    /// The packed x, y, z (`fieldsData`).
    fields_data: BytesRef,
}

impl Geo3DPoint {
    /// `Geo3DPoint.TYPE`: indexing `FieldType`, three dimensions of
    /// `Integer.BYTES`.
    pub fn type_() -> &'static Arc<FieldType> {
        static TYPE: OnceLock<Arc<FieldType>> = OnceLock::new();
        TYPE.get_or_init(|| {
            let ft = FieldType::new();
            ft.set_dimensions(3, 4)
                .expect("INVARIANT: three dimensions of four bytes fit a new, mutable type");
            ft.freeze();
            Arc::new(ft)
        })
    }

    /// `new Geo3DPoint(String name, double lat, double lon)`: a point on the
    /// WGS84 model, from degrees.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid latitude or longitude.
    pub fn new_lat_lon(name: &str, lat: f64, lon: f64) -> Result<Self> {
        Self::new_lat_lon_with_model(name, &WGS84, lat, lon)
    }

    /// `new Geo3DPoint(String name, double x, double y, double z)`: a point
    /// on the WGS84 model.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a value out of the model.
    pub fn new_xyz(name: &str, x: f64, y: f64, z: f64) -> Result<Self> {
        Self::new_xyz_with_model(name, &WGS84, x, y, z)
    }

    /// `new Geo3DPoint(String name, PlanetModel, double x, double y, double z)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a value out of the model.
    pub fn new_xyz_with_model(name: &str, planet_model: &Arc<PlanetModel>, x: f64, y: f64, z: f64) -> Result<Self> {
        Ok(Self {
            name: name.to_string(),
            field_type: Self::type_().clone(),
            planet_model: planet_model.clone(),
            fields_data: Self::fill_fields_data(planet_model, x, y, z)?,
        })
    }

    /// `new Geo3DPoint(String name, PlanetModel, double latitude, double
    /// longitude)`: from degrees.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid latitude or longitude.
    pub fn new_lat_lon_with_model(name: &str, planet_model: &Arc<PlanetModel>, latitude: f64, longitude: f64) -> Result<Self> {
        GeoUtils::check_latitude(latitude)?;
        GeoUtils::check_longitude(longitude)?;
        let point = GeoPoint::new_lat_lon(planet_model, Geo3DUtil::from_degrees(latitude), Geo3DUtil::from_degrees(longitude))?;
        Ok(Self {
            name: name.to_string(),
            field_type: Self::type_().clone(),
            planet_model: planet_model.clone(),
            fields_data: Self::fill_fields_data(planet_model, point.x, point.y, point.z)?,
        })
    }

    /// `newDistanceQuery(String, PlanetModel, double, double, double)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid arguments.
    pub fn new_distance_query(
        field: &str,
        planet_model: &Arc<PlanetModel>,
        latitude: f64,
        longitude: f64,
        radius_meters: f64,
    ) -> Result<Arc<dyn Query>> {
        let shape = Geo3DUtil::from_distance(planet_model, latitude, longitude, radius_meters)?;
        Ok(Self::new_shape_query(field, shape))
    }

    /// `newBoxQuery(String, PlanetModel, double, double, double, double)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid arguments.
    pub fn new_box_query(
        field: &str,
        planet_model: &Arc<PlanetModel>,
        min_latitude: f64,
        max_latitude: f64,
        min_longitude: f64,
        max_longitude: f64,
    ) -> Result<Arc<dyn Query>> {
        let shape = Geo3DUtil::from_box(planet_model, min_latitude, max_latitude, min_longitude, max_longitude)?;
        Ok(Self::new_shape_query(field, shape))
    }

    /// `newPolygonQuery(String, PlanetModel, Polygon...)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid polygons.
    pub fn new_polygon_query(field: &str, planet_model: &Arc<PlanetModel>, polygons: &[Polygon]) -> Result<Arc<dyn Query>> {
        let shape = Geo3DUtil::from_polygon(planet_model, polygons)?;
        Ok(Self::new_shape_query(field, shape))
    }

    /// `newLargePolygonQuery(String, PlanetModel, Polygon...)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid polygons.
    pub fn new_large_polygon_query(field: &str, planet_model: &Arc<PlanetModel>, polygons: &[Polygon]) -> Result<Arc<dyn Query>> {
        let shape = Geo3DUtil::from_large_polygon(planet_model, polygons)?;
        Ok(Self::new_shape_query(field, shape))
    }

    /// `newPathQuery(String, double[], double[], double, PlanetModel)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid arguments.
    pub fn new_path_query(
        field: &str,
        path_latitudes: &[f64],
        path_longitudes: &[f64],
        path_width_meters: f64,
        planet_model: &Arc<PlanetModel>,
    ) -> Result<Arc<dyn Query>> {
        let shape = Geo3DUtil::from_path(planet_model, path_latitudes, path_longitudes, path_width_meters)?;
        Ok(Self::new_shape_query(field, shape))
    }

    fn fill_fields_data(planet_model: &PlanetModel, x: f64, y: f64, z: f64) -> Result<BytesRef> {
        let mut bytes = vec![0u8; 12];
        Self::encode_dimension(x, &mut bytes, 0, planet_model)?;
        Self::encode_dimension(y, &mut bytes, 4, planet_model)?;
        Self::encode_dimension(z, &mut bytes, 2 * 4, planet_model)?;
        Ok(BytesRef::new(bytes))
    }

    /// `encodeDimension(double, byte[], int, PlanetModel)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a value out of the model.
    pub fn encode_dimension(value: f64, bytes: &mut [u8], offset: usize, planet_model: &PlanetModel) -> Result<()> {
        NumericUtils::int_to_sortable_bytes(planet_model.encode_value(value)?, bytes, offset);
        Ok(())
    }

    /// `decodeDimension(byte[], int, PlanetModel)`.
    pub fn decode_dimension(value: &[u8], offset: usize, planet_model: &PlanetModel) -> f64 {
        planet_model.decode_value(NumericUtils::sortable_bytes_to_int(value, offset))
    }

    /// `newShapeQuery(String, GeoShape)`.
    pub fn new_shape_query(field: &str, shape: Arc<dyn GeoShape>) -> Arc<dyn Query> {
        Arc::new(PointInGeo3DShapeQuery::new(field, shape))
    }

    /// The field's name (`Field.name()`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The packed value (`fieldsData`).
    pub fn packed_value(&self) -> &[u8] {
        self.fields_data.slice()
    }
}

impl IndexableField for Geo3DPoint {
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
        Ok(Box::new(BinaryTokenStream::new(self.fields_data.clone())?))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        Some(self.fields_data.clone())
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

impl fmt::Display for Geo3DPoint {
    /// `Geo3DPoint.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self.fields_data.slice();
        write!(
            f,
            "Geo3DPoint <{}: x={} y={} z={}>",
            self.name,
            d2s(Self::decode_dimension(bytes, 0, &self.planet_model)),
            d2s(Self::decode_dimension(bytes, 4, &self.planet_model)),
            d2s(Self::decode_dimension(bytes, 2 * 4, &self.planet_model))
        )
    }
}
