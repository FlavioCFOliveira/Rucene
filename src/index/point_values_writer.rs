//! Per-field buffer of indexed point values, flushed through the points codec
//! when the segment flushes.
//!
//! Equivalent to `org.apache.lucene.index.PointValuesWriter`, together with the
//! two anonymous classes its `flush` builds — the `MutablePointTree` over the
//! buffer and the single-field `PointsReader` that serves it to the codec
//! (`PointValuesWriter.java:203-339`).
//!
//! # Responsibility
//!
//! [`PointValuesWriter`] owns exactly one thing: the point values of **one
//! field** of the segment currently being built. It buffers the packed bytes of
//! every value, in the order the documents supplied them, alongside the doc ID
//! each value belongs to; it counts points and documents; and at flush time it
//! hands the whole buffer to a [`PointsWriter`] behind the
//! [`PointsReader`]/[`PointValues`]/[`MutablePointTree`] interfaces the codec
//! expects. It sorts nothing, builds no tree and writes no file — the BKD
//! writer inside the codec does all three, reordering the points **through**
//! the mutable tree this module exposes.
//!
//! # Lifecycle
//!
//! The Java lifecycle, from `IndexingChain`, is:
//!
//! 1. **Creation.** `initializeFieldInfo` creates the writer for every field
//!    whose `pointDimensionCount != 0`, indexed or not
//!    (`IndexingChain.java:1372-1374`).
//! 2. **One call per field instance.** `processField` calls
//!    `addPackedValue(docID, field.binaryValue())` for *every instance* of a
//!    field that declares points, after the invert-and-store half and after the
//!    doc values (`IndexingChain.java:1393-1395`). A field may therefore
//!    contribute several points to one document; that is what makes
//!    `numPoints` and `numDocs` two different counters.
//! 3. **Flush.** `IndexingChain.writePoints` walks the per-field hash table —
//!    in *table order*, not field-number order, which fixes the order of the
//!    per-field entries inside the `.kdm` — and flushes each writer through a
//!    lazily created `PointsWriter` (`IndexingChain.java:396-435`). The writer
//!    is finished once, after the last field, and closed whether or not a field
//!    threw.
//!
//! # Invariants
//!
//! * Every buffered value is exactly `pointDimensionCount * pointNumBytes`
//!   bytes long. A value of any other length is refused, because the codec
//!   reads the buffer as a flat array of fixed-width records and a short value
//!   would silently shift every value after it.
//! * `values.len() == num_points * packed_bytes_length` and
//!   `doc_ids.len() == num_points` at all times: the two buffers are appended
//!   together and read back in lockstep.
//! * `num_docs` counts *distinct* doc IDs, which Lucene derives from the
//!   single test `docID != lastDocID` — correct precisely because
//!   `IndexingChain` feeds documents in increasing order and finishes one
//!   before starting the next.
//! * Doc IDs are non-decreasing. Java does not check this on the per-value
//!   path — it simply relies on the chain — and neither does this port, so that
//!   `num_docs` is derived by the identical rule.
//!
//! # Divergences from Java, and why
//!
//! * Java buffers the packed values in a `PagedBytes(12)` written through a
//!   `DataOutput` and frozen into a `PagedBytes.Reader` at flush; this port
//!   uses a flat `Vec<u8>`. `PagedBytes` is not ported, the buffer is never
//!   read back through anything but a fixed-offset slice, and the RAM
//!   accounting reports what is actually held. This is the same divergence
//!   already declared by [`crate::index::NormValuesWriter`] and by
//!   [`crate::index::doc_values_writer`], for the same reason.
//! * Java's `flush` takes a `Sorter.DocMap` and, when the segment has an index
//!   sort, wraps the mutable tree in `PointValuesWriter.MutableSortingPointValues`
//!   so that every doc ID is mapped through `oldToNew`
//!   (`PointValuesWriter.java:341-406`). Index sorting is not ported — neither
//!   `Sorter` nor `Sorter.DocMap` exists in this crate, and `IndexingChain`
//!   never builds a sorting consumer — so [`PointValuesWriter::flush`] takes no
//!   map and `MutableSortingPointValues` is not ported either. It is the same
//!   seam [`crate::index::NormValuesWriter`] and the doc-values writers leave
//!   open, and it is the single place a doc map would be threaded through.
//! * The three dense batch paths (`addDense1DIntValues`,
//!   `addDense1DLongValues`, `addDenseNDValues`) take a `LongValuesCursor` or a
//!   `BytesRefValuesCursor` from `org.apache.lucene.document.column`. That
//!   column-batch indexing path does not exist in this crate, so there is no
//!   cursor to accept and no caller to serve; the per-instance path is the only
//!   one, exactly as for the doc-values writers. The static assertion those
//!   paths carry — that `SharedIndexingScratch.BYTES_SCRATCH_SIZE` is at least
//!   `PointValues.MAX_NUM_BYTES * BKDConfig.MAX_DIMS` — guards only the dense
//!   staging buffer, so it has nothing to guard here.
//! * Java's anonymous `PointsReader.getValues` returns a `PointValues` that
//!   throws `UnsupportedOperationException` for everything except
//!   `getPointTree()`, because the codec calls nothing else
//!   (`PointValuesWriter.java:287-327`); its `MutablePointTree` does the same
//!   for `getMinPackedValue`/`getMaxPackedValue`/`visitDocIDs`/`clone`. Every
//!   one of those values is known here, and the Rust signatures for `size`,
//!   `doc_count`, `min_packed_value`, `max_packed_value` and `clone_tree` have
//!   no error channel, so the only alternatives were to panic or to return a
//!   value that is not true. This port answers all of them truthfully instead;
//!   the bounds are computed once, on first request, so a caller that never
//!   asks pays nothing. No caller can observe a difference, because Java's
//!   version would have thrown where this one answers.
//! * Java has no "already flushed" flag. `IndexingChain.writePoints` sets
//!   `perField.pointValuesWriter = null` right after flushing a field
//!   (`IndexingChain.java:420`), so a second flush, or an `addPackedValue`
//!   after the flush, is a `NullPointerException` on a field the chain has
//!   already dropped — it is prevented by ownership, not by a check. This port
//!   keeps the per-field table alive until the end of `flush` (see
//!   `IndexingChain::write_points`), so the same two calls would silently
//!   write a second `.kdm` entry for one field, or append values that no
//!   segment will ever carry. [`PointValuesWriter`] therefore holds a
//!   `flushed` flag and answers both with [`LuceneError::IllegalState`]. It is
//!   a divergence in the *error surface* only: no caller that follows Lucene's
//!   own lifecycle can reach it, and the states it refuses are exactly the
//!   states Java makes unreachable.
//! * Java returns *the same* `MutablePointTree` instance from every
//!   `getPointTree()` call, so a second call would observe the order the first
//!   caller sorted into. This port returns a fresh cursor over the shared
//!   buffer, which is what `PointValues.getPointTree()` documents ("Create a
//!   new PointTree to navigate the index") and what every other
//!   [`PointValues`] in this crate does. The codec calls it once.
//!
//! # The mutable fast path
//!
//! `Lucene90PointsWriter.writeField` tests `values instanceof MutablePointTree`
//! and, when it holds, calls
//! `BKDWriter.writeField(metaOut, indexOut, dataOut, name, tree)`, which sorts
//! and partitions the points in place instead of streaming them through
//! `BKDWriter.add` and re-sorting them offline
//! (`Lucene90PointsWriter.java:157-167`). This crate takes that path:
//! [`crate::codecs::Lucene90PointsWriter`] asks the tree for
//! [`PointTree::as_mutable`] and hands it to `BKDWriter::write_field`, which
//! forks on `numDims` exactly as Java's does.
//!
//! Measured against Apache Lucene Core 10.5.0 itself, both sides flushing the
//! same documents through their own indexing chain and every byte of `.kdd`,
//! `.kdi` and `.kdm` compared:
//!
//! * **324 of 324 shapes byte-identical** on a grid that varies 1 to 4
//!   dimensions with `numIndexDims` at 1 and at `numDims`, `bytesPerDim` of 1,
//!   4 and 16, 20 to 12000 documents, one and two points per document, random
//!   and long-shared-prefix value patterns, and two cardinalities. Before the
//!   work recorded below, 44 of those 324 differed.
//! * **288 of 288** on the narrower earlier grid (1 to 4 dimensions, 7 to 2000
//!   points, three cardinalities, three seeds).
//!
//! Getting there needed five things beyond the entry point itself, and each is
//! documented where it lives:
//!
//! * `BKDWriter::build_mutable` measures a leaf's per-dimension byte
//!   cardinalities from `from + 1`, not from `from` (`BKDWriter.java:1688`), so
//!   the first point of a leaf never counts towards the sorted dimension;
//! * `BKDWriter::sort_by_dim` sorts a leaf with `IntroSorter`, which is
//!   **unstable**, because `sortByDim`'s comparator is not a total order: two
//!   points of one document can tie on it while still differing in another
//!   indexed dimension that the leaf writes;
//! * `BKDWriter::partition_by_dim` *selects* with a `RadixSelector` rather than
//!   sorting, because the arrangement a selection leaves within each side is
//!   what the first rule then reads;
//! * `build` recomputes a node's exact bounds every
//!   `SPLITS_BEFORE_EXACT_BOUNDS` splits above two index dimensions
//!   (`BKDWriter.java:1781-1786`), or `split` chooses from bounds inherited
//!   from an ancestor and takes the whole subtree with it;
//! * `OneDimensionBKDWriter` writes the one-dimensional case, where the points
//!   are already ordered and every leaf is exactly full.
//!
//! `BKDWriter::build` is split into `build_mutable` and `build_offline`,
//! mirroring Java's two overloads, which are **not** the same algorithm. The
//! offline half is the one [`crate::codecs::Lucene90PointsWriter`] falls back
//! to for a tree that is not mutable; what it still owes Lucene is declared on
//! the method.
//!
#![deny(unsafe_code)]

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

