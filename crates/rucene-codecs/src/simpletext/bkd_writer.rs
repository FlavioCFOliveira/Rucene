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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextBKDWriter`.

use std::sync::Arc;

use rucene::codecs::codec_util::check_footer_with_prior;
use rucene::error::{LuceneError, Result};
use rucene::index::documents_writer::SharedDirectory;
use rucene::index::{IntersectVisitor, MutablePointTree, Relation};
use rucene::store::{Directory, IndexOutput, TrackingDirectoryWrapper};
use rucene::util::bkd::{
    BKDConfig, BKDRadixSelector, HeapPointWriter, MutablePointTreeReaderUtils, OfflinePointWriter,
    PathSlice, PointValue, PointWriter,
};
use rucene::util::{ArrayUtil, BytesRef, FixedBitSet, NumericUtils};

use super::points_writer::{
    BLOCK_COUNT, BLOCK_DOC_ID, BLOCK_FP, BLOCK_VALUE, BYTES_PER_DIM, DOC_COUNT, INDEX_COUNT,
    MAX_LEAF_POINTS, MAX_VALUE, MIN_VALUE, NUM_DATA_DIMS, NUM_INDEX_DIMS, POINT_COUNT, SPLIT_COUNT,
    SPLIT_DIM, SPLIT_VALUE,
};
use super::util;

/// `SimpleTextBKDWriter.CODEC_NAME`.
pub const CODEC_NAME: &str = "BKD";
/// `SimpleTextBKDWriter.VERSION_START`.
pub const VERSION_START: i32 = 0;
/// `SimpleTextBKDWriter.VERSION_COMPRESSED_DOC_IDS`.
pub const VERSION_COMPRESSED_DOC_IDS: i32 = 1;
/// `SimpleTextBKDWriter.VERSION_COMPRESSED_VALUES`.
pub const VERSION_COMPRESSED_VALUES: i32 = 2;
/// `SimpleTextBKDWriter.VERSION_IMPLICIT_SPLIT_DIM_1D`.
pub const VERSION_IMPLICIT_SPLIT_DIM_1D: i32 = 3;
/// `SimpleTextBKDWriter.VERSION_CURRENT`.
pub const VERSION_CURRENT: i32 = VERSION_IMPLICIT_SPLIT_DIM_1D;

/// Default maximum heap to use, before spilling to (slower) disk: Java's
/// `float` `16.0f`.
pub const DEFAULT_MAX_MB_SORT_IN_HEAP: f32 = 16.0;

/// Forked from `BKDWriter` and simplified/specialized for SimpleText's usage.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextBKDWriter`.
pub struct SimpleTextBKDWriter {
    /// How many dimensions we are storing at the leaf (data) nodes.
    config: BKDConfig,

    tracking_dir: Arc<TrackingDirectoryWrapper>,
    temp_dir: Arc<dyn Directory>,
    temp_file_name_prefix: String,
    /// Java keeps the constructor argument in a field that nothing reads.
    #[allow(dead_code)]
    max_mb_sort_in_heap: f64,

    scratch_diff: Vec<u8>,
    scratch1: Vec<u8>,
    scratch2: Vec<u8>,
    common_prefix_lengths: Vec<usize>,

    docs_seen: FixedBitSet,

    point_writer: Option<Box<dyn PointWriter>>,
    finished: bool,

    /// The spill file's name, Java's `tempInput`.
    temp_input: Option<String>,

    max_points_sort_in_heap: i32,

    /// Minimum per-dim values, packed.
    min_packed_value: Vec<u8>,

    /// Maximum per-dim values, packed.
    max_packed_value: Vec<u8>,

    point_count: i64,

    /// An upper bound on how many points the caller will add (includes
    /// deletions).
    total_point_count: i64,

    max_doc: i32,
}

impl std::fmt::Debug for SimpleTextBKDWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleTextBKDWriter")
            .field("point_count", &self.point_count)
            .finish_non_exhaustive()
    }
}

/// `Arrays.compareUnsigned` of two `bytes_per_dim` ranges.
fn compare_unsigned(
    a: &[u8],
    a_off: usize,
    b: &[u8],
    b_off: usize,
    len: usize,
) -> std::cmp::Ordering {
    a[a_off..a_off + len].cmp(&b[b_off..b_off + len])
}

/// `Arrays.mismatch` of two ranges: the first differing index, or `None`.
fn mismatch(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter().zip(b.iter()).position(|(x, y)| x != y).or({
        if a.len() == b.len() {
            None
        } else {
            Some(a.len().min(b.len()))
        }
    })
}

impl SimpleTextBKDWriter {
    /// Port of `SimpleTextBKDWriter(int, Directory, String, BKDConfig, double,
    /// long)`.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalArgumentException`s for bad parameters.
    pub fn new(
        max_doc: i32,
        temp_dir: Arc<dyn Directory>,
        temp_file_name_prefix: &str,
        config: BKDConfig,
        max_mb_sort_in_heap: f64,
        total_point_count: i64,
    ) -> Result<Self> {
        Self::verify_params(max_mb_sort_in_heap, total_point_count)?;
        // We use tracking dir to deal with removing files on exception, so each
        // place that creates temp files doesn't need crazy try/finally/sucess
        // logic:
        let tracking_dir = Arc::new(TrackingDirectoryWrapper::new(Box::new(SharedDirectory(
            temp_dir,
        ))));

        let bytes_per_dim = config.bytes_per_dim as usize;
        let packed_bytes_length = config.packed_bytes_length() as usize;
        let packed_index_bytes_length = config.packed_index_bytes_length() as usize;

        // Maximum number of points we hold in memory at any time
        let max_points_sort_in_heap = ((max_mb_sort_in_heap * 1024.0 * 1024.0)
            / f64::from(config.bytes_per_doc() * config.num_dims))
            as i32;

        // Finally, we must be able to hold at least the leaf node in heap
        // during build:
        if max_points_sort_in_heap < config.max_points_in_leaf_node {
            return Err(LuceneError::IllegalArgument(format!(
                "maxMBSortInHeap={} only allows for maxPointsSortInHeap={max_points_sort_in_heap}, \
                 but this is less than config.maxPointsInLeafNode()={}; either increase \
                 maxMBSortInHeap or decrease config.maxPointsInLeafNode()",
                rucene::util::java_fmt::double_to_string(max_mb_sort_in_heap),
                config.max_points_in_leaf_node
            )));
        }

        Ok(Self {
            temp_dir: Arc::clone(&tracking_dir) as Arc<dyn Directory>,
            tracking_dir,
            temp_file_name_prefix: temp_file_name_prefix.to_string(),
            total_point_count,
            max_doc,
            docs_seen: FixedBitSet::new(max_doc.max(0) as usize),
            scratch_diff: vec![0u8; bytes_per_dim],
            scratch1: vec![0u8; packed_bytes_length],
            scratch2: vec![0u8; packed_bytes_length],
            common_prefix_lengths: vec![0usize; config.num_dims as usize],
            min_packed_value: vec![0u8; packed_index_bytes_length],
            max_packed_value: vec![0u8; packed_index_bytes_length],
            max_points_sort_in_heap,
            max_mb_sort_in_heap,
            config,
            point_writer: None,
            finished: false,
            temp_input: None,
            point_count: 0,
        })
    }

