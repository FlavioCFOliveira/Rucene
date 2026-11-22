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


//! The types declared in `Tools.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

/// Static methods globally useful for 3d geometric work.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.Tools`.
pub struct Tools;

impl Tools {
    /// `safeAcos(double)`: `acos` of the value clamped to `[-1, 1]`.
    pub fn safe_acos(mut value: f64) -> f64 {
        if value > 1.0 {
            value = 1.0;
        } else if value < -1.0 {
            value = -1.0;
        }
        value.acos()
    }
}
