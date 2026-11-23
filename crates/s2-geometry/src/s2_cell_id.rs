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

//! The types declared in `S2CellId.java`, of the Java package
//! `com.google.common.geometry`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};
use rucene::util::java_character_digit;

use crate::mutable_integer::MutableInteger;
use crate::s2::{java_round, S2};
use crate::s2_lat_lng::S2LatLng;
use crate::s2_point::S2Point;
use crate::s2_projections::S2Projections;

/// Equivalent to `com.google.common.geometry.S2CellId` (`public final
/// strictfp class S2CellId implements Comparable<S2CellId>`): a 64-bit cell
/// identifier, ordered as an unsigned `long`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct S2CellId {
    id: i64,
}

const LOOKUP_BITS: i32 = 4;
const SWAP_MASK: i32 = 0x01;
const INVERT_MASK: i32 = 0x02;
const LOOKUP_SIZE: usize = 1 << (2 * LOOKUP_BITS + 2);

struct Lookup {
    pos: Vec<i32>,
    ij: Vec<i32>,
}

/// The `LOOKUP_POS` and `LOOKUP_IJ` tables of the Java static initializer.
static LOOKUP: LazyLock<Lookup> = LazyLock::new(|| {
    let mut lookup = Lookup {
        pos: vec![0; LOOKUP_SIZE],
        ij: vec![0; LOOKUP_SIZE],
    };
    init_lookup_cell(&mut lookup, 0, 0, 0, 0, 0, 0);
    init_lookup_cell(&mut lookup, 0, 0, 0, SWAP_MASK, 0, SWAP_MASK);
    init_lookup_cell(&mut lookup, 0, 0, 0, INVERT_MASK, 0, INVERT_MASK);
    init_lookup_cell(
        &mut lookup,
        0,
        0,
        0,
        SWAP_MASK | INVERT_MASK,
        0,
        SWAP_MASK | INVERT_MASK,
    );
    lookup
});

fn init_lookup_cell(
    lookup: &mut Lookup,
    mut level: i32,
    mut i: i32,
    mut j: i32,
    orig_orientation: i32,
    mut pos: i32,
    orientation: i32,
) {
    if level == LOOKUP_BITS {
        let ij = (i << LOOKUP_BITS) + j;
        lookup.pos[((ij << 2) + orig_orientation) as usize] = (pos << 2) + orientation;
        lookup.ij[((pos << 2) + orig_orientation) as usize] = (ij << 2) + orientation;
    } else {
        level += 1;
        i <<= 1;
        j <<= 1;
        pos <<= 2;
        for sub_pos in 0..4 {
            let ij =
                S2::pos_to_ij(orientation, sub_pos).expect("orientation and position in [0,3]");
            let orientation_mask = S2::pos_to_orientation(sub_pos).expect("position in [0,3]");
            init_lookup_cell(
                lookup,
                level,
                i + ((ij as u32) >> 1) as i32,
                j + (ij & 1),
                orig_orientation,
                pos + sub_pos,
                orientation ^ orientation_mask,
            );
        }
    }
}

const MAX_VALUE_DIVS: [i64; 37] = [
    0,
    0, // 0 and 1 are invalid
    9223372036854775807,
    6148914691236517205,
    4611686018427387903, // 2-4
    3689348814741910323,
    3074457345618258602,
    2635249153387078802, // 5-7
    2305843009213693951,
    2049638230412172401,
    1844674407370955161, // 8-10
    1676976733973595601,
    1537228672809129301,
    1418980313362273201, // 11-13
    1317624576693539401,
    1229782938247303441,
    1152921504606846975, // 14-16
    1085102592571150095,
    1024819115206086200,
    970881267037344821, // 17-19
    922337203685477580,
    878416384462359600,
    838488366986797800, // 20-22
    802032351030850070,
    768614336404564650,
    737869762948382064, // 23-25
    709490156681136600,
    683212743470724133,
    658812288346769700, // 26-28
    636094623231363848,
    614891469123651720,
    595056260442243600, // 29-31
    576460752303423487,
    558992244657865200,
    542551296285575047, // 32-34
    527049830677415760,
    512409557603043100, // 35-36
];