use crate::codecs::points::{PointsReader, PointsWriter};
use crate::error::{LuceneError, Result};
use crate::index::point_values::{
    IntersectVisitor, MutablePointTree, PointTree, PointValues, Relation,
};
use crate::index::FieldInfo;
use crate::util::BytesRef;

/// Initial capacity of the doc-ID buffer.
///
/// Equivalent to `PointValuesWriter`'s `docIDs = new int[16]`
/// (`PointValuesWriter.java:67`), whose 64 bytes Lucene charges to the shared
/// counter in the constructor.
const INITIAL_DOC_IDS: usize = 16;

// ---------------------------------------------------------------------------
// The writer
// ---------------------------------------------------------------------------

/// Buffers the point values of one field until the segment flushes.
///
/// Equivalent to `org.apache.lucene.index.PointValuesWriter`.
#[derive(Debug)]
pub struct PointValuesWriter {
    field_info: FieldInfo,
    /// The packed values, concatenated. Java's `PagedBytes`; see the module
    /// documentation.
    values: Vec<u8>,
    /// The doc ID of each buffered value, parallel to `values`.
    doc_ids: Vec<i32>,
    num_points: i32,
    num_docs: i32,
    last_doc_id: i32,
    packed_bytes_length: usize,
    flushed: bool,
    /// The footprint already charged to `iw_bytes_used`, so an update can
    /// report the delta.
    bytes_used: i64,
    iw_bytes_used: Arc<AtomicI64>,
}

