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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextBKDReader`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::index::{IntersectVisitor, PointTree, PointValues};
use rucene::store::IndexInput;
use rucene::util::bkd::BKDConfig;
use rucene::util::MathUtil;

use super::bkd_writer::VERSION_CURRENT;
use super::points_writer::{BLOCK_COUNT, BLOCK_DOC_ID, BLOCK_VALUE};
use super::util::{self, parse_int, JavaScratch};

/// The `IndexInput` the BKD readers of one points reader share: Java's
/// `dataIn`, which `SimpleTextPointsReader.close` closes.
pub(crate) struct SharedDataIn {
    pub(crate) input: Mutex<Option<Box<dyn IndexInput>>>,
}

impl SharedDataIn {
    pub(crate) fn clone_input_pub(&self) -> Result<Box<dyn IndexInput>> {
        self.clone_input()
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        let guard = self.input.lock().map_err(|_| {
            LuceneError::illegal_state("points input lock was poisoned".to_string())
        })?;
        match guard.as_ref() {
            Some(input) => input.clone_input(),
            None => Err(LuceneError::already_closed("this IndexInput is closed")),
        }
    }
}

/// Forked from `BKDReader` and simplified/specialized for SimpleText's usage.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextBKDReader`.
#[derive(Clone)]
pub struct SimpleTextBKDReader {
    inner: Arc<ReaderInner>,
}

struct ReaderInner {
    /// Packed array of byte[] holding all split values in the full binary
    /// tree.
    split_packed_values: Vec<u8>,
    leaf_block_fps: Vec<i64>,
    leaf_node_offset: i32,
    config: BKDConfig,
    bytes_per_index_entry: usize,
    input: Arc<SharedDataIn>,
    min_packed_value: Vec<u8>,
    max_packed_value: Vec<u8>,
    point_count: i64,
    doc_count: i32,
    #[allow(dead_code)]
    version: i32,
}

impl std::fmt::Debug for SimpleTextBKDReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleTextBKDReader")
            .field("point_count", &self.inner.point_count)
            .finish_non_exhaustive()
    }
}

impl SimpleTextBKDReader {
    /// Port of `SimpleTextBKDReader(IndexInput, int, int, int, int, long[],
    /// byte[], byte[], byte[], long, int)`.
    ///
    /// # Errors
    ///
    /// Returns `BKDConfig`'s `IllegalArgumentException` for an invalid
    /// configuration.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        input: Arc<SharedDataIn>,
        num_dims: i32,
        num_index_dims: i32,
        max_points_in_leaf_node: i32,
        bytes_per_dim: i32,
        leaf_block_fps: Vec<i64>,
        split_packed_values: Vec<u8>,
        min_packed_value: Vec<u8>,
        max_packed_value: Vec<u8>,
        point_count: i64,
        doc_count: i32,
    ) -> Result<Self> {
        let config = BKDConfig::new(
            num_dims,
            num_index_dims,
            bytes_per_dim,
            max_points_in_leaf_node,
        )?;
        // no version check here because callers of this API (SimpleText) have
        // no back compat:
        let bytes_per_index_entry = if num_index_dims == 1 {
            bytes_per_dim as usize
        } else {
            bytes_per_dim as usize + 1
        };
        debug_assert_eq!(
            min_packed_value.len(),
            config.packed_index_bytes_length() as usize
        );
        debug_assert_eq!(
            max_packed_value.len(),
            config.packed_index_bytes_length() as usize
        );
        Ok(Self {
            inner: Arc::new(ReaderInner {
                leaf_node_offset: leaf_block_fps.len() as i32,
                leaf_block_fps,
                split_packed_values,
                config,
                bytes_per_index_entry,
                input,
                min_packed_value,
                max_packed_value,
                point_count,
                doc_count,
                version: VERSION_CURRENT,
            }),
        })
    }
}

impl PointValues for SimpleTextBKDReader {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(SimpleTextPointTree::new(
            Arc::clone(&self.inner),
            self.inner.input.clone_input()?,
            1,
            1,
            &self.inner.min_packed_value,
            &self.inner.max_packed_value,
        )))
    }

    fn size(&self) -> Result<i64> {
        Ok(self.inner.point_count)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.inner.doc_count)
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Ok(Some(self.inner.min_packed_value.clone()))
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Ok(Some(self.inner.max_packed_value.clone()))
    }

    fn num_dimensions(&self) -> Result<i32> {
        Ok(self.inner.config.num_dims)
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        Ok(self.inner.config.num_index_dims)
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        Ok(self.inner.config.bytes_per_dim)
    }
}

