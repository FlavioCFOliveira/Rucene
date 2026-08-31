//! Port of `org.apache.lucene.util.bkd.BKDRadixSelector`.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use crate::error::{LuceneError, Result};
use crate::store::Directory;
use crate::util::selector::{intro_select, intro_sort, PivotOps, RadixSelector, RadixSelectorOps};
use crate::util::sorter::{MSBRadixSorter, MSBRadixSorterOps};
use std::sync::Arc;

use super::{BKDConfig, HeapPointWriter, OfflinePointWriter, PointValue, PointWriter};

/// Size of the histogram.
const HISTOGRAM_SIZE: usize = 256;

/// Sliced reference to points in a [`PointWriter`].
///
/// Equivalent to `org.apache.lucene.util.bkd.BKDRadixSelector.PathSlice`.
///
/// # Divergence from Lucene 10.5.0
///
/// Java's record holds a plain reference to the writer, and the two slices returned
/// by [`BKDRadixSelector::select`] for an on-heap input share one writer object.
/// Rust has no shared mutable references, so the writer is held in an
/// `Rc<RefCell<..>>`, which is the same aliasing with the borrow check moved to
/// runtime.
#[derive(Clone)]
pub struct PathSlice {
    /// The writer the points live in.
    pub writer: Rc<RefCell<Box<dyn PointWriter>>>,
    /// Index of the first point of this slice inside `writer`.
    pub start: i64,
    /// How many points this slice holds.
    pub count: i64,
}

impl PathSlice {
    /// Creates a slice over `writer`.
    pub fn new(writer: Rc<RefCell<Box<dyn PointWriter>>>, start: i64, count: i64) -> Self {
        Self {
            writer,
            start,
            count,
        }
    }

    /// Wraps `writer` in a fresh [`Rc`] and slices all of it.
    pub fn of(writer: Box<dyn PointWriter>, start: i64, count: i64) -> Self {
        Self::new(Rc::new(RefCell::new(writer)), start, count)
    }
}

/// Offline radix selector for the BKD tree.
///
/// Equivalent to `org.apache.lucene.util.bkd.BKDRadixSelector`.
///
/// # Divergences from Lucene 10.5.0
///
/// * Java keeps a reusable `offlineBuffer` of 8 KB that it hands to every
///   `OfflinePointReader`; this port's [`PointWriter::get_reader`] owns its buffer,
///   so the field has no counterpart. The bytes read are the same.
/// * `PointValue.packedValueDocIDBytes()` returns a view of the stored record. This
///   port's [`PointValue`] holds the packed value and the doc ID separately, so the
///   record is materialised into a scratch buffer when the algorithm needs to index
///   into it; the layout is the one both writers store, namely the packed value
///   followed by the doc ID big-endian.
pub struct BKDRadixSelector {
    /// Histogram array.
    histogram: Vec<i64>,
    /// Number of bytes to be sorted: `bytesPerDim` for the split dimension, the
    /// data-only dimensions, and the four bytes of the doc ID.
    bytes_sorted: usize,
    /// Flag for when we are moving to sort on heap.
    max_points_sort_in_heap: usize,
    /// Holder for partition points.
    partition_bucket: Vec<i32>,
    /// Scratch array to hold temporary data.
    scratch: Vec<u8>,
    /// Scratch array holding one whole record (packed value plus doc ID).
    record: Vec<u8>,
    /// Directory to create new offline writers in.
    temp_dir: Arc<dyn Directory>,
    /// Prefix for temp files.
    temp_file_name_prefix: String,
    /// BKD tree configuration.
    config: BKDConfig,
}

impl BKDRadixSelector {
    /// Sole constructor.
    pub fn new(
        config: BKDConfig,
        max_points_sort_in_heap: usize,
        temp_dir: Arc<dyn Directory>,
        temp_file_name_prefix: &str,
    ) -> Self {
        // Selection and sorting is done in a given dimension. In case the values of
        // the dimension are equal between two points we tie break first using the
        // data-only dimensions and if those are still equal we tie-break on the docID.
        // Here we account for all bytes used in the process.
        let bytes_per_dim = config.bytes_per_dim as usize;
        let bytes_sorted = bytes_per_dim
            + (config.num_dims - config.num_index_dims) as usize * bytes_per_dim
            + std::mem::size_of::<i32>();
        let bytes_per_doc = config.bytes_per_doc() as usize;
        Self {
            histogram: vec![0i64; HISTOGRAM_SIZE],
            bytes_sorted,
            max_points_sort_in_heap,
            partition_bucket: vec![0i32; bytes_sorted],
            scratch: vec![0u8; bytes_sorted],
            record: vec![0u8; bytes_per_doc],
            temp_dir,
            temp_file_name_prefix: temp_file_name_prefix.to_string(),
            config,
        }
    }

