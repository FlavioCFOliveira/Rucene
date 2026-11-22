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


//! The types declared in `Membership.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use super::vector::Vector;

/// Implemented by objects that may be able to determine membership.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.Membership`.
pub trait Membership {
    /// `isWithin(Vector)`.
    fn is_within(&self, point: &Vector) -> bool {
        self.is_within_xyz(point.x, point.y, point.z)
    }

    /// `isWithin(double, double, double)`.
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool;
}