const MAX_VALUE_MODS: [i32; 37] = [
    0, 0, // 0 and 1 are invalid
    1, 0, 3, 0, 3, 1, 7, 6, 5, 4, 3, 2, 1, 0, 15, 0, 15, 16, 15, 15, // 2-21
    15, 5, 15, 15, 15, 24, 15, 23, 15, 15, 31, 15, 17, 15, 15, // 22-36
];

impl S2CellId {
    /// `FACE_BITS`.
    pub const FACE_BITS: i32 = 3;
    /// `NUM_FACES`.
    pub const NUM_FACES: i32 = 6;
    /// `MAX_LEVEL`: valid levels are `0..=MAX_LEVEL`.
    pub const MAX_LEVEL: i32 = 30;
    /// `POS_BITS`.
    pub const POS_BITS: i32 = 2 * Self::MAX_LEVEL + 1;
    /// `MAX_SIZE`.
    pub const MAX_SIZE: i32 = 1 << Self::MAX_LEVEL;
    /// `MAX_UNSIGNED`: `-1L`, `0xffffffffffffffffL`.
    pub const MAX_UNSIGNED: i64 = -1;

    const WRAP_OFFSET: i64 = (Self::NUM_FACES as i64) << Self::POS_BITS;

    /// `S2CellId(long id)`.
    pub const fn new(id: i64) -> Self {
        Self { id }
    }

    /// `none()`.
    pub fn none() -> Self {
        Self { id: 0 }
    }

    /// `sentinel()`.
    pub fn sentinel() -> Self {
        Self::new(Self::MAX_UNSIGNED)
    }

    /// `fromFacePosLevel(int, long, int)`.
    pub fn from_face_pos_level(face: i32, pos: i64, level: i32) -> Self {
        S2CellId::new((i64::from(face) << Self::POS_BITS).wrapping_add(pos | 1)).parent_at(level)
    }

    /// `fromPoint(S2Point)`.
    pub fn from_point(p: &S2Point) -> Self {
        let face = S2Projections::xyz_to_face(p);
        let uv = S2Projections::valid_face_xyz_to_uv(face, p);
        let i = Self::st_to_ij(S2Projections::uv_to_st(uv.x()));
        let j = Self::st_to_ij(S2Projections::uv_to_st(uv.y()));
        Self::from_face_ij(face, i, j)
    }

    /// `fromLatLng(S2LatLng)`.
    pub fn from_lat_lng(ll: &S2LatLng) -> Self {
        Self::from_point(&ll.to_point())
    }

    /// `toPoint()`.
    pub fn to_point(&self) -> S2Point {
        S2Point::normalize(&self.to_point_raw())
    }

    /// `toPointRaw()`.
    pub fn to_point_raw(&self) -> S2Point {
        let mut i = MutableInteger::new(0);
        let mut j = MutableInteger::new(0);
        let face = self.to_face_ij_orientation(&mut i, &mut j, None);
        let delta = if self.is_leaf() {
            1
        } else if ((i.int_value() ^ ((self.id as i32 as u32) >> 2) as i32) & 1) != 0 {
            2
        } else {
            0
        };
        let si = (i.int_value() << 1) + delta - Self::MAX_SIZE;
        let ti = (j.int_value() << 1) + delta - Self::MAX_SIZE;
        Self::face_si_ti_to_xyz(face, si, ti)
    }

    /// `toLatLng()`.
    pub fn to_lat_lng(&self) -> S2LatLng {
        S2LatLng::from_point(&self.to_point_raw())
    }

    /// `id()`.
    pub fn id(&self) -> i64 {
        self.id
    }

