/*
 * Copyright 2005 Google Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Skeletons of the types declared in `S2Projections.java`, of the Java
//! package `com.google.common.geometry`.

/// Java: `public final strictfp class`.
///
/// Skeleton of `com.google.common.geometry.S2Projections`; ported by rmp task #622.
pub struct S2Projections;

/// Java: `public enum`.
///
/// Skeleton of `com.google.common.geometry.S2Projections.Projections`; ported by rmp task #622.
pub enum Projections {
    /// Java constant `S2_LINEAR_PROJECTION`.
    S2LinearProjection,
    /// Java constant `S2_TAN_PROJECTION`.
    S2TanProjection,
    /// Java constant `S2_QUADRATIC_PROJECTION`.
    S2QuadraticProjection,
}
