//! Port of `org.apache.lucene.util.bkd.MutablePointTreeReaderUtils`.

use crate::error::Result;
use crate::index::point_values::MutablePointTree;
use crate::util::packed::PackedInts;
use crate::util::selector::{intro_select, intro_sort, PivotOps, RadixSelector, RadixSelectorOps};
use crate::util::sorter::{
    stable_reorder, MSBRadixSorterOps, StableMSBRadixSorter, StableMSBRadixSorterOps,
};

use super::BKDConfig;

/// Utility APIs for sorting and partitioning buffered points.
///
/// Equivalent to `org.apache.lucene.util.bkd.MutablePointTreeReaderUtils`.
///
/// # Divergence from Lucene 10.5.0
///
/// Lucene builds an anonymous `StableMSBRadixSorter`, `IntroSorter` and
/// `RadixSelector` per call, each closing over the reader and the config. Rust has
/// no anonymous classes, so each of them is a named struct carrying the same state;
/// where Java's `getFallbackSelector(k)` closes over `k`, the struct stores it in
/// `fallback_d` immediately before the fallback runs, which is equivalent because
/// the fallback always runs to completion before returning.
///
/// Java also passes reusable `BytesRef` scratch buffers into `sortByDim` and
/// `partition` that point into the reader's own storage. `MutablePointTree::value`
/// returns a borrowed slice, which cannot be held across the `&mut self` call to
/// `swap`, so the scratch buffers here own their bytes.
pub struct MutablePointTreeReaderUtils;

impl MutablePointTreeReaderUtils {
    /// Sorts the given [`MutablePointTree`] based on its packed value, then doc ID.
    ///
    /// Equivalent to `MutablePointTreeReaderUtils.sort`.
    ///
    /// # Errors
    ///
    /// Returns an error if the number of bits required for `max_doc` cannot be
    /// computed.
    pub fn sort(
        config: &BKDConfig,
        max_doc: i32,
        reader: &mut dyn MutablePointTree,
        from: usize,
        to: usize,
    ) -> Result<()> {
        let mut sorted_by_doc_id = true;
        let mut prev_doc = 0i32;
        for i in from..to {
            let doc = reader.doc_id(i as i32);
            if doc < prev_doc {
                sorted_by_doc_id = false;
                break;
            }
            prev_doc = doc;
        }

        // No need to tie break on doc IDs if already sorted by doc ID, since we use a
        // stable sort. This should be a common situation as IndexWriter accumulates
        // data in doc ID order when index sorting is not enabled.
        let bits_per_doc_id = if sorted_by_doc_id {
            0
        } else {
            PackedInts::bits_required(i64::from(max_doc - 1))?
        };

        let packed_bytes_length = config.packed_bytes_length() as usize;
        let mut ops = SortOps {
            reader,
            packed_bytes_length,
            bits_per_doc_id,
        };
        let max_length = packed_bytes_length + (bits_per_doc_id as usize).div_ceil(8);
        StableMSBRadixSorter::new(max_length).sort(&mut ops, from, to);
        Ok(())
    }

    /// Sorts points on the given dimension.
    ///
    /// Equivalent to `MutablePointTreeReaderUtils.sortByDim`.
    #[allow(clippy::too_many_arguments)]
    pub fn sort_by_dim(
        config: &BKDConfig,
        sorted_dim: usize,
        _common_prefix_lengths: &[usize],
        reader: &mut dyn MutablePointTree,
        from: usize,
        to: usize,
        scratch1: &mut Vec<u8>,
        scratch2: &mut Vec<u8>,
    ) {
        let bytes_per_dim = config.bytes_per_dim as usize;
        let start = sorted_dim * bytes_per_dim;
        // No need for a fancy radix sort here, this is called on the leaves only so
        // there are not many values to sort.
        let mut ops = SortByDimOps {
            reader,
            bytes_per_dim,
            packed_index_bytes_length: config.packed_index_bytes_length() as usize,
            packed_bytes_length: config.packed_bytes_length() as usize,
            start,
            pivot: scratch1,
            pivot_doc: -1,
            scratch: scratch2,
        };
        intro_sort(&mut ops, from, to);
    }

