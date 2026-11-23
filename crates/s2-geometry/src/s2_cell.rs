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

//! The types declared in `S2Cell.java`, of the Java package
//! `com.google.common.geometry`.

use rucene::util::java_math;
use std::cell::Cell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::mutable_integer::MutableInteger;
use crate::r1_interval::R1Interval;
use crate::r2_vector::R2Vector;
use crate::s1_interval::S1Interval;
use crate::s2::S2;
use crate::s2_cap::S2Cap;
use crate::s2_cell_id::S2CellId;
use crate::s2_lat_lng::S2LatLng;
use crate::s2_lat_lng_rect::S2LatLngRect;
use crate::s2_point::S2Point;
use crate::s2_projections::S2Projections;
use crate::s2_region::S2Region;

const MAX_CELL_SIZE: i32 = 1 << S2CellId::MAX_LEVEL;
const MAX_ERROR: f64 = 1.0 / (1u64 << 51) as f64;

/// `POLE_MIN_LAT`: `Math.asin(Math.sqrt(1.0 / 3.0)) - MAX_ERROR`.
fn pole_min_lat() -> f64 {
    java_math::asin(java_math::sqrt(1.0f64 / 3.0)) - MAX_ERROR
}

/// One row of the Java `double[][] uv` array.
///
/// Java's `clone()` copies `uv` with `uv.clone()`, a shallow copy that shares
/// the two row arrays between the original and the clone; a row is therefore
/// a shared, mutable cell here, so that writes through either cell stay
/// visible to the other, as in Java.
type UvRow = Rc<Cell<[f64; 2]>>;

/// Equivalent to `com.google.common.geometry.S2Cell` (`public final strictfp
/// class S2Cell implements S2Region`): an [`S2CellId`] with its geometry
/// (face, level, orientation and `(u,v)` bounds) precomputed.
#[derive(Debug)]
pub struct S2Cell {
    face: i8,
    level: i8,
    orientation: i8,
    /// `cellId`; `None` stands for the `null` id that `clone()` leaves.
    cell_id: Option<S2CellId>,
    uv: [UvRow; 2],
}

impl S2Cell {
    /// `S2Cell()` (package-private): every field at its Java default.
    pub(crate) fn empty() -> Self {
        Self {
            face: 0,
            level: 0,
            orientation: 0,
            cell_id: None,
            uv: [Rc::new(Cell::new([0.0; 2])), Rc::new(Cell::new([0.0; 2]))],
        }
    }

    /// `S2Cell(S2CellId)`.
    pub fn new(id: S2CellId) -> Self {
        let mut cell = Self::empty();
        cell.init(id);
        cell
    }

    /// `fromFacePosLevel(int, byte, int)`.
    pub fn from_face_pos_level(face: i32, pos: i8, level: i32) -> Self {
        S2Cell::new(S2CellId::from_face_pos_level(face, i64::from(pos), level))
    }

    /// `S2Cell(S2Point)`.
    pub fn from_point(p: &S2Point) -> Self {
        Self::new(S2CellId::from_point(p))
    }

    /// `S2Cell(S2LatLng)`.
    pub fn from_lat_lng(ll: &S2LatLng) -> Self {
        Self::new(S2CellId::from_lat_lng(ll))
    }

    fn uv(&self, d: usize, k: usize) -> f64 {
        self.uv[d].get()[k]
    }

    fn set_uv(&self, d: usize, k: usize, value: f64) {
        let mut row = self.uv[d].get();
        row[k] = value;
        self.uv[d].set(row);
    }

    fn cell_id_or_npe(&self) -> S2CellId {
        self.cell_id
            .expect("java.lang.NullPointerException: cellId is null")
    }

    /// `id()`; `None` for the `null` id of a `clone()`.
    pub fn id(&self) -> Option<S2CellId> {
        self.cell_id
    }

    /// `face()`.
    pub fn face(&self) -> i32 {
        i32::from(self.face)
    }

    /// `level()`.
    pub fn level(&self) -> i8 {
        self.level
    }

    /// `orientation()`.
    pub fn orientation(&self) -> i8 {
        self.orientation
    }