    /// Port of `SimpleTextBKDWriter.verifyParams`.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalArgumentException` for a negative argument.
    pub fn verify_params(max_mb_sort_in_heap: f64, total_point_count: i64) -> Result<()> {
        if max_mb_sort_in_heap < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxMBSortInHeap must be >= 0.0 (got: {})",
                rucene::util::java_fmt::double_to_string(max_mb_sort_in_heap)
            )));
        }
        if total_point_count < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "totalPointCount must be >=0 (got: {total_point_count})"
            )));
        }
        Ok(())
    }

    /// Port of `SimpleTextBKDWriter.add`.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalArgumentException` and
    /// `IllegalStateException`, and propagates the point writer's errors.
    pub fn add(&mut self, packed_value: &[u8], doc_id: i32) -> Result<()> {
        let config = &self.config;
        if packed_value.len() != config.packed_bytes_length() as usize {
            return Err(LuceneError::IllegalArgument(format!(
                "packedValue should be length={} (got: {})",
                config.packed_bytes_length(),
                packed_value.len()
            )));
        }
        if self.point_count >= self.total_point_count {
            return Err(LuceneError::illegal_state(format!(
                "totalPointCount={} was passed when we were created, but we just hit {} values",
                self.total_point_count,
                self.point_count + 1
            )));
        }
        let index_len = config.packed_index_bytes_length() as usize;
        let bytes_per_dim = config.bytes_per_dim as usize;
        if self.point_count == 0 {
            debug_assert!(
                self.point_writer.is_none(),
                "Point writer is already initialized"
            );
            // total point count is an estimation but the final point count
            // must be equal or lower to that number.
            if self.total_point_count > i64::from(self.max_points_sort_in_heap) {
                let writer = OfflinePointWriter::new(
                    self.config.clone(),
                    Arc::clone(&self.temp_dir),
                    &self.temp_file_name_prefix,
                    "spill",
                )?;
                self.temp_input = Some(writer.name().to_string());
                self.point_writer = Some(Box::new(writer));
            } else {
                self.point_writer = Some(Box::new(HeapPointWriter::new(
                    self.config.clone(),
                    rucene::error::to_int_exact(self.total_point_count)? as usize,
                )?));
            }
            self.min_packed_value[..index_len].copy_from_slice(&packed_value[..index_len]);
            self.max_packed_value[..index_len].copy_from_slice(&packed_value[..index_len]);
        } else {
            for dim in 0..self.config.num_index_dims as usize {
                let offset = dim * bytes_per_dim;
                if compare_unsigned(
                    packed_value,
                    offset,
                    &self.min_packed_value,
                    offset,
                    bytes_per_dim,
                )
                .is_lt()
                {
                    self.min_packed_value[offset..offset + bytes_per_dim]
                        .copy_from_slice(&packed_value[offset..offset + bytes_per_dim]);
                }
                if compare_unsigned(
                    packed_value,
                    offset,
                    &self.max_packed_value,
                    offset,
                    bytes_per_dim,
                )
                .is_gt()
                {
                    self.max_packed_value[offset..offset + bytes_per_dim]
                        .copy_from_slice(&packed_value[offset..offset + bytes_per_dim]);
                }
            }
        }
        self.point_writer
            .as_mut()
            .expect("INVARIANT: created with the first point")
            .append(&PointValue::new(packed_value.to_vec(), doc_id))?;
        self.point_count += 1;
        self.set_doc_seen(doc_id)?;
        Ok(())
    }

    /// `docsSeen.set(docID)`.
    fn set_doc_seen(&mut self, doc_id: i32) -> Result<()> {
        if doc_id < 0 || doc_id as usize >= self.docs_seen.length() {
            return Err(LuceneError::illegal_state(format!(
                "index={doc_id}, numBits={}",
                self.docs_seen.length()
            )));
        }
        self.docs_seen.set(doc_id as usize);
        Ok(())
    }

    /// How many points have been added so far.
    pub fn get_point_count(&self) -> i64 {
        self.point_count
    }

    /// Write a field from a [`MutablePointTree`]. This way of writing points
    /// is faster than regular writes with `add` since there is opportunity
    /// for reordering points before writing them to disk. This method does
    /// not use transient disk in order to reorder points.
    ///
    /// Port of `SimpleTextBKDWriter.writeField`.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalStateException`s and propagates the output's
    /// errors.
    pub fn write_field(
        &mut self,
        out: &mut dyn IndexOutput,
        field_name: &str,
        reader: &mut dyn MutablePointTree,
    ) -> Result<i64> {
        if self.config.num_index_dims == 1 {
            self.write_field_1_dim(out, field_name, reader)
        } else {
            self.write_field_n_dims(out, field_name, reader)
        }
    }

    /// In the 2+D case, we recursively pick the split dimension, compute the
    /// median value and partition other values around it.
    fn write_field_n_dims(
        &mut self,
        out: &mut dyn IndexOutput,
        _field_name: &str,
        values: &mut dyn MutablePointTree,
    ) -> Result<i64> {
        if self.point_count != 0 {
            return Err(LuceneError::illegal_state(
                "cannot mix add and writeField".to_string(),
            ));
        }

        // Catch user silliness:
        if self.finished {
            return Err(LuceneError::illegal_state("already finished".to_string()));
        }

        // Mark that we already finished:
        self.finished = true;

        self.point_count = values.size()?;
        let mut count_per_leaf = self.point_count;
        let mut inner_node_count = 1i64;

        while count_per_leaf > i64::from(self.config.max_points_in_leaf_node) {
            count_per_leaf = (count_per_leaf + 1) / 2;
            inner_node_count *= 2;
        }

        let num_leaves = rucene::error::to_int_exact(inner_node_count)?;

        self.check_max_leaf_node_count(num_leaves)?;

        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let mut split_packed_values = vec![0u8; num_leaves as usize * (bytes_per_dim + 1)];
        let mut leaf_block_fps = vec![0i64; num_leaves as usize];

        // compute the min/max for this slice
        self.min_packed_value.fill(0xff);
        self.max_packed_value.fill(0);
        for i in 0..rucene::error::to_int_exact(self.point_count)? {
            let value = values.value(i).into_owned();
            for dim in 0..self.config.num_index_dims as usize {
                let offset = dim * bytes_per_dim;
                if compare_unsigned(
                    &value,
                    offset,
                    &self.min_packed_value,
                    offset,
                    bytes_per_dim,
                )
                .is_lt()
                {
                    self.min_packed_value[offset..offset + bytes_per_dim]
                        .copy_from_slice(&value[offset..offset + bytes_per_dim]);
                }
                if compare_unsigned(
                    &value,
                    offset,
                    &self.max_packed_value,
                    offset,
                    bytes_per_dim,
                )
                .is_gt()
                {
                    self.max_packed_value[offset..offset + bytes_per_dim]
                        .copy_from_slice(&value[offset..offset + bytes_per_dim]);
                }
            }

            self.set_doc_seen(values.doc_id(i))?;
        }

        let min = self.min_packed_value.clone();
        let max = self.max_packed_value.clone();
        let mut spare = vec![0i32; self.config.max_points_in_leaf_node as usize];
        let to = rucene::error::to_int_exact(self.point_count)? as usize;
        self.build_mutable(
            1,
            num_leaves,
            values,
            0,
            to,
            out,
            &min,
            &max,
            &mut split_packed_values,
            &mut leaf_block_fps,
            &mut spare,
        )?;

        let index_fp = out.file_pointer();
        self.write_index(
            out,
            &leaf_block_fps,
            &split_packed_values,
            rucene::error::to_int_exact(count_per_leaf)?,
        )?;
        Ok(index_fp)
    }

    /// In the 1D case, we can simply sort points in ascending order and use
    /// the same writing logic as we use at merge time.
    fn write_field_1_dim(
        &mut self,
        out: &mut dyn IndexOutput,
        _field_name: &str,
        reader: &mut dyn MutablePointTree,
    ) -> Result<i64> {
        let size = rucene::error::to_int_exact(reader.size()?)? as usize;
        MutablePointTreeReaderUtils::sort(&self.config, self.max_doc, reader, 0, size)?;

        let mut one_dim_writer = OneDimensionBKDWriter::new(self)?;

        let mut visitor = OneDimVisitor {
            writer: self,
            one_dim_writer: &mut one_dim_writer,
            out,
        };
        reader.visit_doc_values(&mut visitor)?;
        let OneDimVisitor {
            writer,
            one_dim_writer,
            out,
        } = visitor;

        one_dim_writer.finish(writer, out)
    }

    /// Port of the private `rotateToTree`.
    fn rotate_to_tree(
        &self,
        node_id: usize,
        offset: usize,
        count: usize,
        index: &mut [u8],
        leaf_block_start_values: &[Vec<u8>],
    ) {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        if count == 1 {
            // Leaf index node
            let at = node_id * (1 + bytes_per_dim) + 1;
            index[at..at + bytes_per_dim]
                .copy_from_slice(&leaf_block_start_values[offset][..bytes_per_dim]);
        } else if count > 1 {
            // Internal index node: binary partition of count
            let mut count_at_level = 1usize;
            let mut total_count = 0usize;
            loop {
                let count_left = count - total_count;
                if count_left <= count_at_level {
                    // This is the last level, possibly partially filled:
                    let last_left_count = (count_at_level / 2).min(count_left);
                    // `(totalCount - 1) / 2` in `int` arithmetic, which rounds
                    // toward zero.
                    let left_half = ((total_count as i64 - 1) / 2) as usize + last_left_count;

                    let root_offset = offset + left_half;

                    let at = node_id * (1 + bytes_per_dim) + 1;
                    index[at..at + bytes_per_dim]
                        .copy_from_slice(&leaf_block_start_values[root_offset][..bytes_per_dim]);

                    // TODO: we could optimize/specialize, when we know it's
                    // simply fully balanced binary tree under here, to save
                    // this while loop on each recursion

                    // Recurse left
                    self.rotate_to_tree(
                        2 * node_id,
                        offset,
                        left_half,
                        index,
                        leaf_block_start_values,
                    );

                    // Recurse right
                    self.rotate_to_tree(
                        2 * node_id + 1,
                        root_offset + 1,
                        count - left_half - 1,
                        index,
                        leaf_block_start_values,
                    );
                    return;
                }
                total_count += count_at_level;
                count_at_level *= 2;
            }
        } else {
            debug_assert_eq!(count, 0);
        }
    }

    /// Port of the private `checkMaxLeafNodeCount`.
    fn check_max_leaf_node_count(&self, num_leaves: i32) -> Result<()> {
        if (1 + i64::from(self.config.bytes_per_dim)) * i64::from(num_leaves)
            > ArrayUtil::MAX_ARRAY_LENGTH as i64
        {
            return Err(LuceneError::illegal_state(format!(
                "too many nodes; increase config.maxPointsInLeafNode() (currently {}) and reindex",
                self.config.max_points_in_leaf_node
            )));
        }
        Ok(())
    }

    /// Writes the BKD tree to the provided [`IndexOutput`] and returns the
    /// file offset where index was written.
    ///
    /// Port of `SimpleTextBKDWriter.finish`.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalStateException`s and propagates the writing
    /// errors; on an error the temporary files created are deleted.
    pub fn finish(&mut self, out: &mut dyn IndexOutput) -> Result<i64> {
        // Catch user silliness:
        if self.point_count == 0 {
            return Err(LuceneError::illegal_state(
                "must index at least one point".to_string(),
            ));
        }

        // Catch user silliness:
        if self.finished {
            return Err(LuceneError::illegal_state("already finished".to_string()));
        }

        // mark as finished
        self.finished = true;

        let mut point_writer = self
            .point_writer
            .take()
            .expect("INVARIANT: created with the first point");
        point_writer.close()?;
        let points = PathSlice::of(point_writer, 0, self.point_count);
        // clean up pointers
        self.temp_input = None;

        let mut count_per_leaf = self.point_count;
        let mut inner_node_count = 1i64;

        while count_per_leaf > i64::from(self.config.max_points_in_leaf_node) {
            count_per_leaf = (count_per_leaf + 1) / 2;
            inner_node_count *= 2;
        }

        let num_leaves = inner_node_count as i32;

        self.check_max_leaf_node_count(num_leaves)?;

        // NOTE: we could save the 1+ here, to use a bit less heap at search
        // time, but then we'd need a somewhat costly check at each step of the
        // recursion to recompute the split dim:

        // Indexed by nodeID, but first (root) nodeID is 1. We do 1+ because the
        // lead byte at each recursion says which dim we split on.
        let split_len = i64::from(num_leaves) * i64::from(1 + self.config.bytes_per_dim);
        let split_len = rucene::error::to_int_exact(split_len)?;
        let mut split_packed_values = vec![0u8; split_len as usize];

        // +1 because leaf count is power of 2 (e.g. 8), and innerNodeCount is
        // power of 2 minus 1 (e.g. 7)
        let mut leaf_block_fps = vec![0i64; num_leaves as usize];

        // Make sure the math above "worked":
        debug_assert!(
            self.point_count / i64::from(num_leaves)
                <= i64::from(self.config.max_points_in_leaf_node),
            "pointCount={} numLeaves={num_leaves} config.maxPointsInLeafNode()={}",
            self.point_count,
            self.config.max_points_in_leaf_node
        );

        // We re-use the selector so we do not need to create an object every
        // time.
        let mut radix_selector = BKDRadixSelector::new(
            self.config.clone(),
            self.max_points_sort_in_heap.max(0) as usize,
            Arc::clone(&self.temp_dir),
            &self.temp_file_name_prefix,
        );

        let min = self.min_packed_value.clone();
        let max = self.max_packed_value.clone();
        let mut spare = vec![0i32; self.config.max_points_in_leaf_node as usize];
        let result = self.build_path_slice(
            1,
            num_leaves,
            points,
            out,
            &mut radix_selector,
            &min,
            &max,
            &mut split_packed_values,
            &mut leaf_block_fps,
            &mut spare,
        );
        if let Err(error) = result {
            for name in self.tracking_dir.get_created_files() {
                let _ = self.temp_dir.delete_file(&name);
            }
            return Err(error);
        }
        // If no exception, we should have cleaned everything up:
        debug_assert!(self.tracking_dir.get_created_files().is_empty());

        // Write index:
        let index_fp = out.file_pointer();
        self.write_index(
            out,
            &leaf_block_fps,
            &split_packed_values,
            rucene::error::to_int_exact(count_per_leaf)?,
        )?;
        Ok(index_fp)
    }

    /// Subclass can change how it writes the index.
    ///
    /// Port of the private `writeIndex`.
    fn write_index(
        &self,
        out: &mut dyn IndexOutput,
        leaf_block_fps: &[i64],
        split_packed_values: &[u8],
        max_points_in_leaf_node: i32,
    ) -> Result<()> {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        line(out, NUM_DATA_DIMS, &self.config.num_dims.to_string())?;
        line(out, NUM_INDEX_DIMS, &self.config.num_index_dims.to_string())?;
        line(out, BYTES_PER_DIM, &self.config.bytes_per_dim.to_string())?;
        line(out, MAX_LEAF_POINTS, &max_points_in_leaf_node.to_string())?;
        line(out, INDEX_COUNT, &leaf_block_fps.len().to_string())?;
        line(
            out,
            MIN_VALUE,
            &BytesRef::new(self.min_packed_value.clone()).to_string(),
        )?;
        line(
            out,
            MAX_VALUE,
            &BytesRef::new(self.max_packed_value.clone()).to_string(),
        )?;
        line(out, POINT_COUNT, &self.point_count.to_string())?;
        line(out, DOC_COUNT, &self.docs_seen.cardinality().to_string())?;

        for fp in leaf_block_fps {
            line(out, BLOCK_FP, &fp.to_string())?;
        }

        debug_assert_eq!(split_packed_values.len() % (1 + bytes_per_dim), 0);
        let count = split_packed_values.len() / (1 + bytes_per_dim);
        debug_assert_eq!(count, leaf_block_fps.len());

        line(out, SPLIT_COUNT, &count.to_string())?;

        for i in 0..count {
            line(
                out,
                SPLIT_DIM,
                &split_packed_values[i * (1 + bytes_per_dim)].to_string(),
            )?;
            let start = 1 + i * (1 + bytes_per_dim);
            line(
                out,
                SPLIT_VALUE,
                &BytesRef::new(split_packed_values[start..start + bytes_per_dim].to_vec())
                    .to_string(),
            )?;
        }
        Ok(())
    }

    /// Port of `writeLeafBlockDocs`.
    fn write_leaf_block_docs(
        out: &mut dyn IndexOutput,
        doc_ids: &[i32],
        start: usize,
        count: usize,
    ) -> Result<()> {
        line(out, BLOCK_COUNT, &count.to_string())?;
        for doc_id in &doc_ids[start..start + count] {
            line(out, BLOCK_DOC_ID, &doc_id.to_string())?;
        }
        Ok(())
    }

    /// Port of `writeLeafBlockPackedValues`: NOTE: we don't do prefix coding,
    /// so we ignore `commonPrefixLengths`.
    fn write_leaf_block_packed_values(
        out: &mut dyn IndexOutput,
        count: usize,
        packed_values: &mut dyn FnMut(usize) -> Vec<u8>,
    ) -> Result<()> {
        for i in 0..count {
            let packed_value = packed_values(i);
            line(out, BLOCK_VALUE, &BytesRef::new(packed_value).to_string())?;
        }
        Ok(())
    }

    /// Port of `SimpleTextBKDWriter.close`.
    ///
    /// # Errors
    ///
    /// Propagates the failure to close or delete the spill file.
    pub fn close(&mut self) -> Result<()> {
        if let Some(name) = self.temp_input.take() {
            // NOTE: this should only happen on exception, e.g. caller calls
            // close w/o calling finish:
            let closed = match self.point_writer.as_mut() {
                Some(writer) => writer.close(),
                None => Ok(()),
            };
            let deleted = self.temp_dir.delete_file(&name);
            closed?;
            deleted?;
        }
        Ok(())
    }

    /// Called on exception, to check whether the checksum is also corrupt in
    /// this source, and add that information (checksum matched or didn't) as
    /// a suppressed exception.
    ///
    /// Port of the private `verifyChecksum`.
    fn verify_checksum(
        &self,
        prior_exception: LuceneError,
        writer: &mut dyn PointWriter,
    ) -> LuceneError {
        if let Some(offline) = writer.as_any_mut().downcast_mut::<OfflinePointWriter>() {
            // We are reading from a temp file; go verify the checksum:
            let temp_file_name = offline.name().to_string();
            // `try (ChecksumIndexInput in = ...)`: closed on every exit.
            return match self.temp_dir.open_checksum_input(&temp_file_name) {
                Ok(mut input) => {
                    let error = check_footer_with_prior(&mut *input, prior_exception);
                    // A failure closing it is suppressed into `error` in Java, dropped
                    // here (see `close_after`).
                    let _ = rucene::store::IndexInput::close(input.as_mut());
                    error
                }
                Err(error) => error,
            };
        }

        // We are reading from heap; nothing to add:
        prior_exception
    }

    /// Port of the protected `split`: find which dim has the largest span so
    /// we can split on it.
    fn split(&mut self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<usize> {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let mut split_dim: Option<usize> = None;
        for dim in 0..self.config.num_index_dims as usize {
            NumericUtils::subtract(
                bytes_per_dim,
                dim,
                max_packed_value,
                min_packed_value,
                &mut self.scratch_diff,
            )?;
            if split_dim.is_none()
                || compare_unsigned(&self.scratch_diff, 0, &self.scratch1, 0, bytes_per_dim).is_gt()
            {
                let diff = self.scratch_diff.clone();
                self.scratch1[..bytes_per_dim].copy_from_slice(&diff[..bytes_per_dim]);
                split_dim = Some(dim);
            }
        }

        Ok(split_dim.expect("INVARIANT: there is at least one index dimension"))
    }

    /// Pull a partition back into heap once the point count is low enough
    /// while recursing.
    ///
    /// Port of the private `switchToHeap`.
    fn switch_to_heap(&self, source: &mut dyn PointWriter) -> Result<HeapPointWriter> {
        let count = source.count();
        let result = (|| -> Result<HeapPointWriter> {
            let mut writer = HeapPointWriter::new(self.config.clone(), count)?;
            {
                let mut reader = source.get_reader(0, count)?;
                for _ in 0..count {
                    let has_next = reader.next()?;
                    debug_assert!(has_next);
                    writer.append(reader.point_value())?;
                }
                reader.close()?;
            }
            writer.close()?;
            Ok(writer)
        })();
        result.map_err(|t| self.verify_checksum(t, source))
    }

    /// Recursively reorders the provided reader and writes the bkd-tree on
    /// the fly.
    ///
    /// Port of `build(int, int, MutablePointTree, int, int, IndexOutput,
    /// byte[], byte[], byte[], long[], int[])`.
    #[allow(clippy::too_many_arguments)]
    fn build_mutable(
        &mut self,
        node_id: i32,
        leaf_node_offset: i32,
        reader: &mut dyn MutablePointTree,
        from: usize,
        to: usize,
        out: &mut dyn IndexOutput,
        min_packed_value: &[u8],
        max_packed_value: &[u8],
        split_packed_values: &mut [u8],
        leaf_block_fps: &mut [i64],
        spare_doc_ids: &mut [i32],
    ) -> Result<()> {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let num_dims = self.config.num_dims as usize;
        if node_id >= leaf_node_offset {
            // leaf node
            let count = to - from;
            debug_assert!(count <= self.config.max_points_in_leaf_node as usize);

            // Compute common prefixes
            self.common_prefix_lengths.fill(bytes_per_dim);
            let first = reader.value(from as i32).into_owned();
            for i in from + 1..to {
                let value = reader.value(i as i32);
                for dim in 0..num_dims {
                    let offset = dim * bytes_per_dim;
                    for j in 0..self.common_prefix_lengths[dim] {
                        if first[offset + j] != value[offset + j] {
                            self.common_prefix_lengths[dim] = j;
                            break;
                        }
                    }
                }
            }

            // Find the dimension that has the least number of unique bytes at
            // commonPrefixLengths[dim]
            let mut used_bytes: Vec<Option<FixedBitSet>> = (0..num_dims)
                .map(|dim| {
                    (self.common_prefix_lengths[dim] < bytes_per_dim).then(|| FixedBitSet::new(256))
                })
                .collect();
            for i in from + 1..to {
                for (dim, used) in used_bytes.iter_mut().enumerate() {
                    if let Some(used) = used {
                        let b = reader.byte_at(
                            i as i32,
                            (dim * bytes_per_dim + self.common_prefix_lengths[dim]) as i32,
                        );
                        used.set(usize::from(b));
                    }
                }
            }
            let mut sorted_dim = 0usize;
            let mut sorted_dim_cardinality = usize::MAX;
            for (dim, used) in used_bytes.iter().enumerate() {
                if let Some(used) = used {
                    let cardinality = used.cardinality();
                    if cardinality < sorted_dim_cardinality {
                        sorted_dim = dim;
                        sorted_dim_cardinality = cardinality;
                    }
                }
            }

            // sort by sortedDim
            let mut s1 = self.scratch1.clone();
            let mut s2 = self.scratch2.clone();
            MutablePointTreeReaderUtils::sort_by_dim(
                &self.config,
                sorted_dim,
                &self.common_prefix_lengths,
                reader,
                from,
                to,
                &mut s1,
                &mut s2,
            );

            // Save the block file pointer:
            leaf_block_fps[(node_id - leaf_node_offset) as usize] = out.file_pointer();

            // Write doc IDs
            for i in from..to {
                spare_doc_ids[i - from] = reader.doc_id(i as i32);
            }
            Self::write_leaf_block_docs(out, spare_doc_ids, 0, count)?;

            // Write the common prefixes:
            let first = reader.value(from as i32).into_owned();
            let packed_len = self.config.packed_bytes_length() as usize;
            self.scratch1[..packed_len].copy_from_slice(&first[..packed_len]);

            // Write the full values:
            let mut packed_values = |i: usize| reader.value((from + i) as i32).into_owned();
            Self::write_leaf_block_packed_values(out, count, &mut packed_values)?;
        } else {
            // inner node

            // compute the split dimension and partition around it
            let split_dim = self.split(min_packed_value, max_packed_value)?;
            let mid = (from + to + 1) >> 1;

            let mut common_prefix_len = bytes_per_dim;
            for i in 0..bytes_per_dim {
                if min_packed_value[split_dim * bytes_per_dim + i]
                    != max_packed_value[split_dim * bytes_per_dim + i]
                {
                    common_prefix_len = i;
                    break;
                }
            }
            let mut s1 = self.scratch1.clone();
            let mut s2 = self.scratch2.clone();
            MutablePointTreeReaderUtils::partition(
                &self.config,
                self.max_doc,
                split_dim,
                common_prefix_len,
                reader,
                from,
                to,
                mid,
                &mut s1,
                &mut s2,
            )?;

            // set the split value
            let address = node_id as usize * (1 + bytes_per_dim);
            split_packed_values[address] = split_dim as u8;
            let mid_value = reader.value(mid as i32).into_owned();
            let dim_off = split_dim * bytes_per_dim;
            split_packed_values[address + 1..address + 1 + bytes_per_dim]
                .copy_from_slice(&mid_value[dim_off..dim_off + bytes_per_dim]);

            let index_len = self.config.packed_index_bytes_length() as usize;
            let mut min_split_packed_value = min_packed_value[..index_len].to_vec();
            let mut max_split_packed_value = max_packed_value[..index_len].to_vec();
            min_split_packed_value[dim_off..dim_off + bytes_per_dim]
                .copy_from_slice(&mid_value[dim_off..dim_off + bytes_per_dim]);
            max_split_packed_value[dim_off..dim_off + bytes_per_dim]
                .copy_from_slice(&mid_value[dim_off..dim_off + bytes_per_dim]);

            // recurse
            self.build_mutable(
                node_id * 2,
                leaf_node_offset,
                reader,
                from,
                mid,
                out,
                min_packed_value,
                &max_split_packed_value,
                split_packed_values,
                leaf_block_fps,
                spare_doc_ids,
            )?;
            self.build_mutable(
                node_id * 2 + 1,
                leaf_node_offset,
                reader,
                mid,
                to,
                out,
                &min_split_packed_value,
                max_packed_value,
                split_packed_values,
                leaf_block_fps,
                spare_doc_ids,
            )?;
        }
        Ok(())
    }

    /// The array (sized numDims) of PathSlice describe the cell we have
    /// currently recursed to.
    ///
    /// Port of `build(int, int, BKDRadixSelector.PathSlice, IndexOutput,
    /// BKDRadixSelector, byte[], byte[], byte[], long[], int[])`.
    #[allow(clippy::too_many_arguments)]
    fn build_path_slice(
        &mut self,
        node_id: i32,
        leaf_node_offset: i32,
        points: PathSlice,
        out: &mut dyn IndexOutput,
        radix_selector: &mut BKDRadixSelector,
        min_packed_value: &[u8],
        max_packed_value: &[u8],
        split_packed_values: &mut [u8],
        leaf_block_fps: &mut [i64],
        spare_doc_ids: &mut [i32],
    ) -> Result<()> {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let num_dims = self.config.num_dims as usize;
        if node_id >= leaf_node_offset {
            // Leaf node: write block. We can write the block in any order so
            // by default we write it sorted by the dimension that has the
            // least number of unique bytes at commonPrefixLengths[dim], which
            // makes compression more efficient
            let mut writer = points.writer.borrow_mut();
            let mut switched;
            let heap_source: &mut HeapPointWriter = if writer.as_any_mut().is::<HeapPointWriter>() {
                writer
                    .as_any_mut()
                    .downcast_mut::<HeapPointWriter>()
                    .expect("INVARIANT: checked above")
            } else {
                // Adversarial cases can cause this, e.g. merging big segments
                // with most of the points deleted
                switched = self.switch_to_heap(writer.as_mut())?;
                &mut switched
            };

            let from = rucene::error::to_int_exact(points.start)? as usize;
            let to = rucene::error::to_int_exact(points.start + points.count)? as usize;

            // we store common prefix on scratch1
            self.compute_common_prefix_length(heap_source);

            let mut sorted_dim = 0usize;
            let mut sorted_dim_cardinality = usize::MAX;
            let mut used_bytes: Vec<Option<FixedBitSet>> = (0..num_dims)
                .map(|dim| {
                    (self.common_prefix_lengths[dim] < bytes_per_dim).then(|| FixedBitSet::new(256))
                })
                .collect();
            // Find the dimension to compress
            let heap_count = heap_source.count();
            for (dim, used) in used_bytes.iter_mut().enumerate() {
                let prefix = self.common_prefix_lengths[dim];
                if let Some(used) = used {
                    let offset = dim * bytes_per_dim;
                    for i in 0..heap_count {
                        let bucket = heap_source.byte_at(i, offset + prefix);
                        used.set(usize::from(bucket));
                    }
                    let cardinality = used.cardinality();
                    if cardinality < sorted_dim_cardinality {
                        sorted_dim = dim;
                        sorted_dim_cardinality = cardinality;
                    }
                }
            }

            // sort the chosen dimension
            radix_selector.heap_radix_sort(
                heap_source,
                from,
                to,
                sorted_dim,
                self.common_prefix_lengths[sorted_dim],
            );

            // Save the block file pointer:
            leaf_block_fps[(node_id - leaf_node_offset) as usize] = out.file_pointer();

            // Write docIDs first, as their own chunk, so that at intersect time
            // we can add all docIDs w/o loading the values:
            let count = to - from;
            debug_assert!(
                count > 0,
                "nodeID={node_id} leafNodeOffset={leaf_node_offset}"
            );
            // Write doc IDs
            for (i, doc_id) in spare_doc_ids.iter_mut().enumerate().take(count) {
                *doc_id = heap_source.get_packed_value_slice(from + i).doc_id;
            }
            Self::write_leaf_block_docs(out, spare_doc_ids, 0, count)?;

            // TODO: minor opto: we don't really have to write the actual
            // common prefixes, because BKDReader on recursing can regenerate
            // it for us from the index, much like how terms dict does so from
            // the FST:

            // Write the full values:
            let heap: &HeapPointWriter = heap_source;
            let mut packed_values = |i: usize| heap.get_packed_value_slice(from + i).packed;
            Self::write_leaf_block_packed_values(out, count, &mut packed_values)?;
        } else {
            // Inner node: partition/recurse

            let split_dim = if self.config.num_index_dims > 1 {
                self.split(min_packed_value, max_packed_value)?
            } else {
                0
            };

            debug_assert!(
                (node_id as usize) < split_packed_values.len(),
                "nodeID={node_id} splitValues.length={}",
                split_packed_values.len()
            );

            // How many points will be in the left tree:
            let right_count = points.count / 2;
            let left_count = points.count - right_count;

            let dim_off = split_dim * bytes_per_dim;
            let common_prefix_len = mismatch(
                &min_packed_value[dim_off..dim_off + bytes_per_dim],
                &max_packed_value[dim_off..dim_off + bytes_per_dim],
            )
            .unwrap_or(bytes_per_dim);

            let (split_value, [left, right]) = radix_selector.select(
                &points,
                points.start,
                points.start + points.count,
                points.start + left_count,
                split_dim,
                common_prefix_len,
            )?;
            drop(points);

            let address = node_id as usize * (1 + bytes_per_dim);
            split_packed_values[address] = split_dim as u8;
            split_packed_values[address + 1..address + 1 + bytes_per_dim]
                .copy_from_slice(&split_value[..bytes_per_dim]);

            let index_len = self.config.packed_index_bytes_length() as usize;
            let mut min_split_packed_value = min_packed_value[..index_len].to_vec();
            let mut max_split_packed_value = max_packed_value[..index_len].to_vec();
            min_split_packed_value[dim_off..dim_off + bytes_per_dim]
                .copy_from_slice(&split_value[..bytes_per_dim]);
            max_split_packed_value[dim_off..dim_off + bytes_per_dim]
                .copy_from_slice(&split_value[..bytes_per_dim]);

            // Recurse on left tree:
            self.build_path_slice(
                2 * node_id,
                leaf_node_offset,
                left,
                out,
                radix_selector,
                min_packed_value,
                &max_split_packed_value,
                split_packed_values,
                leaf_block_fps,
                spare_doc_ids,
            )?;

            // TODO: we could "tail recurse" here? have our parent discard its
            // refs as we recurse right?
            // Recurse on right tree:
            self.build_path_slice(
                2 * node_id + 1,
                leaf_node_offset,
                right,
                out,
                radix_selector,
                &min_split_packed_value,
                max_packed_value,
                split_packed_values,
                leaf_block_fps,
                spare_doc_ids,
            )?;
        }
        Ok(())
    }

    /// Port of the private `computeCommonPrefixLength`: over every point of
    /// `heap_point_writer`, from its first one, as Java's loop runs.
    fn compute_common_prefix_length(&mut self, heap_point_writer: &HeapPointWriter) {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let num_dims = self.config.num_dims as usize;
        self.common_prefix_lengths.fill(bytes_per_dim);
        let value = heap_point_writer.get_packed_value_slice(0);
        let packed_value = value.packed_value();
        for dim in 0..num_dims {
            let off = dim * bytes_per_dim;
            self.scratch1[off..off + bytes_per_dim]
                .copy_from_slice(&packed_value[off..off + bytes_per_dim]);
        }
        for i in 1..heap_point_writer.count() {
            let value = heap_point_writer.get_packed_value_slice(i);
            let packed_value = value.packed_value();
            for dim in 0..num_dims {
                if self.common_prefix_lengths[dim] != 0 {
                    let off = dim * bytes_per_dim;
                    let len = self.common_prefix_lengths[dim];
                    if let Some(j) = mismatch(
                        &self.scratch1[off..off + len],
                        &packed_value[off..off + len],
                    ) {
                        self.common_prefix_lengths[dim] = j;
                    }
                }
            }
        }
    }
}