    /// Partitions points around `mid`.
    ///
    /// All values on the left must be less than or equal to it and all values on the
    /// right must be greater than or equal to it. Equivalent to
    /// `MutablePointTreeReaderUtils.partition`.
    ///
    /// # Errors
    ///
    /// Returns an error if the number of bits required for `max_doc` cannot be
    /// computed.
    #[allow(clippy::too_many_arguments)]
    pub fn partition(
        config: &BKDConfig,
        max_doc: i32,
        split_dim: usize,
        common_prefix_len: usize,
        reader: &mut dyn MutablePointTree,
        from: usize,
        to: usize,
        mid: usize,
        scratch1: &mut Vec<u8>,
        scratch2: &mut Vec<u8>,
    ) -> Result<()> {
        let bytes_per_dim = config.bytes_per_dim as usize;
        let dim_offset = split_dim * bytes_per_dim + common_prefix_len;
        let dim_cmp_bytes = bytes_per_dim - common_prefix_len;
        let data_cmp_bytes =
            (config.num_dims - config.num_index_dims) as usize * bytes_per_dim + dim_cmp_bytes;
        let bits_per_doc_id = PackedInts::bits_required(i64::from(max_doc - 1))?;
        let mut ops = PartitionOps {
            reader,
            bytes_per_dim,
            num_dims: config.num_dims as usize,
            packed_index_bytes_length: config.packed_index_bytes_length() as usize,
            packed_bytes_length: config.packed_bytes_length() as usize,
            split_dim,
            dim_offset,
            dim_cmp_bytes,
            data_cmp_bytes,
            bits_per_doc_id,
            fallback_d: 0,
            pivot: scratch1,
            pivot_doc: 0,
            scratch: scratch2,
        };
        let max_length = data_cmp_bytes + (bits_per_doc_id as usize).div_ceil(8);
        RadixSelector::new(max_length).select(&mut ops, from, to, mid);
        Ok(())
    }
}

/// The `StableMSBRadixSorter` `MutablePointTreeReaderUtils.sort` builds.
struct SortOps<'a> {
    reader: &'a mut dyn MutablePointTree,
    packed_bytes_length: usize,
    bits_per_doc_id: i32,
}

impl MSBRadixSorterOps for SortOps<'_> {
    fn byte_at(&self, i: usize, k: usize) -> i32 {
        if k < self.packed_bytes_length {
            i32::from(self.reader.byte_at(i as i32, k as i32))
        } else {
            let shift = self.bits_per_doc_id - (((k - self.packed_bytes_length + 1) << 3) as i32);
            ((self.reader.doc_id(i as i32) as u32) >> shift.max(0)) as i32 & 0xff
        }
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.reader.swap(i as i32, j as i32);
    }

    fn reorder(
        &mut self,
        from: usize,
        to: usize,
        start_offsets: &mut [i32],
        end_offsets: &[i32],
        k: usize,
    ) {
        stable_reorder(self, from, to, start_offsets, end_offsets, k);
    }
}

impl StableMSBRadixSorterOps for SortOps<'_> {
    fn save(&mut self, i: usize, j: usize) {
        self.reader.save(i as i32, j as i32);
    }

    fn restore(&mut self, i: usize, j: usize) {
        self.reader.restore(i as i32, j as i32);
    }
}

/// The `IntroSorter` `MutablePointTreeReaderUtils.sortByDim` builds.
struct SortByDimOps<'a> {
    reader: &'a mut dyn MutablePointTree,
    bytes_per_dim: usize,
    packed_index_bytes_length: usize,
    packed_bytes_length: usize,
    start: usize,
    pivot: &'a mut Vec<u8>,
    pivot_doc: i32,
    scratch: &'a mut Vec<u8>,
}

