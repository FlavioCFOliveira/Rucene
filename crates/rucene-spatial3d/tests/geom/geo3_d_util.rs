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

//! Port of the test helper `org.apache.lucene.spatial3d.geom.Geo3DUtil`
//! (`lucene/spatial3d/src/test`).

/// `Geo3DUtil` of the `geom` test package: static methods only.
pub enum Geo3DUtil {}

impl Geo3DUtil {
    /// How many radians are in one degree.
    pub const RADIANS_PER_DEGREE: f64 = std::f64::consts::PI / 180.0;

    /// How many degrees in a radian.
    pub const DEGREES_PER_RADIAN: f64 = 180.0 / std::f64::consts::PI;

    /// Converts degress to radians.
    pub fn from_degrees(degrees: f64) -> f64 {
        degrees * Self::RADIANS_PER_DEGREE
    }

    /// Converts radians to degrees.
    pub fn to_degrees(radians: f64) -> f64 {
        radians * Self::DEGREES_PER_RADIAN
    }
}