    /// `isValid()`.
    pub fn is_valid(&self) -> bool {
        self.face() < Self::NUM_FACES && ((self.lowest_on_bit() & 0x1555555555555555) != 0)
    }

    /// `face()`.
    pub fn face(&self) -> i32 {
        ((self.id as u64) >> Self::POS_BITS) as i32
    }

    /// `pos()`.
    pub fn pos(&self) -> i64 {
        self.id & ((-1i64 as u64) >> Self::FACE_BITS) as i64
    }

    /// `level()`.
    pub fn level(&self) -> i32 {
        if self.is_leaf() {
            return Self::MAX_LEVEL;
        }
        let mut x = self.id as i32;
        let mut level = -1;
        if x != 0 {
            level += 16;
        } else {
            x = ((self.id as u64) >> 32) as i32;
        }
        x &= x.wrapping_neg(); // Get lowest bit.
        if (x & 0x00005555) != 0 {
            level += 8;
        }
        if (x & 0x00550055) != 0 {
            level += 4;
        }
        if (x & 0x05050505) != 0 {
            level += 2;
        }
        if (x & 0x11111111) != 0 {
            level += 1;
        }
        level
    }

    /// `isLeaf()`.
    pub fn is_leaf(&self) -> bool {
        (self.id as i32 & 1) != 0
    }

    /// `isFace()`.
    pub fn is_face(&self) -> bool {
        (self.id & (Self::lowest_on_bit_for_level(0) - 1)) == 0
    }

    /// `childPosition(int level)`.
    pub fn child_position(&self, level: i32) -> i32 {
        (((self.id as u64) >> (2 * (Self::MAX_LEVEL - level) + 1)) as i32) & 3
    }

    /// `rangeMin()`.
    pub fn range_min(&self) -> S2CellId {
        S2CellId::new(self.id.wrapping_sub(self.lowest_on_bit().wrapping_sub(1)))
    }

    /// `rangeMax()`.
    pub fn range_max(&self) -> S2CellId {
        S2CellId::new(self.id.wrapping_add(self.lowest_on_bit().wrapping_sub(1)))
    }

    /// `contains(S2CellId)`.
    pub fn contains(&self, other: &S2CellId) -> bool {
        other.greater_or_equals(&self.range_min()) && other.less_or_equals(&self.range_max())
    }

    /// `intersects(S2CellId)`.
    pub fn intersects(&self, other: &S2CellId) -> bool {
        other.range_min().less_or_equals(&self.range_max())
            && other.range_max().greater_or_equals(&self.range_min())
    }

    /// `parent()`.
    pub fn parent(&self) -> S2CellId {
        let new_lsb = self.lowest_on_bit().wrapping_shl(2);
        S2CellId::new((self.id & new_lsb.wrapping_neg()) | new_lsb)
    }

    /// `parent(int level)`.
    pub fn parent_at(&self, level: i32) -> S2CellId {
        let new_lsb = Self::lowest_on_bit_for_level(level);
        S2CellId::new((self.id & new_lsb.wrapping_neg()) | new_lsb)
    }

    /// `childBegin()`.
    pub fn child_begin(&self) -> S2CellId {
        let old_lsb = self.lowest_on_bit();
        S2CellId::new(
            self.id
                .wrapping_sub(old_lsb)
                .wrapping_add(((old_lsb as u64) >> 2) as i64),
        )
    }

    /// `childBegin(int level)`.
    pub fn child_begin_at(&self, level: i32) -> S2CellId {
        S2CellId::new(
            self.id
                .wrapping_sub(self.lowest_on_bit())
                .wrapping_add(Self::lowest_on_bit_for_level(level)),
        )
    }

    /// `childEnd()`.
    pub fn child_end(&self) -> S2CellId {
        let old_lsb = self.lowest_on_bit();
        S2CellId::new(
            self.id
                .wrapping_add(old_lsb)
                .wrapping_add(((old_lsb as u64) >> 2) as i64),
        )
    }