/// Port of the inner class `org.apache.lucene.codecs.simpletext.SimpleTextBKDReader.SimpleTextPointTree`.
struct SimpleTextPointTree {
    reader: Arc<ReaderInner>,
    scratch_doc_ids: Vec<i32>,
    scratch_packed_value: Vec<u8>,
    node_id: i32,
    level: usize,
    root_node: i32,
    /// holds the min / max value of the current node.
    min_packed_value: Vec<u8>,
    max_packed_value: Vec<u8>,
    /// holds the previous value of the split dimension
    split_dim_value_stack: Vec<Option<Vec<u8>>>,
    /// holds the splitDim for each level:
    split_dims: Vec<usize>,
    input: Box<dyn IndexInput>,
}

impl SimpleTextPointTree {
    fn new(
        reader: Arc<ReaderInner>,
        input: Box<dyn IndexInput>,
        node_id: i32,
        level: usize,
        min_packed_value: &[u8],
        max_packed_value: &[u8],
    ) -> Self {
        let tree_depth = Self::get_tree_depth(reader.leaf_node_offset);
        Self {
            scratch_doc_ids: vec![0; reader.config.max_points_in_leaf_node as usize],
            scratch_packed_value: vec![0; reader.config.packed_bytes_length() as usize],
            node_id,
            root_node: node_id,
            level,
            max_packed_value: max_packed_value.to_vec(),
            min_packed_value: min_packed_value.to_vec(),
            split_dim_value_stack: vec![None; tree_depth + 1],
            split_dims: vec![0; tree_depth + 1],
            input,
            reader,
        }
    }

    /// First +1 because all the non-leave nodes makes another power of 2;
    /// e.g. to have a fully balanced tree with 4 leaves you need a depth=3
    /// tree. Second +1 because MathUtil.log computes floor of the logarithm;
    /// e.g. with 5 leaves you need a depth=4 tree.
    fn get_tree_depth(num_leaves: i32) -> usize {
        (MathUtil::log(i64::from(num_leaves), 2).unwrap_or(0) + 2) as usize
    }

    fn is_leaf_node(&self) -> bool {
        self.node_id >= self.reader.leaf_node_offset
    }

    /// The split dimension and value address of the current node:
    /// `splitPackedValues[address++] & 0xff` for multi-dimensional points.
    fn read_split_dim(&mut self) -> Result<i64> {
        let reader = &self.reader;
        let mut address = i64::from(self.node_id) * reader.bytes_per_index_entry as i64;
        if reader.config.num_index_dims == 1 {
            self.split_dims[self.level] = 0;
        } else {
            let length = reader.split_packed_values.len();
            let byte = usize::try_from(address)
                .ok()
                .and_then(|a| reader.split_packed_values.get(a))
                .ok_or_else(|| util::array_index_out_of_bounds(address, length))?;
            self.split_dims[self.level] = usize::from(*byte);
            address += 1;
        }
        Ok(address)
    }

    /// The message of `pushLeft`'s and `pushRight`'s assertions.
    fn split_assert_message(&self) -> String {
        let config = &self.reader.config;
        format!(
            "config.bytesPerDim()={} splitDim={} config.numIndexDims()={} config.numDims={}",
            config.bytes_per_dim,
            self.split_dims[self.level],
            config.num_index_dims,
            config.num_dims
        )
    }

    /// Port of `pushLeft()` (`SimpleTextBKDReader.java:139-172`), with the
    /// `System.arraycopy` bounds checks and the ordering assertion.
    fn push_left(&mut self) -> Result<()> {
        let bytes_per_dim = self.reader.config.bytes_per_dim;
        let address = self.read_split_dim()?;
        let split_dim_pos = self.split_dims[self.level] as i64 * i64::from(bytes_per_dim);
        // save the dimension we are going to change
        let saved = self.split_dim_value_stack[self.level]
            .get_or_insert_with(|| vec![0; bytes_per_dim.max(0) as usize]);
        util::arraycopy(
            &self.max_packed_value,
            split_dim_pos,
            saved,
            0,
            i64::from(bytes_per_dim),
        )?;
        debug_assert!(
            {
                let (p, a, n) = (
                    split_dim_pos as usize,
                    address as usize,
                    bytes_per_dim as usize,
                );
                self.max_packed_value[p..p + n] >= self.reader.split_packed_values[a..a + n]
            },
            "{}",
            self.split_assert_message()
        );
        self.node_id *= 2;
        self.level += 1;
        // add the split dim value:
        util::arraycopy(
            &self.reader.split_packed_values,
            address,
            &mut self.max_packed_value,
            split_dim_pos,
            i64::from(bytes_per_dim),
        )
    }

