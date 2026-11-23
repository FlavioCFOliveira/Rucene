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

//! The types declared in `PlanetModel.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::util::java_math;
use std::any::Any;
use std::f64::consts::PI;
use std::fmt;
use std::io::{Read, Write};
use std::sync::{Arc, LazyLock};

use rucene::error::Result;

use super::geo_point::GeoPoint;
use super::serializable_object::{self as so, SerializableObject};
use super::vector::Vector;
use super::{d2s, dhash, iae, java_max, java_min};

/// Holds mathematical constants associated with the model of a planet.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.PlanetModel`. The poles
/// (`NORTH_POLE`, ...) are the snake_case fields `north_pole`, ...; the
/// static models are [`SPHERE`], [`WGS84`] and [`CLARKE_1866`].
pub struct PlanetModel {
    /// Semi-major axis.
    pub a: f64,
    /// Semi-minor axis.
    pub b: f64,
    /// The x/y scaling factor.
    pub xy_scaling: f64,
    /// The z scaling factor.
    pub z_scaling: f64,
    /// The inverse of `xy_scaling`.
    pub inverse_xy_scaling: f64,
    /// The inverse of `z_scaling`.
    pub inverse_z_scaling: f64,
    /// The square of the inverse of `xy_scaling`.
    pub inverse_xy_scaling_squared: f64,
    /// The square of the inverse of `z_scaling`.
    pub inverse_z_scaling_squared: f64,
    /// The scaled flattening value.
    pub scaled_flattening: f64,
    /// `(ab^2 - c^2) / c^2`.
    pub square_ratio: f64,
    /// The mean radius of the planet model.
    pub mean_radius: f64,
    /// The scale of the planet.
    pub scale: f64,
    /// The inverse of `scale`.
    pub inverse_scale: f64,
    /// North pole.
    pub north_pole: GeoPoint,
    /// South pole.
    pub south_pole: GeoPoint,
    /// Min X pole.
    pub min_x_pole: GeoPoint,
    /// Max X pole.
    pub max_x_pole: GeoPoint,
    /// Min Y pole.
    pub min_y_pole: GeoPoint,
    /// Max Y pole.
    pub max_y_pole: GeoPoint,
    /// Minimum surface distance between poles.
    pub minimum_pole_distance: f64,
    /// Max absolute value of an encoded coordinate (`MAX_VALUE`).
    pub max_value: f64,
    mul: f64,
    /// The decoding step (`DECODE`).
    pub decode: f64,
    /// Max encoded value (`MAX_ENCODED_VALUE`).
    pub max_encoded_value: i32,
    /// Min encoded value (`MIN_ENCODED_VALUE`).
    pub min_encoded_value: i32,
    /// The doc-values encoder (`docValueEncoder`).
    pub doc_value_encoder: DocValueEncoder,
}

/// `PlanetModel.SPHERE`: the planet model of a unit sphere.
pub static SPHERE: LazyLock<Arc<PlanetModel>> =
    LazyLock::new(|| Arc::new(PlanetModel::new(1.0, 1.0)));

/// `PlanetModel.WGS84`: the WGS84 ellipsoid.
pub static WGS84: LazyLock<Arc<PlanetModel>> =
    LazyLock::new(|| Arc::new(PlanetModel::new(6378137.0, 6356752.314245)));

/// `PlanetModel.CLARKE_1866`: the Clarke 1866 ellipsoid.
pub static CLARKE_1866: LazyLock<Arc<PlanetModel>> =
    LazyLock::new(|| Arc::new(PlanetModel::new(6378206.4, 6356583.8)));

impl PlanetModel {
    /// Number of bits used to encode a coordinate.
    const BITS: i32 = 32;