impl PointValuesWriter {
    /// Creates a writer for `field_info`, charging its initial buffers to
    /// `iw_bytes_used`.
    ///
    /// Equivalent to `PointValuesWriter(Counter, FieldInfo,
    /// SharedIndexingScratch)`; the scratch is only used by the dense batch
    /// paths, which are not ported (see the module documentation).
    ///
    /// Like Java's constructor this validates nothing: `initializeFieldInfo`
    /// only builds a writer for a field whose `pointDimensionCount != 0`, and
    /// `FieldInfo` has already refused a field that declares dimensions
    /// without a byte width.
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let packed_bytes_length =
            field_info.point_dimension_count as usize * field_info.point_num_bytes as usize;
        let mut writer = Self {
            field_info,
            values: Vec::new(),
            doc_ids: Vec::with_capacity(INITIAL_DOC_IDS),
            num_points: 0,
            num_docs: 0,
            last_doc_id: -1,
            packed_bytes_length,
            flushed: false,
            bytes_used: 0,
            iw_bytes_used,
        };
        writer.update_bytes_used();
        writer
    }

    /// Returns the field these points belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Buffers one packed point value for `doc_id`.
    ///
    /// Equivalent to `PointValuesWriter.addPackedValue(int, BytesRef)`. A field
    /// may add several values to the same document; only the first of them
    /// increments the document count.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `value` is not exactly
    /// `pointDimensionCount * pointNumBytes` bytes long — Lucene's
    /// `IllegalArgumentException` for the same case — and
    /// [`LuceneError::IllegalState`] when the field has already been flushed.
    pub fn add_packed_value(&mut self, doc_id: i32, value: &BytesRef) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the point values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        if value.length != self.packed_bytes_length {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{}\": this field's value has length={} but should be {}",
                self.field_info.name, value.length, self.packed_bytes_length
            )));
        }
        self.values.extend_from_slice(value.slice());
        self.doc_ids.push(doc_id);
        if doc_id != self.last_doc_id {
            self.num_docs += 1;
            self.last_doc_id = doc_id;
        }
        self.num_points += 1;
        self.update_bytes_used();
        Ok(())
    }

    /// Number of buffered points.
    ///
    /// Java keeps this as the package-private field `numPoints`; the codec
    /// reads it through `MutablePointTree.size()`.
    pub fn num_points(&self) -> i32 {
        self.num_points
    }

    /// Number of distinct documents that carried a value.
    ///
    /// Equivalent to `PointValuesWriter.getNumDocs()`.
    pub fn num_docs(&self) -> i32 {
        self.num_docs
    }

    /// Reports the delta between the footprint already charged and the current
    /// one.
    ///
    /// Java charges the counter at each of the two growth points instead
    /// (`PointValuesWriter.java:68`, `:90`, `:94`); reporting the whole
    /// footprint as a delta reaches the same total and, unlike Java's, is
    /// refunded when the writer is dropped.
    fn update_bytes_used(&mut self) {
        let new_bytes_used = self.ram_bytes_used();
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// Approximate heap held by the buffers.
    ///
    /// Capacity rather than length, because that is what is actually
    /// allocated and what Java's `docIDs.length` and `PagedBytes.ramBytesUsed`
    /// both report.
    pub fn ram_bytes_used(&self) -> i64 {
        self.values.capacity() as i64
            + (self.doc_ids.capacity() * std::mem::size_of::<i32>()) as i64
    }

    /// Freezes `values` and `doc_ids` into the shared buffer the codec reads.
    ///
    /// Equivalent to `bytes.freeze(false)` plus the fields Java's anonymous
    /// classes capture from the enclosing writer.
    fn freeze(&self, values: Vec<u8>, doc_ids: Vec<i32>) -> Arc<BufferedPoints> {
        Arc::new(BufferedPoints {
            values,
            doc_ids,
            num_points: self.num_points,
            num_docs: self.num_docs,
            packed_bytes_length: self.packed_bytes_length,
            packed_index_bytes_length: self.field_info.point_index_dimension_count as usize
                * self.field_info.point_num_bytes as usize,
            num_dimensions: self.field_info.point_dimension_count,
            num_index_dimensions: self.field_info.point_index_dimension_count,
            bytes_per_dimension: self.field_info.point_num_bytes,
            bounds: OnceLock::new(),
        })
    }

    /// Hands the buffered points to `writer`, which builds the BKD tree.
    ///
    /// Equivalent to `PointValuesWriter.flush(SegmentWriteState, Sorter.DocMap,
    /// PointsWriter)` without the doc map; see the module documentation. Java
    /// declares the `SegmentWriteState` parameter and never reads it, so this
    /// port does not take one.
    ///
    /// Flushing consumes the buffers, so the heap they held is given back to
    /// the shared counter here rather than when the writer is dropped, and the
    /// values are handed over rather than copied.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed — writing it twice would put two entries for one field into the
    /// same `.kdm`. Otherwise propagates whatever the codec raises while
    /// writing the field.
    pub fn flush(&mut self, writer: &mut dyn PointsWriter) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the point values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        self.flushed = true;
        let values = std::mem::take(&mut self.values);
        let doc_ids = std::mem::take(&mut self.doc_ids);
        let buffer = self.freeze(values, doc_ids);
        self.update_bytes_used();
        let reader = BufferedPointsReader {
            field_name: self.field_info.name.clone(),
            buffer,
        };
        writer.write_field(&self.field_info, &reader)
    }
}