    /// Port of `pushRight()` (`SimpleTextBKDReader.java:183-217`).
    fn push_right(&mut self) -> Result<()> {
        let bytes_per_dim = self.reader.config.bytes_per_dim;
        let address = self.read_split_dim()?;
        let split_dim_pos = self.split_dims[self.level] as i64 * i64::from(bytes_per_dim);
        // we should have already visit the left node
        debug_assert!(self.split_dim_value_stack[self.level].is_some());
        // save the dimension we are going to change
        let saved = self.split_dim_value_stack[self.level]
            .get_or_insert_with(|| vec![0; bytes_per_dim.max(0) as usize]);
        util::arraycopy(
            &self.min_packed_value,
            split_dim_pos,
            saved,
            0,
            i64::from(bytes_per_dim),
        )?;
        debug_assert!(
            {
                let (p, a, n) = (
                    split_dim_pos as usize,
                    address as usize,
                    bytes_per_dim as usize,
                );
                self.min_packed_value[p..p + n] <= self.reader.split_packed_values[a..a + n]
            },
            "{}",
            self.split_assert_message()
        );
        self.node_id = 2 * self.node_id + 1;
        self.level += 1;
        // add the split dim value:
        util::arraycopy(
            &self.reader.split_packed_values,
            address,
            &mut self.min_packed_value,
            split_dim_pos,
            i64::from(bytes_per_dim),
        )
    }

    /// Port of `pop(boolean)` (`SimpleTextBKDReader.java:229-249`).
    fn pop(&mut self, is_left: bool) -> Result<()> {
        let bytes_per_dim = i64::from(self.reader.config.bytes_per_dim);
        self.node_id /= 2;
        self.level -= 1;
        // restore the split dimension
        let pos = self.split_dims[self.level] as i64 * bytes_per_dim;
        let saved = self.split_dim_value_stack[self.level]
            .as_ref()
            .expect("INVARIANT: pushed before popped");
        if is_left {
            util::arraycopy(saved, 0, &mut self.max_packed_value, pos, bytes_per_dim)
        } else {
            util::arraycopy(saved, 0, &mut self.min_packed_value, pos, bytes_per_dim)
        }
    }

    fn size_from_balanced_tree(
        &self,
        left_most_leaf_node: i32,
        right_most_leaf_node: i32,
    ) -> Result<i64> {
        let reader = &self.reader;
        // number of points that need to be distributed between leaves, one
        // per leaf
        let extra_points = rucene::error::to_int_exact(
            i64::from(reader.config.max_points_in_leaf_node) * i64::from(reader.leaf_node_offset)
                - reader.point_count,
        )?;
        debug_assert!(
            extra_points < reader.leaf_node_offset,
            "point excess should be lower than leafNodeOffset"
        );
        // offset where we stop adding one point to the leaves
        let node_offset = reader.leaf_node_offset - extra_points;
        let mut count = 0i64;
        for node in left_most_leaf_node..=right_most_leaf_node {
            // offsetPosition provides which extra point will be added to this
            // node
            if Self::balance_tree_node_position(
                0,
                reader.leaf_node_offset,
                node - reader.leaf_node_offset,
                0,
                0,
            ) < node_offset
            {
                count += i64::from(reader.config.max_points_in_leaf_node);
            } else {
                count += i64::from(reader.config.max_points_in_leaf_node) - 1;
            }
        }
        Ok(count)
    }

    fn balance_tree_node_position(
        min_node: i32,
        max_node: i32,
        node: i32,
        position: i32,
        level: i32,
    ) -> i32 {
        if max_node - min_node == 1 {
            return position;
        }
        let mid = ((min_node + max_node + 1) as u32 >> 1) as i32;
        if mid > node {
            Self::balance_tree_node_position(min_node, mid, node, position, level + 1)
        } else {
            Self::balance_tree_node_position(
                mid,
                max_node,
                node,
                position + (1 << level),
                level + 1,
            )
        }
    }

    fn get_num_leaves_slow(&self, node: i32) -> i32 {
        let leaf_node_offset = self.reader.leaf_node_offset;
        if node >= 2 * leaf_node_offset {
            0
        } else if node >= leaf_node_offset {
            1
        } else {
            let left_count = self.get_num_leaves_slow(node * 2);
            let right_count = self.get_num_leaves_slow(node * 2 + 1);
            left_count + right_count
        }
    }