    /// `new PlanetModel(double semiMajorAxis, double semiMinorAxis)`.
    pub fn new(semi_major_axis: f64, semi_minor_axis: f64) -> Self {
        let a = semi_major_axis;
        let b = semi_minor_axis;
        let mean_radius = (2.0 * semi_major_axis + semi_minor_axis) / 3.0;
        let xy_scaling = semi_major_axis / mean_radius;
        let z_scaling = semi_minor_axis / mean_radius;
        let scale = (2.0 * xy_scaling + z_scaling) / 3.0;
        let inverse_xy_scaling = 1.0 / xy_scaling;
        let inverse_z_scaling = 1.0 / z_scaling;
        let scaled_flattening = (xy_scaling - z_scaling) * inverse_xy_scaling;
        let square_ratio =
            (xy_scaling * xy_scaling - z_scaling * z_scaling) / (z_scaling * z_scaling);
        let inverse_xy_scaling_squared = inverse_xy_scaling * inverse_xy_scaling;
        let inverse_z_scaling_squared = inverse_z_scaling * inverse_z_scaling;
        // The pole latitudes and longitudes are all in range.
        let pole = |m: f64, x: f64, y: f64, z: f64, lat: f64, lon: f64| {
            GeoPoint::new_magnitude_lat_lon(m, x, y, z, lat, lon)
                .unwrap_or_else(|e| super::unchecked_panic(e))
        };
        let north_pole = pole(z_scaling, 0.0, 0.0, 1.0, PI * 0.5, 0.0);
        let south_pole = pole(z_scaling, 0.0, 0.0, -1.0, -PI * 0.5, 0.0);
        let min_x_pole = pole(xy_scaling, -1.0, 0.0, 0.0, 0.0, -PI);
        let max_x_pole = pole(xy_scaling, 1.0, 0.0, 0.0, 0.0, 0.0);
        let min_y_pole = pole(xy_scaling, 0.0, -1.0, 0.0, 0.0, -PI * 0.5);
        let max_y_pole = pole(xy_scaling, 0.0, 1.0, 0.0, 0.0, PI * 0.5);
        let inverse_scale = 1.0 / scale;
        let mut pm = Self {
            a,
            b,
            xy_scaling,
            z_scaling,
            inverse_xy_scaling,
            inverse_z_scaling,
            inverse_xy_scaling_squared,
            inverse_z_scaling_squared,
            scaled_flattening,
            square_ratio,
            mean_radius,
            scale,
            inverse_scale,
            north_pole,
            south_pole,
            min_x_pole,
            max_x_pole,
            min_y_pole,
            max_y_pole,
            minimum_pole_distance: 0.0,
            max_value: 0.0,
            mul: 0.0,
            decode: 0.0,
            max_encoded_value: 0,
            min_encoded_value: 0,
            doc_value_encoder: DocValueEncoder::new_placeholder(),
        };
        pm.minimum_pole_distance = java_min(
            pm.surface_distance(&pm.north_pole, &pm.south_pole),
            pm.surface_distance(&pm.min_x_pole, &pm.max_x_pole),
        );
        pm.max_value = pm.get_maximum_magnitude();
        pm.mul = (1i64 << Self::BITS) as f64 / (2.0 * pm.max_value);
        pm.decode = Self::get_next_safe_double(1.0 / pm.mul);
        // `encodeValue(-MAX_VALUE)` and `encodeValue(MAX_VALUE)` are in range.
        pm.min_encoded_value = pm
            .encode_value(-pm.max_value)
            .unwrap_or_else(|e| super::unchecked_panic(e));
        pm.max_encoded_value = pm
            .encode_value(pm.max_value)
            .unwrap_or_else(|e| super::unchecked_panic(e));
        pm.doc_value_encoder = DocValueEncoder::new(&pm);
        pm
    }

    /// `new PlanetModel(InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream.
    pub fn from_stream(input: &mut dyn Read) -> Result<Self> {
        let a = so::read_double(input)?;
        let b = so::read_double(input)?;
        Ok(Self::new(a, b))
    }

    /// `isSphere()`.
    pub fn is_sphere(&self) -> bool {
        self.xy_scaling == self.z_scaling
    }

    /// `getMinimumMagnitude()`.
    pub fn get_minimum_magnitude(&self) -> f64 {
        java_min(self.xy_scaling, self.z_scaling)
    }

    /// `getMaximumMagnitude()`.
    pub fn get_maximum_magnitude(&self) -> f64 {
        java_max(self.xy_scaling, self.z_scaling)
    }

    /// `getMinimumXValue()`.
    pub fn get_minimum_x_value(&self) -> f64 {
        -self.xy_scaling
    }

    /// `getMaximumXValue()`.
    pub fn get_maximum_x_value(&self) -> f64 {
        self.xy_scaling
    }