impl Drop for SimpleTextBKDWriter {
    /// Java callers use the writer in try-with-resources; the port closes it
    /// on drop, ignoring the error a destructor cannot report.
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// A `name value` line.
fn line(out: &mut dyn IndexOutput, prefix: &[u8], value: &str) -> Result<()> {
    util::write_bytes(out, prefix)?;
    util::write_str(out, value)?;
    util::write_newline(out)
}

/// Port of the inner class `org.apache.lucene.codecs.simpletext.SimpleTextBKDWriter.OneDimensionBKDWriter`.
struct OneDimensionBKDWriter {
    leaf_block_fps: Vec<i64>,
    leaf_block_start_values: Vec<Vec<u8>>,
    leaf_values: Vec<u8>,
    leaf_docs: Vec<i32>,
    value_count: i64,
    leaf_count: usize,
    // for asserts
    last_packed_value: Vec<u8>,
    last_doc_id: i32,
}

impl OneDimensionBKDWriter {
    fn new(w: &mut SimpleTextBKDWriter) -> Result<Self> {
        if w.config.num_index_dims != 1 {
            return Err(LuceneError::UnsupportedOperation(format!(
                "config.numIndexDims() must be 1 but got {}",
                w.config.num_index_dims
            )));
        }
        if w.point_count != 0 {
            return Err(LuceneError::illegal_state(
                "cannot mix add and merge".to_string(),
            ));
        }

        // Catch user silliness:
        if w.finished {
            return Err(LuceneError::illegal_state("already finished".to_string()));
        }

        // Mark that we already finished:
        w.finished = true;

        let packed_len = w.config.packed_bytes_length() as usize;
        Ok(Self {
            leaf_block_fps: Vec::new(),
            leaf_block_start_values: Vec::new(),
            leaf_values: vec![0u8; w.config.max_points_in_leaf_node as usize * packed_len],
            leaf_docs: vec![0i32; w.config.max_points_in_leaf_node as usize],
            value_count: 0,
            leaf_count: 0,
            last_packed_value: vec![0u8; packed_len],
            last_doc_id: 0,
        })
    }