    /// `isLeaf()`.
    pub fn is_leaf(&self) -> bool {
        i32::from(self.level) == S2CellId::MAX_LEVEL
    }

    /// `getVertex(int)`.
    pub fn get_vertex(&self, k: i32) -> S2Point {
        S2Point::normalize(&self.get_vertex_raw(k))
    }

    /// `getVertexRaw(int)`.
    pub fn get_vertex_raw(&self, k: i32) -> S2Point {
        S2Projections::face_uv_to_xyz(
            i32::from(self.face),
            self.uv(0, ((k >> 1) ^ (k & 1)) as usize),
            self.uv(1, (k >> 1) as usize),
        )
    }

    /// `getEdge(int)`.
    pub fn get_edge(&self, k: i32) -> S2Point {
        S2Point::normalize(&self.get_edge_raw(k))
    }

    /// `getEdgeRaw(int)`.
    pub fn get_edge_raw(&self, k: i32) -> S2Point {
        let face = i32::from(self.face);
        match k {
            0 => S2Projections::get_v_norm(face, self.uv(1, 0)), // South
            1 => S2Projections::get_u_norm(face, self.uv(0, 1)), // East
            2 => S2Point::neg(&S2Projections::get_v_norm(face, self.uv(1, 1))), // North
            _ => S2Point::neg(&S2Projections::get_u_norm(face, self.uv(0, 0))), // West
        }
    }

    /// `subdivide(S2Cell[] children)`: fills the 4 children; `false` for a
    /// leaf.
    pub fn subdivide(&self, children: &mut [S2Cell]) -> bool {
        if self.cell_id_or_npe().is_leaf() {
            return false;
        }
        let uv_mid = self.get_center_uv();
        let mut id = self.cell_id_or_npe().child_begin();
        for pos in 0..4 {
            let child = &mut children[pos as usize];
            child.face = self.face;
            child.level = self.level.wrapping_add(1);
            child.orientation = (i32::from(self.orientation)
                ^ S2::pos_to_orientation(pos).expect("position in [0,3]"))
                as i8;
            child.cell_id = Some(id);
            let ij = S2::pos_to_ij(i32::from(self.orientation), pos)
                .expect("orientation and position in [0,3]");
            for d in 0..2usize {
                let m = (1 - ((ij >> (1 - d as i32)) & 1)) as usize;
                let mid = if d == 0 { uv_mid.x() } else { uv_mid.y() };
                child.set_uv(d, m, mid);
                child.set_uv(d, 1 - m, self.uv(d, 1 - m));
            }
            id = id.next();
        }
        true
    }

    /// `getCenter()`.
    pub fn get_center(&self) -> S2Point {
        S2Point::normalize(&self.get_center_raw())
    }

    /// `getCenterRaw()`.
    pub fn get_center_raw(&self) -> S2Point {
        self.cell_id_or_npe().to_point_raw()
    }

    /// `getCenterUV()`.
    pub fn get_center_uv(&self) -> R2Vector {
        let mut i = MutableInteger::new(0);
        let mut j = MutableInteger::new(0);
        self.cell_id_or_npe()
            .to_face_ij_orientation(&mut i, &mut j, None);
        let cell_size = 1 << (S2CellId::MAX_LEVEL - i32::from(self.level));
        let si = (i.int_value() & -cell_size) * 2 + cell_size - MAX_CELL_SIZE;
        let x = S2Projections::st_to_uv((1.0 / f64::from(MAX_CELL_SIZE)) * f64::from(si));
        let sj = (j.int_value() & -cell_size) * 2 + cell_size - MAX_CELL_SIZE;
        let y = S2Projections::st_to_uv((1.0 / f64::from(MAX_CELL_SIZE)) * f64::from(sj));
        R2Vector::new(x, y)
    }

    /// `averageArea(int level)`.
    pub fn average_area_at(level: i32) -> f64 {
        S2Projections::AVG_AREA.get_value(level)
    }

    /// `averageArea()`.
    pub fn average_area(&self) -> f64 {
        Self::average_area_at(i32::from(self.level))
    }

