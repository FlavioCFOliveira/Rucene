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

//! The types declared in `GeohashUtils.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::sync::{Arc, LazyLock};

use rucene::error::Result;

use crate::context::spatial_context::SpatialContext;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;

const BASE_32: [char; 32] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'j', 'k',
    'm', 'n', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
]; //note: this is sorted

/// `BASE_32_IDX`: sparse array of indexes from '0' to 'z'.
static BASE_32_IDX: LazyLock<Vec<i32>> = LazyLock::new(|| {
    let mut idx = vec![-500; (BASE_32[BASE_32.len() - 1] as usize) - (BASE_32[0] as usize) + 1];
    debug_assert!(idx.len() < 100); //reasonable length
    for (i, c) in BASE_32.iter().enumerate() {
        idx[(*c as usize) - (BASE_32[0] as usize)] = i as i32;
    }
    idx
});

const BITS: [i32; 5] = [16, 8, 4, 2, 1];

/// `hashLenToLatHeight` and `hashLenToLonWidth`.
static HASH_LEN_TO: LazyLock<(Vec<f64>, Vec<f64>)> = LazyLock::new(|| {
    let max = GeohashUtils::MAX_PRECISION as usize;
    let mut lat_height = vec![0.0; max + 1];
    let mut lon_width = vec![0.0; max + 1];
    lat_height[0] = 90.0 * 2.0;
    lon_width[0] = 180.0 * 2.0;
    let mut even = false;
    for i in 1..=max {
        lat_height[i] = lat_height[i - 1] / if even { 8.0 } else { 4.0 };
        lon_width[i] = lon_width[i - 1] / if even { 4.0 } else { 8.0 };
        even = !even;
    }
    (lat_height, lon_width)
});

/// Equivalent to `org.locationtech.spatial4j.io.GeohashUtils` (`public
/// class`): geohash encoding and decoding.
pub struct GeohashUtils;

impl GeohashUtils {
    /// `MAX_PRECISION`.
    pub const MAX_PRECISION: i32 = 24; //DWS: I forget what level results in needless more precision but it's about this

    /// `encodeLatLon(double latitude, double longitude)`: 12 characters.
    pub fn encode_lat_lon(latitude: f64, longitude: f64) -> String {
        Self::encode_lat_lon_with(latitude, longitude, 12)
    }

    /// `encodeLatLon(double latitude, double longitude, int precision)`.
    pub fn encode_lat_lon_with(latitude: f64, longitude: f64, precision: i32) -> String {
        let mut lat_interval = [-90.0, 90.0];
        let mut lng_interval = [-180.0, 180.0];
        let mut geohash = String::with_capacity(precision.max(0) as usize);
        let mut is_even = true;
        let mut bit = 0usize;
        let mut ch = 0i32;
        while (geohash.len() as i32) < precision {
            let mid;
            if is_even {
                mid = (lng_interval[0] + lng_interval[1]) / 2.0;
                if longitude > mid {
                    ch |= BITS[bit];
                    lng_interval[0] = mid;
                } else {
                    lng_interval[1] = mid;
                }
            } else {
                mid = (lat_interval[0] + lat_interval[1]) / 2.0;
                if latitude > mid {
                    ch |= BITS[bit];
                    lat_interval[0] = mid;
                } else {
                    lat_interval[1] = mid;
                }
            }
            is_even = !is_even;
            if bit < 4 {
                bit += 1;
            } else {
                geohash.push(BASE_32[ch as usize]);
                bit = 0;
                ch = 0;
            }
        }
        geohash
    }

    /// `decode(String geohash, SpatialContext ctx)`: the center of the
    /// geohash's rectangle.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makePoint`.
    pub fn decode(geohash: &str, ctx: &SpatialContext) -> Result<Arc<dyn Point>> {
        let rect = Self::decode_boundary(geohash, ctx)?;
        let latitude = (rect.get_min_y() + rect.get_max_y()) / 2.0;
        let longitude = (rect.get_min_x() + rect.get_max_x()) / 2.0;
        ctx.make_point(longitude, latitude)
    }

    /// `decodeBoundary(String geohash, SpatialContext ctx)`.
    ///
    /// Java indexes `BASE_32_IDX` with the character's offset from `'0'`
    /// without a check (`//TODO check successful?`), so a character outside
    /// `'0'..='z'` throws `ArrayIndexOutOfBoundsException`; this panics with
    /// that text.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makeRectangle`.
    pub fn decode_boundary(geohash: &str, ctx: &SpatialContext) -> Result<Arc<dyn Rectangle>> {
        let (mut min_y, mut max_y, mut min_x, mut max_x) = (-90.0, 90.0, -180.0, 180.0);
        let mut is_even = true;
        for unit in geohash.encode_utf16() {
            let mut c = i32::from(unit);
            if (i32::from(b'A')..=i32::from(b'Z')).contains(&c) {
                c -= i32::from(b'A') - i32::from(b'a');
            }
            let offset = c - BASE_32[0] as i32;
            let cd = usize::try_from(offset)
                .ok()
                .and_then(|o| BASE_32_IDX.get(o).copied())
                .unwrap_or_else(|| panic!("java.lang.ArrayIndexOutOfBoundsException: Index {offset} out of bounds for length {}", BASE_32_IDX.len()));
            for mask in BITS {
                if is_even {
                    if (cd & mask) != 0 {
                        min_x = (min_x + max_x) / 2.0;
                    } else {
                        max_x = (min_x + max_x) / 2.0;
                    }
                } else if (cd & mask) != 0 {
                    min_y = (min_y + max_y) / 2.0;
                } else {
                    max_y = (min_y + max_y) / 2.0;
                }
                is_even = !is_even;
            }
        }
        ctx.make_rectangle(min_x, max_x, min_y, max_y)
    }

    /// `getSubGeohashes(String baseGeohash)`: the 32 geohashes one level
    /// below, sorted.
    pub fn get_sub_geohashes(base_geohash: &str) -> Vec<String> {
        BASE_32
            .iter()
            .map(|c| format!("{base_geohash}{c}"))
            .collect()
    }

    /// `lookupDegreesSizeForHashLen(int hashLen)`: `[latHeight, lonWidth]`.
    pub fn lookup_degrees_size_for_hash_len(hash_len: usize) -> [f64; 2] {
        [HASH_LEN_TO.0[hash_len], HASH_LEN_TO.1[hash_len]]
    }

    /// `lookupHashLenForWidthHeight(double lonErr, double latErr)`.
    pub fn lookup_hash_len_for_width_height(lon_err: f64, lat_err: f64) -> i32 {
        //loop through hash length arrays from beginning till we find one.
        for len in 1..Self::MAX_PRECISION as usize {
            let lat_height = HASH_LEN_TO.0[len];
            let lon_width = HASH_LEN_TO.1[len];
            if lat_height < lat_err && lon_width < lon_err {
                return len as i32;
            }
        }
        Self::MAX_PRECISION
    }
}