impl Drop for PointValuesWriter {
    /// Gives back the bytes this writer charged to the shared counter.
    ///
    /// Java has no equivalent: `DocumentsWriterPerThread` throws its whole
    /// `Counter` away with the chain. Rust drops writers individually — a
    /// document that fails validation can leave one behind — so the counter is
    /// balanced here instead of drifting upwards. Same reasoning as
    /// [`crate::index::NormValuesWriter`].
    fn drop(&mut self) {
        self.iw_bytes_used
            .fetch_sub(self.bytes_used, Ordering::AcqRel);
    }
}

// ---------------------------------------------------------------------------
// The buffer the codec reads
// ---------------------------------------------------------------------------

/// The frozen buffer of one field, shared by the reader, the values and every
/// cursor over them.
///
/// Equivalent to the state Java's anonymous classes capture from the enclosing
/// `PointValuesWriter`: the frozen `PagedBytes.Reader`, `docIDs`, `numPoints`
/// and `packedBytesLength`.
#[derive(Debug)]
struct BufferedPoints {
    values: Vec<u8>,
    doc_ids: Vec<i32>,
    num_points: i32,
    num_docs: i32,
    packed_bytes_length: usize,
    packed_index_bytes_length: usize,
    num_dimensions: i32,
    num_index_dimensions: i32,
    bytes_per_dimension: i32,
    /// Per-index-dimension minimum and maximum over every buffered value,
    /// computed on first request. See the module documentation for why this
    /// exists where Java throws.
    bounds: OnceLock<(Vec<u8>, Vec<u8>)>,
}

impl BufferedPoints {
    /// Returns the packed bytes of the value stored at buffer slot `slot`.
    fn value(&self, slot: usize) -> &[u8] {
        let offset = slot * self.packed_bytes_length;
        &self.values[offset..offset + self.packed_bytes_length]
    }

    /// Returns the per-index-dimension bounds over every buffered value.
    ///
    /// The same computation as `BKDWriter.computePackedValueBounds`
    /// (`BKDWriter.java:468-518`): per index dimension, the unsigned minimum
    /// and maximum of that dimension's bytes. Permuting the points cannot
    /// change it, so one shared result serves every cursor.
    fn bounds(&self) -> &(Vec<u8>, Vec<u8>) {
        self.bounds.get_or_init(|| {
            let width = self.packed_index_bytes_length;
            if self.num_points == 0 || width == 0 {
                return (Vec::new(), Vec::new());
            }
            let first = &self.value(0)[..width];
            let mut min = first.to_vec();
            let mut max = first.to_vec();
            let bytes_per_dim = self.bytes_per_dimension as usize;
            for slot in 1..self.num_points as usize {
                let packed = &self.value(slot)[..width];
                for dim in 0..self.num_index_dimensions as usize {
                    let start = dim * bytes_per_dim;
                    let end = start + bytes_per_dim;
                    if packed[start..end] < min[start..end] {
                        min[start..end].copy_from_slice(&packed[start..end]);
                    } else if packed[start..end] > max[start..end] {
                        max[start..end].copy_from_slice(&packed[start..end]);
                    }
                }
            }
            (min, max)
        })
    }
}

// ---------------------------------------------------------------------------
// The reader handed to the codec
// ---------------------------------------------------------------------------

/// A single-field [`PointsReader`] over the buffer of one flushing field.
///
/// Equivalent to the anonymous `PointsReader` of `PointValuesWriter.flush`
/// (`PointValuesWriter.java:280-337`).
#[derive(Debug, Clone)]
struct BufferedPointsReader {
    field_name: String,
    buffer: Arc<BufferedPoints>,
}

impl PointsReader for BufferedPointsReader {
    /// Java throws `UnsupportedOperationException`: the flush path never
    /// verifies a buffer it has just built. Nothing in this crate calls it on
    /// a flushing reader either, and the answer is unambiguous, so it succeeds.
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    /// Returns the buffered values, refusing any other field.
    ///
    /// Equivalent to the `fieldName.equals(fieldInfo.name) == false` guard,
    /// whose message Java words as "fieldName must be the same".
    fn get_values(&self, field: &str) -> Result<Box<dyn PointValues>> {
        if field != self.field_name {
            return Err(LuceneError::IllegalArgument(format!(
                "fieldName must be the same: asked for \"{field}\" but this reader serves \"{}\"",
                self.field_name
            )));
        }
        Ok(Box::new(BufferedPointValues {
            buffer: Arc::clone(&self.buffer),
        }))
    }

    /// Java does not override `getMergeInstance`, whose default returns `this`;
    /// the buffer is shared, so a clone of the handle is the same thing.
    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(self.clone()))
    }

    /// Equivalent to the empty `close()`: the buffer is owned by the writer.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The values and the mutable tree over them
// ---------------------------------------------------------------------------

/// The [`PointValues`] view of one flushing field's buffer.
///
/// Equivalent to the anonymous `PointValues` returned by the flush-time
/// `PointsReader.getValues` (`PointValuesWriter.java:287-327`), which
/// implements only `getPointTree()`; see the module documentation for why this
/// one answers the rest.
#[derive(Debug)]
struct BufferedPointValues {
    buffer: Arc<BufferedPoints>,
}

