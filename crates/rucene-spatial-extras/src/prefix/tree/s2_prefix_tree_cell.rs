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

//! The types declared in `S2PrefixTreeCell.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use rucene::util::java_math;
use std::any::Any;
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use rucene::util::BytesRef;
use s2_geometry::s2_cell_id::S2CellId;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::prefix::tree::cell::{Cell, CellRef};
use crate::prefix::tree::cell_can_prune::CellCanPrune;
use crate::prefix::tree::cell_iterator::{CellIterator, ListCellIterator};
use crate::prefix::tree::filter_cell_iterator::FilterCellIterator;
use crate::prefix::tree::s2_prefix_tree::S2PrefixTree;
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;

/// `FACES`: the faces of S2 Geometry.
static FACES: LazyLock<[S2CellId; 6]> = LazyLock::new(|| {
    [
        S2CellId::from_face_pos_level(0, 0, 0),
        S2CellId::from_face_pos_level(1, 0, 0),
        S2CellId::from_face_pos_level(2, 0, 0),
        S2CellId::from_face_pos_level(3, 0, 0),
        S2CellId::from_face_pos_level(4, 0, 0),
        S2CellId::from_face_pos_level(5, 0, 0),
    ]
});

/// `LEAF`.
const LEAF: u8 = b'+';

/// `TOKENS`.
const TOKENS: [u8; 64] = *b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// `PIXELS.get(b)`: the position of `b` in [`TOKENS`].
///
/// # Panics
///
/// For a byte that is not a token: Java unboxes the `null` the map returns
/// (`NullPointerException`).
fn pixels(b: u8) -> i32 {
    TOKENS
        .iter()
        .position(|&t| t == b)
        .map(|i| i as i32)
        .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
}