    /// `approxArea()`.
    pub fn approx_area(&self) -> f64 {
        if self.level < 2 {
            return Self::average_area_at(i32::from(self.level));
        }
        let flat_area = 0.5
            * S2Point::cross_prod(
                &S2Point::sub(&self.get_vertex(2), &self.get_vertex(0)),
                &S2Point::sub(&self.get_vertex(3), &self.get_vertex(1)),
            )
            .norm();
        flat_area * 2.0 / (1.0 + java_math::sqrt(1.0 - (S2::M_1_PI * flat_area).min(1.0)))
    }

    /// `exactArea()`.
    pub fn exact_area(&self) -> f64 {
        let v0 = self.get_vertex(0);
        let v1 = self.get_vertex(1);
        let v2 = self.get_vertex(2);
        let v3 = self.get_vertex(3);
        S2::area(&v0, &v1, &v2) + S2::area(&v0, &v2, &v3)
    }

    /// `clone()`: copies face, level and orientation, shares the `uv` rows
    /// (Java `uv.clone()` is shallow) and leaves the cell id `null`, as the
    /// Java method does.
    pub fn clone_region(&self) -> S2Cell {
        S2Cell {
            face: self.face,
            level: self.level,
            orientation: self.orientation,
            cell_id: None,
            uv: [Rc::clone(&self.uv[0]), Rc::clone(&self.uv[1])],
        }
    }

    /// `contains(S2Point)`.
    pub fn contains_point(&self, p: &S2Point) -> bool {
        let Some(uv_point) = S2Projections::face_xyz_to_uv(i32::from(self.face), p) else {
            return false;
        };
        uv_point.x() >= self.uv(0, 0)
            && uv_point.x() <= self.uv(0, 1)
            && uv_point.y() >= self.uv(1, 0)
            && uv_point.y() <= self.uv(1, 1)
    }

    fn init(&mut self, id: S2CellId) {
        self.cell_id = Some(id);
        let mut ij = [MutableInteger::new(0), MutableInteger::new(0)];
        let mut m_orientation = MutableInteger::new(0);
        let [ij0, ij1] = &mut ij;
        self.face = id.to_face_ij_orientation(ij0, ij1, Some(&mut m_orientation)) as i8;
        self.orientation = m_orientation.int_value() as i8; // Compress int to a byte.
        self.level = id.level() as i8;
        let cell_size = 1 << (S2CellId::MAX_LEVEL - i32::from(self.level));
        for (d, ij_d) in ij.iter().enumerate() {
            // Java `int` arithmetic: at level 0 `cellSize * 2` is 2^31, which
            // wraps (and `sijHi` wraps back to the face's upper bound).
            let sij_lo = (ij_d.int_value() & -cell_size)
                .wrapping_mul(2)
                .wrapping_sub(MAX_CELL_SIZE);
            let sij_hi = sij_lo.wrapping_add(cell_size.wrapping_mul(2));
            self.set_uv(
                d,
                0,
                S2Projections::st_to_uv((1.0 / f64::from(MAX_CELL_SIZE)) * f64::from(sij_lo)),
            );
            self.set_uv(
                d,
                1,
                S2Projections::st_to_uv((1.0 / f64::from(MAX_CELL_SIZE)) * f64::from(sij_hi)),
            );
        }
    }

    fn get_latitude(&self, i: usize, j: usize) -> f64 {
        let p = S2Projections::face_uv_to_xyz(i32::from(self.face), self.uv(0, i), self.uv(1, j));
        java_math::atan2(p.z, java_math::sqrt(p.x * p.x + p.y * p.y))
    }

    fn get_longitude(&self, i: usize, j: usize) -> f64 {
        let p = S2Projections::face_uv_to_xyz(i32::from(self.face), self.uv(0, i), self.uv(1, j));
        java_math::atan2(p.y, p.x)
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut value: i32 = 17;
        value = 37i32
            .wrapping_mul(
                37i32
                    .wrapping_mul(37i32.wrapping_mul(value).wrapping_add(i32::from(self.face)))
                    .wrapping_add(i32::from(self.orientation)),
            )
            .wrapping_add(i32::from(self.level));
        37i32
            .wrapping_mul(value)
            .wrapping_add(self.cell_id_or_npe().hash_code())
    }
}