    /// `childEnd(int level)`.
    pub fn child_end_at(&self, level: i32) -> S2CellId {
        S2CellId::new(
            self.id
                .wrapping_add(self.lowest_on_bit())
                .wrapping_add(Self::lowest_on_bit_for_level(level)),
        )
    }

    /// `next()`.
    pub fn next(&self) -> S2CellId {
        S2CellId::new(self.id.wrapping_add(self.lowest_on_bit().wrapping_shl(1)))
    }

    /// `prev()`.
    pub fn prev(&self) -> S2CellId {
        S2CellId::new(self.id.wrapping_sub(self.lowest_on_bit().wrapping_shl(1)))
    }

    /// `nextWrap()`.
    pub fn next_wrap(&self) -> S2CellId {
        let n = self.next();
        if Self::unsigned_long_less_than(n.id, Self::WRAP_OFFSET) {
            return n;
        }
        S2CellId::new(n.id.wrapping_sub(Self::WRAP_OFFSET))
    }

    /// `prevWrap()`: Java compares signed here, as written.
    pub fn prev_wrap(&self) -> S2CellId {
        let p = self.prev();
        if p.id < Self::WRAP_OFFSET {
            return p;
        }
        S2CellId::new(p.id.wrapping_add(Self::WRAP_OFFSET))
    }

    /// `begin(int level)`.
    pub fn begin(level: i32) -> S2CellId {
        Self::from_face_pos_level(0, 0, 0).child_begin_at(level)
    }

    /// `end(int level)`.
    pub fn end(level: i32) -> S2CellId {
        Self::from_face_pos_level(5, 0, 0).child_end_at(level)
    }

    /// `fromToken(String)`; `None` stands for a Java `null` token.
    ///
    /// # Errors
    ///
    /// `NumberFormatException` for a `null` or empty token, a non-hex digit,
    /// or an overflow.
    pub fn from_token(token: Option<&str>) -> Result<S2CellId> {
        let Some(token) = token else {
            return Err(LuceneError::unchecked(
                "java.lang.NumberFormatException",
                "Null string in S2CellId.fromToken",
            ));
        };
        let units: Vec<u16> = token.encode_utf16().collect();
        if units.is_empty() {
            return Err(LuceneError::unchecked(
                "java.lang.NumberFormatException",
                "Empty string in S2CellId.fromToken",
            ));
        }
        if units.len() > 16 || "X" == token {
            return Ok(Self::none());
        }
        let mut value: i64 = 0;
        for pos in 0..16 {
            let mut digit: i32 = 0;
            if pos < units.len() {
                digit = char::from_u32(u32::from(units[pos]))
                    .and_then(|c| java_character_digit(c, 16))
                    .map_or(-1, |d| d as i32);
                if digit == -1 {
                    return Err(LuceneError::unchecked(
                        "java.lang.NumberFormatException",
                        token.to_string(),
                    ));
                }
                if Self::overflow_in_parse(value, digit) {
                    return Err(LuceneError::unchecked(
                        "java.lang.NumberFormatException",
                        format!("Too large for unsigned long: {token}"),
                    ));
                }
            }
            value = value.wrapping_mul(16).wrapping_add(i64::from(digit));
        }
        Ok(S2CellId::new(value))
    }

    /// `toToken()`.
    pub fn to_token(&self) -> String {
        if self.id == 0 {
            return "X".to_string();
        }
        let hex = format!("{:x}", self.id as u64);
        let mut sb = String::with_capacity(16);
        for _ in hex.len()..16 {
            sb.push('0');
        }
        sb.push_str(&hex);
        let bytes = sb.as_bytes();
        for len in (1..=16).rev() {
            if bytes[len - 1] != b'0' {
                return sb[..len].to_string();
            }
        }
        unreachable!("Shouldn't make it here")
    }