    /// Port of `addAll`.
    fn add_all(&mut self, visitor: &mut dyn IntersectVisitor, mut grown: bool) -> Result<()> {
        if !grown {
            let size = self.size()?;
            if size <= i64::from(i32::MAX) {
                visitor.grow(size as i32)?;
                grown = true;
            }
        }
        if self.is_leaf_node() {
            // Leaf node
            let mut scratch = JavaScratch::new();
            let fp =
                self.reader.leaf_block_fps[(self.node_id - self.reader.leaf_node_offset) as usize];
            self.input.seek(fp)?;
            scratch.read_line(self.input.as_mut())?;
            let count = parse_int_prefix(&scratch, BLOCK_COUNT)?;
            for _ in 0..count {
                scratch.read_line(self.input.as_mut())?;
                visitor.visit(parse_int_prefix(&scratch, BLOCK_DOC_ID)?)?;
            }
        } else {
            self.push_left()?;
            self.add_all(visitor, grown)?;
            self.pop(true)?;
            self.push_right()?;
            self.add_all(visitor, grown)?;
            self.pop(false)?;
        }
        Ok(())
    }

    /// Port of `readDocIDs`.
    fn read_doc_ids(&mut self, block_fp: i64) -> Result<i32> {
        let mut scratch = JavaScratch::new();
        self.input.seek(block_fp)?;
        scratch.read_line(self.input.as_mut())?;
        let count = parse_int_prefix(&scratch, BLOCK_COUNT)?;
        // `for (int i = 0; i < count; i++)`: no iteration for a negative
        // count, which is returned as is.
        for i in 0..count.max(0) as usize {
            scratch.read_line(self.input.as_mut())?;
            let doc = parse_int_prefix(&scratch, BLOCK_DOC_ID)?;
            let length = self.scratch_doc_ids.len();
            *self.scratch_doc_ids.get_mut(i).ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ArrayIndexOutOfBoundsException",
                    format!("Index {i} out of bounds for length {length}"),
                )
            })? = doc;
        }
        Ok(count)
    }
}

/// `parseInt(scratch, prefix)`: `Integer.parseInt(stripPrefix(...))`.
fn parse_int_prefix(scratch: &JavaScratch, prefix: &[u8]) -> Result<i32> {
    debug_assert!(scratch.starts_with(prefix));
    parse_int(&scratch.new_string(prefix.len())?)
}