    /// Partitions `points` around `partition_point`.
    ///
    /// Uses the provided `points` from `from` to `to` to produce two path slices, so
    /// that the slice at position 0 contains `partition_point - from` points whose
    /// value on `dim` is lower than or equal to the `to - partition_point` points of
    /// the slice at position 1. `dim_common_prefix` is a hint for the length of the
    /// common prefix of `dim`.
    ///
    /// Returns the value of `dim` at the partition point, plus the two slices. If
    /// `points` wraps an [`OfflinePointWriter`], that writer is destroyed in the
    /// process to save disk space.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `partition_point` is outside
    /// `[from, to)`, and an I/O error if a temporary file cannot be written.
    #[allow(clippy::too_many_arguments)]
    pub fn select(
        &mut self,
        points: &PathSlice,
        from: i64,
        to: i64,
        partition_point: i64,
        dim: usize,
        dim_common_prefix: usize,
    ) -> Result<(Vec<u8>, [PathSlice; 2])> {
        self.check_args(from, to, partition_point)?;

        let is_heap = points
            .writer
            .borrow_mut()
            .as_any_mut()
            .is::<HeapPointWriter>();

        // If we are on heap then we just select on heap
        if is_heap {
            let partition = {
                let mut writer = points.writer.borrow_mut();
                let heap = writer
                    .as_any_mut()
                    .downcast_mut::<HeapPointWriter>()
                    .expect("INVARIANT: checked with `is::<HeapPointWriter>` above");
                Self::heap_radix_select(
                    &self.config,
                    self.bytes_sorted,
                    &mut self.scratch,
                    heap,
                    dim,
                    from as usize,
                    to as usize,
                    partition_point as usize,
                    dim_common_prefix,
                )
            };
            let slices = [
                PathSlice::new(points.writer.clone(), from, partition_point - from),
                PathSlice::new(points.writer.clone(), partition_point, to - partition_point),
            ];
            return Ok((partition, slices));
        }

        let mut left = self.get_point_writer(partition_point - from, &format!("left{}", dim))?;
        let mut right = self.get_point_writer(to - partition_point, &format!("right{}", dim))?;
        let partition = {
            let mut writer = points.writer.borrow_mut();
            self.build_histogram_and_partition(
                writer.as_mut(),
                left.as_mut(),
                right.as_mut(),
                from,
                to,
                partition_point,
                0,
                dim_common_prefix,
                dim,
            )?
        };
        left.close()?;
        right.close()?;
        let slices = [
            PathSlice::of(left, 0, partition_point - from),
            PathSlice::of(right, 0, to - partition_point),
        ];
        Ok((partition, slices))
    }

    /// Validates the arguments of [`BKDRadixSelector::select`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `partition_point` is outside
    /// `[from, to)`.
    pub fn check_args(&self, from: i64, to: i64, partition_point: i64) -> Result<()> {
        if partition_point < from {
            return Err(LuceneError::IllegalArgument(
                "partitionPoint must be >= from".to_string(),
            ));
        }
        if partition_point >= to {
            return Err(LuceneError::IllegalArgument(
                "partitionPoint must be < to".to_string(),
            ));
        }
        Ok(())
    }

