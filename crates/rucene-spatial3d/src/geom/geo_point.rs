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

//! The types declared in `GeoPoint.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::util::java_math;
use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};

use rucene::error::Result;

use super::planet_model::PlanetModel;
use super::serializable_object::{self as so, SerializableObject};
use super::tools::Tools;
use super::vector::Vector;
use super::{d2s, iae};

/// This class represents a point on the surface of a sphere or ellipsoid.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPoint`.
///
/// Java's `extends Vector` is a [`Deref`] to the vector part, which carries
/// the cached magnitude (see [`Vector`]).
pub struct GeoPoint {
    v: Vector,
    /// Lazily-evaluated latitude (`NEGATIVE_INFINITY` when not computed).
    latitude: AtomicU64,
    /// Lazily-evaluated longitude (`NEGATIVE_INFINITY` when not computed).
    longitude: AtomicU64,
}

const NEG_INF_BITS: u64 = 0xfff0_0000_0000_0000;

impl Deref for GeoPoint {
    type Target = Vector;
    #[inline]
    fn deref(&self) -> &Vector {
        &self.v
    }
}

impl AsRef<Vector> for GeoPoint {
    fn as_ref(&self) -> &Vector {
        &self.v
    }
}

impl Clone for GeoPoint {
    fn clone(&self) -> Self {
        Self {
            v: self.v.clone(),
            latitude: AtomicU64::new(self.latitude.load(Ordering::Relaxed)),
            longitude: AtomicU64::new(self.longitude.load(Ordering::Relaxed)),
        }
    }
}

impl fmt::Debug for GeoPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl GeoPoint {
    fn raw(x: f64, y: f64, z: f64, magnitude: f64, lat: f64, lon: f64) -> Self {
        Self {
            v: Vector {
                x,
                y,
                z,
                point_magnitude: Some(AtomicU64::new(magnitude.to_bits())),
            },
            latitude: AtomicU64::new(lat.to_bits()),
            longitude: AtomicU64::new(lon.to_bits()),
        }
    }

    /// `new GeoPoint(PlanetModel, double sinLat, double sinLon, double cosLat,
    /// double cosLon, double lat, double lon)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the latitude or longitude is out of
    /// range.
    pub fn new_sin_cos_lat_lon(
        planet_model: &PlanetModel,
        sin_lat: f64,
        sin_lon: f64,
        cos_lat: f64,
        cos_lon: f64,
        lat: f64,
        lon: f64,
    ) -> Result<Self> {
        Self::new_magnitude_lat_lon(
            Vector::compute_desired_ellipsoid_magnitude(
                planet_model,
                cos_lat * cos_lon,
                cos_lat * sin_lon,
                sin_lat,
            ),
            cos_lat * cos_lon,
            cos_lat * sin_lon,
            sin_lat,
            lat,
            lon,
        )
    }

    /// `new GeoPoint(PlanetModel, double sinLat, double sinLon, double cosLat,
    /// double cosLon)`.
    pub fn new_sin_cos(
        planet_model: &PlanetModel,
        sin_lat: f64,
        sin_lon: f64,
        cos_lat: f64,
        cos_lon: f64,
    ) -> Self {
        Self::new_magnitude(
            Vector::compute_desired_ellipsoid_magnitude(
                planet_model,
                cos_lat * cos_lon,
                cos_lat * sin_lon,
                sin_lat,
            ),
            cos_lat * cos_lon,
            cos_lat * sin_lon,
            sin_lat,
        )
    }

    /// `new GeoPoint(PlanetModel, double lat, double lon)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the latitude or longitude is out of
    /// range.
    pub fn new_lat_lon(planet_model: &PlanetModel, lat: f64, lon: f64) -> Result<Self> {
        Self::new_sin_cos_lat_lon(
            planet_model,
            java_math::sin(lat),
            java_math::sin(lon),
            java_math::cos(lat),
            java_math::cos(lon),
            lat,
            lon,
        )
    }

    /// `new GeoPoint(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream.
    pub fn from_stream_with_model(
        _planet_model: &PlanetModel,
        input: &mut dyn Read,
    ) -> Result<Self> {
        Self::from_stream(input)
    }

    /// `new GeoPoint(InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream.
    pub fn from_stream(input: &mut dyn Read) -> Result<Self> {
        let lat = so::read_double(input)?;
        let lon = so::read_double(input)?;
        let x = so::read_double(input)?;
        let y = so::read_double(input)?;
        let z = so::read_double(input)?;
        Ok(Self::new_lat_lon_xyz(lat, lon, x, y, z))
    }

    /// `new GeoPoint(double lat, double lon, double x, double y, double z)`.
    pub fn new_lat_lon_xyz(lat: f64, lon: f64, x: f64, y: f64, z: f64) -> Self {
        Self::raw(x, y, z, f64::NEG_INFINITY, lat, lon)
    }

    /// `new GeoPoint(double magnitude, double x, double y, double z, double lat,
    /// double lon)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the latitude or longitude is out of
    /// range.
    pub fn new_magnitude_lat_lon(
        magnitude: f64,
        x: f64,
        y: f64,
        z: f64,
        lat: f64,
        lon: f64,
    ) -> Result<Self> {
        if lat > std::f64::consts::PI * 0.5 || lat < -std::f64::consts::PI * 0.5 {
            return Err(iae(format!(
                "Latitude {} is out of range: must range from -Math.PI/2 to Math.PI/2",
                d2s(lat)
            )));
        }
        if lon < -std::f64::consts::PI || lon > std::f64::consts::PI {
            return Err(iae(format!(
                "Longitude {} is out of range: must range from -Math.PI to Math.PI",
                d2s(lon)
            )));
        }
        Ok(Self::raw(
            x * magnitude,
            y * magnitude,
            z * magnitude,
            magnitude,
            lat,
            lon,
        ))
    }

