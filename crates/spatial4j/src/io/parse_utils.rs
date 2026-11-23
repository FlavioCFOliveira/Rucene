/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *    http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The types declared in `ParseUtils.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use rucene::error::Result;
use rucene::util::java_fmt::double_to_string;

use crate::exception::invalid_shape_exception::InvalidShapeException;
use crate::jdk::parse_double;

/// Equivalent to `org.locationtech.spatial4j.io.ParseUtils` (`@Deprecated
/// public class`): parsing of comma-separated coordinates.
///
/// Java indexes the string by `char` (UTF-16 unit) and trims spaces only;
/// the separators and the trimmed characters are ASCII, so byte offsets of
/// the UTF-8 string find the same boundaries here.
pub struct ParseUtils;

impl ParseUtils {
    /// `parsePoint(String[] out, String externalVal, int dimension)`.
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` when the value does not hold `dimension`
    /// values.
    pub fn parse_point(
        out: Option<Vec<String>>,
        external_val: &str,
        dimension: usize,
    ) -> Result<Vec<String>> {
        let mut out = match out {
            Some(o) if o.len() == dimension => o,
            _ => vec![String::new(); dimension],
        };
        let bytes = external_val.as_bytes();
        let mut idx = external_val.find(',').map_or(-1, |i| i as isize);
        let mut end = idx;
        let mut start: isize = 0;
        let mut i = 0usize;
        if idx == -1 && dimension == 1 && !external_val.is_empty() {
            //we have a single point, dimension better be 1
            out[0] = external_val.trim_matches(|c: char| c <= ' ').to_string();
            i = 1;
        } else if idx > 0 {
            //if it is zero, that is an error
            //Parse out a comma separated list of point values, as in: 73.5,89.2,7773.4
            while i < dimension {
                while start < end && bytes[start as usize] == b' ' {
                    start += 1;
                }
                while end > start && bytes[(end - 1) as usize] == b' ' {
                    end -= 1;
                }
                if start == end {
                    break;
                }
                out[i] = external_val[start as usize..end as usize].to_string();
                start = idx + 1;
                end = external_val[start as usize..]
                    .find(',')
                    .map_or(-1, |p| p as isize + start);
                idx = end;
                if end == -1 {
                    end = external_val.len() as isize;
                }
                i += 1;
            }
        }
        if i != dimension {
            return Err(InvalidShapeException::error(format!(
                "incompatible dimension ({dimension}) and values ({external_val}).  Only {i} values specified"
            )));
        }
        Ok(out)
    }

    /// `parsePointDouble(double[] out, String externalVal, int dimension)`.
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` when the value does not hold `dimension`
    /// values; `NumberFormatException` for a value that is not a number.
    pub fn parse_point_double(
        out: Option<Vec<f64>>,
        external_val: &str,
        dimension: usize,
    ) -> Result<Vec<f64>> {
        let mut out = match out {
            Some(o) if o.len() == dimension => o,
            _ => vec![0.0; dimension],
        };
        let bytes = external_val.as_bytes();
        let mut idx = external_val.find(',').map_or(-1, |i| i as isize);
        let mut end = idx;
        let mut start: isize = 0;
        let mut i = 0usize;
        if idx == -1 && dimension == 1 && !external_val.is_empty() {
            //we have a single point, dimension better be 1
            out[0] = parse_double(external_val.trim_matches(|c: char| c <= ' '))?;
            i = 1;
        } else if idx > 0 {
            //if it is zero, that is an error
            //Parse out a comma separated list of point values, as in: 73.5,89.2,7773.4
            while i < dimension {
                //TODO: abstract common code with other parsePoint
                while start < end && bytes[start as usize] == b' ' {
                    start += 1;
                }
                while end > start && bytes[(end - 1) as usize] == b' ' {
                    end -= 1;
                }
                if start == end {
                    break;
                }
                out[i] = parse_double(&external_val[start as usize..end as usize])?;
                start = idx + 1;
                end = external_val[start as usize..]
                    .find(',')
                    .map_or(-1, |p| p as isize + start);
                idx = end;
                if end == -1 {
                    end = external_val.len() as isize;
                }
                i += 1;
            }
        }
        if i != dimension {
            return Err(InvalidShapeException::error(format!(
                "incompatible dimension ({dimension}) and values ({external_val}).  Only {i} values specified"
            )));
        }
        Ok(out)
    }

    /// `parseLatitudeLongitude(String latLonStr)`.
    ///
    /// # Errors
    ///
    /// As [`parse_latitude_longitude_into`](Self::parse_latitude_longitude_into).
    pub fn parse_latitude_longitude(lat_lon_str: &str) -> Result<Vec<f64>> {
        Self::parse_latitude_longitude_into(None, lat_lon_str)
    }

    /// `parseLatitudeLongitude(double[] outLatLon, String latLonStr)`.
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for a malformed value or one out of range.
    pub fn parse_latitude_longitude_into(
        out_lat_lon: Option<Vec<f64>>,
        lat_lon_str: &str,
    ) -> Result<Vec<f64>> {
        let out_lat_lon = Self::parse_point_double(out_lat_lon, lat_lon_str, 2)?;
        if out_lat_lon[0] < -90.0 || out_lat_lon[0] > 90.0 {
            return Err(InvalidShapeException::error(format!(
                "Invalid latitude: latitudes are range -90 to 90: provided lat: [{}]",
                double_to_string(out_lat_lon[0])
            )));
        }
        if out_lat_lon[1] < -180.0 || out_lat_lon[1] > 180.0 {
            return Err(InvalidShapeException::error(format!(
                "Invalid longitude: longitudes are range -180 to 180: provided lon: [{}]",
                double_to_string(out_lat_lon[1])
            )));
        }
        Ok(out_lat_lon)
    }
}