    fn find_common_prefix_and_histogram(
        &mut self,
        points: &mut dyn PointWriter,
        from: i64,
        to: i64,
        dim: usize,
        dim_common_prefix: usize,
    ) -> Result<usize> {
        // find common prefix
        let mut common_prefix_position = self.bytes_sorted;
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let packed_index_bytes_length = self.config.packed_index_bytes_length() as usize;
        let data_and_doc_len = (self.config.num_dims - self.config.num_index_dims) as usize
            * bytes_per_dim
            + std::mem::size_of::<i32>();
        let offset = dim * bytes_per_dim;
        {
            let mut reader = points.get_reader(from as usize, (to - from) as usize)?;
            debug_assert!(common_prefix_position > dim_common_prefix);
            if !reader.next()? {
                return Err(LuceneError::IllegalState(
                    "BKDRadixSelector: no points to read".to_string(),
                ));
            }
            let mut point_value = reader.point_value().clone();
            {
                let mut record = std::mem::take(&mut self.record);
                point_value.write_packed_value_doc_id_bytes(&mut record);
                // copy dimension
                self.scratch[0..bytes_per_dim]
                    .copy_from_slice(&record[offset..offset + bytes_per_dim]);
                // copy data dimensions and docID
                self.scratch[bytes_per_dim..bytes_per_dim + data_and_doc_len].copy_from_slice(
                    &record
                        [packed_index_bytes_length..packed_index_bytes_length + data_and_doc_len],
                );
                self.record = record;
            }

            let mut i = from + 1;
            while i < to {
                if !reader.next()? {
                    break;
                }
                point_value = reader.point_value().clone();
                if common_prefix_position == dim_common_prefix {
                    let bucket = self.get_bucket(offset, common_prefix_position, &point_value);
                    self.histogram[bucket] += 1;
                    // we do not need to check for common prefix anymore, just finish
                    // the histogram and break
                    let mut j = i + 1;
                    while j < to {
                        if !reader.next()? {
                            break;
                        }
                        let pv = reader.point_value().clone();
                        let bucket = self.get_bucket(offset, common_prefix_position, &pv);
                        self.histogram[bucket] += 1;
                        j += 1;
                    }
                    break;
                } else {
                    // Check common prefix and adjust histogram
                    let start_index = dim_common_prefix.min(bytes_per_dim);
                    let end_index = common_prefix_position.min(bytes_per_dim);
                    let mut record = std::mem::take(&mut self.record);
                    point_value.write_packed_value_doc_id_bytes(&mut record);
                    let j = mismatch(
                        &self.scratch,
                        start_index,
                        end_index,
                        &record,
                        offset + start_index,
                        offset + end_index,
                    );
                    if j == -1 {
                        if common_prefix_position > bytes_per_dim {
                            // Tie-break on data dimensions + docID
                            let start_tie_break = packed_index_bytes_length;
                            let end_tie_break =
                                start_tie_break + common_prefix_position - bytes_per_dim;
                            let k = mismatch(
                                &self.scratch,
                                bytes_per_dim,
                                common_prefix_position,
                                &record,
                                start_tie_break,
                                end_tie_break,
                            );
                            if k != -1 {
                                common_prefix_position = bytes_per_dim + k as usize;
                                self.histogram.iter_mut().for_each(|h| *h = 0);
                                self.histogram[self.scratch[common_prefix_position] as usize] =
                                    i - from;
                            }
                        }
                    } else {
                        common_prefix_position = dim_common_prefix + j as usize;
                        self.histogram.iter_mut().for_each(|h| *h = 0);
                        self.histogram[self.scratch[common_prefix_position] as usize] = i - from;
                    }
                    self.record = record;
                    if common_prefix_position != self.bytes_sorted {
                        let bucket = self.get_bucket(offset, common_prefix_position, &point_value);
                        self.histogram[bucket] += 1;
                    }
                }
                i += 1;
            }
            reader.close()?;
        }

        // Build partition buckets up to commonPrefix
        for i in 0..common_prefix_position {
            self.partition_bucket[i] = i32::from(self.scratch[i]);
        }
        Ok(common_prefix_position)
    }