    fn overflow_in_parse(current: i64, digit: i32) -> bool {
        Self::overflow_in_parse_radix(current, digit, 10)
    }

    fn overflow_in_parse_radix(current: i64, digit: i32, radix: usize) -> bool {
        if current >= 0 {
            if current < MAX_VALUE_DIVS[radix] {
                return false;
            }
            if current > MAX_VALUE_DIVS[radix] {
                return true;
            }
            return digit > MAX_VALUE_MODS[radix];
        }
        true
    }

    /// `getEdgeNeighbors(S2CellId[] neighbors)`.
    pub fn get_edge_neighbors(&self, neighbors: &mut [S2CellId]) {
        let mut i = MutableInteger::new(0);
        let mut j = MutableInteger::new(0);
        let level = self.level();
        let size = 1 << (Self::MAX_LEVEL - level);
        let face = self.to_face_ij_orientation(&mut i, &mut j, None);
        let (i, j) = (i.int_value(), j.int_value());
        neighbors[0] = Self::from_face_ij_same(face, i, j - size, j - size >= 0).parent_at(level);
        neighbors[1] =
            Self::from_face_ij_same(face, i + size, j, i + size < Self::MAX_SIZE).parent_at(level);
        neighbors[2] =
            Self::from_face_ij_same(face, i, j + size, j + size < Self::MAX_SIZE).parent_at(level);
        neighbors[3] = Self::from_face_ij_same(face, i - size, j, i - size >= 0).parent_at(level);
    }

    /// `getVertexNeighbors(int level, List<S2CellId> output)`.
    pub fn get_vertex_neighbors(&self, level: i32, output: &mut Vec<S2CellId>) {
        let mut i = MutableInteger::new(0);
        let mut j = MutableInteger::new(0);
        let face = self.to_face_ij_orientation(&mut i, &mut j, None);
        let (i, j) = (i.int_value(), j.int_value());
        let halfsize = 1 << (Self::MAX_LEVEL - (level + 1));
        let size = halfsize << 1;
        let (ioffset, isame) = if (i & halfsize) != 0 {
            (size, (i + size) < Self::MAX_SIZE)
        } else {
            (-size, (i - size) >= 0)
        };
        let (joffset, jsame) = if (j & halfsize) != 0 {
            (size, (j + size) < Self::MAX_SIZE)
        } else {
            (-size, (j - size) >= 0)
        };
        output.push(self.parent_at(level));
        output.push(Self::from_face_ij_same(face, i + ioffset, j, isame).parent_at(level));
        output.push(Self::from_face_ij_same(face, i, j + joffset, jsame).parent_at(level));
        if isame || jsame {
            output.push(
                Self::from_face_ij_same(face, i + ioffset, j + joffset, isame && jsame)
                    .parent_at(level),
            );
        }
    }

    /// `getAllNeighbors(int nbrLevel, List<S2CellId> output)`.
    pub fn get_all_neighbors(&self, nbr_level: i32, output: &mut Vec<S2CellId>) {
        let mut i = MutableInteger::new(0);
        let mut j = MutableInteger::new(0);
        let face = self.to_face_ij_orientation(&mut i, &mut j, None);
        let size = 1 << (Self::MAX_LEVEL - self.level());
        i.set_value(i.int_value() & -size);
        j.set_value(j.int_value() & -size);
        let (i, j) = (i.int_value(), j.int_value());
        let nbr_size = 1 << (Self::MAX_LEVEL - nbr_level);
        let mut k = -nbr_size;
        loop {
            let same_face;
            if k < 0 {
                same_face = j + k >= 0;
            } else if k >= size {
                same_face = j + k < Self::MAX_SIZE;
            } else {
                same_face = true;
                output.push(
                    Self::from_face_ij_same(face, i + k, j - nbr_size, j - size >= 0)
                        .parent_at(nbr_level),
                );
                output.push(
                    Self::from_face_ij_same(face, i + k, j + size, j + size < Self::MAX_SIZE)
                        .parent_at(nbr_level),
                );
            }
            output.push(
                Self::from_face_ij_same(face, i - nbr_size, j + k, same_face && i - size >= 0)
                    .parent_at(nbr_level),
            );
            output.push(
                Self::from_face_ij_same(
                    face,
                    i + size,
                    j + k,
                    same_face && i + size < Self::MAX_SIZE,
                )
                .parent_at(nbr_level),
            );
            if k >= size {
                break;
            }
            k += nbr_size;
        }
    }