    fn add(
        &mut self,
        w: &mut SimpleTextBKDWriter,
        out: &mut dyn IndexOutput,
        packed_value: &[u8],
        doc_id: i32,
    ) -> Result<()> {
        let packed_len = w.config.packed_bytes_length() as usize;
        debug_assert!({
            let ord = self.value_count + self.leaf_count as i64;
            let ok = ord == 0 || self.last_packed_value.as_slice() <= packed_value;
            self.last_packed_value
                .copy_from_slice(&packed_value[..packed_len]);
            ok
        });

        self.leaf_values[self.leaf_count * packed_len..(self.leaf_count + 1) * packed_len]
            .copy_from_slice(&packed_value[..packed_len]);
        self.leaf_docs[self.leaf_count] = doc_id;
        w.set_doc_seen(doc_id)?;
        self.leaf_count += 1;

        if self.value_count > w.total_point_count {
            return Err(LuceneError::illegal_state(format!(
                "totalPointCount={} was passed when we were created, but we just hit {} values",
                w.total_point_count, w.point_count
            )));
        }

        if self.leaf_count == w.config.max_points_in_leaf_node as usize {
            // We write a block once we hit exactly the max count ... this is
            // different from when we flush a new segment, where we write
            // between max/2 and max per leaf block, so merged segments will
            // behave differently from newly flushed segments:
            self.write_leaf_block(w, out)?;
            self.leaf_count = 0;
        }

        debug_assert!({
            self.last_doc_id = doc_id;
            self.last_doc_id >= 0
        }); // only assign when asserts are enabled
        Ok(())
    }