impl PointTree for SimpleTextPointTree {
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        let mut index = SimpleTextPointTree::new(
            Arc::clone(&self.reader),
            self.input.clone_input()?,
            self.node_id,
            self.level,
            &self.min_packed_value,
            &self.max_packed_value,
        );
        if !self.is_leaf_node() {
            // copy node data
            index.split_dims[self.level] = self.split_dims[self.level];
            index.split_dim_value_stack[self.level] =
                self.split_dim_value_stack[self.level].clone();
        }
        Ok(Box::new(index))
    }

    fn move_to_child(&mut self) -> Result<bool> {
        if self.is_leaf_node() {
            return Ok(false);
        }
        self.push_left()?;
        Ok(true)
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        if self.node_id != self.root_node && (self.node_id & 1) == 0 {
            self.pop(true)?;
            self.push_right()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        if self.node_id == self.root_node {
            return Ok(false);
        }
        self.pop((self.node_id & 1) == 0)?;
        Ok(true)
    }

    fn min_packed_value(&self) -> Result<&[u8]> {
        Ok(&self.min_packed_value)
    }

    fn max_packed_value(&self) -> Result<&[u8]> {
        Ok(&self.max_packed_value)
    }

    fn size(&self) -> Result<i64> {
        let leaf_node_offset = self.reader.leaf_node_offset;
        let mut left_most_leaf_node = self.node_id;
        while left_most_leaf_node < leaf_node_offset {
            left_most_leaf_node *= 2;
        }
        let mut right_most_leaf_node = self.node_id;
        while right_most_leaf_node < leaf_node_offset {
            right_most_leaf_node = right_most_leaf_node * 2 + 1;
        }
        let num_leaves = if right_most_leaf_node >= left_most_leaf_node {
            // both are on the same level
            right_most_leaf_node - left_most_leaf_node + 1
        } else {
            // left is one level deeper than right
            right_most_leaf_node - left_most_leaf_node + 1 + leaf_node_offset
        };
        debug_assert_eq!(num_leaves, self.get_num_leaves_slow(self.node_id));
        self.size_from_balanced_tree(left_most_leaf_node, right_most_leaf_node)
    }

    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        self.add_all(visitor, false)
    }

    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        if self.is_leaf_node() {
            // Leaf node
            let leaf_id = (self.node_id - self.reader.leaf_node_offset) as usize;

            // Leaf node; scan and filter all points in this block:
            let count = self.read_doc_ids(self.reader.leaf_block_fps[leaf_id])?;

            // Again, this time reading values and checking with the visitor
            visitor.grow(count)?;
            // NOTE: we don't do prefix coding, so we ignore commonPrefixLengths
            let packed_len = self.reader.config.packed_bytes_length() as usize;
            debug_assert_eq!(self.scratch_packed_value.len(), packed_len);
            let mut scratch = JavaScratch::new();
            for i in 0..count.max(0) as usize {
                scratch.read_line(self.input.as_mut())?;
                debug_assert!(scratch.starts_with(BLOCK_VALUE));
                let br = util::from_bytes_ref_string(&scratch.new_string(BLOCK_VALUE.len())?)?;
                debug_assert_eq!(br.length, packed_len);
                util::arraycopy(
                    br.slice(),
                    0,
                    &mut self.scratch_packed_value,
                    0,
                    packed_len as i64,
                )?;
                let doc = self.scratch_doc_ids[i];
                visitor.visit_with_value(doc, &self.scratch_packed_value)?;
            }
        } else {
            self.push_left()?;
            self.visit_doc_values(visitor)?;
            self.pop(true)?;
            self.push_right()?;
            self.visit_doc_values(visitor)?;
            self.pop(false)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simpletext::test_support::unchecked;
    use rucene::index::Relation;
    use rucene::store::{ByteBuffersDataInput, ByteBuffersIndexInput};

    /// A reader over `data` with the given geometry.
    fn reader(
        num_index_dims: i32,
        leaf_count: usize,
        split_packed_values: Vec<u8>,
        min: Vec<u8>,
        max: Vec<u8>,
        data: &str,
    ) -> SimpleTextBKDReader {
        let input = ByteBuffersIndexInput::new(
            ByteBuffersDataInput::new(vec![data.as_bytes().to_vec()]).unwrap(),
            "dim",
        );
        let shared = Arc::new(SharedDataIn {
            input: Mutex::new(Some(Box::new(input))),
        });
        SimpleTextBKDReader::new(
            shared,
            num_index_dims,
            num_index_dims,
            4,
            1,
            vec![0; leaf_count],
            split_packed_values,
            min,
            max,
            1,
            1,
        )
        .unwrap()
    }

    #[derive(Default)]
    struct Recorder {
        grown: Vec<i32>,
    }

    impl IntersectVisitor for Recorder {
        fn visit(&mut self, _doc_id: i32) -> Result<()> {
            Ok(())
        }
        fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
            Ok(())
        }
        fn compare(&self, _min: &[u8], _max: &[u8]) -> Result<Relation> {
            Ok(Relation::CellCrossesQuery)
        }
        fn grow(&mut self, count: i32) -> Result<()> {
            self.grown.push(count);
            Ok(())
        }
    }

    /// Regression (rmp #526): `readDocIDs` loops `i < count` (no iteration
    /// for a negative count) and returns the count, which `visitDocValues`
    /// hands to `grow` (`SimpleTextBKDReader.java:324-358`); the port read
    /// `count as usize` lines.
    #[test]
    fn negative_block_count_reads_nothing() {
        let reader = reader(1, 1, vec![0], vec![0], vec![9], "block count -2\n");
        let mut tree = reader.point_tree().unwrap();
        let mut visitor = Recorder::default();
        tree.visit_doc_values(&mut visitor).unwrap();
        assert_eq!(visitor.grown, vec![-2]);
    }

    /// Regression (rmp #526): `pushLeft` asserts that the split value is
    /// not above the cell's max (`SimpleTextBKDReader.java:154-167`); the
    /// port omitted the assertion.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(
        expected = "config.bytesPerDim()=1 splitDim=0 config.numIndexDims()=1 config.numDims=1"
    )]
    fn push_left_asserts_split_value_within_the_cell() {
        let reader = reader(1, 2, vec![0, 5], vec![0], vec![3], "");
        let mut tree = reader.point_tree().unwrap();
        let _ = tree.move_to_child();
    }

    /// Regression (rmp #526): a split dimension past the packed value fails
    /// in `System.arraycopy` (the port panicked on the slice).
    #[test]
    fn corrupt_split_dimension_fails_in_arraycopy() {
        let reader = reader(2, 2, vec![0, 0, 9, 0], vec![0, 0], vec![9, 9], "");
        let mut tree = reader.point_tree().unwrap();
        let error = tree.move_to_child().unwrap_err();
        assert_eq!(
            unchecked(&error),
            (
                "java.lang.ArrayIndexOutOfBoundsException",
                "arraycopy: last source index 10 out of bounds for byte[2]".to_string()
            )
        );
    }
}