    /// `fromFaceIJ(int, int, int)`.
    pub fn from_face_ij(face: i32, i: i32, j: i32) -> S2CellId {
        let mut n: [i64; 2] = [
            0,
            i64::from(face.wrapping_shl((Self::POS_BITS - 33) as u32)),
        ];
        let mut bits = face & SWAP_MASK;
        for k in (0..=7).rev() {
            bits = Self::get_bits(&mut n, i, j, k, bits);
        }
        S2CellId::new(
            (n[1].wrapping_shl(32).wrapping_add(n[0]))
                .wrapping_shl(1)
                .wrapping_add(1),
        )
    }

    fn get_bits(n: &mut [i64; 2], i: i32, j: i32, k: i32, mut bits: i32) -> i32 {
        let mask = (1 << LOOKUP_BITS) - 1;
        bits += ((i >> (k * LOOKUP_BITS)) & mask) << (LOOKUP_BITS + 2);
        bits += ((j >> (k * LOOKUP_BITS)) & mask) << 2;
        bits = LOOKUP.pos[bits as usize];
        n[(k >> 2) as usize] |= (i64::from(bits) >> 2) << ((k & 3) * 2 * LOOKUP_BITS);
        bits &= SWAP_MASK | INVERT_MASK;
        bits
    }

    /// `toFaceIJOrientation(MutableInteger pi, MutableInteger pj,
    /// MutableInteger orientation)`; `None` stands for a `null` orientation.
    pub fn to_face_ij_orientation(
        &self,
        pi: &mut MutableInteger,
        pj: &mut MutableInteger,
        orientation: Option<&mut MutableInteger>,
    ) -> i32 {
        let face = self.face();
        let mut bits = face & SWAP_MASK;
        for k in (0..=7).rev() {
            bits = self.get_bits1(pi, pj, k, bits);
        }
        if let Some(orientation) = orientation {
            if (self.lowest_on_bit() & 0x1111111111111110) != 0 {
                bits ^= S2::SWAP_MASK;
            }
            orientation.set_value(bits);
        }
        face
    }

    fn get_bits1(
        &self,
        i: &mut MutableInteger,
        j: &mut MutableInteger,
        k: i32,
        mut bits: i32,
    ) -> i32 {
        let nbits = if k == 7 {
            Self::MAX_LEVEL - 7 * LOOKUP_BITS
        } else {
            LOOKUP_BITS
        };
        bits += ((((self.id as u64) >> (k * 2 * LOOKUP_BITS + 1)) as i32)
            & ((1 << (2 * nbits)) - 1))
            << 2;
        bits = LOOKUP.ij[bits as usize];
        i.set_value(i.int_value() + ((bits >> (LOOKUP_BITS + 2)) << (k * LOOKUP_BITS)));
        j.set_value(
            j.int_value() + (((bits >> 2) & ((1 << LOOKUP_BITS) - 1)) << (k * LOOKUP_BITS)),
        );
        bits &= SWAP_MASK | INVERT_MASK;
        bits
    }

    /// `lowestOnBit()`.
    pub fn lowest_on_bit(&self) -> i64 {
        self.id & self.id.wrapping_neg()
    }

    /// `lowestOnBitForLevel(int level)`.
    pub fn lowest_on_bit_for_level(level: i32) -> i64 {
        1i64.wrapping_shl((2 * (Self::MAX_LEVEL - level)) as u32)
    }