    /// `getMinimumYValue()`.
    pub fn get_minimum_y_value(&self) -> f64 {
        -self.xy_scaling
    }

    /// `getMaximumYValue()`.
    pub fn get_maximum_y_value(&self) -> f64 {
        self.xy_scaling
    }

    /// `getMinimumZValue()`.
    pub fn get_minimum_z_value(&self) -> f64 {
        -self.z_scaling
    }

    /// `getMaximumZValue()`.
    pub fn get_maximum_z_value(&self) -> f64 {
        self.z_scaling
    }

    /// `getMeanRadius()`.
    pub fn get_mean_radius(&self) -> f64 {
        self.mean_radius
    }

    /// `encodeValue(double)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the value is out of bounds.
    pub fn encode_value(&self, mut x: f64) -> Result<i32> {
        if x > self.get_maximum_magnitude() {
            return Err(iae(format!(
                "value={} is out-of-bounds (greater than planetMax={})",
                d2s(x),
                d2s(self.get_maximum_magnitude())
            )));
        }
        if x == self.get_maximum_magnitude() {
            x = x.next_down();
        }
        if x < -self.get_maximum_magnitude() {
            return Err(iae(format!(
                "value={} is out-of-bounds (less than than -planetMax={})",
                d2s(x),
                d2s(-self.get_maximum_magnitude())
            )));
        }
        let result = (x / self.decode).floor() as i64;
        debug_assert!(result >= i32::MIN as i64);
        debug_assert!(result <= i32::MAX as i64);
        Ok(result as i32)
    }

    /// `decodeValue(int)`.
    pub fn decode_value(&self, x: i32) -> f64 {
        let result = if x == self.min_encoded_value {
            -self.max_value
        } else if x == self.max_encoded_value {
            self.max_value
        } else {
            (x as f64 + 0.5) * self.decode
        };
        debug_assert!(result >= -self.max_value && result <= self.max_value);
        result
    }

    /// `getDocValueEncoder()`.
    pub fn get_doc_value_encoder(&self) -> &DocValueEncoder {
        &self.doc_value_encoder
    }

    fn get_next_safe_double(x: f64) -> f64 {
        let x_bits = if x.is_nan() {
            0x7ff8_0000_0000_0000u64 as i64
        } else {
            x.to_bits() as i64
        };
        let mut bits = x_bits.wrapping_add(i32::MAX as i64);
        bits &= !(i32::MAX as i64);
        let result = f64::from_bits(bits as u64);
        debug_assert!(result >= x);
        result
    }

    /// `pointOnSurface(Vector)`.
    pub fn point_on_surface(&self, v: &Vector) -> bool {
        self.point_on_surface_xyz(v.x, v.y, v.z)
    }