    /// `new GeoPoint(double magnitude, double x, double y, double z)`.
    pub fn new_magnitude(magnitude: f64, x: f64, y: f64, z: f64) -> Self {
        Self::raw(
            x * magnitude,
            y * magnitude,
            z * magnitude,
            magnitude,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        )
    }

    /// `new GeoPoint(double x, double y, double z)`.
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self::raw(
            x,
            y,
            z,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        )
    }

    /// The vector part (Java's `super`).
    #[inline]
    pub fn as_vector(&self) -> &Vector {
        &self.v
    }

    /// `arcDistance(Vector)`.
    pub fn arc_distance(&self, v: &Vector) -> f64 {
        Tools::safe_acos(self.dot_product(v) / (self.magnitude() * v.magnitude()))
    }

    /// `arcDistance(double, double, double)`.
    pub fn arc_distance_xyz(&self, x: f64, y: f64, z: f64) -> f64 {
        Tools::safe_acos(
            self.dot_product_xyz(x, y, z) / (self.magnitude() * Vector::magnitude_of(x, y, z)),
        )
    }

    /// `getLatitude()`.
    pub fn get_latitude(&self) -> f64 {
        let mut lat = f64::from_bits(self.latitude.load(Ordering::Relaxed));
        if lat == f64::NEG_INFINITY {
            lat = java_math::asin(self.v.z / self.magnitude());
            self.latitude.store(lat.to_bits(), Ordering::Relaxed);
        }
        lat
    }

    /// `getLongitude()`.
    pub fn get_longitude(&self) -> f64 {
        let mut lon = f64::from_bits(self.longitude.load(Ordering::Relaxed));
        if lon == f64::NEG_INFINITY {
            if self.v.x.abs() < Vector::MINIMUM_RESOLUTION
                && self.v.y.abs() < Vector::MINIMUM_RESOLUTION
            {
                lon = 0.0;
            } else {
                lon = java_math::atan2(self.v.y, self.v.x);
            }
            self.longitude.store(lon.to_bits(), Ordering::Relaxed);
        }
        lon
    }

    /// `magnitude()` (cached).
    #[inline]
    pub fn magnitude(&self) -> f64 {
        self.v.magnitude()
    }

    /// The `latitude` field as stored (`NEGATIVE_INFINITY` when not computed),
    /// which subclasses read directly.
    pub(crate) fn latitude_field(&self) -> f64 {
        f64::from_bits(self.latitude.load(Ordering::Relaxed))
    }

    /// The `longitude` field as stored (`NEGATIVE_INFINITY` when not
    /// computed), which subclasses read directly.
    pub(crate) fn longitude_field(&self) -> f64 {
        f64::from_bits(self.longitude.load(Ordering::Relaxed))
    }

    /// `isIdentical(GeoPoint)`.
    pub fn is_identical(&self, p: &GeoPoint) -> bool {
        self.is_identical_xyz(p.x, p.y, p.z)
    }

    /// `isIdentical(double, double, double)`.
    pub fn is_identical_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        (self.v.x - x).abs() < Vector::MINIMUM_RESOLUTION
            && (self.v.y - y).abs() < Vector::MINIMUM_RESOLUTION
            && (self.v.z - z).abs() < Vector::MINIMUM_RESOLUTION
    }

    /// `Vector.equals(Object)` (inherited).
    pub fn java_equals(&self, other: &Vector) -> bool {
        self.v.java_equals(other)
    }

    /// `Vector.hashCode()` (inherited).
    pub fn java_hash_code(&self) -> i32 {
        self.v.java_hash_code()
    }
}

impl PartialEq for GeoPoint {
    fn eq(&self, other: &Self) -> bool {
        self.v.java_equals(&other.v)
    }
}

impl Eq for GeoPoint {}

impl Hash for GeoPoint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.v.java_hash_code());
    }
}

impl fmt::Display for GeoPoint {
    /// `GeoPoint.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.longitude.load(Ordering::Relaxed) == NEG_INF_BITS {
            return fmt::Display::fmt(&self.v, f);
        }
        write!(
            f,
            "[lat={}, lon={}({})]",
            d2s(self.get_latitude()),
            d2s(self.get_longitude()),
            self.v
        )
    }
}

impl SerializableObject for GeoPoint {
    fn write(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.get_latitude())?;
        so::write_double(output, self.get_longitude())?;
        so::write_double(output, self.v.x)?;
        so::write_double(output, self.v.y)?;
        so::write_double(output, self.v.z)
    }
    fn class_name(&self) -> &'static str {
        "org.apache.lucene.spatial3d.geom.GeoPoint"
    }
    fn java_equals(&self, o: &dyn Any) -> bool {
        o.downcast_ref::<GeoPoint>()
            .is_some_and(|p| self.v.java_equals(&p.v))
    }
    fn java_hash_code(&self) -> i32 {
        self.v.java_hash_code()
    }
}