/// The mutable fields of an [`S2PrefixTreeCell`].
#[derive(Debug)]
struct S2State {
    cell_id: Option<S2CellId>,
    level: i32, // cache level
    tree: Arc<S2PrefixTree>,
    shape_rel: Option<SpatialRelation>,
    is_leaf: bool,
    shape: Option<Arc<dyn Shape>>,
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.S2PrefixTreeCell`
/// (package-private `class S2PrefixTreeCell implements CellCanPrune`): this
/// class represents a S2 pixel in the RPT. A `None` cell id is the world
/// cell.
///
/// Java's class, constructor, `readCell` and `cellId` are package-private;
/// they are public here for the port of `TestS2PrefixTree`, which is in the
/// same Java package.
#[derive(Debug)]
pub struct S2PrefixTreeCell {
    state: Mutex<S2State>,
}

impl S2PrefixTreeCell {
    /// `S2PrefixTreeCell(S2PrefixTree tree, S2CellId cellId)`.
    pub fn new(tree: &Arc<S2PrefixTree>, cell_id: Option<S2CellId>) -> Self {
        let cell = S2PrefixTreeCell {
            state: Mutex::new(S2State {
                cell_id,
                level: 0,
                tree: Arc::clone(tree),
                shape_rel: None,
                is_leaf: false,
                shape: None,
            }),
        };
        {
            let mut s = cell.lock();
            Self::set_level(&mut s);
            if s.level == s.tree.get_max_levels() {
                s.is_leaf = true;
            }
        }
        cell
    }

    /// `cellId` (package-private field); `None` for the world cell.
    pub fn cell_id(&self) -> Option<S2CellId> {
        self.lock().cell_id
    }

    fn lock(&self) -> MutexGuard<'_, S2State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `readCell(S2PrefixTree tree, BytesRef ref)`.
    pub fn read_cell(&self, tree: &Arc<S2PrefixTree>, r: &BytesRef) {
        let mut s = self.lock();
        s.is_leaf = false;
        s.shape = None;
        s.shape_rel = None;
        s.tree = Arc::clone(tree);
        s.cell_id = Self::get_s2_cell_id_from_bytes_ref(tree.arity, r);
        Self::set_level(&mut s);
        if Self::is_leaf_ref(r) || s.level == s.tree.get_max_levels() {
            s.is_leaf = true;
        }
    }

    fn set_level(s: &mut S2State) {
        match &s.cell_id {
            None => s.level = 0,
            Some(cell_id) => {
                debug_assert_eq!(cell_id.level() % s.tree.arity, 0);
                s.level = (cell_id.level() / s.tree.arity) + 1;
            }
        }
    }

    fn is_leaf_ref(r: &BytesRef) -> bool {
        r.bytes[r.offset + r.length - 1] == LEAF
    }

    fn get_s2_cell_id_from_bytes_ref(arity: i32, r: &BytesRef) -> Option<S2CellId> {
        let mut length = r.length;
        if Self::is_leaf_ref(r) {
            length -= 1;
        }
        if length == 0 {
            return None; // world cell
        }
        let face = pixels(r.bytes[r.offset]);
        let cell_id = FACES[face as usize];
        let mut id = cell_id.id();
        for i in (r.offset + 1)..(r.offset + length) {
            let this_level = (i - r.offset) as i32;
            let pos = i64::from(pixels(r.bytes[i]));
            // first child at level
            id = id.wrapping_sub(id & id.wrapping_neg()).wrapping_add(
                1i64.wrapping_shl((2 * (S2CellId::MAX_LEVEL - this_level * arity)) as u32),
            );
            // next until pos
            id = id.wrapping_add(pos.wrapping_mul((id & id.wrapping_neg()).wrapping_shl(1)));
        }
        Some(S2CellId::new(id))
    }

    fn get_bytes_ref_from_s2_cell_id(s: &S2State, bref: &mut BytesRef) {
        let Some(cell_id) = &s.cell_id else {
            // world cell
            bref.length = 0;
            return;
        };
        let level = s.level;
        let arity = s.tree.arity;
        let length = (level + 1) as usize;
        let mut b = if bref.bytes.len() >= length {
            std::mem::take(&mut bref.bytes)
        } else {
            vec![0u8; length]
        };
        b[0] = TOKENS[cell_id.face() as usize];
        for i in 1..level {
            let mut offset = 0;
            let lvl = arity * i;
            for j in 1..arity {
                offset = 4 * offset + cell_id.child_position(lvl - arity + j);
            }
            b[i as usize] = TOKENS[(4 * offset + cell_id.child_position(lvl)) as usize];
        }
        bref.bytes = b;
        bref.length = level as usize;
        bref.offset = 0;
    }
}

impl fmt::Display for S2PrefixTreeCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.lock().cell_id {
            None => f.write_str("0"),
            Some(cell_id) => write!(f, "{cell_id}"),
        }
    }
}

impl CellCanPrune for S2PrefixTreeCell {
    fn get_sub_cells_size(&self) -> i32 {
        let s = self.lock();
        if s.cell_id.is_none() {
            // root node
            return 6;
        }
        java_math::pow(4.0, f64::from(s.tree.arity)) as i32
    }
}

impl Cell for S2PrefixTreeCell {
    fn get_shape_rel(&self) -> Option<SpatialRelation> {
        self.lock().shape_rel
    }

    fn set_shape_rel(&self, rel: Option<SpatialRelation>) {
        self.lock().shape_rel = rel;
    }

    fn is_leaf(&self) -> bool {
        self.lock().is_leaf
    }

    fn set_leaf(&self) {
        self.lock().is_leaf = true;
    }

    fn get_token_bytes_with_leaf(&self, result: Option<BytesRef>) -> BytesRef {
        let mut result = self.get_token_bytes_no_leaf(result);
        let s = self.lock();
        // max levels do not have leaf
        if s.is_leaf && !(s.level == s.tree.get_max_levels()) {
            // Add leaf byte
            let end = result.offset + result.length;
            if result.bytes.len() <= end {
                result.bytes.resize(end + 1, 0);
            }
            result.bytes[end] = LEAF;
            result.length += 1;
        }
        result
    }

    fn get_token_bytes_no_leaf(&self, result: Option<BytesRef>) -> BytesRef {
        let mut result = result.unwrap_or_else(|| BytesRef::new(Vec::new()));
        Self::get_bytes_ref_from_s2_cell_id(&self.lock(), &mut result);
        result
    }

    fn get_level(&self) -> i32 {
        self.lock().level
    }

    fn get_next_level_cells(&self, shape_filter: Option<Arc<dyn Shape>>) -> Box<dyn CellIterator> {
        let (cell_id, tree) = {
            let s = self.lock();
            (s.cell_id, Arc::clone(&s.tree))
        };
        let children: Vec<S2CellId> = match cell_id {
            None => FACES.to_vec(), // this is the world cell
            Some(cell_id) => {
                let n_children = java_math::pow(4.0, f64::from(tree.arity)) as usize;
                let mut children = Vec::with_capacity(n_children);
                children.push(cell_id.child_begin_at(cell_id.level() + tree.arity));
                for i in 1..n_children {
                    let next = children[i - 1].next();
                    children.push(next);
                }
                children
            }
        };
        let cells: Vec<CellRef> = children
            .into_iter()
            .map(|pixel| Arc::new(S2PrefixTreeCell::new(&tree, Some(pixel))) as CellRef)
            .collect();
        Box::new(FilterCellIterator::new(
            Box::new(ListCellIterator::new(cells)),
            shape_filter,
        ))
    }

    fn get_shape(&self) -> Arc<dyn Shape> {
        let (cell_id, tree) = {
            let s = self.lock();
            if let Some(shape) = &s.shape {
                return Arc::clone(shape);
            }
            (s.cell_id, Arc::clone(&s.tree))
        };
        let shape: Arc<dyn Shape> = match cell_id {
            None => tree.get_spatial_context().get_world_bounds(), // World cell
            Some(cell_id) => tree.s2_shape_factory().get_s2_cell_shape(cell_id),
        };
        self.lock().shape = Some(Arc::clone(&shape));
        shape
    }

    fn is_prefix_of(&self, c: &dyn Cell) -> bool {
        let Some(cell_id) = self.lock().cell_id else {
            return true;
        };
        let cell = cast(c);
        let other = cell.lock().cell_id;
        // `cellId.contains(null)` dereferences the null.
        cell_id.contains(&other.unwrap_or_else(|| panic!("java.lang.NullPointerException")))
    }

    fn compare_to_no_leaf(&self, from_cell: &dyn Cell) -> i32 {
        let Some(cell_id) = self.lock().cell_id else {
            return 1;
        };
        let cell = cast(from_cell);
        let other = cell.lock().cell_id;
        cell_id.compare_to(&other.unwrap_or_else(|| panic!("java.lang.NullPointerException")))
    }

    fn cell_equals(&self, other: &dyn Cell) -> bool {
        let cell = cast(other);
        if std::ptr::eq(self, cell) {
            return true;
        }
        self.lock().cell_id == cell.lock().cell_id
    }

    fn cell_hash_code(&self) -> i32 {
        match &self.lock().cell_id {
            // `System.identityHashCode(this)`: the core's identity-hash stand-in.
            None => {
                crate::jdk::identity_hash("org.apache.lucene.spatial.prefix.tree.S2PrefixTreeCell")
            }
            Some(cell_id) => cell_id.hash_code(),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_cell_can_prune(&self) -> Option<&dyn CellCanPrune> {
        Some(self)
    }
}

/// `(S2PrefixTreeCell) c`.
fn cast(c: &dyn Cell) -> &S2PrefixTreeCell {
    c.as_any()
        .downcast_ref::<S2PrefixTreeCell>()
        .unwrap_or_else(|| {
            panic!("java.lang.ClassCastException: {c:?} cannot be cast to S2PrefixTreeCell")
        })
}