impl S2Region for S2Cell {
    /// `getCapBound()`.
    fn get_cap_bound(&self) -> S2Cap {
        let u = 0.5 * (self.uv(0, 0) + self.uv(0, 1));
        let v = 0.5 * (self.uv(1, 0) + self.uv(1, 1));
        let mut cap = S2Cap::from_axis_height(
            S2Point::normalize(&S2Projections::face_uv_to_xyz(i32::from(self.face), u, v)),
            0.0,
        );
        for k in 0..4 {
            cap = cap.add_point(&self.get_vertex(k));
        }
        cap
    }

    /// `getRectBound()`.
    fn get_rect_bound(&self) -> S2LatLngRect {
        if self.level > 0 {
            let u = self.uv(0, 0) + self.uv(0, 1);
            let v = self.uv(1, 0) + self.uv(1, 1);
            let face = i32::from(self.face);
            let i: usize = if S2Projections::get_u_axis(face).z == 0.0 {
                usize::from(u < 0.0)
            } else {
                usize::from(u > 0.0)
            };
            let j: usize = if S2Projections::get_v_axis(face).z == 0.0 {
                usize::from(v < 0.0)
            } else {
                usize::from(v > 0.0)
            };
            let mut lat = R1Interval::from_point_pair(
                self.get_latitude(i, j),
                self.get_latitude(1 - i, 1 - j),
            );
            lat = lat
                .expanded(MAX_ERROR)
                .intersection(&S2LatLngRect::full_lat());
            if lat.lo() == -S2::M_PI_2 || lat.hi() == S2::M_PI_2 {
                return S2LatLngRect::new(lat, S1Interval::full());
            }
            let lng = S1Interval::from_point_pair(
                self.get_longitude(i, 1 - j),
                self.get_longitude(1 - i, j),
            );
            return S2LatLngRect::new(lat, lng.expanded(MAX_ERROR));
        }
        match self.face {
            0 => S2LatLngRect::new(
                R1Interval::new(-S2::M_PI_4, S2::M_PI_4),
                S1Interval::new(-S2::M_PI_4, S2::M_PI_4),
            ),
            1 => S2LatLngRect::new(
                R1Interval::new(-S2::M_PI_4, S2::M_PI_4),
                S1Interval::new(S2::M_PI_4, 3.0 * S2::M_PI_4),
            ),
            2 => S2LatLngRect::new(
                R1Interval::new(pole_min_lat(), S2::M_PI_2),
                S1Interval::new(-S2::M_PI, S2::M_PI),
            ),
            3 => S2LatLngRect::new(
                R1Interval::new(-S2::M_PI_4, S2::M_PI_4),
                S1Interval::new(3.0 * S2::M_PI_4, -3.0 * S2::M_PI_4),
            ),
            4 => S2LatLngRect::new(
                R1Interval::new(-S2::M_PI_4, S2::M_PI_4),
                S1Interval::new(-3.0 * S2::M_PI_4, -S2::M_PI_4),
            ),
            _ => S2LatLngRect::new(
                R1Interval::new(-S2::M_PI_2, -pole_min_lat()),
                S1Interval::new(-S2::M_PI, S2::M_PI),
            ),
        }
    }

    /// `mayIntersect(S2Cell)`.
    fn may_intersect(&self, cell: &S2Cell) -> bool {
        self.cell_id_or_npe().intersects(&cell.cell_id_or_npe())
    }

    /// `contains(S2Cell)`.
    fn contains_cell(&self, cell: &S2Cell) -> bool {
        self.cell_id_or_npe().contains(&cell.cell_id_or_npe())
    }
}

impl PartialEq for S2Cell {
    /// `equals(Object)`.
    fn eq(&self, that: &Self) -> bool {
        self.face == that.face
            && self.level == that.level
            && self.orientation == that.orientation
            && Some(self.cell_id_or_npe()) == that.cell_id
    }
}

impl Hash for S2Cell {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for S2Cell {
    /// `toString()`: `[face, level, orientation, cellId]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = match self.cell_id {
            Some(id) => id.to_string(),
            None => "null".to_string(),
        };
        write!(
            f,
            "[{}, {}, {}, {}]",
            self.face, self.level, self.orientation, id
        )
    }
}