    /// `pointOnSurface(double, double, double)`.
    pub fn point_on_surface_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        (x * x * self.inverse_xy_scaling * self.inverse_xy_scaling
            + y * y * self.inverse_xy_scaling * self.inverse_xy_scaling
            + z * z * self.inverse_z_scaling * self.inverse_z_scaling
            - 1.0)
            .abs()
            < Vector::MINIMUM_RESOLUTION
    }

    /// `pointOutside(Vector)`.
    pub fn point_outside(&self, v: &Vector) -> bool {
        self.point_outside_xyz(v.x, v.y, v.z)
    }

    /// `pointOutside(double, double, double)`.
    pub fn point_outside_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        (x * x + y * y) * self.inverse_xy_scaling * self.inverse_xy_scaling
            + z * z * self.inverse_z_scaling * self.inverse_z_scaling
            - 1.0
            > Vector::MINIMUM_RESOLUTION
    }

    /// `createSurfacePoint(Vector)`.
    pub fn create_surface_point(&self, vector: &Vector) -> GeoPoint {
        self.create_surface_point_xyz(vector.x, vector.y, vector.z)
    }

    /// `createSurfacePoint(double, double, double)`.
    pub fn create_surface_point_xyz(&self, x: f64, y: f64, z: f64) -> GeoPoint {
        let t = java_math::sqrt(
            1.0 / (x * x * self.inverse_xy_scaling_squared
                + y * y * self.inverse_xy_scaling_squared
                + z * z * self.inverse_z_scaling_squared),
        );
        GeoPoint::new(t * x, t * y, t * z)
    }

    /// `bisection(GeoPoint, GeoPoint)`: `None` when the points are opposite.
    pub fn bisection(&self, pt1: &GeoPoint, pt2: &GeoPoint) -> Option<GeoPoint> {
        let a0 = (pt1.x + pt2.x) * 0.5;
        let b0 = (pt1.y + pt2.y) * 0.5;
        let c0 = (pt1.z + pt2.z) * 0.5;
        let denom = self.inverse_xy_scaling_squared * a0 * a0
            + self.inverse_xy_scaling_squared * b0 * b0
            + self.inverse_z_scaling_squared * c0 * c0;
        if denom < Vector::MINIMUM_RESOLUTION {
            return None;
        }
        let t = java_math::sqrt(1.0 / denom);
        Some(GeoPoint::new(t * a0, t * b0, t * c0))
    }

    /// `surfaceDistance(GeoPoint, GeoPoint)`: Vincenty's inverse formula.
    pub fn surface_distance(&self, pt1: &GeoPoint, pt2: &GeoPoint) -> f64 {
        let l = pt2.get_longitude() - pt1.get_longitude();
        let u1 =
            java_math::atan((1.0 - self.scaled_flattening) * java_math::tan(pt1.get_latitude()));
        let u2 =
            java_math::atan((1.0 - self.scaled_flattening) * java_math::tan(pt2.get_latitude()));
        let sin_u1 = java_math::sin(u1);
        let cos_u1 = java_math::cos(u1);
        let sin_u2 = java_math::sin(u2);
        let cos_u2 = java_math::cos(u2);
        let d_cos_u1_cos_u2 = cos_u1 * cos_u2;
        let d_cos_u1_sin_u2 = cos_u1 * sin_u2;
        let d_sin_u1_sin_u2 = sin_u1 * sin_u2;
        let d_sin_u1_cos_u2 = sin_u1 * cos_u2;
        let mut lambda = l;
        let mut lambda_p;
        let mut iter_limit = 0;
        let mut cos_sq_alpha;
        let mut sin_sigma;
        let mut cos2_sigma_m;
        let mut cos_sigma;
        let mut sigma;
        let mut sin_alpha;
        let mut c;
        let mut sin_lambda;
        let mut cos_lambda;
        loop {
            sin_lambda = java_math::sin(lambda);
            cos_lambda = java_math::cos(lambda);
            sin_sigma = java_math::sqrt(
                (cos_u2 * sin_lambda) * (cos_u2 * sin_lambda)
                    + (d_cos_u1_sin_u2 - d_sin_u1_cos_u2 * cos_lambda)
                        * (d_cos_u1_sin_u2 - d_sin_u1_cos_u2 * cos_lambda),
            );
            if sin_sigma == 0.0 {
                return 0.0;
            }
            cos_sigma = d_sin_u1_sin_u2 + d_cos_u1_cos_u2 * cos_lambda;
            sigma = java_math::atan2(sin_sigma, cos_sigma);
            sin_alpha = d_cos_u1_cos_u2 * sin_lambda / sin_sigma;
            cos_sq_alpha = 1.0 - sin_alpha * sin_alpha;
            cos2_sigma_m = cos_sigma - 2.0 * d_sin_u1_sin_u2 / cos_sq_alpha;
            if cos2_sigma_m.is_nan() {
                cos2_sigma_m = 0.0;
            }
            c = self.scaled_flattening / 16.0
                * cos_sq_alpha
                * (4.0 + self.scaled_flattening * (4.0 - 3.0 * cos_sq_alpha));
            lambda_p = lambda;
            lambda = l
                + (1.0 - c)
                    * self.scaled_flattening
                    * sin_alpha
                    * (sigma
                        + c * sin_sigma
                            * (cos2_sigma_m
                                + c * cos_sigma * (-1.0 + 2.0 * cos2_sigma_m * cos2_sigma_m)));
            if !((lambda - lambda_p).abs() >= Vector::MINIMUM_RESOLUTION && {
                iter_limit += 1;
                iter_limit < 100
            }) {
                break;
            }
        }
        let u_sq = cos_sq_alpha * self.square_ratio;
        let a_ = 1.0 + u_sq / 16384.0 * (4096.0 + u_sq * (-768.0 + u_sq * (320.0 - 175.0 * u_sq)));
        let b_ = u_sq / 1024.0 * (256.0 + u_sq * (-128.0 + u_sq * (74.0 - 47.0 * u_sq)));
        let delta_sigma = b_
            * sin_sigma
            * (cos2_sigma_m
                + b_ / 4.0
                    * (cos_sigma * (-1.0 + 2.0 * cos2_sigma_m * cos2_sigma_m)
                        - b_ / 6.0
                            * cos2_sigma_m
                            * (-3.0 + 4.0 * sin_sigma * sin_sigma)
                            * (-3.0 + 4.0 * cos2_sigma_m * cos2_sigma_m)));
        self.z_scaling * self.inverse_scale * a_ * (sigma - delta_sigma)
    }

    /// `surfacePointOnBearing(GeoPoint, double, double)`: Vincenty's direct
    /// formula.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the computed point is out of range.
    pub fn surface_point_on_bearing(
        &self,
        from: &GeoPoint,
        dist: f64,
        bearing: f64,
    ) -> Result<GeoPoint> {
        let lat = from.get_latitude();
        let lon = from.get_longitude();
        let sinalpha1 = java_math::sin(bearing);
        let cosalpha1 = java_math::cos(bearing);
        let tan_u1 = (1.0 - self.scaled_flattening) * java_math::tan(lat);
        let cos_u1 = 1.0 / java_math::sqrt(1.0 + tan_u1 * tan_u1);
        let sin_u1 = tan_u1 * cos_u1;
        let sigma1 = java_math::atan2(tan_u1, cosalpha1);
        let sinalpha = cos_u1 * sinalpha1;
        let cos_sqalpha = 1.0 - sinalpha * sinalpha;
        let u_sq = cos_sqalpha * self.square_ratio;
        let a_ = 1.0 + u_sq / 16384.0 * (4096.0 + u_sq * (-768.0 + u_sq * (320.0 - 175.0 * u_sq)));
        let b_ = u_sq / 1024.0 * (256.0 + u_sq * (-128.0 + u_sq * (74.0 - 47.0 * u_sq)));
        let mut cos2sigma_m;
        let mut sinsigma;
        let mut cossigma;
        let mut deltasigma;
        let mut sigma = dist / (self.z_scaling * self.inverse_scale * a_);
        let mut sigmaprime;
        let mut iterations: f64 = 0.0;
        loop {
            cos2sigma_m = java_math::cos(2.0 * sigma1 + sigma);
            sinsigma = java_math::sin(sigma);
            cossigma = java_math::cos(sigma);
            deltasigma = b_
                * sinsigma
                * (cos2sigma_m
                    + b_ / 4.0
                        * (cossigma * (-1.0 + 2.0 * cos2sigma_m * cos2sigma_m)
                            - b_ / 6.0
                                * cos2sigma_m
                                * (-3.0 + 4.0 * sinsigma * sinsigma)
                                * (-3.0 + 4.0 * cos2sigma_m * cos2sigma_m)));
            sigmaprime = sigma;
            sigma = dist / (self.z_scaling * self.inverse_scale * a_) + deltasigma;
            if !((sigma - sigmaprime).abs() >= Vector::MINIMUM_RESOLUTION && {
                iterations += 1.0;
                iterations < 100.0
            }) {
                break;
            }
        }
        let x = sin_u1 * sinsigma - cos_u1 * cossigma * cosalpha1;
        let phi2 = java_math::atan2(
            sin_u1 * cossigma + cos_u1 * sinsigma * cosalpha1,
            (1.0 - self.scaled_flattening) * java_math::sqrt(sinalpha * sinalpha + x * x),
        );
        let lambda = java_math::atan2(
            sinsigma * sinalpha1,
            cos_u1 * cossigma - sin_u1 * sinsigma * cosalpha1,
        );
        let c = self.scaled_flattening / 16.0
            * cos_sqalpha
            * (4.0 + self.scaled_flattening * (4.0 - 3.0 * cos_sqalpha));
        let l = lambda
            - (1.0 - c)
                * self.scaled_flattening
                * sinalpha
                * (sigma
                    + c * sinsigma
                        * (cos2sigma_m + c * cossigma * (-1.0 + 2.0 * cos2sigma_m * cos2sigma_m)));
        let lambda2 = (lon + l + 3.0 * PI) % (2.0 * PI) - PI;
        GeoPoint::new_lat_lon(self, phi2, lambda2)
    }
}