    /// Returns the histogram bucket of `point_value` at byte
    /// `common_prefix_position` of the sorted key.
    fn get_bucket(
        &self,
        offset: usize,
        common_prefix_position: usize,
        point_value: &PointValue,
    ) -> usize {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        if common_prefix_position < bytes_per_dim {
            usize::from(point_value.packed[offset + common_prefix_position])
        } else {
            let index = self.config.packed_index_bytes_length() as usize + common_prefix_position
                - bytes_per_dim;
            let packed_len = point_value.packed.len();
            if index < packed_len {
                usize::from(point_value.packed[index])
            } else {
                // The four doc ID bytes, most significant first.
                let b = index - packed_len;
                ((point_value.doc_id as u32) >> (24 - 8 * b)) as usize & 0xff
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_histogram_and_partition(
        &mut self,
        points: &mut dyn PointWriter,
        left: &mut dyn PointWriter,
        right: &mut dyn PointWriter,
        from: i64,
        to: i64,
        partition_point: i64,
        iteration: i32,
        base_common_prefix: usize,
        dim: usize,
    ) -> Result<Vec<u8>> {
        // Find common prefix from baseCommonPrefix and build histogram
        let mut common_prefix =
            self.find_common_prefix_and_histogram(points, from, to, dim, base_common_prefix)?;

        // If all equals we just partition the points
        if common_prefix == self.bytes_sorted {
            self.offline_partition(
                points,
                left,
                right,
                None,
                from,
                to,
                dim,
                common_prefix - 1,
                partition_point,
            )?;
            return Ok(self.partition_point_from_common_prefix());
        }

        let mut left_count = 0i64;
        let mut right_count = 0i64;

        // Count left points and record the partition point
        for i in 0..HISTOGRAM_SIZE {
            let size = self.histogram[i];
            if left_count + size > partition_point - from {
                self.partition_bucket[common_prefix] = i as i32;
                break;
            }
            left_count += size;
        }
        // Count right points
        for i in (self.partition_bucket[common_prefix] as usize + 1)..HISTOGRAM_SIZE {
            right_count += self.histogram[i];
        }

        let delta = self.histogram[self.partition_bucket[common_prefix] as usize];
        debug_assert!(left_count + right_count + delta == to - from);

        // Special case when points are equal except the last byte: we can just
        // tie-break
        if common_prefix == self.bytes_sorted - 1 {
            let tie_break_count = partition_point - from - left_count;
            self.offline_partition(
                points,
                left,
                right,
                None,
                from,
                to,
                dim,
                common_prefix,
                tie_break_count,
            )?;
            return Ok(self.partition_point_from_common_prefix());
        }

        // Create the delta points writer
        let mut delta_points = self.get_delta_point_writer(left, right, delta, iteration)?;
        // Divide the points. This actually destroys the current writer.
        self.offline_partition(
            points,
            left,
            right,
            Some(delta_points.as_mut()),
            from,
            to,
            dim,
            common_prefix,
            0,
        )?;
        delta_points.close()?;

        let new_partition_point = partition_point - from - left_count;
        common_prefix += 1;

        let delta_is_heap = delta_points.as_any_mut().is::<HeapPointWriter>();
        if delta_is_heap {
            let count = delta_points.count();
            let mut heap = delta_points;
            let heap = heap
                .as_any_mut()
                .downcast_mut::<HeapPointWriter>()
                .expect("INVARIANT: checked with `is::<HeapPointWriter>` above");
            self.heap_partition(
                heap,
                left,
                right,
                dim,
                0,
                count,
                new_partition_point as usize,
                common_prefix,
            )
        } else {
            let count = delta_points.count() as i64;
            self.build_histogram_and_partition(
                delta_points.as_mut(),
                left,
                right,
                0,
                count,
                new_partition_point,
                iteration + 1,
                common_prefix,
                dim,
            )
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn offline_partition(
        &mut self,
        points: &mut dyn PointWriter,
        left: &mut dyn PointWriter,
        right: &mut dyn PointWriter,
        mut delta_points: Option<&mut dyn PointWriter>,
        from: i64,
        to: i64,
        dim: usize,
        byte_position: usize,
        num_docs_tiebreak: i64,
    ) -> Result<()> {
        debug_assert!(byte_position == self.bytes_sorted - 1 || delta_points.is_some());
        let offset = dim * self.config.bytes_per_dim as usize;
        let mut tiebreak_counter = 0i64;
        {
            let mut reader = points.get_reader(from as usize, (to - from) as usize)?;
            while reader.next()? {
                let point_value = reader.point_value().clone();
                let bucket = self.get_bucket(offset, byte_position, &point_value);
                match (bucket as i32).cmp(&self.partition_bucket[byte_position]) {
                    Ordering::Less => {
                        // to the left side
                        left.append(&point_value)?;
                    }
                    Ordering::Greater => {
                        // to the right side
                        right.append(&point_value)?;
                    }
                    Ordering::Equal => {
                        if byte_position == self.bytes_sorted - 1 {
                            if tiebreak_counter < num_docs_tiebreak {
                                left.append(&point_value)?;
                                tiebreak_counter += 1;
                            } else {
                                right.append(&point_value)?;
                            }
                        } else {
                            delta_points
                                .as_mut()
                                .expect("INVARIANT: asserted above")
                                .append(&point_value)?;
                        }
                    }
                }
            }
            reader.close()?;
        }
        // Delete original file
        points.destroy()?;
        Ok(())
    }

    fn partition_point_from_common_prefix(&self) -> Vec<u8> {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let mut partition = vec![0u8; bytes_per_dim];
        for (i, slot) in partition.iter_mut().enumerate() {
            *slot = self.partition_bucket[i] as u8;
        }
        partition
    }

    #[allow(clippy::too_many_arguments)]
    fn heap_partition(
        &mut self,
        points: &mut HeapPointWriter,
        left: &mut dyn PointWriter,
        right: &mut dyn PointWriter,
        dim: usize,
        from: usize,
        to: usize,
        partition_point: usize,
        common_prefix: usize,
    ) -> Result<Vec<u8>> {
        let partition = Self::heap_radix_select(
            &self.config,
            self.bytes_sorted,
            &mut self.scratch,
            points,
            dim,
            from,
            to,
            partition_point,
            common_prefix,
        );
        for i in from..to {
            let value = points.get_packed_value_slice(i);
            if i < partition_point {
                left.append(&value)?;
            } else {
                right.append(&value)?;
            }
        }
        Ok(partition)
    }

    #[allow(clippy::too_many_arguments)]
    fn heap_radix_select(
        config: &BKDConfig,
        bytes_sorted: usize,
        scratch: &mut [u8],
        points: &mut HeapPointWriter,
        dim: usize,
        from: usize,
        to: usize,
        partition_point: usize,
        common_prefix_length: usize,
    ) -> Vec<u8> {
        let bytes_per_dim = config.bytes_per_dim as usize;
        let dim_offset = dim * bytes_per_dim + common_prefix_length;
        let dim_cmp_bytes = bytes_per_dim - common_prefix_length;
        let data_offset = config.packed_index_bytes_length() as usize - dim_cmp_bytes;
        {
            let mut ops = HeapPointOps {
                points,
                scratch,
                bytes_per_dim,
                dim,
                dim_offset,
                dim_cmp_bytes,
                data_offset,
                common_prefix_length,
                fallback_d: 0,
            };
            RadixSelector::new(bytes_sorted - common_prefix_length).select(
                &mut ops,
                from,
                to,
                partition_point,
            );
        }

        let mut partition = vec![0u8; bytes_per_dim];
        let point_value = points.get_packed_value_slice(partition_point);
        let start = dim * bytes_per_dim;
        partition.copy_from_slice(&point_value.packed[start..start + bytes_per_dim]);
        partition
    }

    /// Sorts the heap writer by the specified dimension.
    ///
    /// This is used to sort the leaves of the tree. Equivalent to
    /// `BKDRadixSelector.heapRadixSort`.
    pub fn heap_radix_sort(
        &mut self,
        points: &mut HeapPointWriter,
        from: usize,
        to: usize,
        dim: usize,
        common_prefix_length: usize,
    ) {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let dim_offset = dim * bytes_per_dim + common_prefix_length;
        let dim_cmp_bytes = bytes_per_dim - common_prefix_length;
        let data_offset = self.config.packed_index_bytes_length() as usize - dim_cmp_bytes;
        let mut ops = HeapPointOps {
            points,
            scratch: &mut self.scratch,
            bytes_per_dim,
            dim,
            dim_offset,
            dim_cmp_bytes,
            data_offset,
            common_prefix_length,
            fallback_d: 0,
        };
        MSBRadixSorter::new(self.bytes_sorted - common_prefix_length).sort(&mut ops, from, to);
    }

    fn get_delta_point_writer(
        &self,
        left: &mut dyn PointWriter,
        right: &mut dyn PointWriter,
        delta: i64,
        iteration: i32,
    ) -> Result<Box<dyn PointWriter>> {
        if delta <= self.get_max_points_sort_in_heap(left, right) as i64 {
            Ok(Box::new(HeapPointWriter::new(
                self.config.clone(),
                delta as usize,
            )?))
        } else {
            Ok(Box::new(OfflinePointWriter::new(
                self.config.clone(),
                self.temp_dir.clone(),
                &self.temp_file_name_prefix,
                &format!("delta{}", iteration),
            )?))
        }
    }

    fn get_max_points_sort_in_heap(
        &self,
        left: &mut dyn PointWriter,
        right: &mut dyn PointWriter,
    ) -> usize {
        let mut points_used = 0usize;
        if let Some(heap) = left.as_any_mut().downcast_mut::<HeapPointWriter>() {
            points_used += heap.size();
        }
        if let Some(heap) = right.as_any_mut().downcast_mut::<HeapPointWriter>() {
            points_used += heap.size();
        }
        debug_assert!(self.max_points_sort_in_heap >= points_used);
        self.max_points_sort_in_heap - points_used
    }

    /// Creates a point writer able to hold `count` points.
    ///
    /// As we recurse, we hold two on-heap point writers at any point; therefore the
    /// maximum size for these objects is half of the total points we can hold on
    /// heap.
    ///
    /// # Errors
    ///
    /// Returns an error if the temporary file cannot be created.
    pub fn get_point_writer(&self, count: i64, desc: &str) -> Result<Box<dyn PointWriter>> {
        if count <= (self.max_points_sort_in_heap / 2) as i64 {
            Ok(Box::new(HeapPointWriter::new(
                self.config.clone(),
                count as usize,
            )?))
        } else {
            Ok(Box::new(OfflinePointWriter::new(
                self.config.clone(),
                self.temp_dir.clone(),
                &self.temp_file_name_prefix,
                desc,
            )?))
        }
    }
}

/// Java's `Arrays.mismatch(a, aFrom, aTo, b, bFrom, bTo)`.
fn mismatch(a: &[u8], a_from: usize, a_to: usize, b: &[u8], b_from: usize, b_to: usize) -> i32 {
    let a_len = a_to - a_from;
    let b_len = b_to - b_from;
    let len = a_len.min(b_len);
    for i in 0..len {
        if a[a_from + i] != b[b_from + i] {
            return i as i32;
        }
    }
    if a_len != b_len {
        len as i32
    } else {
        -1
    }
}

/// The `RadixSelector` / `MSBRadixSorter` and their fallback `IntroSelector` /
/// `IntroSorter` that `heapRadixSelect` and `heapRadixSort` build.
///
/// Java builds the fallback per call, closing over the byte depth; this port stores
/// the depth in `fallback_d` right before running it, which is equivalent because
/// the fallback always finishes before returning.
struct HeapPointOps<'a> {
    points: &'a mut HeapPointWriter,
    scratch: &'a mut [u8],
    bytes_per_dim: usize,
    dim: usize,
    dim_offset: usize,
    dim_cmp_bytes: usize,
    data_offset: usize,
    common_prefix_length: usize,
    fallback_d: usize,
}

impl HeapPointOps<'_> {
    fn key_byte_at(&self, i: usize, k: usize) -> i32 {
        let offset = if k < self.dim_cmp_bytes {
            self.dim_offset + k
        } else {
            self.data_offset + k
        };
        i32::from(self.points.byte_at(i, offset))
    }

    fn skipped_bytes(&self) -> usize {
        self.fallback_d + self.common_prefix_length
    }

    fn dim_start(&self) -> usize {
        self.dim * self.bytes_per_dim
    }
}

impl RadixSelectorOps for HeapPointOps<'_> {
    fn byte_at(&self, i: usize, k: usize) -> i32 {
        self.key_byte_at(i, k)
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.points.swap(i, j);
    }

    fn fallback_select(&mut self, from: usize, to: usize, k: usize, d: usize) {
        self.fallback_d = d;
        intro_select(self, from, to, k);
    }
}

impl MSBRadixSorterOps for HeapPointOps<'_> {
    fn byte_at(&self, i: usize, k: usize) -> i32 {
        self.key_byte_at(i, k)
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.points.swap(i, j);
    }

    fn fallback_sort(&mut self, from: usize, to: usize, k: usize, _max_length: usize) {
        self.fallback_d = k;
        intro_sort(self, from, to);
    }
}

impl PivotOps for HeapPointOps<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.points.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        if self.skipped_bytes() < self.bytes_per_dim {
            self.points.copy_dim(i, self.dim_start(), self.scratch, 0);
        }
        self.points
            .copy_data_dims_and_doc(i, self.scratch, self.bytes_per_dim);
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        if self.skipped_bytes() < self.bytes_per_dim {
            let cmp = self.points.compare_dim(i, j, self.dim_start());
            if cmp != Ordering::Equal {
                return ordering_sign(cmp);
            }
        }
        ordering_sign(self.points.compare_data_dims_and_doc(i, j))
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        if self.skipped_bytes() < self.bytes_per_dim {
            let cmp = self
                .points
                .compare_dim_to(j, self.scratch, 0, self.dim_start());
            if cmp != Ordering::Equal {
                return ordering_sign(cmp);
            }
        }
        ordering_sign(
            self.points
                .compare_data_dims_and_doc_to(j, self.scratch, self.bytes_per_dim),
        )
    }
}

/// Java's comparators return the difference of the first differing byte; only the
/// sign of that value is ever read.
fn ordering_sign(ordering: Ordering) -> i32 {
    match ordering {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}
