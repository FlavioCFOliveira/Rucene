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

//! The types declared in `S2Region.java`, of the Java package
//! `com.google.common.geometry`.

use crate::s2_cap::S2Cap;
use crate::s2_cell::S2Cell;
use crate::s2_lat_lng_rect::S2LatLngRect;

/// Equivalent to `com.google.common.geometry.S2Region` (`public interface`):
/// a two-dimensional region over the unit sphere, restricted to the methods
/// that approximate it.
pub trait S2Region {
    /// `getCapBound()`: a bounding spherical cap.
    fn get_cap_bound(&self) -> S2Cap;

    /// `getRectBound()`: a bounding latitude-longitude rectangle.
    fn get_rect_bound(&self) -> S2LatLngRect;

    /// `contains(S2Cell)`: `true` only when the region surely contains the
    /// cell.
    fn contains_cell(&self, cell: &S2Cell) -> bool;

    /// `mayIntersect(S2Cell)`: `false` only when the region surely does not
    /// intersect the cell.
    fn may_intersect(&self, cell: &S2Cell) -> bool;
}