impl SerializableObject for PlanetModel {
    fn write(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.a)?;
        so::write_double(output, self.b)
    }
    fn class_name(&self) -> &'static str {
        "org.apache.lucene.spatial3d.geom.PlanetModel"
    }
    fn java_equals(&self, o: &dyn Any) -> bool {
        o.downcast_ref::<PlanetModel>()
            .is_some_and(|other| self == other)
    }
    /// Equivalent to `PlanetModel.hashCode()`: `Double.hashCode(a) +
    /// Double.hashCode(b)`.
    fn java_hash_code(&self) -> i32 {
        dhash(self.a).wrapping_add(dhash(self.b))
    }
}

impl PartialEq for PlanetModel {
    /// `PlanetModel.equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.a == other.a && self.b == other.b
    }
}

impl fmt::Debug for PlanetModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for PlanetModel {
    /// `PlanetModel.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == **SPHERE {
            f.write_str("PlanetModel.SPHERE")
        } else if *self == **WGS84 {
            f.write_str("PlanetModel.WGS84")
        } else if *self == **CLARKE_1866 {
            f.write_str("PlanetModel.CLARKE_1866")
        } else {
            write!(
                f,
                "PlanetModel(xyScaling={} zScaling={})",
                d2s(self.a),
                d2s(self.b)
            )
        }
    }
}