impl PointValues for BufferedPointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(BufferedPointTree::new(Arc::clone(&self.buffer))))
    }

    fn size(&self) -> i64 {
        i64::from(self.buffer.num_points)
    }

    fn doc_count(&self) -> i32 {
        self.buffer.num_docs
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        let (min, _) = self.buffer.bounds();
        Ok((!min.is_empty()).then(|| min.clone()))
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        let (_, max) = self.buffer.bounds();
        Ok((!max.is_empty()).then(|| max.clone()))
    }

    fn num_dimensions(&self) -> Result<i32> {
        Ok(self.buffer.num_dimensions)
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        Ok(self.buffer.num_index_dimensions)
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        Ok(self.buffer.bytes_per_dimension)
    }
}

/// A [`MutablePointTree`] over one flushing field's buffer.
///
/// Equivalent to the anonymous `MutablePointTree` of `PointValuesWriter.flush`
/// (`PointValuesWriter.java:206-272`).
///
/// The buffer itself never moves. Order is carried entirely by `ords`, an
/// indirection the codec permutes through [`swap`](MutablePointTree::swap),
/// [`save`](MutablePointTree::save) and
/// [`restore`](MutablePointTree::restore), exactly as Java's does — which is
/// what lets the BKD writer sort a hundred million points without copying a
/// byte of payload.
#[derive(Debug)]
struct BufferedPointTree {
    buffer: Arc<BufferedPoints>,
    /// `ords[i]` is the buffer slot currently at logical position `i`.
    ords: Vec<i32>,
    /// The scratch `save`/`restore` use, allocated on first `save` exactly as
    /// Java's `temp` is.
    temp: Option<Vec<i32>>,
}

impl BufferedPointTree {
    /// Creates a cursor in buffer order, `ords[i] == i`.
    fn new(buffer: Arc<BufferedPoints>) -> Self {
        let ords = (0..buffer.num_points).collect();
        Self {
            buffer,
            ords,
            temp: None,
        }
    }

    /// The buffer slot at logical position `i`.
    fn slot(&self, i: i32) -> usize {
        self.ords[i as usize] as usize
    }
}

impl PointTree for BufferedPointTree {
    /// Java's `MutablePointTree.clone()` throws `UnsupportedOperationException`
    /// because nothing needs it. The Rust signature has no error channel, and a
    /// panic in a public method is a worse answer than a correct one, so this
    /// returns an independent cursor over the same buffer, in the order this
    /// one currently holds. Re-rooting is trivial: a mutable tree is a single
    /// leaf that is also its own root.
    fn clone_tree(&self) -> Box<dyn PointTree> {
        Box::new(Self {
            buffer: Arc::clone(&self.buffer),
            ords: self.ords.clone(),
            temp: None,
        })
    }

    /// Always `false`: a mutable tree is one leaf. Java makes this `final`.
    fn move_to_child(&mut self) -> Result<bool> {
        Ok(false)
    }

    /// Always `false`: a mutable tree is one leaf. Java makes this `final`.
    fn move_to_sibling(&mut self) -> Result<bool> {
        Ok(false)
    }

    /// Always `false`: the single leaf is also the root. Java makes this
    /// `final`.
    fn move_to_parent(&mut self) -> Result<bool> {
        Ok(false)
    }

    /// Java throws here; see the module documentation. The bounds are the
    /// per-index-dimension minimum over every buffered value, computed once.
    fn min_packed_value(&self) -> &[u8] {
        &self.buffer.bounds().0
    }

    /// Java throws here; see the module documentation.
    fn max_packed_value(&self) -> &[u8] {
        &self.buffer.bounds().1
    }

    /// Java throws here; the flush path visits values, never bare doc IDs.
    /// Visiting them is well defined over a buffer, so this port does it in the
    /// order the cursor currently holds — the same order
    /// [`visit_doc_values`](PointTree::visit_doc_values) would use.
    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        visitor.grow(self.buffer.num_points);
        for i in 0..self.buffer.num_points {
            visitor.visit(self.buffer.doc_ids[self.slot(i)])?;
        }
        Ok(())
    }

    /// Visits every buffered point in the order the cursor currently holds.
    ///
    /// Equivalent to the anonymous tree's `visitDocValues`
    /// (`PointValuesWriter.java:222-232`). Java copies each value into a
    /// reusable `byte[]` because `PagedBytes.fillSlice` hands back a `BytesRef`
    /// with an offset; a borrowed slice of the flat buffer is the same bytes
    /// without the copy.
    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        for i in 0..self.buffer.num_points {
            let slot = self.slot(i);
            visitor.visit_with_value(self.buffer.doc_ids[slot], self.buffer.value(slot))?;
        }
        Ok(())
    }

    fn size(&self) -> i64 {
        i64::from(self.buffer.num_points)
    }

    fn as_mutable(&mut self) -> Option<&mut dyn MutablePointTree> {
        Some(self)
    }
}

impl MutablePointTree for BufferedPointTree {
    fn value(&self, i: i32) -> &[u8] {
        self.buffer.value(self.slot(i))
    }

    fn byte_at(&self, i: i32, k: i32) -> u8 {
        let offset = self.slot(i) * self.buffer.packed_bytes_length + k as usize;
        self.buffer.values[offset]
    }

    fn doc_id(&self, i: i32) -> i32 {
        self.buffer.doc_ids[self.slot(i)]
    }

    fn swap(&mut self, i: i32, j: i32) {
        self.ords.swap(i as usize, j as usize);
    }

    fn save(&mut self, i: i32, j: i32) {
        let temp = self.temp.get_or_insert_with(|| vec![0i32; self.ords.len()]);
        temp[j as usize] = self.ords[i as usize];
    }