    fn finish(&mut self, w: &mut SimpleTextBKDWriter, out: &mut dyn IndexOutput) -> Result<i64> {
        if self.leaf_count > 0 {
            self.write_leaf_block(w, out)?;
            self.leaf_count = 0;
        }

        if self.value_count == 0 {
            return Ok(-1);
        }

        w.point_count = self.value_count;

        let index_fp = out.file_pointer();

        let num_inner_nodes = self.leaf_block_start_values.len();

        let bytes_per_dim = w.config.bytes_per_dim as usize;
        let mut index = vec![0u8; (1 + num_inner_nodes) * (1 + bytes_per_dim)];
        w.rotate_to_tree(
            1,
            0,
            num_inner_nodes,
            &mut index,
            &self.leaf_block_start_values,
        );
        let arr = self.leaf_block_fps.clone();
        w.write_index(out, &arr, &index, w.config.max_points_in_leaf_node)?;
        Ok(index_fp)
    }

    fn write_leaf_block(
        &mut self,
        w: &mut SimpleTextBKDWriter,
        out: &mut dyn IndexOutput,
    ) -> Result<()> {
        debug_assert_ne!(self.leaf_count, 0);
        let packed_len = w.config.packed_bytes_length() as usize;
        let index_len = w.config.packed_index_bytes_length() as usize;
        let bytes_per_dim = w.config.bytes_per_dim as usize;
        if self.value_count == 0 {
            w.min_packed_value[..index_len].copy_from_slice(&self.leaf_values[..index_len]);
        }
        let last = (self.leaf_count - 1) * packed_len;
        w.max_packed_value[..index_len].copy_from_slice(&self.leaf_values[last..last + index_len]);

        self.value_count += self.leaf_count as i64;

        if !self.leaf_block_fps.is_empty() {
            // Save the first (minimum) value in each leaf block except the
            // first, to build the split value index in the end:
            self.leaf_block_start_values
                .push(self.leaf_values[..packed_len].to_vec());
        }
        self.leaf_block_fps.push(out.file_pointer());
        w.check_max_leaf_node_count(self.leaf_block_fps.len() as i32)?;

        w.common_prefix_lengths.fill(bytes_per_dim);
        // Find per-dim common prefix:
        for dim in 0..w.config.num_dims as usize {
            let offset1 = dim * bytes_per_dim;
            let offset2 = last + offset1;
            for j in 0..w.common_prefix_lengths[dim] {
                if self.leaf_values[offset1 + j] != self.leaf_values[offset2 + j] {
                    w.common_prefix_lengths[dim] = j;
                    break;
                }
            }
        }

        SimpleTextBKDWriter::write_leaf_block_docs(out, &self.leaf_docs, 0, self.leaf_count)?;

        let leaf_values = &self.leaf_values;
        let mut packed_values =
            |i: usize| leaf_values[packed_len * i..packed_len * (i + 1)].to_vec();
        SimpleTextBKDWriter::write_leaf_block_packed_values(
            out,
            self.leaf_count,
            &mut packed_values,
        )
    }
}

/// The anonymous `IntersectVisitor` of `writeField1Dim`.
struct OneDimVisitor<'a> {
    writer: &'a mut SimpleTextBKDWriter,
    one_dim_writer: &'a mut OneDimensionBKDWriter,
    out: &'a mut dyn IndexOutput,
}

impl IntersectVisitor for OneDimVisitor<'_> {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Err(LuceneError::illegal_state(String::new()))
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.one_dim_writer
            .add(self.writer, self.out, packed_value, doc_id)
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Result<Relation> {
        Ok(Relation::CellCrossesQuery)
    }
}