/// Utility class for encoding / decoding from lat/lon (decimal degrees) into
/// sortable doc value numerics (integers).
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.PlanetModel.DocValueEncoder`.
/// Java keeps a reference to its planet model and reads its six coordinate
/// limits; the port copies the limits it reads.
#[derive(Clone, Debug)]
pub struct DocValueEncoder {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
    min_z: f64,
    max_z: f64,
    inverse_x_factor: f64,
    inverse_y_factor: f64,
    inverse_z_factor: f64,
    x_factor: f64,
    y_factor: f64,
    z_factor: f64,
    x_step: f64,
    y_step: f64,
    z_step: f64,
}

impl DocValueEncoder {
    /// `1.0 / (double) 0x1FFFFF`.
    const INVERSE_MAXIMUM_VALUE: f64 = 1.0 / (0x1FFFFF as f64);
    /// Step amount for rounding.
    const STEP_FUDGE: f64 = 10.0;

    fn new_placeholder() -> Self {
        Self {
            min_x: 0.0,
            max_x: 0.0,
            min_y: 0.0,
            max_y: 0.0,
            min_z: 0.0,
            max_z: 0.0,
            inverse_x_factor: 0.0,
            inverse_y_factor: 0.0,
            inverse_z_factor: 0.0,
            x_factor: 0.0,
            y_factor: 0.0,
            z_factor: 0.0,
            x_step: 0.0,
            y_step: 0.0,
            z_step: 0.0,
        }
    }

    fn new(planet_model: &PlanetModel) -> Self {
        let inverse_x_factor = (planet_model.get_maximum_x_value()
            - planet_model.get_minimum_x_value())
            * Self::INVERSE_MAXIMUM_VALUE;
        let inverse_y_factor = (planet_model.get_maximum_y_value()
            - planet_model.get_minimum_y_value())
            * Self::INVERSE_MAXIMUM_VALUE;
        let inverse_z_factor = (planet_model.get_maximum_z_value()
            - planet_model.get_minimum_z_value())
            * Self::INVERSE_MAXIMUM_VALUE;
        Self {
            min_x: planet_model.get_minimum_x_value(),
            max_x: planet_model.get_maximum_x_value(),
            min_y: planet_model.get_minimum_y_value(),
            max_y: planet_model.get_maximum_y_value(),
            min_z: planet_model.get_minimum_z_value(),
            max_z: planet_model.get_maximum_z_value(),
            inverse_x_factor,
            inverse_y_factor,
            inverse_z_factor,
            x_factor: 1.0 / inverse_x_factor,
            y_factor: 1.0 / inverse_y_factor,
            z_factor: 1.0 / inverse_z_factor,
            x_step: inverse_x_factor * Self::STEP_FUDGE,
            y_step: inverse_y_factor * Self::STEP_FUDGE,
            z_step: inverse_z_factor * Self::STEP_FUDGE,
        }
    }

    /// `encodePoint(GeoPoint)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a coordinate is out of the model.
    pub fn encode_point(&self, point: &GeoPoint) -> Result<i64> {
        self.encode_point_xyz(point.x, point.y, point.z)
    }

    /// `encodePoint(double, double, double)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a coordinate is out of the model.
    pub fn encode_point_xyz(&self, x: f64, y: f64, z: f64) -> Result<i64> {
        let x_encoded = self.encode_x(x)?;
        let y_encoded = self.encode_y(y)?;
        let z_encoded = self.encode_z(z)?;
        Ok((((x_encoded & 0x1FFFFF) as i64) << 42)
            | (((y_encoded & 0x1FFFFF) as i64) << 21)
            | ((z_encoded & 0x1FFFFF) as i64))
    }

    /// `decodePoint(long)`.
    pub fn decode_point(&self, doc_value: i64) -> GeoPoint {
        GeoPoint::new(
            self.decode_x(((doc_value >> 42) as i32) & 0x1FFFFF),
            self.decode_y(((doc_value >> 21) as i32) & 0x1FFFFF),
            self.decode_z((doc_value as i32) & 0x1FFFFF),
        )
    }

    /// `decodeXValue(long)`.
    pub fn decode_x_value(&self, doc_value: i64) -> f64 {
        self.decode_x(((doc_value >> 42) as i32) & 0x1FFFFF)
    }

    /// `decodeYValue(long)`.
    pub fn decode_y_value(&self, doc_value: i64) -> f64 {
        self.decode_y(((doc_value >> 21) as i32) & 0x1FFFFF)
    }

    /// `decodeZValue(long)`.
    pub fn decode_z_value(&self, doc_value: i64) -> f64 {
        self.decode_z((doc_value as i32) & 0x1FFFFF)
    }

    /// `roundDownX(double)`.
    pub fn round_down_x(&self, start_value: f64) -> f64 {
        start_value - self.x_step
    }

    /// `roundUpX(double)`.
    pub fn round_up_x(&self, start_value: f64) -> f64 {
        start_value + self.x_step
    }

    /// `roundDownY(double)`.
    pub fn round_down_y(&self, start_value: f64) -> f64 {
        start_value - self.y_step
    }

    /// `roundUpY(double)`.
    pub fn round_up_y(&self, start_value: f64) -> f64 {
        start_value + self.y_step
    }

    /// `roundDownZ(double)`.
    pub fn round_down_z(&self, start_value: f64) -> f64 {
        start_value - self.z_step
    }

    /// `roundUpZ(double)`.
    pub fn round_up_z(&self, start_value: f64) -> f64 {
        start_value + self.z_step
    }

    fn encode_x(&self, x: f64) -> Result<i32> {
        if x > self.max_x {
            return Err(iae("x value exceeds planet model maximum"));
        } else if x < self.min_x {
            return Err(iae("x value less than planet model minimum"));
        }
        Ok(((x - self.min_x) * self.x_factor + 0.5).floor() as i32)
    }

    fn decode_x(&self, x: i32) -> f64 {
        x as f64 * self.inverse_x_factor + self.min_x
    }

    fn encode_y(&self, y: f64) -> Result<i32> {
        if y > self.max_y {
            return Err(iae("y value exceeds planet model maximum"));
        } else if y < self.min_y {
            return Err(iae("y value less than planet model minimum"));
        }
        Ok(((y - self.min_y) * self.y_factor + 0.5).floor() as i32)
    }

    fn decode_y(&self, y: i32) -> f64 {
        y as f64 * self.inverse_y_factor + self.min_y
    }

    fn encode_z(&self, z: f64) -> Result<i32> {
        if z > self.max_z {
            return Err(iae("z value exceeds planet model maximum"));
        } else if z < self.min_z {
            return Err(iae("z value less than planet model minimum"));
        }
        Ok(((z - self.min_z) * self.z_factor + 0.5).floor() as i32)
    }

    fn decode_z(&self, z: i32) -> f64 {
        z as f64 * self.inverse_z_factor + self.min_z
    }
}