    fn restore(&mut self, i: i32, j: i32) {
        if let Some(temp) = self.temp.as_ref() {
            let range = i as usize..j as usize;
            self.ords[range.clone()].copy_from_slice(&temp[range]);
        }
    }
}

// ---------------------------------------------------------------------------
// A visitor that only needs the values, used by the tests and by callers that
// want the buffer back.
// ---------------------------------------------------------------------------

/// Collects `(docID, packedValue)` pairs in visit order.
///
/// Not a port of anything: it exists so that a caller — the unit tests here,
/// and any diagnostic that wants to see what a field buffered — can read a
/// [`MutablePointTree`] without writing a visitor of its own.
#[derive(Debug, Default)]
pub struct CollectingPointVisitor {
    /// The `(docID, packedValue)` pairs seen so far, in visit order.
    pub visited: Vec<(i32, Vec<u8>)>,
}

impl IntersectVisitor for CollectingPointVisitor {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.visited.push((doc_id, Vec::new()));
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.visited.push((doc_id, packed_value.to_vec()));
        Ok(())
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Relation {
        Relation::CellCrossesQuery
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::points::EmptyPointsWriter;
    use crate::index::field_infos::FieldInfo;

    fn counter() -> Arc<AtomicI64> {
        Arc::new(AtomicI64::new(0))
    }

    fn field(name: &str, dims: i32, index_dims: i32, bytes: i32) -> FieldInfo {
        let mut info = FieldInfo::new(name, 0);
        info.point_dimension_count = dims;
        info.point_index_dimension_count = index_dims;
        info.point_num_bytes = bytes;
        info
    }

    fn packed(bytes: &[u8]) -> BytesRef {
        BytesRef::new(bytes.to_vec())
    }

    fn writer_1d(bytes_used: &Arc<AtomicI64>) -> PointValuesWriter {
        PointValuesWriter::new(field("p", 1, 1, 4), Arc::clone(bytes_used))
    }

    /// Builds exactly the tree `flush` would hand the codec, without needing a
    /// codec and without consuming the writer's buffers.
    fn tree_of(writer: &PointValuesWriter) -> Box<dyn PointTree> {
        let buffer = writer.freeze(writer.values.clone(), writer.doc_ids.clone());
        Box::new(BufferedPointTree::new(buffer))
    }

    // -- buffering and bookkeeping -----------------------------------------

    #[test]
    fn several_values_in_one_document_count_once_as_a_document() {
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        writer.add_packed_value(0, &packed(&[0, 0, 0, 1])).unwrap();
        writer.add_packed_value(0, &packed(&[0, 0, 0, 2])).unwrap();
        writer.add_packed_value(0, &packed(&[0, 0, 0, 3])).unwrap();
        writer.add_packed_value(7, &packed(&[0, 0, 0, 4])).unwrap();
        // `numDocs` counts distinct doc IDs, `numPoints` counts values: the
        // single `docID != lastDocID` test Lucene uses.
        assert_eq!(writer.num_points(), 4);
        assert_eq!(writer.num_docs(), 2);
    }

    #[test]
    fn a_returning_doc_id_is_counted_again_exactly_as_lucene_does() {
        // Lucene compares against `lastDocID` only, so an out-of-order repeat
        // would be counted twice. The chain never produces one; this pins the
        // rule rather than a stricter invention.
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        writer.add_packed_value(0, &packed(&[0, 0, 0, 1])).unwrap();
        writer.add_packed_value(1, &packed(&[0, 0, 0, 2])).unwrap();
        writer.add_packed_value(0, &packed(&[0, 0, 0, 3])).unwrap();
        assert_eq!(writer.num_docs(), 3);
    }

    #[test]
    fn a_value_of_the_wrong_length_is_refused() {
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        let error = writer
            .add_packed_value(0, &packed(&[0, 0, 0]))
            .expect_err("a short value would shift every later value");
        match error {
            LuceneError::IllegalArgument(message) => {
                assert!(message.contains("has length=3"), "{message}");
                assert!(message.contains("should be 4"), "{message}");
            }
            other => panic!("expected IllegalArgument, got {other:?}"),
        }
        // Nothing was buffered.
        assert_eq!(writer.num_points(), 0);
        assert_eq!(writer.num_docs(), 0);
    }

    #[test]
    fn the_packed_length_spans_every_dimension() {
        let bytes_used = counter();
        let mut writer = PointValuesWriter::new(field("p", 3, 2, 4), Arc::clone(&bytes_used));
        writer
            .add_packed_value(0, &packed(&[0; 12]))
            .expect("3 dimensions of 4 bytes");
        writer
            .add_packed_value(1, &packed(&[0; 8]))
            .expect_err("the index dimension count does not shorten a value");
    }

    // -- RAM accounting ----------------------------------------------------

    #[test]
    fn the_shared_counter_tracks_and_is_refunded() {
        let bytes_used = counter();
        {
            let mut writer = writer_1d(&bytes_used);
            let after_construction = bytes_used.load(Ordering::Acquire);
            assert!(
                after_construction >= (INITIAL_DOC_IDS * 4) as i64,
                "the initial doc-ID buffer must be charged, got {after_construction}"
            );
            for doc in 0..1000 {
                writer
                    .add_packed_value(doc, &packed(&doc.to_be_bytes()))
                    .unwrap();
            }
            assert!(
                bytes_used.load(Ordering::Acquire) > after_construction,
                "buffering must charge the counter"
            );
            assert_eq!(bytes_used.load(Ordering::Acquire), writer.ram_bytes_used());
        }
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            0,
            "dropping the writer must refund everything it charged"
        );
    }