    fn st_to_ij(s: f64) -> i32 {
        let m = Self::MAX_SIZE / 2; // scaling multiplier
        0i64.max(i64::from(2 * m - 1).min(java_round(f64::from(m) * s + (f64::from(m) - 0.5))))
            as i32
    }

    fn face_si_ti_to_xyz(face: i32, si: i32, ti: i32) -> S2Point {
        let k_scale = 1.0 / f64::from(Self::MAX_SIZE);
        let u = S2Projections::st_to_uv(k_scale * f64::from(si));
        let v = S2Projections::st_to_uv(k_scale * f64::from(ti));
        S2Projections::face_uv_to_xyz(face, u, v)
    }

    fn from_face_ij_wrap(face: i32, i: i32, j: i32) -> S2CellId {
        let i = (-1).max(Self::MAX_SIZE.min(i));
        let j = (-1).max(Self::MAX_SIZE.min(j));
        let k_scale = 1.0 / f64::from(Self::MAX_SIZE);
        let s = k_scale * f64::from((i << 1) + 1 - Self::MAX_SIZE);
        let t = k_scale * f64::from((j << 1) + 1 - Self::MAX_SIZE);
        let p = S2Projections::face_uv_to_xyz(face, s, t);
        let face = S2Projections::xyz_to_face(&p);
        let st = S2Projections::valid_face_xyz_to_uv(face, &p);
        Self::from_face_ij(face, Self::st_to_ij(st.x()), Self::st_to_ij(st.y()))
    }

    /// `fromFaceIJSame(int, int, int, boolean)`.
    pub fn from_face_ij_same(face: i32, i: i32, j: i32, same_face: bool) -> S2CellId {
        if same_face {
            S2CellId::from_face_ij(face, i, j)
        } else {
            S2CellId::from_face_ij_wrap(face, i, j)
        }
    }

    /// `unsignedLongLessThan(long, long)`.
    pub fn unsigned_long_less_than(x1: i64, x2: i64) -> bool {
        (x1 as u64) < (x2 as u64)
    }

    /// `unsignedLongGreaterThan(long, long)`.
    pub fn unsigned_long_greater_than(x1: i64, x2: i64) -> bool {
        (x1 as u64) > (x2 as u64)
    }

    /// `lessThan(S2CellId)`.
    pub fn less_than(&self, x: &S2CellId) -> bool {
        Self::unsigned_long_less_than(self.id, x.id)
    }

    /// `greaterThan(S2CellId)`.
    pub fn greater_than(&self, x: &S2CellId) -> bool {
        Self::unsigned_long_greater_than(self.id, x.id)
    }

    /// `lessOrEquals(S2CellId)`.
    pub fn less_or_equals(&self, x: &S2CellId) -> bool {
        Self::unsigned_long_less_than(self.id, x.id) || self.id == x.id
    }

    /// `greaterOrEquals(S2CellId)`.
    pub fn greater_or_equals(&self, x: &S2CellId) -> bool {
        Self::unsigned_long_greater_than(self.id, x.id) || self.id == x.id
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        (((self.id as u64) >> 32) as i64).wrapping_add(self.id) as i32
    }

    /// `compareTo(S2CellId)`.
    pub fn compare_to(&self, that: &S2CellId) -> i32 {
        if Self::unsigned_long_less_than(self.id, that.id) {
            -1
        } else if Self::unsigned_long_greater_than(self.id, that.id) {
            1
        } else {
            0
        }
    }
}

impl PartialOrd for S2CellId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for S2CellId {
    /// `compareTo(S2CellId)`: unsigned order.
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare_to(other).cmp(&0)
    }
}

impl Hash for S2CellId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for S2CellId {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(face={}, pos={:x}, level={})",
            self.face(),
            self.pos() as u64,
            self.level()
        )
    }
}