impl PivotOps for SortByDimOps<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.reader.swap(i as i32, j as i32);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot.clear();
        self.pivot.extend_from_slice(self.reader.value(i as i32));
        self.pivot_doc = self.reader.doc_id(i as i32);
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.scratch.clear();
        self.scratch.extend_from_slice(self.reader.value(j as i32));
        let end = self.start + self.bytes_per_dim;
        let cmp = self.pivot[self.start..end].cmp(&self.scratch[self.start..end]);
        if cmp != std::cmp::Ordering::Equal {
            return ordering_sign(cmp);
        }
        let cmp = self.pivot[self.packed_index_bytes_length..self.packed_bytes_length]
            .cmp(&self.scratch[self.packed_index_bytes_length..self.packed_bytes_length]);
        if cmp != std::cmp::Ordering::Equal {
            return ordering_sign(cmp);
        }
        self.pivot_doc - self.reader.doc_id(j as i32)
    }
}

/// The `RadixSelector` and its fallback `IntroSelector` that
/// `MutablePointTreeReaderUtils.partition` builds.
struct PartitionOps<'a> {
    reader: &'a mut dyn MutablePointTree,
    bytes_per_dim: usize,
    num_dims: usize,
    packed_index_bytes_length: usize,
    packed_bytes_length: usize,
    split_dim: usize,
    dim_offset: usize,
    dim_cmp_bytes: usize,
    data_cmp_bytes: usize,
    bits_per_doc_id: i32,
    /// The byte depth the fallback selector was asked for; Java's `k`.
    fallback_d: usize,
    pivot: &'a mut Vec<u8>,
    pivot_doc: i32,
    scratch: &'a mut Vec<u8>,
}

impl RadixSelectorOps for PartitionOps<'_> {
    fn byte_at(&self, i: usize, k: usize) -> i32 {
        if k < self.dim_cmp_bytes {
            i32::from(self.reader.byte_at(i as i32, (self.dim_offset + k) as i32))
        } else if k < self.data_cmp_bytes {
            i32::from(self.reader.byte_at(
                i as i32,
                (self.packed_index_bytes_length + k - self.dim_cmp_bytes) as i32,
            ))
        } else {
            let shift = self.bits_per_doc_id - (((k - self.data_cmp_bytes + 1) << 3) as i32);
            ((self.reader.doc_id(i as i32) as u32) >> shift.max(0)) as i32 & 0xff
        }
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.reader.swap(i as i32, j as i32);
    }

    fn fallback_select(&mut self, from: usize, to: usize, k: usize, d: usize) {
        self.fallback_d = d;
        intro_select(self, from, to, k);
    }
}

impl PivotOps for PartitionOps<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.reader.swap(i as i32, j as i32);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot.clear();
        self.pivot.extend_from_slice(self.reader.value(i as i32));
        self.pivot_doc = self.reader.doc_id(i as i32);
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        let k = self.fallback_d;
        let dim_start = self.split_dim * self.bytes_per_dim;
        let data_start = if k < self.dim_cmp_bytes {
            self.packed_index_bytes_length
        } else {
            self.packed_index_bytes_length + k - self.dim_cmp_bytes
        };
        let data_end = self.num_dims * self.bytes_per_dim;
        debug_assert!(data_end <= self.packed_bytes_length);

        if k < self.dim_cmp_bytes {
            self.scratch.clear();
            self.scratch.extend_from_slice(self.reader.value(j as i32));
            let end = dim_start + self.bytes_per_dim;
            let cmp = self.pivot[dim_start..end].cmp(&self.scratch[dim_start..end]);
            if cmp != std::cmp::Ordering::Equal {
                return ordering_sign(cmp);
            }
        }
        if k < self.data_cmp_bytes {
            self.scratch.clear();
            self.scratch.extend_from_slice(self.reader.value(j as i32));
            let cmp = self.pivot[data_start..data_end].cmp(&self.scratch[data_start..data_end]);
            if cmp != std::cmp::Ordering::Equal {
                return ordering_sign(cmp);
            }
        }
        self.pivot_doc - self.reader.doc_id(j as i32)
    }
}

/// Java's comparators return the difference of the first differing byte; only the
/// sign of that value is ever read, so `-1`/`0`/`1` is interchangeable with it.
fn ordering_sign(ordering: std::cmp::Ordering) -> i32 {
    match ordering {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}