    #[test]
    fn flushing_returns_the_buffers_to_the_counter() {
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        for doc in 0..100 {
            writer
                .add_packed_value(doc, &packed(&doc.to_be_bytes()))
                .unwrap();
        }
        let mut points_writer = EmptyPointsWriter;
        writer.flush(&mut points_writer).expect("flush");
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            0,
            "flush hands the buffers over, so their bytes are given back"
        );
    }

    #[test]
    fn a_field_cannot_be_flushed_twice() {
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        writer.add_packed_value(0, &packed(&[0, 0, 0, 1])).unwrap();
        let mut points_writer = EmptyPointsWriter;
        writer.flush(&mut points_writer).expect("first flush");
        let error = writer
            .flush(&mut points_writer)
            .expect_err("two entries for one field would corrupt the .kdm");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
        let error = writer
            .add_packed_value(1, &packed(&[0, 0, 0, 2]))
            .expect_err("a value added after the flush would be lost");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn an_empty_field_still_flushes() {
        // Lucene reaches `writeField` for a writer that buffered nothing; the
        // BKD writer then produces no metadata entry. Nothing may fail here.
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        let mut points_writer = EmptyPointsWriter;
        writer.flush(&mut points_writer).expect("flush");
        assert_eq!(writer.num_points(), 0);
    }

    // -- the reader the codec sees -----------------------------------------

    #[test]
    fn the_reader_serves_only_its_own_field() {
        let buffer = Arc::new(BufferedPoints {
            values: vec![0, 0, 0, 1],
            doc_ids: vec![3],
            num_points: 1,
            num_docs: 1,
            packed_bytes_length: 4,
            packed_index_bytes_length: 4,
            num_dimensions: 1,
            num_index_dimensions: 1,
            bytes_per_dimension: 4,
            bounds: OnceLock::new(),
        });
        let reader = BufferedPointsReader {
            field_name: "p".to_string(),
            buffer,
        };
        let values = reader.get_values("p").expect("its own field");
        assert_eq!(values.size(), 1);
        assert_eq!(values.doc_count(), 1);
        assert_eq!(values.num_dimensions().unwrap(), 1);
        assert_eq!(values.bytes_per_dimension().unwrap(), 4);
        match reader.get_values("other") {
            Err(LuceneError::IllegalArgument(_)) => {}
            Err(other) => panic!("expected IllegalArgument, got {other:?}"),
            Ok(_) => panic!("Lucene refuses any other field name"),
        }
    }

    // -- the mutable-tree contract -----------------------------------------

    #[test]
    fn the_tree_reads_the_buffer_in_insertion_order() {
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        for (doc, value) in [(0i32, 30u32), (0, 10), (5, 20)] {
            writer
                .add_packed_value(doc, &packed(&value.to_be_bytes()))
                .unwrap();
        }
        let mut tree = tree_of(&writer);
        assert_eq!(tree.size(), 3);
        let mut visitor = CollectingPointVisitor::default();
        tree.visit_doc_values(&mut visitor).unwrap();
        assert_eq!(
            visitor.visited,
            vec![
                (0, 30u32.to_be_bytes().to_vec()),
                (0, 10u32.to_be_bytes().to_vec()),
                (5, 20u32.to_be_bytes().to_vec()),
            ],
            "a mutable tree starts in buffer order; the codec is what sorts it"
        );
    }

    #[test]
    fn swap_permutes_the_view_without_touching_the_buffer() {
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        for (doc, value) in [(0i32, 30u32), (1, 10), (2, 20)] {
            writer
                .add_packed_value(doc, &packed(&value.to_be_bytes()))
                .unwrap();
        }
        let mut tree = tree_of(&writer);
        {
            let mutable = tree.as_mutable().expect("a flushing tree is mutable");
            mutable.swap(0, 1);
            assert_eq!(mutable.doc_id(0), 1);
            assert_eq!(mutable.doc_id(1), 0);
            assert_eq!(mutable.value(0), 10u32.to_be_bytes());
            assert_eq!(mutable.value(1), 30u32.to_be_bytes());
            // `byte_at` must follow the same indirection as `value`.
            for i in 0..3i32 {
                let value = mutable.value(i).to_vec();
                for (k, expected) in value.iter().enumerate() {
                    assert_eq!(mutable.byte_at(i, k as i32), *expected, "i={i} k={k}");
                }
            }
        }
        let mut visitor = CollectingPointVisitor::default();
        tree.visit_doc_values(&mut visitor).unwrap();
        assert_eq!(
            visitor.visited,
            vec![
                (1, 10u32.to_be_bytes().to_vec()),
                (0, 30u32.to_be_bytes().to_vec()),
                (2, 20u32.to_be_bytes().to_vec()),
            ],
            "visiting must follow the permuted order"
        );
    }

    #[test]
    fn save_and_restore_are_asymmetric_exactly_as_lucene_specifies() {
        // `save(i, j)` copies one element from live position `i` to scratch
        // position `j`; `restore(i, j)` copies the half-open range `[i, j)`
        // from the scratch back over the live order. A symmetric reading of
        // the pair — which is the natural mistake — reverses the effect.
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        for doc in 0..6i32 {
            writer
                .add_packed_value(doc, &packed(&(doc as u32).to_be_bytes()))
                .unwrap();
        }
        let mut tree = tree_of(&writer);
        let mutable = tree.as_mutable().expect("mutable");

        // Stash the live order of [1, 4) into scratch slots 1..4, reversed.
        mutable.save(1, 3);
        mutable.save(2, 2);
        mutable.save(3, 1);
        // Now scramble the live order in that range.
        mutable.swap(1, 3);
        mutable.swap(2, 3);
        assert_ne!(
            (mutable.doc_id(1), mutable.doc_id(2), mutable.doc_id(3)),
            (1, 2, 3)
        );
        // Restoring [1, 4) puts back exactly what was saved into 1..4.
        mutable.restore(1, 4);
        assert_eq!(
            (mutable.doc_id(1), mutable.doc_id(2), mutable.doc_id(3)),
            (3, 2, 1),
            "restore copies scratch[i..j] over ords[i..j], so the saved order returns"
        );
        // Outside the restored range nothing moved.
        assert_eq!(mutable.doc_id(0), 0);
        assert_eq!(mutable.doc_id(4), 4);
        assert_eq!(mutable.doc_id(5), 5);
    }

    #[test]
    fn restore_before_any_save_is_a_no_op() {
        // Java guards with `if (temp != null)`, because the sorter can call
        // `restore` on a range it never saved.
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        for doc in 0..4i32 {
            writer
                .add_packed_value(doc, &packed(&(doc as u32).to_be_bytes()))
                .unwrap();
        }
        let mut tree = tree_of(&writer);
        let mutable = tree.as_mutable().expect("mutable");
        mutable.restore(0, 4);
        assert_eq!(
            (0..4).map(|i| mutable.doc_id(i)).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn a_full_permutation_survives_save_and_restore() {
        // A stand-in for what a stable radix sort does: save the whole order,
        // scramble it completely, restore the whole order.
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        for doc in 0..32i32 {
            writer
                .add_packed_value(doc, &packed(&(doc as u32).to_be_bytes()))
                .unwrap();
        }
        let mut tree = tree_of(&writer);
        let mutable = tree.as_mutable().expect("mutable");
        for i in 0..32 {
            mutable.save(i, 31 - i);
        }
        for i in 0..16 {
            mutable.swap(i, 31 - i);
        }
        mutable.restore(0, 32);
        assert_eq!(
            (0..32).map(|i| mutable.doc_id(i)).collect::<Vec<_>>(),
            (0..32).rev().collect::<Vec<_>>(),
            "the scratch held the reversed order, so that is what comes back"
        );
    }

    #[test]
    fn a_mutable_tree_is_a_single_leaf_that_is_its_own_root() {
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        writer.add_packed_value(0, &packed(&[0, 0, 0, 1])).unwrap();
        let mut tree = tree_of(&writer);
        assert!(!tree.move_to_child().unwrap());
        assert!(!tree.move_to_sibling().unwrap());
        assert!(!tree.move_to_parent().unwrap());
    }

    #[test]
    fn a_cloned_cursor_keeps_the_order_and_is_independent() {
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        for doc in 0..4i32 {
            writer
                .add_packed_value(doc, &packed(&(doc as u32).to_be_bytes()))
                .unwrap();
        }
        let mut tree = tree_of(&writer);
        tree.as_mutable().expect("mutable").swap(0, 3);
        let mut copy = tree.clone_tree();
        assert_eq!(copy.as_mutable().expect("mutable").doc_id(0), 3);
        copy.as_mutable().expect("mutable").swap(0, 3);
        assert_eq!(
            tree.as_mutable().expect("mutable").doc_id(0),
            3,
            "the original must not follow the copy"
        );
    }

    #[test]
    fn the_bounds_span_the_index_dimensions_only() {
        // Two index dimensions of three, so the third must not appear in the
        // bounds: `computePackedValueBounds` works over
        // `packedIndexBytesLength`.
        let bytes_used = counter();
        let mut writer = PointValuesWriter::new(field("p", 3, 2, 1), Arc::clone(&bytes_used));
        writer.add_packed_value(0, &packed(&[5, 9, 200])).unwrap();
        writer.add_packed_value(1, &packed(&[7, 2, 1])).unwrap();
        writer.add_packed_value(2, &packed(&[3, 4, 90])).unwrap();
        let tree = tree_of(&writer);
        assert_eq!(tree.min_packed_value(), &[3, 2]);
        assert_eq!(tree.max_packed_value(), &[7, 9]);
    }

    #[test]
    fn the_bounds_compare_unsigned() {
        // The high bit must not read as negative: 0x80 is above 0x7f.
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        writer
            .add_packed_value(0, &packed(&[0x7f, 0, 0, 0]))
            .unwrap();
        writer
            .add_packed_value(1, &packed(&[0x80, 0, 0, 0]))
            .unwrap();
        let tree = tree_of(&writer);
        assert_eq!(tree.min_packed_value(), &[0x7f, 0, 0, 0]);
        assert_eq!(tree.max_packed_value(), &[0x80, 0, 0, 0]);
    }

    #[test]
    fn the_bounds_of_an_empty_buffer_are_empty() {
        let bytes_used = counter();
        let writer = writer_1d(&bytes_used);
        let tree = tree_of(&writer);
        assert!(tree.min_packed_value().is_empty());
        assert!(tree.max_packed_value().is_empty());
    }

    #[test]
    fn visiting_doc_ids_follows_the_current_order() {
        let bytes_used = counter();
        let mut writer = writer_1d(&bytes_used);
        for doc in 0..4i32 {
            writer
                .add_packed_value(doc, &packed(&(doc as u32).to_be_bytes()))
                .unwrap();
        }
        let mut tree = tree_of(&writer);
        tree.as_mutable().expect("mutable").swap(0, 3);
        let mut visitor = CollectingPointVisitor::default();
        tree.visit_doc_ids(&mut visitor).unwrap();
        assert_eq!(
            visitor
                .visited
                .iter()
                .map(|(doc, _)| *doc)
                .collect::<Vec<_>>(),
            vec![3, 1, 2, 0]
        );
    }
}
