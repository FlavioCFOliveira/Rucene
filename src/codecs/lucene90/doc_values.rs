//! Lucene 9.0 doc-values format implementation.
//!
//! Ports `org.apache.lucene.codecs.lucene90.Lucene90DocValuesFormat`,
//! `Lucene90DocValuesConsumer`, `Lucene90DocValuesProducer` and `IndexedDISI`
//! from Apache Lucene Core 10.5.0.
//!
//! Three files are written per segment:
//!
//! * `.dvd` – doc-values data.
//! * `.dvm` – doc-values metadata.
//! * `.dvs` – doc-values skip index.
//!
//! The format supports numeric, binary, sorted, sorted-set and sorted-numeric
//! doc values, using `IndexedDISI` for documents-with-values, `DirectMonotonic`
//! for monotonic addresses, `DirectWriter` for bit-packed values, and
//! `BlockPacked` for multi-block varying-BPV numerics.

#![deny(unsafe_code)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use crate::codecs::codec_util::{
    check_index_header, checksum_entire_file, retrieve_checksum, write_footer, write_index_header,
};
use crate::codecs::doc_values::{
    BinaryDocValues, DocValuesConsumer, DocValuesFormat, DocValuesProducer, DocValuesSkipper,
    EmptyBinaryDocValues, EmptyNumericDocValues, EmptySortedDocValues, EmptySortedNumericDocValues,
    EmptySortedSetDocValues, NumericDocValues, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues,
};
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::doc_values::DocValuesIterator;
use crate::index::index_file_names::segment_file_name;
use crate::search::{DocIdSetIterator, SortedSetSelector, SortedSetSelectorType, NO_MORE_DOCS};
use crate::store::{
    ByteArrayDataOutput, ByteBuffersDataOutput, ByteBuffersIndexOutput, DataOutput, IOContext,
    IndexInput, IndexOutput, RandomAccessInput,
};
use crate::util::compress::{FastCompressionHashTable, Lz4};
use crate::util::packed::{
    DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter, DirectReader, DirectWriter,
};
use crate::util::{ArrayUtil, BytesRef, BytesRefBuilder, StringHelper};

// -----------------------------------------------------------------------------
// Format constants
// -----------------------------------------------------------------------------

const DATA_CODEC: &str = "Lucene90DocValuesData";
const META_CODEC: &str = "Lucene90DocValuesMetadata";
const SKIP_INDEX_CODEC: &str = "Lucene90DocValuesSkipIndex";

const DATA_EXTENSION: &str = "dvd";
const META_EXTENSION: &str = "dvm";
const SKIP_INDEX_EXTENSION: &str = "dvs";

const VERSION_START: i32 = 0;
#[allow(dead_code)]
const VERSION_SKIPPER_SEPARATE_FILE: i32 = 1;
#[allow(dead_code)]
const VERSION_SKIPPER_MAX_VALUE_COUNT: i32 = 2;
const VERSION_CURRENT: i32 = VERSION_SKIPPER_MAX_VALUE_COUNT;

const TYPE_NUMERIC: u8 = 0;
const TYPE_BINARY: u8 = 1;
const TYPE_SORTED: u8 = 2;
const TYPE_SORTED_SET: u8 = 3;
const TYPE_SORTED_NUMERIC: u8 = 4;

const DIRECT_MONOTONIC_BLOCK_SHIFT: i32 = 16;

const NUMERIC_BLOCK_SHIFT: i32 = 14;
const NUMERIC_BLOCK_SIZE: usize = 1 << NUMERIC_BLOCK_SHIFT;

const TERMS_DICT_BLOCK_LZ4_SHIFT: i32 = 6;
const TERMS_DICT_BLOCK_LZ4_SIZE: usize = 1 << TERMS_DICT_BLOCK_LZ4_SHIFT;
const TERMS_DICT_BLOCK_LZ4_MASK: usize = TERMS_DICT_BLOCK_LZ4_SIZE - 1;

const TERMS_DICT_REVERSE_INDEX_SHIFT: i32 = 10;
const TERMS_DICT_REVERSE_INDEX_SIZE: usize = 1 << TERMS_DICT_REVERSE_INDEX_SHIFT;
const TERMS_DICT_REVERSE_INDEX_MASK: usize = TERMS_DICT_REVERSE_INDEX_SIZE - 1;

const DEFAULT_SKIP_INDEX_INTERVAL_SIZE: i32 = 4096;
#[allow(dead_code)]
const SKIP_INDEX_INTERVAL_BYTES: i64 = 29;
#[allow(dead_code)]
const SKIP_INDEX_LEVEL_SHIFT: i32 = 3;
#[allow(dead_code)]
const SKIP_INDEX_MAX_LEVEL: usize = 4;

/// Jump length per level, pre-computed from Java's static initializer
/// (`Lucene90DocValuesFormat.java:232-246`). The skipper's `advance` skips
/// this many bytes past an interval whose `maxDocID` is below the target
/// (`Lucene90DocValuesProducer.java:2408-2411`).
const SKIP_INDEX_JUMP_LENGTH_PER_LEVEL: [i64; SKIP_INDEX_MAX_LEVEL] = [
    SKIP_INDEX_INTERVAL_BYTES - 5,
    SKIP_INDEX_INTERVAL_BYTES - 5 + (1 << SKIP_INDEX_LEVEL_SHIFT) * SKIP_INDEX_INTERVAL_BYTES - 1,
    SKIP_INDEX_INTERVAL_BYTES - 5
        + (1 << SKIP_INDEX_LEVEL_SHIFT) * SKIP_INDEX_INTERVAL_BYTES
        + (1 << (2 * SKIP_INDEX_LEVEL_SHIFT)) * SKIP_INDEX_INTERVAL_BYTES
        - (1 << SKIP_INDEX_LEVEL_SHIFT)
        - 1,
    SKIP_INDEX_INTERVAL_BYTES - 5
        + (1 << SKIP_INDEX_LEVEL_SHIFT) * SKIP_INDEX_INTERVAL_BYTES
        + (1 << (2 * SKIP_INDEX_LEVEL_SHIFT)) * SKIP_INDEX_INTERVAL_BYTES
        + (1 << (3 * SKIP_INDEX_LEVEL_SHIFT)) * SKIP_INDEX_INTERVAL_BYTES
        - (1 << SKIP_INDEX_LEVEL_SHIFT)
        - (1 << (2 * SKIP_INDEX_LEVEL_SHIFT))
        - 1,
];

// -----------------------------------------------------------------------------
// IndexedDISI
// -----------------------------------------------------------------------------

use crate::codecs::lucene90::indexed_disi::{write_bit_set, IndexedDISI, DEFAULT_DENSE_RANK_POWER};

// -----------------------------------------------------------------------------

// Format
// -----------------------------------------------------------------------------

/// Lucene 9.0 doc-values format.
#[derive(Debug, Clone, Copy)]
pub struct Lucene90DocValuesFormat {
    skip_index_interval_size: i32,
}

impl Lucene90DocValuesFormat {
    /// Creates a new format instance with the default skip-index interval size.
    pub fn new() -> Self {
        Self::with_interval(DEFAULT_SKIP_INDEX_INTERVAL_SIZE)
    }

    /// Creates a new format instance with the given skip-index interval size.
    pub fn with_interval(skip_index_interval_size: i32) -> Self {
        assert!(
            skip_index_interval_size >= 2,
            "skipIndexIntervalSize must be >= 2"
        );
        Self {
            skip_index_interval_size,
        }
    }
}

impl Default for Lucene90DocValuesFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl DocValuesFormat for Lucene90DocValuesFormat {
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn DocValuesConsumer + 'a>> {
        Ok(Box::new(Lucene90DocValuesConsumer::new(
            state,
            self.skip_index_interval_size,
        )?))
    }

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Lucene90DocValuesProducer::new(state)?))
    }
}

// -----------------------------------------------------------------------------
// Consumer
// -----------------------------------------------------------------------------

/// Writer for [`Lucene90DocValuesFormat`].
///
/// The three outputs are opened in the segment's directory by the constructor
/// and written as each field arrives, as Java's are
/// (`Lucene90DocValuesConsumer.java:73-125`).
pub struct Lucene90DocValuesConsumer<'a> {
    data: Box<dyn IndexOutput>,
    meta: Box<dyn IndexOutput>,
    skip_index: Box<dyn IndexOutput>,
    /// Set once [`close`](DocValuesConsumer::close) ran: Java nulls the three
    /// outputs (`:147`), so a second close does nothing.
    closed: bool,
    /// Scratch buffer for the terms dictionary, reused across every field of
    /// the segment that writes one.
    ///
    /// Equivalent to the field `termsDictBuffer`
    /// (`Lucene90DocValuesConsumer.java:69`), allocated once at `1 << 14`
    /// bytes in the constructor (`:83`) and grown in place by
    /// [`Self::maybe_grow_buffer`]. It is a field, and not a local of
    /// [`Self::add_terms_dict_values`], because Lucene makes it one: a second
    /// terms-dictionary field in the same segment starts from the length the
    /// first grew it to.
    terms_dict_buffer: Vec<u8>,
    max_doc: i32,
    #[allow(dead_code)]
    skip_index_interval_size: i32,
    _context: std::marker::PhantomData<&'a dyn IOContext>,
}

/// One interval of the doc-values skip index: the document range it covers and
/// the value range within it.
///
/// Equivalent to `Lucene90DocValuesConsumer.SkipAccumulator`. A range query can
/// skip a whole interval whose value range does not meet it, which is the
/// entire purpose of the `.dvs` file.
#[derive(Debug, Clone)]
struct SkipAccumulator {
    min_doc_id: i32,
    max_doc_id: i32,
    doc_count: i32,
    min_value: i64,
    max_value: i64,
}

impl SkipAccumulator {
    /// Starts an interval at `doc_id`.
    fn new(doc_id: i32) -> Self {
        Self {
            min_doc_id: doc_id,
            max_doc_id: doc_id,
            doc_count: 0,
            min_value: i64::MAX,
            max_value: i64::MIN,
        }
    }

    /// Returns whether this interval is full.
    ///
    /// Equivalent to `SkipAccumulator.isDone(int, int, long, int)`. Past the
    /// interval size the accumulator keeps taking documents while doing so
    /// costs nothing: while the next document is single-valued, holds the one
    /// value this interval already holds, and continues it without a gap. Such
    /// documents narrow no range, so splitting on them would only add intervals
    /// a query still has to read.
    fn is_done(
        &self,
        skip_index_interval_size: i32,
        value_count: i32,
        next_value: i64,
        next_doc: i32,
    ) -> bool {
        if self.doc_count < skip_index_interval_size {
            return false;
        }
        value_count > 1
            || self.min_value != self.max_value
            || self.min_value != next_value
            || self.doc_count != next_doc - self.min_doc_id
    }

    /// Widens the value range by one value.
    ///
    /// Equivalent to `SkipAccumulator.accumulate(long)`.
    fn accumulate(&mut self, value: i64) {
        self.min_value = self.min_value.min(value);
        self.max_value = self.max_value.max(value);
    }

    /// Widens this interval to cover another one.
    ///
    /// Equivalent to `SkipAccumulator.accumulate(SkipAccumulator)`.
    fn accumulate_other(&mut self, other: &Self) {
        debug_assert!(self.min_doc_id <= other.min_doc_id && self.max_doc_id < other.max_doc_id);
        self.max_doc_id = other.max_doc_id;
        self.min_value = self.min_value.min(other.min_value);
        self.max_value = self.max_value.max(other.max_value);
        self.doc_count += other.doc_count;
    }

    /// Extends this interval to `doc_id`.
    ///
    /// Equivalent to `SkipAccumulator.nextDoc(int)`.
    fn next_doc(&mut self, doc_id: i32) {
        self.max_doc_id = doc_id;
        self.doc_count += 1;
    }

    /// Merges `length` consecutive intervals into one.
    ///
    /// Equivalent to `SkipAccumulator.merge(List, int, int)`, which is how a
    /// higher level of the skip index is built out of the level below it.
    fn merge(list: &[Self], index: usize, length: usize) -> Self {
        let mut acc = Self::new(list[index].min_doc_id);
        for i in 0..length {
            acc.accumulate_other(&list[index + i]);
        }
        acc
    }
}

/// Builds the level above `accumulators` by merging every group of
/// `1 << SKIP_INDEX_LEVEL_SHIFT` intervals.
///
/// Equivalent to `Lucene90DocValuesConsumer.buildLevel(List)`. A trailing group
/// that is not full is dropped: a partial interval at a higher level would
/// claim to cover documents the level below still has to be read for.
fn build_level(accumulators: &[SkipAccumulator]) -> Vec<SkipAccumulator> {
    let level_size = 1usize << SKIP_INDEX_LEVEL_SHIFT;
    let mut collector = Vec::new();
    if accumulators.len() < level_size {
        return collector;
    }
    let mut i = 0usize;
    while i + level_size <= accumulators.len() {
        collector.push(SkipAccumulator::merge(accumulators, i, level_size));
        i += level_size;
    }
    collector
}

/// Returns how many levels the interval at `index` starts.
///
/// Equivalent to `Lucene90DocValuesConsumer.getLevels(int, int)`. An interval
/// starts a level only where its index is aligned to that level's span and
/// enough intervals remain to fill it, which is what lets a reader skip a whole
/// level by reading one entry.
fn get_levels(index: usize, size: usize) -> usize {
    if (index as u32).trailing_zeros() >= SKIP_INDEX_LEVEL_SHIFT as u32 {
        let left = size - index;
        for level in (1..SKIP_INDEX_MAX_LEVEL).rev() {
            let number_intervals = 1usize << (SKIP_INDEX_LEVEL_SHIFT as usize * level);
            if left >= number_intervals && index % number_intervals == 0 {
                return level + 1;
            }
        }
    }
    1
}

impl fmt::Debug for Lucene90DocValuesConsumer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90DocValuesConsumer")
            .field("max_doc", &self.max_doc)
            .finish_non_exhaustive()
    }
}

impl Lucene90DocValuesConsumer<'_> {
    /// Writes one field's skip index: the value range of every interval, at
    /// every level.
    ///
    /// Equivalent to `Lucene90DocValuesConsumer.writeSkipIndex(FieldInfo, DocValuesProducer)`.
    /// The metadata entry that follows records where the index starts, how long
    /// it is, and the field's global range — which is what lets a query decide
    /// in one read whether the field can match at all.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the values or writing the index raises.
    fn write_skip_index(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        debug_assert_ne!(
            field.doc_values_skip_index_type,
            crate::index::DocValuesSkipIndexType::NONE
        );
        let start = self.skip_index.file_pointer();
        let mut values =
            crate::codecs::doc_values::non_null(values_producer.get_sorted_numeric(field)?, field)?;
        let mut global_max_value = i64::MIN;
        let mut global_min_value = i64::MAX;
        let mut global_doc_count = 0i32;
        let mut global_max_value_count = 0i32;
        let mut max_doc_id = -1i32;
        let mut accumulators: Vec<SkipAccumulator> = Vec::new();
        let mut accumulator: Option<SkipAccumulator> = None;
        let max_accumulators =
            1usize << (SKIP_INDEX_LEVEL_SHIFT as usize * (SKIP_INDEX_MAX_LEVEL - 1));

        let mut doc = values.next_doc()?;
        while doc != NO_MORE_DOCS {
            let value_count = values.doc_value_count()?;
            let first_value = values.next_value()?;
            global_max_value_count = global_max_value_count.max(value_count);
            if let Some(acc) = accumulator.as_ref() {
                if acc.is_done(self.skip_index_interval_size, value_count, first_value, doc) {
                    global_max_value = global_max_value.max(acc.max_value);
                    global_min_value = global_min_value.min(acc.min_value);
                    global_doc_count += acc.doc_count;
                    max_doc_id = acc.max_doc_id;
                    accumulator = None;
                    if accumulators.len() == max_accumulators {
                        self.write_levels(&accumulators)?;
                        accumulators.clear();
                    }
                }
            }
            if accumulator.is_none() {
                accumulator = Some(SkipAccumulator::new(doc));
            }
            let acc = accumulator
                .as_mut()
                .expect("INVARIANT: an accumulator was just created if there was none");
            acc.next_doc(doc);
            acc.accumulate(first_value);
            for _ in 1..value_count {
                let value = values.next_value()?;
                acc.accumulate(value);
            }
            if accumulators.len() < max_accumulators {
                // The interval is recorded once, when it is opened; the loop
                // above keeps widening the one at the end.
                if accumulators.last().map(|last| last.min_doc_id) != Some(acc.min_doc_id) {
                    accumulators.push(acc.clone());
                } else if let Some(last) = accumulators.last_mut() {
                    *last = acc.clone();
                }
            }
            doc = values.next_doc()?;
        }

        if let Some(acc) = accumulator.as_ref() {
            global_max_value = global_max_value.max(acc.max_value);
            global_min_value = global_min_value.min(acc.min_value);
            global_doc_count += acc.doc_count;
            max_doc_id = acc.max_doc_id;
        }
        if !accumulators.is_empty() {
            self.write_levels(&accumulators)?;
        }

        self.meta.write_long(start)?;
        self.meta
            .write_long(self.skip_index.file_pointer() - start)?;
        debug_assert!(global_doc_count == 0 || global_max_value >= global_min_value);
        self.meta.write_long(global_max_value)?;
        self.meta.write_long(global_min_value)?;
        debug_assert!(global_doc_count <= max_doc_id + 1);
        self.meta.write_int(global_doc_count)?;
        self.meta.write_int(max_doc_id)?;
        self.meta.write_int(global_max_value_count)?;
        Ok(())
    }

    /// Writes every interval, each preceded by the number of levels it starts.
    ///
    /// Equivalent to `Lucene90DocValuesConsumer.writeLevels(List)`. The levels
    /// of one interval are written from the highest down, so a reader that
    /// skips at the top level never has to read the lower ones.
    fn write_levels(&mut self, accumulators: &[SkipAccumulator]) -> Result<()> {
        let mut levels: Vec<Vec<SkipAccumulator>> = Vec::with_capacity(SKIP_INDEX_MAX_LEVEL);
        levels.push(accumulators.to_vec());
        for i in 0..SKIP_INDEX_MAX_LEVEL - 1 {
            let next = build_level(&levels[i]);
            levels.push(next);
        }
        let total_accumulators = accumulators.len();
        for index in 0..total_accumulators {
            let level_count = get_levels(index, total_accumulators);
            self.skip_index.write_byte(level_count as u8)?;
            for level in (0..level_count).rev() {
                let accumulator =
                    &levels[level][index >> (SKIP_INDEX_LEVEL_SHIFT as usize * level)];
                self.skip_index.write_int(accumulator.max_doc_id)?;
                self.skip_index.write_int(accumulator.min_doc_id)?;
                self.skip_index.write_long(accumulator.max_value)?;
                self.skip_index.write_long(accumulator.min_value)?;
                self.skip_index.write_int(accumulator.doc_count)?;
            }
        }
        Ok(())
    }
}

impl<'a> Lucene90DocValuesConsumer<'a> {
    fn new(state: &SegmentWriteState<'a>, skip_index_interval_size: i32) -> Result<Self> {
        let data_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            DATA_EXTENSION,
        );
        let mut data = state.directory.create_output(&data_name, state.context)?;
        write_index_header(
            data.as_mut(),
            DATA_CODEC,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let meta_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta = state.directory.create_output(&meta_name, state.context)?;
        write_index_header(
            meta.as_mut(),
            META_CODEC,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let skip_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            SKIP_INDEX_EXTENSION,
        );
        let mut skip_index = state.directory.create_output(&skip_name, state.context)?;
        write_index_header(
            skip_index.as_mut(),
            SKIP_INDEX_CODEC,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        Ok(Self {
            data,
            meta,
            skip_index,
            // `this.termsDictBuffer = new byte[1 << 14]`,
            // `Lucene90DocValuesConsumer.java:83`. A Java array of that length
            // is zero-filled and *has* that length; a `Vec::with_capacity`
            // would have length zero, which would make every growth test in
            // `maybe_grow_buffer` compare against nothing.
            terms_dict_buffer: vec![0u8; 1 << 14],
            max_doc: state.segment_info.max_doc()?,
            skip_index_interval_size,
            closed: false,
            _context: std::marker::PhantomData,
        })
    }
}

#[derive(Debug)]
struct BufferedDocIdSetIterator {
    docs: Vec<i32>,
    index: usize,
    doc: i32,
}

impl BufferedDocIdSetIterator {
    fn new(docs: Vec<i32>) -> Self {
        Self {
            docs,
            index: 0,
            doc: -1,
        }
    }
}

impl DocIdSetIterator for BufferedDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.index < self.docs.len() {
            self.doc = self.docs[self.index];
            self.index += 1;
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        while self.index < self.docs.len() && self.docs[self.index] < target {
            self.index += 1;
        }
        self.next_doc()
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

#[derive(Debug, Clone)]
struct NumericStats {
    min: i64,
    max: i64,
    gcd: i64,
    unique_values: Option<Vec<i64>>,
    do_blocks: bool,
}

impl NumericStats {
    /// The statistics pass of `writeValues`
    /// (`Lucene90DocValuesConsumer.java:373-430`): the GCD of the deltas from
    /// the first value, the global and per-block bit budgets, and the unique
    /// values up to 256 of them.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] with Java's messages when
    /// ordinals do not start at zero or share a GCD other than one
    /// (`Lucene90DocValuesConsumer.java:421-430`).
    fn compute(values: &[i64], ords: bool) -> Result<Self> {
        // `firstValue` is the first value of the first document, or `0` when no
        // document has one (`Lucene90DocValuesConsumer.java:376-381`). A field
        // whose every value belongs to a deleted document has none left after
        // a merge; the trackers then keep their initial bounds and the `gcd`
        // its initial `0`, and `writeValues` writes them as they are.
        let first = values.first().copied().unwrap_or(0);
        let mut min_max = MinMaxTracker::new();
        let mut block_min_max = MinMaxTracker::new();
        let mut gcd = 0i64;
        let mut unique = if ords { None } else { Some(HashSet::new()) };

        for &v in values {
            // Ordinals take the same GCD computation as any other value; Java
            // only checks the result afterwards.
            if gcd != 1 {
                if !(i64::MIN / 2..=i64::MAX / 2).contains(&v) {
                    // In that case `v - minValue` might overflow and make the
                    // GCD computation return wrong results.
                    gcd = 1;
                } else {
                    gcd = gcd_i64(gcd, v - first);
                }
            }

            block_min_max.update(v);
            if block_min_max.num_values == NUMERIC_BLOCK_SIZE as i64 {
                min_max.update_from(&block_min_max);
                block_min_max.next_block();
            }

            if let Some(ref mut set) = unique {
                if set.insert(v) && set.len() > 256 {
                    unique = None;
                }
            }
        }

        min_max.update_from(&block_min_max);
        min_max.finish();
        block_min_max.finish();

        if ords && min_max.num_values > 0 {
            if min_max.min != 0 {
                return Err(LuceneError::IllegalState(format!(
                    "The min value for ordinals should always be 0, got {}",
                    min_max.min
                )));
            }
            if min_max.max != 0 && gcd != 1 {
                return Err(LuceneError::IllegalState(format!(
                    "GCD compression should never be used on ordinals, found gcd={gcd}"
                )));
            }
        }
        debug_assert!(block_min_max.space_in_bits <= min_max.space_in_bits);

        let unique_values = unique.map(|set| {
            let mut vec: Vec<i64> = set.into_iter().collect();
            vec.sort_unstable();
            vec
        });

        // "We do blocks if that appears to save 10+% storage"
        // (`Lucene90DocValuesConsumer.java:485-487`). `writeValues` only
        // consults it once the unique-value table has been ruled out.
        let do_blocks = min_max.space_in_bits > 0
            && (block_min_max.space_in_bits as f64 / min_max.space_in_bits as f64) <= 0.9;

        Ok(Self {
            min: min_max.min,
            max: min_max.max,
            gcd,
            unique_values,
            do_blocks,
        })
    }
}

/// Tracks the bounds of a run of values and the packed bits they need.
///
/// Equivalent to `Lucene90DocValuesConsumer.MinMaxTracker`
/// (`Lucene90DocValuesConsumer.java:169-209`): `spaceInBits` accumulates over
/// every [`finish`](Self::finish), and [`reset`](Self::reset) leaves it alone,
/// so the per-block tracker ends up holding the sum over all blocks.
#[derive(Debug)]
struct MinMaxTracker {
    min: i64,
    max: i64,
    num_values: i64,
    space_in_bits: i64,
}

impl MinMaxTracker {
    fn new() -> Self {
        Self {
            min: i64::MAX,
            max: i64::MIN,
            num_values: 0,
            space_in_bits: 0,
        }
    }

    /// `reset()`: the bounds and the count, never the space.
    fn reset(&mut self) {
        self.min = i64::MAX;
        self.max = i64::MIN;
        self.num_values = 0;
    }

    /// `update(long)`.
    fn update(&mut self, v: i64) {
        self.min = self.min.min(v);
        self.max = self.max.max(v);
        self.num_values += 1;
    }

    /// `update(MinMaxTracker)`.
    fn update_from(&mut self, other: &Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        self.num_values += other.num_values;
    }

    /// `finish()`: adds the bits of the current run. Java's `max - min`
    /// wraps for a range wider than `Long.MAX_VALUE`, and
    /// `unsignedBitsRequired` then answers 64.
    fn finish(&mut self) {
        if self.max > self.min {
            // `spaceInBits += bitsRequired * numValues`, in Java's wrapping `long`.
            self.space_in_bits = self.space_in_bits.wrapping_add(
                i64::from(DirectWriter::unsigned_bits_required(
                    self.max.wrapping_sub(self.min),
                ))
                .wrapping_mul(self.num_values),
            );
        }
    }

    /// `nextBlock()`.
    fn next_block(&mut self) {
        self.finish();
        self.reset();
    }
}

fn gcd_i64(a: i64, b: i64) -> i64 {
    let mut a = a.unsigned_abs();
    let mut b = b.unsigned_abs();
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a as i64
}

impl<'a> Lucene90DocValuesConsumer<'a> {
    fn write_docs_with_field(&mut self, docs: &[i32]) -> Result<(i64, i64, i16, i8)> {
        if docs.is_empty() {
            return Ok((-2, 0, -1, -1));
        }
        if docs.len() == self.max_doc as usize {
            return Ok((-1, 0, -1, -1));
        }
        let offset = self.data.file_pointer();
        let mut it = BufferedDocIdSetIterator::new(docs.to_vec());
        let jump_count = write_bit_set(&mut it, self.data.as_mut(), DEFAULT_DENSE_RANK_POWER)?;
        let length = self.data.file_pointer() - offset;
        Ok((offset, length, jump_count, DEFAULT_DENSE_RANK_POWER))
    }

    fn write_numeric_values_impl(
        &mut self,
        _field: &FieldInfo,
        docs: &[i32],
        values: &[i64],
        ords: bool,
    ) -> Result<(i32, i64)> {
        let num_docs_with_value = docs.len() as i32;
        let (docs_offset, docs_length, jump_count, dense_rank_power) =
            self.write_docs_with_field(docs)?;
        self.meta.write_long(docs_offset)?;
        self.meta.write_long(docs_length)?;
        self.meta.write_short(jump_count)?;
        self.meta.write_byte(dense_rank_power as u8)?;

        let num_values = values.len() as i64;
        self.meta.write_long(num_values)?;

        let mut stats = NumericStats::compute(values, ords)?;
        let mut min = stats.min;
        let mut gcd = stats.gcd;

        // Java's control flow: the unique-value table is only used when it is
        // strictly cheaper than direct packing; otherwise the table is
        // discarded (`uniqueValues = null`) and the values fall through to the
        // doBlocks / single-block path, which includes the `min = 0`
        // normalization that keeps small positive ranges at their natural bit
        // width. The fall-through must apply the same normalization — skipping
        // it writes a different `min` than Lucene for ranges like 1..=11.
        let (num_bits_per_value, table_size, table, encode) = if num_values == 0
            || stats.min >= stats.max
        {
            (0u8, -1i32, None, None)
        } else if let Some(unique) = stats.unique_values.take_if(|u| {
            DirectWriter::unsigned_bits_required((u.len() as i64) - 1)
                < DirectWriter::unsigned_bits_required(
                    stats.max.wrapping_sub(stats.min) / stats.gcd,
                )
        }) {
            let bits = DirectWriter::unsigned_bits_required((unique.len() as i64) - 1) as u8;
            let mut encode = HashMap::with_capacity(unique.len());
            for (i, &v) in unique.iter().enumerate() {
                encode.insert(v, i as i64);
            }
            min = 0;
            gcd = 1;
            (bits, unique.len() as i32, Some(unique), Some(encode))
        } else {
            let direct_bits =
                DirectWriter::unsigned_bits_required(stats.max.wrapping_sub(stats.min) / stats.gcd);
            if stats.do_blocks {
                (0xFFu8, -2 - NUMERIC_BLOCK_SHIFT, None, None)
            } else {
                let single_bits = direct_bits;
                if gcd == 1
                    && min > 0
                    && DirectWriter::unsigned_bits_required(stats.max) == single_bits
                {
                    min = 0;
                }
                (single_bits as u8, -1, None, None)
            }
        };

        self.meta.write_int(table_size)?;
        if let Some(ref table) = table {
            for &v in table {
                self.meta.write_long(v)?;
            }
        }
        self.meta.write_byte(num_bits_per_value)?;
        self.meta.write_long(min)?;
        self.meta.write_long(gcd)?;
        let value_offset = self.data.file_pointer();
        self.meta.write_long(value_offset)?;

        let jump_table_offset = if num_values == 0 || num_bits_per_value == 0 {
            -1i64
        } else if num_bits_per_value == 0xFF {
            self.write_values_multiple_blocks(values, gcd)?
        } else {
            self.write_values_single_block(
                values,
                num_values,
                num_bits_per_value as i32,
                min,
                gcd,
                encode.as_ref(),
            )?
        };

        let values_length = self.data.file_pointer() - value_offset;
        self.meta.write_long(values_length)?;
        self.meta.write_long(jump_table_offset)?;

        Ok((num_docs_with_value, num_values))
    }

    fn write_values_single_block(
        &mut self,
        values: &[i64],
        num_values: i64,
        num_bits_per_value: i32,
        min: i64,
        gcd: i64,
        encode: Option<&HashMap<i64, i64>>,
    ) -> Result<i64> {
        let mut writer = DirectWriter::new(self.data.as_mut(), num_values, num_bits_per_value)?;
        for &v in values {
            let encoded = if let Some(map) = encode {
                *map.get(&v).expect("value not in table")
            } else {
                v.wrapping_sub(min) / gcd
            };
            writer.add(encoded)?;
        }
        writer.finish()?;
        Ok(-1)
    }

    fn write_values_multiple_blocks(&mut self, values: &[i64], gcd: i64) -> Result<i64> {
        let mut offsets: Vec<i64> = Vec::new();
        let mut buffer = vec![0i64; NUMERIC_BLOCK_SIZE];
        let mut encode_buffer = ByteBuffersDataOutput::new_resettable_instance();
        let mut up_to = 0;
        for &v in values {
            buffer[up_to] = v;
            up_to += 1;
            if up_to == NUMERIC_BLOCK_SIZE {
                offsets.push(self.data.file_pointer());
                self.write_block(&buffer[..NUMERIC_BLOCK_SIZE], gcd, &mut encode_buffer)?;
                up_to = 0;
            }
        }
        if up_to > 0 {
            offsets.push(self.data.file_pointer());
            self.write_block(&buffer[..up_to], gcd, &mut encode_buffer)?;
        }
        let offsets_origo = self.data.file_pointer();
        for &off in &offsets {
            self.data.write_long(off)?;
        }
        self.data.write_long(offsets_origo)?;
        Ok(offsets_origo)
    }

    fn write_block(
        &mut self,
        values: &[i64],
        gcd: i64,
        encode_buffer: &mut ByteBuffersDataOutput,
    ) -> Result<()> {
        debug_assert!(!values.is_empty());
        let mut min = values[0];
        let mut max = values[0];
        for &v in values.iter().skip(1) {
            // Java's `assert Math.floorMod(values[i] - min, gcd) == 0`
            // (`Lucene90DocValuesConsumer.java:584`), with its wrapping
            // subtraction: an assertion, so compiled out of release builds as
            // Java runs without `-ea`.
            debug_assert_eq!(v.wrapping_sub(min).rem_euclid(gcd), 0);
            min = min.min(v);
            max = max.max(v);
        }
        if min == max {
            self.data.write_byte(0)?;
            self.data.write_long(min)?;
        } else {
            let bits_per_value = DirectWriter::unsigned_bits_required(max.wrapping_sub(min) / gcd);
            encode_buffer.reset();
            let mut writer = DirectWriter::new(encode_buffer, values.len() as i64, bits_per_value)?;
            for &v in values {
                writer.add(v.wrapping_sub(min) / gcd)?;
            }
            writer.finish()?;
            self.data.write_byte(bits_per_value as u8)?;
            self.data.write_long(min)?;
            self.data.write_int(encode_buffer.size() as i32)?;
            encode_buffer.copy_to(self.data.as_mut())?;
        }
        Ok(())
    }

    fn add_binary_internal(
        &mut self,
        _field: &FieldInfo,
        docs: &[i32],
        bytes: &[Vec<u8>],
    ) -> Result<()> {
        debug_assert_eq!(docs.len(), bytes.len());
        let start = self.data.file_pointer();
        for b in bytes {
            self.data.write_bytes(b, 0, b.len())?;
        }
        let data_length = self.data.file_pointer() - start;
        self.meta.write_long(start)?;
        self.meta.write_long(data_length)?;

        let (docs_offset, docs_length, jump_count, dense_rank_power) =
            self.write_docs_with_field(docs)?;
        self.meta.write_long(docs_offset)?;
        self.meta.write_long(docs_length)?;
        self.meta.write_short(jump_count)?;
        self.meta.write_byte(dense_rank_power as u8)?;

        let num_docs = docs.len() as i32;
        let mut min_length = i32::MAX;
        let mut max_length = 0i32;
        for b in bytes {
            let len = b.len() as i32;
            min_length = min_length.min(len);
            max_length = max_length.max(len);
        }
        self.meta.write_int(num_docs)?;
        self.meta.write_int(min_length)?;
        self.meta.write_int(max_length)?;

        if max_length > min_length {
            let start = self.data.file_pointer();
            self.meta.write_long(start)?;
            self.meta.write_v_int(DIRECT_MONOTONIC_BLOCK_SHIFT)?;
            let mut writer = DirectMonotonicWriter::new(
                self.meta.as_mut(),
                self.data.as_mut(),
                num_docs as i64 + 1,
                DIRECT_MONOTONIC_BLOCK_SHIFT,
            )?;
            let mut addr = 0i64;
            writer.add(addr)?;
            for b in bytes {
                addr += b.len() as i64;
                writer.add(addr)?;
            }
            writer.finish()?;
            self.meta.write_long(self.data.file_pointer() - start)?;
        }
        Ok(())
    }

    /// Returns whether every document carries at most one value.
    ///
    /// A singleton view answers immediately; otherwise the whole set is scanned
    /// for a document with more than one value, which is what decides between
    /// the two `SORTED_SET` layouts.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while scanning the values.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.Lucene90DocValuesConsumer#isSingleValued`.
    fn is_single_valued(mut values: Box<dyn SortedSetDocValues>) -> Result<bool> {
        if values.is_singleton() {
            return Ok(true);
        }
        debug_assert_eq!(values.doc_id(), -1);
        let mut doc = values.next_doc()?;
        while doc != NO_MORE_DOCS {
            let doc_value_count = values.doc_value_count()?;
            debug_assert!(doc_value_count > 0);
            if doc_value_count > 1 {
                return Ok(false);
            }
            doc = values.next_doc()?;
        }
        Ok(true)
    }

    /// Writes one `SORTED` field: its ordinals laid out like a numeric field,
    /// followed by its terms dictionary.
    ///
    /// `add_type_byte` is set only by the sorted-set path, which prefixes the
    /// layout with a `0` byte saying the field turned out to be single-valued.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the values or writing the
    /// metadata and data files.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.Lucene90DocValuesConsumer#doAddSortedField`.
    fn do_add_sorted_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
        add_type_byte: bool,
    ) -> Result<()> {
        // Lucene wraps the sorted values in an `EmptyDocValuesProducer` whose
        // `getSortedNumeric` is the singleton view of their ordinals, and feeds
        // that to both the skip index and `writeValues`
        // (`Lucene90DocValuesConsumer.java:682-741`).
        let producer = SortedOrdinalsProducer {
            inner: values_producer,
        };
        if field.doc_values_skip_index_type != crate::index::DocValuesSkipIndexType::NONE {
            self.write_skip_index(field, &producer)?;
        }
        if add_type_byte {
            self.meta.write_byte(0u8)?; // multiValued (0 = singleValued)
        }

        let mut sorted =
            crate::codecs::doc_values::non_null(values_producer.get_sorted(field)?, field)?;
        let value_count = sorted.get_value_count()? as i64;
        let mut docs = Vec::new();
        let mut ords = Vec::new();
        let mut doc = sorted.next_doc()?;
        while doc != NO_MORE_DOCS {
            docs.push(doc);
            ords.push(sorted.ord_value()? as i64);
            doc = sorted.next_doc()?;
        }
        self.write_numeric_values_impl(field, &docs, &ords, true)?;
        self.add_terms_dict_values(value_count, |ord| sorted.lookup_ord(ord as i32))?;
        Ok(())
    }

    /// Writes one `SORTED_NUMERIC` field: its values laid out like a numeric
    /// field, then the number of documents with a value and, when some document
    /// carries more than one, the monotonic addresses that delimit each
    /// document's run.
    ///
    /// `ords` is set only by the sorted-set path, which prefixes the layout
    /// with a `1` byte saying the field is multi-valued and whose values are
    /// ordinals rather than user values.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the values or writing the
    /// metadata and data files.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.Lucene90DocValuesConsumer#doAddSortedNumericField`.
    fn do_add_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
        ords: bool,
    ) -> Result<()> {
        if field.doc_values_skip_index_type != crate::index::DocValuesSkipIndexType::NONE {
            self.write_skip_index(field, values_producer)?;
        }
        if ords {
            self.meta.write_byte(1u8)?; // multiValued (1 = multiValued)
        }

        let mut numeric =
            crate::codecs::doc_values::non_null(values_producer.get_sorted_numeric(field)?, field)?;
        let mut docs = Vec::new();
        let mut counts = Vec::new();
        let mut all_values = Vec::new();
        let mut doc = numeric.next_doc()?;
        while doc != NO_MORE_DOCS {
            docs.push(doc);
            let count = numeric.doc_value_count()?;
            counts.push(count);
            for _ in 0..count {
                all_values.push(numeric.next_value()?);
            }
            doc = numeric.next_doc()?;
        }
        let (num_docs_with_field, num_values) =
            self.write_numeric_values_impl(field, &docs, &all_values, ords)?;
        debug_assert!(num_values >= num_docs_with_field as i64);

        self.meta.write_int(num_docs_with_field)?;
        if num_values > num_docs_with_field as i64 {
            let start = self.data.file_pointer();
            self.meta.write_long(start)?;
            self.meta.write_v_int(DIRECT_MONOTONIC_BLOCK_SHIFT)?;

            let mut addresses_writer = DirectMonotonicWriter::new(
                self.meta.as_mut(),
                self.data.as_mut(),
                num_docs_with_field as i64 + 1,
                DIRECT_MONOTONIC_BLOCK_SHIFT,
            )?;
            let mut addr = 0i64;
            addresses_writer.add(addr)?;
            for &count in &counts {
                addr += count as i64;
                addresses_writer.add(addr)?;
            }
            addresses_writer.finish()?;
            self.meta.write_long(self.data.file_pointer() - start)?;
        }
        Ok(())
    }

    /// Compresses one terms-dictionary block into the data file and returns its
    /// uncompressed length in bytes.
    ///
    /// The block's own first term is its LZ4 dictionary, so the compressed
    /// payload covers only the bytes written after it.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while writing to the data file.
    ///
    /// Divergence: Lucene reads `this.data` and `this.termsDictBuffer` from the
    /// instance (`Lucene90DocValuesConsumer.java:831-838`). Here the caller is
    /// holding a `DirectMonotonicWriter` that borrows `self.meta` for the whole
    /// terms-dictionary loop, so `&mut self` cannot be taken again; the data
    /// output and the buffer are passed explicitly instead. Nothing else
    /// changes: the same bytes are written in the same order.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.Lucene90DocValuesConsumer#compressAndGetTermsDictBlockLength`.
    fn compress_and_get_terms_dict_block_length(
        data: &mut dyn IndexOutput,
        terms_dict_buffer: &[u8],
        buffered_position: usize,
        dict_length: usize,
        ht: &mut FastCompressionHashTable,
    ) -> Result<usize> {
        let uncompressed_length = buffered_position - dict_length;
        data.write_v_int(uncompressed_length as i32)?;
        Lz4::compress_with_dictionary(
            terms_dict_buffer,
            0,
            dict_length,
            uncompressed_length,
            data,
            ht,
        )?;
        Ok(uncompressed_length)
    }

    /// Grows the terms-dictionary buffer when `term_length` more bytes would
    /// not fit into it.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.Lucene90DocValuesConsumer#maybeGrowBuffer`
    /// (`Lucene90DocValuesConsumer.java:840-847`). Java rebuilds the
    /// `ByteArrayDataOutput` over the new array at the position the old one had
    /// reached and returns it; here the array and the position are the two
    /// things the caller carries, and the view over them is built per write
    /// burst (see [`Self::write_buffered`]), so there is no output object to
    /// rebuild and only the array is touched.
    ///
    /// The test is spelled `pos + term_length + 1 >= original_length` where
    /// Java spells it `pos + termLength >= originalLength - 1`. The two are the
    /// same inequality over integers; Java may evaluate `originalLength - 1` on
    /// an empty array and get `-1` harmlessly, whereas the same subtraction on
    /// a `usize` would wrap.
    fn maybe_grow_buffer(terms_dict_buffer: &mut Vec<u8>, position: usize, term_length: usize) {
        let original_length = terms_dict_buffer.len();
        if position + term_length + 1 >= original_length {
            *terms_dict_buffer = ArrayUtil::grow(terms_dict_buffer, original_length + term_length);
        }
    }

    /// Writes into `terms_dict_buffer` at `position` through a
    /// [`ByteArrayDataOutput`], and returns the position the output reached.
    ///
    /// This is Lucene's `bufferedOutput` (`Lucene90DocValuesConsumer.java:768`):
    /// a `ByteArrayDataOutput` over the terms-dictionary array, positioned where
    /// the last write left off and limited by the array's length. Every write in
    /// [`Self::add_terms_dict_values`] goes through it, so the encoding — the
    /// `writeVInt` in particular — is the ported one and not a second copy of it.
    ///
    /// # Divergence: the view is rebuilt per burst instead of being held
    ///
    /// Java keeps one `ByteArrayDataOutput` alive across the whole loop while
    /// also reading `termsDictBuffer` directly, both to compress a finished
    /// block (`:836`) and to test its length before growing it (`:841`). Those
    /// are two live references to one array, one of them writing, which Rust
    /// forbids: a `ByteArrayDataOutput<'a>` holds `&'a mut [u8]` for as long as
    /// it lives, and nothing else may look at the array meanwhile. The array
    /// and the position are therefore what the loop carries, and the view is
    /// constructed around them for each burst of writes and dropped again —
    /// which is exactly the object Java rebuilds on every growth
    /// (`new ByteArrayDataOutput(termsDictBuffer, pos, termsDictBuffer.length - pos)`,
    /// `:844`), only built more often. It allocates nothing: the view is a
    /// slice reference and two indices.
    fn write_buffered<F>(terms_dict_buffer: &mut [u8], position: usize, write: F) -> Result<usize>
    where
        F: FnOnce(&mut ByteArrayDataOutput<'_>) -> Result<()>,
    {
        let limit = terms_dict_buffer.len();
        let mut buffered_output =
            ByteArrayDataOutput::with_range(terms_dict_buffer, position, limit - position);
        write(&mut buffered_output)?;
        Ok(buffered_output.position())
    }

    fn add_terms_dict_values<F>(&mut self, value_count: i64, mut lookup: F) -> Result<()>
    where
        F: FnMut(i64) -> Result<BytesRef>,
    {
        self.meta.write_v_long(value_count)?;
        self.meta.write_int(DIRECT_MONOTONIC_BLOCK_SHIFT)?;

        let num_blocks =
            (value_count + TERMS_DICT_BLOCK_LZ4_MASK as i64) >> TERMS_DICT_BLOCK_LZ4_SHIFT;
        let address_buffer = ByteBuffersDataOutput::new();
        let mut address_output =
            ByteBuffersIndexOutput::new(address_buffer, "terms_dict_address", "terms_dict_address");
        let mut writer = DirectMonotonicWriter::new(
            self.meta.as_mut(),
            &mut address_output,
            num_blocks,
            DIRECT_MONOTONIC_BLOCK_SHIFT,
        )?;

        let mut previous = BytesRefBuilder::new();
        let start = self.data.file_pointer();
        let mut max_length = 0i32;
        let mut max_block_length = 0i32;
        let mut ht = FastCompressionHashTable::new();
        // Lucene's `bufferedOutput` is a `ByteArrayDataOutput` over
        // `termsDictBuffer` (`Lucene90DocValuesConsumer.java:768`). Here the
        // array is `self.terms_dict_buffer` and the output's position is
        // carried alongside it; `Self::write_buffered` builds the view for each
        // burst of writes. See its documentation for why it cannot be held.
        let terms_dict_buffer = &mut self.terms_dict_buffer;
        let mut buffered_position = 0usize;
        let mut dict_length = 0usize;

        for ord in 0..value_count {
            let term = lookup(ord)?;
            if (ord as usize & TERMS_DICT_BLOCK_LZ4_MASK) == 0 {
                if ord != 0 {
                    // flush the previous block
                    let uncompressed_length = Self::compress_and_get_terms_dict_block_length(
                        self.data.as_mut(),
                        terms_dict_buffer,
                        buffered_position,
                        dict_length,
                        &mut ht,
                    )?;
                    max_block_length = max_block_length.max(uncompressed_length as i32);
                    // `bufferedOutput.reset(termsDictBuffer)` (`:778`): back to
                    // position 0 over the whole array, however long it has
                    // grown by now.
                    buffered_position = 0;
                }

                writer.add(self.data.file_pointer() - start)?;
                // Write the first term both to the index output, and to the buffer where we'll
                // use it as a dictionary for compression
                self.data.write_v_int(term.length as i32)?;
                self.data
                    .write_bytes(term.slice(), term.offset, term.length)?;
                Self::maybe_grow_buffer(terms_dict_buffer, buffered_position, term.length);
                buffered_position =
                    Self::write_buffered(terms_dict_buffer, buffered_position, |out| {
                        out.write_bytes(term.slice(), term.offset, term.length)
                    })?;
                dict_length = term.length;
            } else {
                let prefix = StringHelper::bytes_difference(&previous.get(), &term)? as usize;
                let suffix_length = term.length - prefix;
                assert!(suffix_length > 0);
                // Will write (suffixLength + 1 byte + 2 vint) bytes. Grow the buffer in need.
                Self::maybe_grow_buffer(terms_dict_buffer, buffered_position, suffix_length + 11);
                let prefix_clipped = prefix.min(15);
                let suffix_clipped = (suffix_length - 1).min(15);
                let b = (prefix_clipped | (suffix_clipped << 4)) as u8;
                buffered_position =
                    Self::write_buffered(terms_dict_buffer, buffered_position, |out| {
                        out.write_byte(b)?;
                        if prefix >= 15 {
                            out.write_v_int((prefix - 15) as i32)?;
                        }
                        if suffix_length >= 16 {
                            out.write_v_int((suffix_length - 16) as i32)?;
                        }
                        out.write_bytes(term.slice(), term.offset + prefix, suffix_length)
                    })?;
            }
            max_length = max_length.max(term.length as i32);
            previous.copy_ref(&term);
        }
        // Compress and write out the last block
        if buffered_position > dict_length {
            let uncompressed_length = Self::compress_and_get_terms_dict_block_length(
                self.data.as_mut(),
                terms_dict_buffer,
                buffered_position,
                dict_length,
                &mut ht,
            )?;
            max_block_length = max_block_length.max(uncompressed_length as i32);
        }

        writer.finish()?;
        self.meta.write_int(max_length)?;
        self.meta.write_int(max_block_length)?;
        self.meta.write_long(start)?;
        self.meta.write_long(self.data.file_pointer() - start)?;

        let start = self.data.file_pointer();
        address_output.to_array_copy()?.iter().for_each(|&b| {
            let _ = self.data.write_byte(b);
        });
        self.meta.write_long(start)?;
        self.meta.write_long(self.data.file_pointer() - start)?;

        self.write_terms_index_values(value_count, lookup)?;
        Ok(())
    }

    fn write_terms_index_values<F>(&mut self, value_count: i64, mut lookup: F) -> Result<()>
    where
        F: FnMut(i64) -> Result<BytesRef>,
    {
        self.meta.write_int(TERMS_DICT_REVERSE_INDEX_SHIFT)?;
        let start = self.data.file_pointer();
        let num_blocks = 1
            + ((value_count + TERMS_DICT_REVERSE_INDEX_MASK as i64)
                >> TERMS_DICT_REVERSE_INDEX_SHIFT);
        let address_buffer = ByteBuffersDataOutput::new();
        let mut address_output = ByteBuffersIndexOutput::new(
            address_buffer,
            "terms_index_address",
            "terms_index_address",
        );
        let mut writer = DirectMonotonicWriter::new(
            self.meta.as_mut(),
            &mut address_output,
            num_blocks,
            DIRECT_MONOTONIC_BLOCK_SHIFT,
        )?;
        let mut previous = BytesRefBuilder::new();
        let mut offset = 0i64;
        for ord in 0..value_count {
            let term = lookup(ord)?;
            if (ord as usize & TERMS_DICT_REVERSE_INDEX_MASK) == 0 {
                writer.add(offset)?;
                let sort_key_length = if ord == 0 {
                    0
                } else {
                    StringHelper::sort_key_length(&previous.get(), &term)?
                };
                offset += sort_key_length as i64;
                self.data
                    .write_bytes(term.slice(), term.offset, sort_key_length as usize)?;
            } else if (ord as usize & TERMS_DICT_REVERSE_INDEX_MASK)
                == TERMS_DICT_REVERSE_INDEX_MASK
            {
                previous.copy_ref(&term);
            }
        }
        writer.add(offset)?;
        writer.finish()?;
        self.meta.write_long(start)?;
        self.meta.write_long(self.data.file_pointer() - start)?;
        let start = self.data.file_pointer();
        address_output.to_array_copy()?.iter().for_each(|&b| {
            let _ = self.data.write_byte(b);
        });
        self.meta.write_long(start)?;
        self.meta.write_long(self.data.file_pointer() - start)?;
        Ok(())
    }
}

impl<'a> DocValuesConsumer for Lucene90DocValuesConsumer<'a> {
    fn add_numeric_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.meta.write_int(field.number)?;
        self.meta.write_byte(TYPE_NUMERIC)?;
        // Lucene writes the skip index between the type byte and the values,
        // and the reader reads it in the same place
        // (`Lucene90DocValuesConsumer.java:160-165`). A numeric field is seen
        // as a single-valued sorted-numeric one, which is the shape the skip
        // index is built from.
        if field.doc_values_skip_index_type != crate::index::DocValuesSkipIndexType::NONE {
            let producer = SingletonNumericProducer {
                inner: values,
                field: field.clone(),
            };
            self.write_skip_index(field, &producer)?;
        }

        let mut numeric = crate::codecs::doc_values::non_null(values.get_numeric(field)?, field)?;
        let mut docs = Vec::new();
        let mut values_vec = Vec::new();
        let mut doc = numeric.next_doc()?;
        while doc != NO_MORE_DOCS {
            docs.push(doc);
            values_vec.push(numeric.long_value()?);
            doc = numeric.next_doc()?;
        }
        self.write_numeric_values_impl(field, &docs, &values_vec, false)?;
        Ok(())
    }

    fn add_binary_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.meta.write_int(field.number)?;
        self.meta.write_byte(TYPE_BINARY)?;

        let mut binary = crate::codecs::doc_values::non_null(values.get_binary(field)?, field)?;
        let mut docs = Vec::new();
        let mut bytes = Vec::new();
        let mut doc = binary.next_doc()?;
        while doc != NO_MORE_DOCS {
            docs.push(doc);
            let value = binary.binary_value()?;
            bytes.push(BytesRef::deep_copy_of(&value).bytes);
            doc = binary.next_doc()?;
        }
        self.add_binary_internal(field, &docs, &bytes)?;
        Ok(())
    }

    fn add_sorted_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.meta.write_int(field.number)?;
        self.meta.write_byte(TYPE_SORTED)?;
        self.do_add_sorted_field(field, values, false)
    }

    fn add_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.meta.write_int(field.number)?;
        self.meta.write_byte(TYPE_SORTED_NUMERIC)?;
        self.do_add_sorted_numeric_field(field, values, false)
    }

    fn add_sorted_set_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.meta.write_int(field.number)?;
        self.meta.write_byte(TYPE_SORTED_SET)?;

        // Java decides the layout with `isSingleValued`
        // (Lucene90DocValuesConsumer.java:942-956): a full scan of the sorted
        // set for any document carrying more than one value. The scan runs on
        // one iterator and both write paths below request fresh iterators, so
        // producers must return independent cursors per call.
        if Self::is_single_valued(crate::codecs::doc_values::non_null(
            values.get_sorted_set(field)?,
            field,
        )?)? {
            // Java's `doAddSortedField(field, ..., addTypeByte=true)` route:
            // meta byte 0, the minimum ord of every document laid out exactly
            // like `TYPE_SORTED`, then the terms dictionary
            // (Lucene90DocValuesConsumer.java:962-975).
            let producer = MinOrdinalOfSortedSetProducer { inner: values };
            return self.do_add_sorted_field(field, &producer, true);
        }

        // Multi-valued: Java's `doAddSortedNumericField(field, ..., ords=true)`
        // route (Lucene90DocValuesConsumer.java:977-1046).
        let producer = OrdinalsAsSortedNumericProducer { inner: values };
        self.do_add_sorted_numeric_field(field, &producer, true)?;

        let sorted_set = crate::codecs::doc_values::non_null(values.get_sorted_set(field)?, field)?;
        let value_count = sorted_set.get_value_count()?;
        self.add_terms_dict_values(value_count, |ord| sorted_set.lookup_ord(ord))?;
        Ok(())
    }

    /// Equivalent to `Lucene90DocValuesConsumer.close()`
    /// (`Lucene90DocValuesConsumer.java:127-149`): the EOF marker and the three
    /// footers, then the outputs closed in Java's order.
    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let written = (|| -> Result<()> {
            self.meta.write_int(-1)?; // write EOF marker
            write_footer(self.meta.as_mut())?;
            write_footer(self.data.as_mut())?;
            write_footer(self.skip_index.as_mut())
        })();
        // `IOUtils.close(data, meta, skipIndex)`: every output is closed, and
        // the first failure is the one reported.
        let closes = [
            self.data.close(),
            self.meta.close(),
            self.skip_index.close(),
        ];
        written?;
        closes.into_iter().collect::<Result<Vec<()>>>()?;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Producer
// -----------------------------------------------------------------------------

/// Allocates a zeroed buffer, reporting a length the process cannot satisfy
/// instead of aborting on it.
///
/// Java sizes the same buffers from the same metadata and answers an impossible
/// length with an `OutOfMemoryError`, which the caller can catch; Rust aborts
/// the process when an allocation fails. `try_reserve_exact` restores Java's
/// outcome without introducing a bound Lucene does not have.
fn try_zeroed(length: usize) -> Result<Vec<u8>> {
    let mut buffer: Vec<u8> = Vec::new();
    buffer
        .try_reserve_exact(length)
        .map_err(|_| LuceneError::CorruptIndex(format!("cannot allocate {length} bytes")))?;
    buffer.resize(length, 0);
    Ok(buffer)
}

fn corrupt<T>(message: String) -> Result<T> {
    Err(LuceneError::CorruptIndex(message))
}

/// The largest table `readNumeric` accepts, verbatim from
/// `Lucene90DocValuesProducer.java:317-320`.
const MAX_TABLE_SIZE: i32 = 256;

/// One `NUMERIC` metadata entry, exactly the fields `readNumeric` reads
/// (`Lucene90DocValuesProducer.java:311-341`). Nothing is decoded here: the
/// values stay in the file until an iterator asks for one.
#[derive(Debug, Clone)]
struct NumericEntry {
    docs_with_field_offset: i64,
    docs_with_field_length: i64,
    jump_table_entry_count: i16,
    dense_rank_power: i8,
    num_values: i64,
    table: Option<Vec<i64>>,
    /// `-1` unless the entry was written with per-block widths.
    block_shift: i32,
    bits_per_value: u8,
    min_value: i64,
    gcd: i64,
    values_offset: i64,
    values_length: i64,
    /// Where the per-block offsets start in the data file, `-1` without them.
    ///
    /// Equivalent to `NumericEntry.valueJumpTableOffset`.
    value_jump_table_offset: i64,
}

/// One `BINARY` metadata entry (`Lucene90DocValuesProducer.java:343-378`).
#[derive(Debug, Clone)]
struct BinaryEntry {
    data_offset: i64,
    data_length: i64,
    docs_with_field_offset: i64,
    docs_with_field_length: i64,
    jump_table_entry_count: i16,
    dense_rank_power: i8,
    num_docs_with_field: i32,
    min_length: i32,
    max_length: i32,
    addresses_offset: i64,
    addresses_meta: Option<DirectMonotonicMeta>,
    addresses_length: i64,
}

/// One term dictionary's metadata (`Lucene90DocValuesProducer.java:408-429`).
#[derive(Debug, Clone)]
struct TermsDictEntry {
    terms_dict_size: i64,
    terms_addresses_meta: DirectMonotonicMeta,
    terms_data_offset: i64,
    terms_data_length: i64,
    terms_addresses_offset: i64,
    terms_addresses_length: i64,
}

#[derive(Debug, Clone)]
struct SortedEntry {
    ords: NumericEntry,
    terms_dict: TermsDictEntry,
}

/// A `SORTED_NUMERIC` entry, or the ordinals of a multi-valued `SORTED_SET`.
#[derive(Debug, Clone)]
struct SortedNumericEntry {
    numeric: NumericEntry,
    num_docs_with_field: i32,
    addresses_offset: i64,
    addresses_meta: Option<DirectMonotonicMeta>,
    addresses_length: i64,
}

/// `SORTED_SET` has two layouts, chosen by the `multiValued` byte
/// (`Lucene90DocValuesProducer.java:389-406`).
#[derive(Debug, Clone)]
enum SortedSetEntry {
    /// Every document carries at most one value: written exactly like `SORTED`.
    Single(SortedEntry),
    Multi {
        ords: SortedNumericEntry,
        terms_dict: TermsDictEntry,
    },
}

/// One doc-values skip-index metadata entry, exactly the fields
/// `readDocValueSkipperMeta` reads (`Lucene90DocValuesProducer.java:309-325`).
#[derive(Debug, Clone, Copy)]
struct DocValuesSkipperEntry {
    offset: i64,
    length: i64,
    min_value: i64,
    max_value: i64,
    doc_count: i32,
    max_doc_id: i32,
    max_value_count: i32,
}

/// The skipper `getSkipper` returns.
///
/// Equivalent to the anonymous `DocValuesSkipper` of
/// `Lucene90DocValuesProducer.getSkipper(FieldInfo)`
/// (`Lucene90DocValuesProducer.java:2375-2468`).
struct Lucene90DocValuesSkipper {
    entry: DocValuesSkipperEntry,
    input: Box<dyn IndexInput>,
    min_doc_id: [i32; SKIP_INDEX_MAX_LEVEL],
    max_doc_id: [i32; SKIP_INDEX_MAX_LEVEL],
    min_value: [i64; SKIP_INDEX_MAX_LEVEL],
    max_value: [i64; SKIP_INDEX_MAX_LEVEL],
    doc_count: [i32; SKIP_INDEX_MAX_LEVEL],
    levels: i32,
}

impl Lucene90DocValuesSkipper {
    fn new(entry: DocValuesSkipperEntry, input: Box<dyn IndexInput>) -> Self {
        Self {
            entry,
            input,
            min_doc_id: [-1; SKIP_INDEX_MAX_LEVEL],
            max_doc_id: [-1; SKIP_INDEX_MAX_LEVEL],
            min_value: [0; SKIP_INDEX_MAX_LEVEL],
            max_value: [0; SKIP_INDEX_MAX_LEVEL],
            doc_count: [0; SKIP_INDEX_MAX_LEVEL],
            levels: 1,
        }
    }
}

impl DocValuesSkipper for Lucene90DocValuesSkipper {
    fn advance(&mut self, target: i32) -> Result<()> {
        if target > self.entry.max_doc_id {
            // skipper is exhausted
            self.min_doc_id = [crate::search::NO_MORE_DOCS; SKIP_INDEX_MAX_LEVEL];
            self.max_doc_id = [crate::search::NO_MORE_DOCS; SKIP_INDEX_MAX_LEVEL];
            return Ok(());
        }
        // find next interval
        debug_assert!(
            target > self.max_doc_id[0],
            "target {target} must be bigger that current interval {}",
            self.max_doc_id[0]
        );
        loop {
            self.levels = i32::from(self.input.read_byte()? as i8);
            // `Lucene90DocValuesProducer.java:2402-2420`: Java asserts
            // `0 < levels <= SKIP_INDEX_MAX_LEVEL`; without assertions a level
            // above the maximum throws ArrayIndexOutOfBoundsException at
            // `maxDocID[levels - 1]`, and a negative level throws it at
            // `maxDocID[levels]` in the level adjustment once the empty
            // per-level loop leaves the interval valid. Both are reported as a
            // corrupt index, the port's mapping of an index-driven
            // out-of-bounds access. A zero level reads no level, stays valid,
            // and `maxDocID[0] < target` leaves `levels == 0`, as in Java.
            if self.levels > SKIP_INDEX_MAX_LEVEL as i32 || self.levels < 0 {
                return Err(LuceneError::corrupt_index(
                    format!("level out of range [{}]", self.levels),
                    self.input.resource_description(),
                ));
            }
            debug_assert!(self.levels > 0, "level out of range [{}]", self.levels);
            let mut valid = true;
            // check if current interval is competitive or we can jump to the
            // next position
            for level in (0..self.levels as usize).rev() {
                self.max_doc_id[level] = self.input.read_int()?;
                if self.max_doc_id[level] < target {
                    // the jump for the level
                    self.input
                        .skip_bytes(SKIP_INDEX_JUMP_LENGTH_PER_LEVEL[level])?;
                    valid = false;
                    break;
                }
                self.min_doc_id[level] = self.input.read_int()?;
                self.max_value[level] = self.input.read_long()?;
                self.min_value[level] = self.input.read_long()?;
                self.doc_count[level] = self.input.read_int()?;
            }
            if valid {
                // adjust levels
                while (self.levels as usize) < SKIP_INDEX_MAX_LEVEL
                    && self.max_doc_id[self.levels as usize] >= target
                {
                    self.levels += 1;
                }
                return Ok(());
            }
        }
    }

    fn num_levels(&self) -> i32 {
        self.levels
    }

    fn min_doc_id(&self, level: i32) -> i32 {
        self.min_doc_id[level as usize]
    }

    fn max_doc_id(&self, level: i32) -> i32 {
        self.max_doc_id[level as usize]
    }

    fn min_value(&self, level: i32) -> i64 {
        self.min_value[level as usize]
    }

    fn max_value(&self, level: i32) -> i64 {
        self.max_value[level as usize]
    }

    fn doc_count(&self, level: i32) -> i32 {
        self.doc_count[level as usize]
    }

    fn global_min_value(&self) -> i64 {
        self.entry.min_value
    }

    fn global_max_value(&self) -> i64 {
        self.entry.max_value
    }

    fn global_doc_count(&self) -> i32 {
        self.entry.doc_count
    }

    fn max_value_count(&self) -> i32 {
        self.entry.max_value_count
    }
}

#[derive(Debug, Clone)]
enum FieldEntry {
    Numeric(NumericEntry),
    Binary(BinaryEntry),
    Sorted(SortedEntry),
    SortedNumeric(SortedNumericEntry),
    SortedSet(SortedSetEntry),
}

/// Reader for [`Lucene90DocValuesFormat`].
///
/// The entries hold metadata only; every value is read from `data` when an
/// iterator asks for it, as `Lucene90DocValuesProducer` does. Nothing that
/// comes off disk sizes an allocation at open time.
pub struct Lucene90DocValuesProducer {
    data: Box<dyn IndexInput>,
    max_doc: i32,
    entries: HashMap<i32, FieldEntry>,
    skippers: HashMap<i32, DocValuesSkipperEntry>,
    /// The `.dvs` skip-index file, opened when the segment was written at
    /// [`VERSION_SKIPPER_SEPARATE_FILE`] or later; older segments keep their
    /// skip index inside the data file.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.skipIndexData`
    /// (`Lucene90DocValuesProducer.java:70,155-183`).
    skip_index_data: Option<Box<dyn IndexInput>>,
    /// Set on the instance `get_merge_instance` hands out, exactly as Lucene's
    /// `merging` field is (`Lucene90DocValuesProducer.java:2295-2310`).
    merging: bool,
}

impl fmt::Debug for Lucene90DocValuesProducer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90DocValuesProducer")
            .field("max_doc", &self.max_doc)
            .field("fields", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl Lucene90DocValuesProducer {
    fn new(state: &SegmentReadState) -> Result<Self> {
        let data_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            DATA_EXTENSION,
        );
        let mut data = state.directory.open_input(&data_name, state.context)?;
        check_index_header(
            &mut *data,
            DATA_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let meta_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta = state.directory.open_input(&meta_name, state.context)?;
        let version = check_index_header(
            &mut *meta,
            META_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let max_doc = state.segment_info.max_doc()?;
        let (entries, mut skippers) = Self::read_fields(&mut *meta, state.field_infos, version)?;

        if version < VERSION_SKIPPER_MAX_VALUE_COUNT {
            Self::infer_max_value_counts(&mut skippers, &entries, state.field_infos);
        }

        let _ = retrieve_checksum(&mut *meta)?;

        // `Lucene90DocValuesProducer.java:155-183`: from
        // VERSION_SKIPPER_SEPARATE_FILE on, the skip index lives in its own
        // file, whose header must carry the metadata's version.
        let skip_index_data = if version >= VERSION_SKIPPER_SEPARATE_FILE {
            let skip_index_name = segment_file_name(
                &state.segment_info.name,
                &state.segment_suffix,
                SKIP_INDEX_EXTENSION,
            );
            let mut skip_in = state
                .directory
                .open_input(&skip_index_name, state.context)?;
            let skip_version = check_index_header(
                &mut *skip_in,
                SKIP_INDEX_CODEC,
                VERSION_SKIPPER_SEPARATE_FILE,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            if version != skip_version {
                return Err(LuceneError::corrupt_index(
                    format!("Format versions mismatch: meta={version}, skipIndex={skip_version}"),
                    skip_in.resource_description(),
                ));
            }
            let _ = retrieve_checksum(&mut *skip_in)?;
            Some(skip_in)
        } else {
            None
        };
        Ok(Self {
            data,
            max_doc,
            entries,
            skippers,
            skip_index_data,
            merging: false,
        })
    }

    // -------------------------------------------------------------------------
    // Metadata parsing — no data file access at all
    // -------------------------------------------------------------------------

    /// Reads one metadata entry per field, until the terminating `-1`.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.readFields(IndexInput, FieldInfos)`.
    /// The field number is resolved against the field infos, and one the
    /// segment does not declare is refused before a single offset of its entry
    /// is used.
    fn read_fields(
        meta: &mut dyn IndexInput,
        field_infos: &crate::index::FieldInfos,
        version: i32,
    ) -> Result<(
        HashMap<i32, FieldEntry>,
        HashMap<i32, DocValuesSkipperEntry>,
    )> {
        let mut entries = HashMap::new();
        let mut skippers = HashMap::new();
        loop {
            let field_number = meta.read_int()?;
            if field_number == -1 {
                break;
            }
            let Some(info) = field_infos.field_info_by_number(field_number) else {
                return corrupt(format!("Invalid field number: {field_number}"));
            };
            let kind = meta.read_byte()?;
            if info.doc_values_skip_index_type != crate::index::DocValuesSkipIndexType::NONE {
                skippers.insert(
                    info.number,
                    Self::read_doc_value_skipper_meta(meta, version)?,
                );
            }
            let entry = match kind {
                TYPE_NUMERIC => FieldEntry::Numeric(Self::read_numeric(meta)?),
                TYPE_BINARY => FieldEntry::Binary(Self::read_binary(meta)?),
                TYPE_SORTED => FieldEntry::Sorted(Self::read_sorted(meta)?),
                TYPE_SORTED_SET => FieldEntry::SortedSet(Self::read_sorted_set(meta)?),
                TYPE_SORTED_NUMERIC => FieldEntry::SortedNumeric(Self::read_sorted_numeric(meta)?),
                _ => return corrupt(format!("invalid type: {kind}")),
            };
            entries.insert(field_number, entry);
        }
        Ok((entries, skippers))
    }

    /// Reads one field's skip-index metadata: where its intervals live in the
    /// `.dvs` file and the global bounds they summarise.
    ///
    /// A segment written before [`VERSION_SKIPPER_MAX_VALUE_COUNT`] carries no
    /// maximum value count, so it is left at `-1` (unknown) for a non-empty
    /// field and `0` for an empty one, which
    /// [`infer_max_value_counts`](Self::infer_max_value_counts) then tries to
    /// resolve from the field's own entry.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the metadata file.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.Lucene90DocValuesProducer#readDocValueSkipperMeta`.
    fn read_doc_value_skipper_meta(
        meta: &mut dyn IndexInput,
        version: i32,
    ) -> Result<DocValuesSkipperEntry> {
        let offset = meta.read_long()?;
        let length = meta.read_long()?;
        let max_value = meta.read_long()?;
        let min_value = meta.read_long()?;
        let doc_count = meta.read_int()?;
        let max_doc_id = meta.read_int()?;
        let max_value_count = if version >= VERSION_SKIPPER_MAX_VALUE_COUNT {
            meta.read_int()?
        } else if doc_count == 0 {
            0
        } else {
            -1
        };

        Ok(DocValuesSkipperEntry {
            offset,
            length,
            min_value,
            max_value,
            doc_count,
            max_doc_id,
            max_value_count,
        })
    }

    /// Fills in the maximum value count of every skipper read from a segment
    /// written before [`VERSION_SKIPPER_MAX_VALUE_COUNT`], wherever the field's
    /// own entry proves it can only be `1`.
    ///
    /// A numeric or sorted field carries exactly one value per document by
    /// definition; a sorted-numeric or sorted-set one does when it wrote as
    /// many values as it has documents with a value. Any other field keeps
    /// `-1`, meaning "unknown".
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.Lucene90DocValuesProducer#inferMaxValueCounts`.
    fn infer_max_value_counts(
        skippers: &mut HashMap<i32, DocValuesSkipperEntry>,
        entries: &HashMap<i32, FieldEntry>,
        field_infos: &crate::index::FieldInfos,
    ) {
        for (field_number, entry) in skippers.iter_mut() {
            if entry.max_value_count != -1 || entry.doc_count == 0 {
                continue;
            }
            let mut inferred_max_value_count = -1i32;
            if let Some(info) = field_infos.field_info_by_number(*field_number) {
                match info.doc_values_type {
                    crate::index::DocValuesType::NUMERIC | crate::index::DocValuesType::SORTED => {
                        inferred_max_value_count = 1;
                    }
                    crate::index::DocValuesType::SORTED_NUMERIC => {
                        if let Some(FieldEntry::SortedNumeric(sne)) = entries.get(field_number) {
                            if sne.numeric.num_values == i64::from(sne.num_docs_with_field) {
                                inferred_max_value_count = 1;
                            }
                        }
                    }
                    crate::index::DocValuesType::SORTED_SET => match entries.get(field_number) {
                        Some(FieldEntry::SortedSet(SortedSetEntry::Single(_))) => {
                            inferred_max_value_count = 1;
                        }
                        Some(FieldEntry::SortedSet(SortedSetEntry::Multi { ords, .. })) => {
                            if ords.numeric.num_values == i64::from(ords.num_docs_with_field) {
                                inferred_max_value_count = 1;
                            }
                        }
                        _ => {}
                    },
                    // Every other doc-values type leaves the count unknown.
                    _ => {}
                }
            }
            if inferred_max_value_count != -1 {
                entry.max_value_count = inferred_max_value_count;
            }
        }
    }

    /// Returns the packed-integer reader to use for a values slice, choosing
    /// the merge-specialised one while this producer is serving a merge.
    ///
    /// Divergence: this port's numeric decoding keeps a concrete
    /// [`DirectReader`] rather than a `LongValues`, because it reads values
    /// through `DirectReader::get_checked`, which reports a read past the end
    /// of a corrupt block instead of panicking as `LongValues::get` would.
    /// This method is therefore ported for parity but is not yet on that path;
    /// wiring it in means first giving the merge instance a fallible accessor
    /// of its own.
    ///
    /// # Errors
    ///
    /// Returns the error `DirectReader` raises for an unsupported width.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.Lucene90DocValuesProducer#getDirectReaderInstance`.
    #[allow(dead_code)]
    fn get_direct_reader_instance(
        &self,
        slice: Rc<RefCell<Box<dyn RandomAccessInput>>>,
        bits_per_value: i32,
        offset: i64,
        num_values: i64,
    ) -> Result<Box<dyn crate::util::extra::LongValues>> {
        if self.merging {
            Ok(Box::new(DirectReader::get_merge_instance(
                slice,
                bits_per_value,
                offset,
                num_values,
            )))
        } else {
            Ok(Box::new(DirectReader::with_random_access(
                slice,
                bits_per_value,
                offset,
            )?))
        }
    }

    /// Returns whether an entry's values can be read a whole byte at a time.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.canBulkDecodeByteAligned(NumericEntry)`:
    /// only a single-block entry whose width is a positive multiple of eight
    /// bits sits on byte boundaries.
    ///
    /// The values source keeps the entry's `blockShift` and `bitsPerValue`
    /// rather than the entry itself, so those two fields are what this takes.
    fn can_bulk_decode_byte_aligned(block_shift: i32, bits_per_value: i32) -> bool {
        block_shift < 0 && bits_per_value > 0 && (bits_per_value & 0x07) == 0
    }

    /// Returns whether `docs[docs_offset..docs_offset + size]` is a run of
    /// consecutive document numbers.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.isContiguous(int, int[], int)`.
    /// Only a contiguous run maps onto a contiguous byte range, which is what
    /// makes a bulk read possible.
    fn is_contiguous(size: usize, docs: &[i32], docs_offset: usize) -> bool {
        size == 0 || docs[docs_offset + size - 1] - docs[docs_offset] == size as i32 - 1
    }

    /// Returns how many bytes past the values a bulk decode may read.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.paddingBytesNeededForBulkDecode(int)`.
    /// The decoder reads a whole `long` for the wider widths and a whole `int`
    /// for 24 bits, so the tail must be padded to keep the last read in bounds.
    fn padding_bytes_needed_for_bulk_decode(bits_per_value: i32) -> i32 {
        match bits_per_value {
            24 => 1,
            40 | 48 | 56 => std::mem::size_of::<i64>() as i32 - bits_per_value / 8,
            _ => 0,
        }
    }

    /// Replaces each decoded ordinal with the table value it stands for.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.applyTable(long[], int, long[], int)`,
    /// the last step of reading a table-compressed numeric field.
    fn apply_table(values: &mut [i64], values_offset: usize, table: &[i64], size: usize) {
        for value in values.iter_mut().skip(values_offset).take(size) {
            *value = table[*value as usize];
        }
    }

    /// Undoes the GCD and minimum-value compression of a numeric field.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.applyGcdDelta(long[], int, long, long, int)`.
    fn apply_gcd_delta(
        values: &mut [i64],
        values_offset: usize,
        mul: i64,
        delta: i64,
        size: usize,
    ) {
        for value in values.iter_mut().skip(values_offset).take(size) {
            *value = mul.wrapping_mul(*value).wrapping_add(delta);
        }
    }

    /// Returns the fixed number of values every document of a sorted-numeric
    /// field holds, or `-1` when it varies.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.fixedCardinality(SortedNumericEntry, DocValuesSkipperEntry)`.
    /// A fixed cardinality is what lets the range scan address a document's
    /// values by multiplication rather than through the offsets.
    fn fixed_cardinality(
        num_values: i64,
        num_docs_with_field: i32,
        skipper_max_value_count: Option<i32>,
    ) -> i32 {
        let Some(max_value_count) = skipper_max_value_count else {
            return -1;
        };
        if max_value_count <= 1 || num_docs_with_field == 0 {
            return -1;
        }
        let num_docs = i64::from(num_docs_with_field);
        if num_values % num_docs != 0 {
            return -1;
        }
        let cardinality = num_values / num_docs;
        if cardinality > i64::from(i32::MAX) || cardinality != i64::from(max_value_count) {
            return -1;
        }
        cardinality as i32
    }

    /// Returns whether any value in `[start, end)` falls in `[min_value, max_value]`.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.sortedNumericMatchesRange(LongValues, long, long, long, long)`.
    /// The values of a document are sorted, so the first one at or above the
    /// lower bound decides the answer.
    ///
    /// `values` is the fallible accessor of the field's values, so a corrupt
    /// index reports an error where Java's `LongValues.get` throws.
    fn sorted_numeric_matches_range(
        values: &dyn Fn(i64) -> Result<i64>,
        start: i64,
        end: i64,
        min_value: i64,
        max_value: i64,
    ) -> Result<bool> {
        for value_offset in start..end {
            let value = values(value_offset)?;
            if value >= min_value {
                return Ok(value <= max_value);
            }
        }
        Ok(false)
    }

    /// Sets a bit for every document in `[from_doc, to_doc)` whose sorted
    /// numeric values reach the range, when every document holds exactly
    /// `cardinality` values.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.sortedNumericScalarRangeIntoBitSet(LongValues, int, int, int, long, long, FixedBitSet, int)`.
    #[allow(clippy::too_many_arguments)]
    fn sorted_numeric_scalar_range_into_bit_set(
        values: &dyn Fn(i64) -> Result<i64>,
        from_doc: i32,
        to_doc: i32,
        cardinality: i32,
        min_value: i64,
        max_value: i64,
        bit_set: &mut crate::util::FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        for doc in from_doc..to_doc {
            let value_offset = i64::from(doc) * i64::from(cardinality);
            for i in 0..i64::from(cardinality) {
                let value = values(value_offset + i)?;
                if value >= min_value {
                    if value <= max_value {
                        bit_set.set((doc - offset) as usize);
                    }
                    break;
                }
            }
        }
        Ok(())
    }

    /// Sets a bit for every document in `[from_doc, to_doc)` whose single value
    /// falls in `[min_value, max_value]`.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.rangeIntoBitSet(LongValues, int, int, long, long, FixedBitSet, int)`,
    /// which Java forwards to the vectorised `DocValuesRangeSupport`.
    ///
    /// **Divergence from Lucene 10.5.0.** That support has a Panama path over
    /// vector lanes; stable Rust has no portable SIMD, so the scalar loop —
    /// which is what Java's own fallback runs — is the whole of it here. The
    /// bits set are identical either way.
    #[allow(clippy::too_many_arguments)]
    fn range_into_bit_set(
        values: &dyn Fn(i64) -> Result<i64>,
        from_doc: i32,
        to_doc: i32,
        min_value: i64,
        max_value: i64,
        bit_set: &mut crate::util::FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        for doc in from_doc..to_doc {
            let value = values(i64::from(doc))?;
            if value >= min_value && value <= max_value {
                bit_set.set((doc - offset) as usize);
            }
        }
        Ok(())
    }

    /// Reads a contiguous run of byte-aligned values in one go, decoding them
    /// into `values`.
    ///
    /// Equivalent to `Lucene90DocValuesProducer.bulkDecodeByteAlignedValues(RandomAccessInput, NumericEntry, int, int[], int, long[], int, byte[])`.
    /// Returns `None` — and leaves `values` untouched — whenever the fast path
    /// does not apply, so the caller falls back to reading one value per
    /// document. The scratch buffer is returned so the caller can keep it.
    #[allow(clippy::too_many_arguments)]
    fn bulk_decode_byte_aligned_values(
        slice: &mut dyn RandomAccessInput,
        bits_per_value: i32,
        block_shift: i32,
        size: usize,
        docs: &[i32],
        docs_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        bytes: &mut Vec<u8>,
    ) -> Result<bool> {
        if !Self::can_bulk_decode_byte_aligned(block_shift, bits_per_value)
            || !Self::is_contiguous(size, docs, docs_offset)
        {
            return Ok(false);
        }
        let bytes_per_value = (bits_per_value / 8) as i64;
        let byte_count_long = size as i64 * bytes_per_value;
        if byte_count_long > i64::from(i32::MAX) {
            return Ok(false);
        }
        let byte_count = byte_count_long as usize;
        let read_byte_count = if byte_count == 0 {
            0
        } else {
            byte_count + Self::padding_bytes_needed_for_bulk_decode(bits_per_value) as usize
        };
        let offset = if byte_count == 0 {
            0
        } else {
            i64::from(docs[docs_offset]) * bytes_per_value
        };
        if offset + read_byte_count as i64 > slice.length() {
            return Ok(false);
        }
        if bytes.len() < read_byte_count {
            bytes.resize(read_byte_count, 0);
        }
        if byte_count != 0 {
            slice.read_bytes_at(offset, bytes, 0, read_byte_count)?;
            crate::internal::vectorization::get_instance()
                .get_doc_values_bulk_decode_support()
                .decode_byte_aligned(
                    bytes,
                    0,
                    bits_per_value as u32,
                    values,
                    values_offset,
                    size,
                )?;
        }
        Ok(true)
    }

    fn read_numeric(meta: &mut dyn IndexInput) -> Result<NumericEntry> {
        let docs_with_field_offset = meta.read_long()?;
        let docs_with_field_length = meta.read_long()?;
        let jump_table_entry_count = meta.read_short()?;
        let dense_rank_power = meta.read_byte()? as i8;
        let num_values = meta.read_long()?;
        let table_size = meta.read_int()?;
        if table_size > MAX_TABLE_SIZE {
            return corrupt(format!("invalid table size: {table_size}"));
        }
        let table = if table_size > 0 {
            Some(
                (0..table_size)
                    .map(|_| meta.read_long())
                    .collect::<Result<Vec<_>>>()?,
            )
        } else {
            None
        };
        // `tableSize < -1` encodes the per-block width layout as
        // `-2 - blockShift` (`Lucene90DocValuesProducer.java:325-329`).
        let block_shift = if table_size < -1 { -2 - table_size } else { -1 };
        let bits_per_value = meta.read_byte()?;
        let min_value = meta.read_long()?;
        let gcd = meta.read_long()?;
        let values_offset = meta.read_long()?;
        let values_length = meta.read_long()?;
        let value_jump_table_offset = meta.read_long()?;
        Ok(NumericEntry {
            docs_with_field_offset,
            docs_with_field_length,
            jump_table_entry_count,
            dense_rank_power,
            num_values,
            table,
            block_shift,
            bits_per_value,
            min_value,
            gcd,
            values_offset,
            values_length,
            value_jump_table_offset,
        })
    }

    fn read_binary(meta: &mut dyn IndexInput) -> Result<BinaryEntry> {
        let data_offset = meta.read_long()?;
        let data_length = meta.read_long()?;
        let docs_with_field_offset = meta.read_long()?;
        let docs_with_field_length = meta.read_long()?;
        let jump_table_entry_count = meta.read_short()?;
        let dense_rank_power = meta.read_byte()? as i8;
        let num_docs_with_field = meta.read_int()?;
        let min_length = meta.read_int()?;
        let max_length = meta.read_int()?;
        let mut addresses_offset = 0;
        let mut addresses_meta = None;
        let mut addresses_length = 0;
        if min_length < max_length {
            addresses_offset = meta.read_long()?;
            let num_addresses = i64::from(num_docs_with_field).wrapping_add(1);
            let block_shift = meta.read_v_int()?;
            addresses_meta = Some(DirectMonotonicMeta::load(meta, num_addresses, block_shift)?);
            addresses_length = meta.read_long()?;
        }
        Ok(BinaryEntry {
            data_offset,
            data_length,
            docs_with_field_offset,
            docs_with_field_length,
            jump_table_entry_count,
            dense_rank_power,
            num_docs_with_field,
            min_length,
            max_length,
            addresses_offset,
            addresses_meta,
            addresses_length,
        })
    }

    fn read_sorted(meta: &mut dyn IndexInput) -> Result<SortedEntry> {
        let ords = Self::read_numeric(meta)?;
        let terms_dict = Self::read_term_dict(meta)?;
        Ok(SortedEntry { ords, terms_dict })
    }

    fn read_sorted_set(meta: &mut dyn IndexInput) -> Result<SortedSetEntry> {
        let multi_valued = meta.read_byte()?;
        match multi_valued {
            0 => Ok(SortedSetEntry::Single(Self::read_sorted(meta)?)),
            1 => {
                let ords = Self::read_sorted_numeric(meta)?;
                let terms_dict = Self::read_term_dict(meta)?;
                Ok(SortedSetEntry::Multi { ords, terms_dict })
            }
            other => corrupt(format!("Invalid multiValued flag: {other}")),
        }
    }

    fn read_sorted_numeric(meta: &mut dyn IndexInput) -> Result<SortedNumericEntry> {
        let numeric = Self::read_numeric(meta)?;
        let num_docs_with_field = meta.read_int()?;
        let mut addresses_offset = 0;
        let mut addresses_meta = None;
        let mut addresses_length = 0;
        if i64::from(num_docs_with_field) != numeric.num_values {
            addresses_offset = meta.read_long()?;
            let block_shift = meta.read_v_int()?;
            addresses_meta = Some(DirectMonotonicMeta::load(
                meta,
                i64::from(num_docs_with_field).wrapping_add(1),
                block_shift,
            )?);
            addresses_length = meta.read_long()?;
        }
        Ok(SortedNumericEntry {
            numeric,
            num_docs_with_field,
            addresses_offset,
            addresses_meta,
            addresses_length,
        })
    }

    fn read_term_dict(meta: &mut dyn IndexInput) -> Result<TermsDictEntry> {
        let terms_dict_size = meta.read_v_long()?;
        let block_shift = meta.read_int()?;
        let addresses_size = shift_up_round(terms_dict_size, TERMS_DICT_BLOCK_LZ4_SHIFT as u32);
        let terms_addresses_meta = DirectMonotonicMeta::load(meta, addresses_size, block_shift)?;
        let _max_term_length = meta.read_int()?;
        let _max_block_length = meta.read_int()?;
        let terms_data_offset = meta.read_long()?;
        let terms_data_length = meta.read_long()?;
        let terms_addresses_offset = meta.read_long()?;
        let terms_addresses_length = meta.read_long()?;
        // **Divergence.** Lucene keeps the reverse index and uses it to narrow
        // `seekCeil`/`lookupTerm` to one block before scanning
        // (`TermsDict.seekTermsIndex`). This port answers `lookup_term` with
        // the trait's binary search over `lookup_ord`, so the reverse index is
        // consumed here only to keep the metadata cursor aligned and then
        // discarded. The results agree — both find the same term — but a
        // lookup costs O(log n) block seeks instead of one index-guided seek.
        // Porting the reverse index would remove the difference.
        let index_shift = meta.read_int()?;
        let index_size = shift_up_round(terms_dict_size, index_shift as u32);
        let _index_meta = DirectMonotonicMeta::load(meta, index_size.wrapping_add(1), block_shift)?;
        let _index_offset = meta.read_long()?;
        let _index_length = meta.read_long()?;
        let _index_addresses_offset = meta.read_long()?;
        let _index_addresses_length = meta.read_long()?;
        Ok(TermsDictEntry {
            terms_dict_size,
            terms_addresses_meta,
            terms_data_offset,
            terms_data_length,
            terms_addresses_offset,
            terms_addresses_length,
        })
    }
}

/// Java's `(value + (1L << shift) - 1) >>> shift`.
///
/// The shift can come off disk, and Java takes its low six bits rather than
/// refusing it, so a corrupt file has to produce here whatever it produces
/// there — an absurd block count the metadata load then runs out of input on,
/// never an overflow.
fn shift_up_round(value: i64, shift: u32) -> i64 {
    let shift = shift & 63;
    let value = value as u64;
    (value.wrapping_add(1u64 << shift).wrapping_sub(1) >> shift) as i64
}

/// Checks a region against the data file before slicing it.
///
/// `IndexInput.slice` throws `IllegalArgumentException` for a region the file
/// does not hold; both numbers here come off disk, so the same check has to
/// happen before the slice is built.
fn check_region(data: &dyn IndexInput, offset: i64, length: i64) -> Result<()> {
    let file_length = data.length();
    if offset < 0 || length < 0 || offset > file_length || length > file_length - offset {
        return corrupt(format!(
            "doc-values region [{offset}, {offset}+{length}) lies outside the \
             {file_length}-byte data file"
        ));
    }
    Ok(())
}

fn checked_slice(
    data: &dyn IndexInput,
    description: &str,
    offset: i64,
    length: i64,
) -> Result<Box<dyn IndexInput>> {
    check_region(data, offset, length)?;
    data.slice(description, offset, length)
}

type SharedRandomAccess = Rc<RefCell<Box<dyn RandomAccessInput>>>;

fn checked_random_access(
    data: &dyn IndexInput,
    offset: i64,
    length: i64,
) -> Result<SharedRandomAccess> {
    check_region(data, offset, length)?;
    Ok(Rc::new(RefCell::new(
        data.random_access_slice(offset, length)?,
    )))
}

// -----------------------------------------------------------------------------
// Lazy views over the data file
// -----------------------------------------------------------------------------

/// The documents that carry a value, and the ordinal of the current one within
/// the value stream.
///
/// Mirrors the split `Lucene90DocValuesProducer` makes between its
/// `Dense*DocValues` and `Sparse*DocValues` inner classes: a dense field needs
/// no documents-with-field stream and indexes its values by document id, while
/// a sparse one indexes them by `IndexedDISI.index()`.
enum DocsCursor {
    /// `docsWithFieldOffset == -2`: no document carries the field.
    Empty { doc: i32 },
    /// `docsWithFieldOffset == -1`: every document carries it.
    Dense { max_doc: i32, doc: i32 },
    /// An `IndexedDISI` stream names the documents.
    Sparse { disi: IndexedDISI },
}

impl DocsCursor {
    /// Returns whether every document of the segment carries the field.
    ///
    /// The bulk read path applies only to a dense field: a sparse one has no
    /// value for every document of a contiguous run.
    fn is_dense(&self) -> bool {
        matches!(self, Self::Dense { .. })
    }

    /// Leaves the cursor on `doc`, as Lucene's bulk override does after
    /// reading a run.
    fn set_doc_id(&mut self, doc_id: i32) {
        if let Self::Dense { doc, .. } = self {
            *doc = doc_id;
        }
    }

    fn new(
        data: &dyn IndexInput,
        max_doc: i32,
        offset: i64,
        length: i64,
        jump_table_entry_count: i16,
        dense_rank_power: i8,
        cost: i64,
    ) -> Result<Self> {
        if offset == -2 {
            return Ok(Self::Empty { doc: -1 });
        }
        if offset == -1 {
            return Ok(Self::Dense { max_doc, doc: -1 });
        }
        check_region(data, offset, length)?;
        Ok(Self::Sparse {
            disi: IndexedDISI::new(
                data,
                offset,
                length,
                i32::from(jump_table_entry_count),
                dense_rank_power,
                cost,
            )?,
        })
    }

    fn doc_id(&self) -> i32 {
        match self {
            Self::Empty { doc } | Self::Dense { doc, .. } => *doc,
            Self::Sparse { disi } => disi.doc_id(),
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        match self {
            Self::Empty { doc } => {
                *doc = NO_MORE_DOCS;
                Ok(NO_MORE_DOCS)
            }
            Self::Dense { doc, .. } => {
                let target = doc.wrapping_add(1);
                self.advance(target)
            }
            Self::Sparse { disi } => disi.next_doc(),
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        match self {
            Self::Empty { doc } => {
                *doc = NO_MORE_DOCS;
                Ok(NO_MORE_DOCS)
            }
            // `DenseNumericDocValues.advance` (`Lucene90DocValuesProducer.java:
            // 695-701`) tests only the upper bound. A `next_doc` past the end
            // overflows `doc + 1` to `Integer.MIN_VALUE` in Java exactly as it
            // wraps to `i32::MIN` here, and Java then stores that as the
            // document id; the iterator contract forbids the call, and both
            // sides answer it with the same rubbish rather than a check.
            Self::Dense { max_doc, doc } => {
                *doc = if target >= *max_doc {
                    NO_MORE_DOCS
                } else {
                    target
                };
                Ok(*doc)
            }
            Self::Sparse { disi } => disi.advance(target),
        }
    }

    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        match self {
            Self::Empty { .. } => Ok(false),
            // `DenseNumericDocValues.advanceExact` (`Lucene90DocValuesProducer
            // .java:703-707`) sets the document and answers `true` without
            // testing anything: every document of a dense field has a value by
            // definition, and a target outside the segment is a caller error
            // the contract leaves undefined.
            Self::Dense { doc, .. } => {
                *doc = target;
                Ok(true)
            }
            Self::Sparse { disi } => disi.advance_exact(target),
        }
    }

    /// The ordinal of the current document within the value stream.
    ///
    /// A dense field indexes its values by document id and a sparse one by
    /// `IndexedDISI.index()`, which is what Lucene's `Dense*DocValues` and
    /// `Sparse*DocValues` do. The cursor is **not** checked for being
    /// positioned: `DocValuesIterator` leaves the result undefined unless it
    /// is, and Lucene reads at whatever `doc` holds — a constant field even
    /// answers with its constant after the iterator is exhausted
    /// (`Lucene90DocValuesProducer.java:789-793`). An out-of-range ordinal
    /// still cannot read out of bounds: it reaches `DirectReader::get_checked`,
    /// which reports the read past the end rather than serving it.
    fn index(&self) -> i64 {
        match self {
            Self::Empty { .. } => -1,
            Self::Dense { doc, .. } => i64::from(*doc),
            Self::Sparse { disi } => i64::from(disi.index()),
        }
    }

    fn cost(&self, cost: i64) -> i64 {
        match self {
            Self::Empty { .. } => 0,
            Self::Dense { max_doc, .. } => i64::from(*max_doc),
            Self::Sparse { .. } => cost,
        }
    }
}

/// How a numeric entry's values are decoded, one at a time, from the file.
///
/// The three shapes are the ones `getNumeric` branches on
/// (`Lucene90DocValuesProducer.java:783-...`): a constant that lives in the
/// metadata, an ordinal into a unique-value table, and `gcd * packed + min`.
enum NumericValueSource {
    Constant(i64),
    /// Values split into blocks of `2^blockShift` with their own width, read
    /// through Java's `VaryingBPVReader`.
    Varying(RefCell<VaryingBpvReader>),
    Table {
        reader: DirectReader,
        table: Vec<i64>,
        /// The packed region, kept for the bulk read path.
        slice: Rc<RefCell<Box<dyn RandomAccessInput>>>,
        bits_per_value: i32,
        block_shift: i32,
    },
    Packed {
        reader: DirectReader,
        min: i64,
        gcd: i64,
        /// The packed region, kept for the bulk read path.
        slice: Rc<RefCell<Box<dyn RandomAccessInput>>>,
        bits_per_value: i32,
        block_shift: i32,
    },
}

impl NumericValueSource {
    fn new(data: &dyn IndexInput, entry: &NumericEntry) -> Result<Self> {
        if entry.bits_per_value == 0 {
            return Ok(Self::Constant(entry.min_value));
        }
        let slice = checked_random_access(data, entry.values_offset, entry.values_length)?;
        if entry.block_shift >= 0 {
            // Dense or sparse, "split into blocks of different bits per value"
            // (`Lucene90DocValuesProducer.java:819-823, 1003-1007, 1063-1065`).
            return Ok(Self::Varying(RefCell::new(VaryingBpvReader::new(
                data, entry, slice,
            )?)));
        }
        let reader = DirectReader::with_random_access(
            Rc::clone(&slice),
            i32::from(entry.bits_per_value),
            0,
        )?;
        let bits_per_value = i32::from(entry.bits_per_value);
        Ok(match entry.table {
            Some(ref table) => Self::Table {
                reader,
                table: table.clone(),
                slice,
                bits_per_value,
                block_shift: entry.block_shift,
            },
            None => Self::Packed {
                reader,
                min: entry.min_value,
                gcd: entry.gcd,
                slice,
                bits_per_value,
                block_shift: entry.block_shift,
            },
        })
    }

    fn value(&self, index: i64) -> Result<i64> {
        match self {
            Self::Constant(value) => Ok(*value),
            Self::Varying(reader) => reader.borrow_mut().get_long_value(index),
            Self::Table { reader, table, .. } => {
                let ordinal = reader.get_checked(index)?;
                // The ordinal is `bitsPerValue` wide and the table at most 256
                // long: a corrupt width makes the two disagree, and Java
                // answers that with an `ArrayIndexOutOfBoundsException` rather
                // than a value from somewhere else.
                usize::try_from(ordinal)
                    .ok()
                    .and_then(|ordinal| table.get(ordinal).copied())
                    .map_or_else(
                        || {
                            corrupt(format!(
                                "table ordinal {ordinal} is outside the {}-entry table",
                                table.len()
                            ))
                        },
                        Ok,
                    )
            }
            // Java's arithmetic wraps here; a corrupt `min` or `gcd` must
            // produce the value Java would produce, not an overflow panic.
            Self::Packed {
                reader, min, gcd, ..
            } => Ok(gcd
                .wrapping_mul(reader.get_checked(index)?)
                .wrapping_add(*min)),
        }
    }
}

/// Reader for longs split into blocks of different bits per value.
///
/// Equivalent to `Lucene90DocValuesProducer.VaryingBPVReader`
/// (`Lucene90DocValuesProducer.java:2295-2365`). Each block starts with its
/// width byte and its `delta` long, followed, for a non-zero width, by the
/// byte length of its packed values and the values themselves. The values must
/// be requested in increasing order: the next block is reached by walking
/// from the current one, and any other block through the offsets jump table
/// the consumer writes after the last block.
///
/// Like every reader of this module it decodes through
/// [`DirectReader::get_checked`] and never takes Java's
/// `getDirectReaderInstance` merge variant; see
/// [`Lucene90DocValuesProducer::get_direct_reader_instance`].
struct VaryingBpvReader {
    slice: SharedRandomAccess,
    /// The jump table: the data-file offset of every block, `None` when the
    /// entry has none (`valueJumpTableOffset == -1`).
    rank_slice: Option<SharedRandomAccess>,
    values_offset: i64,
    shift: i32,
    mul: i64,
    mask: i64,
    block: i64,
    delta: i64,
    offset: i64,
    block_end_offset: i64,
    /// `None` is Java's `LongValues.ZEROES`, the values of a width-0 block.
    values: Option<DirectReader>,
}

impl VaryingBpvReader {
    fn new(data: &dyn IndexInput, entry: &NumericEntry, slice: SharedRandomAccess) -> Result<Self> {
        let rank_slice = if entry.value_jump_table_offset == -1 {
            None
        } else {
            Some(checked_random_access(
                data,
                entry.value_jump_table_offset,
                data.length().wrapping_sub(entry.value_jump_table_offset),
            )?)
        };
        let shift = entry.block_shift;
        Ok(Self {
            slice,
            rank_slice,
            values_offset: entry.values_offset,
            shift,
            mul: entry.gcd,
            // Java's `(1 << shift) - 1` is an `int`.
            mask: i64::from(1i32.wrapping_shl(shift as u32).wrapping_sub(1)),
            block: -1,
            delta: 0,
            offset: 0,
            block_end_offset: 0,
            values: None,
        })
    }

    /// `getLongValue(long)`.
    fn get_long_value(&mut self, index: i64) -> Result<i64> {
        // Java's `index >>> shift` on a `long`, whose shift count is masked to
        // six bits.
        let block = ((index as u64).wrapping_shr(self.shift as u32)) as i64;
        if self.block != block {
            let mut bits_per_value;
            loop {
                // If the needed block is the one directly following the
                // current block, it is cheaper to avoid the cache.
                if let Some(ref rank_slice) = self.rank_slice {
                    if block != self.block.wrapping_add(1) {
                        // Java's `long` arithmetic wraps: a corrupt jump-table
                        // entry yields an offset the next checked read rejects.
                        self.block_end_offset = rank_slice
                            .borrow_mut()
                            .read_long_at(block.wrapping_mul(8))?
                            .wrapping_sub(self.values_offset);
                        self.block = block.wrapping_sub(1);
                    }
                }
                let mut slice = self.slice.borrow_mut();
                self.offset = self.block_end_offset;
                // `slice.readByte` is signed in Java.
                bits_per_value = i32::from(slice.read_byte_at(self.offset)? as i8);
                self.offset = self.offset.wrapping_add(1);
                self.delta = slice.read_long_at(self.offset)?;
                self.offset = self.offset.wrapping_add(8);
                if bits_per_value == 0 {
                    self.block_end_offset = self.offset;
                } else {
                    let length = slice.read_int_at(self.offset)?;
                    self.offset = self.offset.wrapping_add(4);
                    self.block_end_offset = self.offset.wrapping_add(i64::from(length));
                }
                self.block = self.block.wrapping_add(1);
                if self.block == block {
                    break;
                }
            }
            self.values = if bits_per_value == 0 {
                None
            } else {
                Some(DirectReader::with_random_access(
                    Rc::clone(&self.slice),
                    bits_per_value,
                    self.offset,
                )?)
            };
        }
        let value = match self.values {
            Some(ref values) => values.get_checked(index & self.mask)?,
            None => 0,
        };
        Ok(self.mul.wrapping_mul(value).wrapping_add(self.delta))
    }
}

/// A `NUMERIC` field, read one value at a time.
struct LazyNumericDocValues {
    docs: DocsCursor,
    values: NumericValueSource,
    cost: i64,
    /// Scratch buffer the bulk read path reuses between calls.
    ///
    /// Equivalent to the `private byte[] bulkBytes` each of Lucene's dense
    /// numeric views keeps.
    bulk_bytes: Vec<u8>,
}

impl LazyNumericDocValues {
    fn new(data: &dyn IndexInput, max_doc: i32, entry: &NumericEntry) -> Result<Self> {
        let docs = DocsCursor::new(
            data,
            max_doc,
            entry.docs_with_field_offset,
            entry.docs_with_field_length,
            entry.jump_table_entry_count,
            entry.dense_rank_power,
            entry.num_values,
        )?;
        Ok(Self {
            docs,
            values: NumericValueSource::new(data, entry)?,
            cost: entry.num_values,
            bulk_bytes: Vec::new(),
        })
    }

    /// Reads one value per document, which is what the bulk path falls back to.
    ///
    /// Equivalent to the `super.longValues(...)` call each of Lucene's
    /// overrides makes when the fast path does not apply.
    fn long_values_one_by_one(
        &mut self,
        size: i32,
        docs: &[i32],
        docs_offset: i32,
        values: &mut [i64],
        values_offset: i32,
        default_value: i64,
    ) -> Result<()> {
        let size = size as usize;
        if docs.len() < docs_offset as usize + size {
            return Err(LuceneError::IllegalArgument(
                "docs buffer too small for requested size/offset".to_string(),
            ));
        }
        if values.len() < values_offset as usize + size {
            return Err(LuceneError::IllegalArgument(
                "values buffer too small for requested size/offset".to_string(),
            ));
        }
        for i in 0..size {
            let doc = docs[docs_offset as usize + i];
            values[values_offset as usize + i] = if self.advance_exact(doc)? {
                self.long_value()?
            } else {
                default_value
            };
        }
        Ok(())
    }
}

impl DocIdSetIterator for LazyNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.docs.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.docs.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.docs.advance(target)
    }

    fn cost(&self) -> i64 {
        self.docs.cost(self.cost)
    }
}

impl DocValuesIterator for LazyNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.docs.advance_exact(target)
    }
}

impl NumericDocValues for LazyNumericDocValues {
    /// Reads a run of documents' values in one go where the layout allows it.
    ///
    /// Equivalent to the `longValues` override each of Lucene's dense numeric
    /// views carries: when the values are byte-aligned and the documents form a
    /// contiguous run, the whole run is read as bytes and decoded in bulk, and
    /// the table or GCD is applied afterwards. Anything else falls back to the
    /// generic one-value-per-document path.
    fn long_values(
        &mut self,
        size: i32,
        docs: &[i32],
        docs_offset: i32,
        values: &mut [i64],
        values_offset: i32,
        default_value: i64,
    ) -> Result<()> {
        if size < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "size must be >= 0, got {size}"
            )));
        }
        // The bulk path only applies to a dense field: a sparse one has no
        // value for every document in the run.
        if !self.docs.is_dense() {
            return self.long_values_one_by_one(
                size,
                docs,
                docs_offset,
                values,
                values_offset,
                default_value,
            );
        }
        let size_usize = size as usize;
        let mut bulk_bytes = std::mem::take(&mut self.bulk_bytes);
        let decoded = match &self.values {
            NumericValueSource::Table {
                slice,
                bits_per_value,
                block_shift,
                table,
                reader: _,
            } => {
                let mut borrowed = slice.borrow_mut();
                let done = Lucene90DocValuesProducer::bulk_decode_byte_aligned_values(
                    borrowed.as_mut(),
                    *bits_per_value,
                    *block_shift,
                    size_usize,
                    docs,
                    docs_offset as usize,
                    values,
                    values_offset as usize,
                    &mut bulk_bytes,
                )?;
                if done {
                    Lucene90DocValuesProducer::apply_table(
                        values,
                        values_offset as usize,
                        table,
                        size_usize,
                    );
                }
                done
            }
            NumericValueSource::Packed {
                slice,
                bits_per_value,
                block_shift,
                min,
                gcd,
                reader: _,
            } => {
                let mut borrowed = slice.borrow_mut();
                let done = Lucene90DocValuesProducer::bulk_decode_byte_aligned_values(
                    borrowed.as_mut(),
                    *bits_per_value,
                    *block_shift,
                    size_usize,
                    docs,
                    docs_offset as usize,
                    values,
                    values_offset as usize,
                    &mut bulk_bytes,
                )?;
                if done && !(*gcd == 1 && *min == 0) {
                    Lucene90DocValuesProducer::apply_gcd_delta(
                        values,
                        values_offset as usize,
                        *gcd,
                        *min,
                        size_usize,
                    );
                }
                done
            }
            NumericValueSource::Constant(_) | NumericValueSource::Varying(_) => false,
        };
        self.bulk_bytes = bulk_bytes;
        if decoded {
            if size_usize != 0 {
                // The cursor is left on the last document of the run, as
                // Lucene's override leaves it.
                self.docs
                    .set_doc_id(docs[docs_offset as usize + size_usize - 1]);
            }
            return Ok(());
        }
        self.long_values_one_by_one(
            size,
            docs,
            docs_offset,
            values,
            values_offset,
            default_value,
        )
    }

    /// Equivalent to the `rangeIntoBitSet` overrides of Lucene's dense,
    /// single-block, untabled numeric views
    /// (`Lucene90DocValuesProducer.java:911-922, 958-970`): the values are
    /// read by document number without moving the iterator — through
    /// `rangeIntoBitSet` when `gcd == 1 && minValue == 0`, and per document
    /// with the GCD and delta applied otherwise. Every other layout keeps
    /// `NumericDocValues.rangeIntoBitSet`, as in Java.
    fn range_into_bit_set(
        &mut self,
        from_doc: i32,
        to_doc: i32,
        min_value: i64,
        max_value: i64,
        bit_set: &mut crate::util::FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        if self.docs.is_dense() {
            if let NumericValueSource::Packed {
                reader, min, gcd, ..
            } = &self.values
            {
                if *gcd == 1 && *min == 0 {
                    // Bulk range evaluation via DocValuesRangeSupport.
                    return Lucene90DocValuesProducer::range_into_bit_set(
                        &|index| reader.get_checked(index),
                        from_doc,
                        to_doc,
                        min_value,
                        max_value,
                        bit_set,
                        offset,
                    );
                }
                // Per-doc evaluation for gcd/delta encoded fields.
                for d in from_doc..to_doc {
                    let v = gcd
                        .wrapping_mul(reader.get_checked(i64::from(d))?)
                        .wrapping_add(*min);
                    if v >= min_value && v <= max_value {
                        bit_set.set((d - offset) as usize);
                    }
                }
                return Ok(());
            }
        }
        // `NumericDocValues.rangeIntoBitSet`.
        for d in from_doc..to_doc {
            if self.advance_exact(d)? {
                let v = self.long_value()?;
                if v >= min_value && v <= max_value {
                    bit_set.set((d - offset) as usize);
                }
            }
        }
        Ok(())
    }

    fn long_value(&self) -> Result<i64> {
        // No positioning check, for the reason given on `DocsCursor::index`:
        // Lucene reads at whatever the cursor holds, and a constant field
        // answers with its constant whether or not the iterator is positioned.
        // A packed or table-encoded field instead reaches `get_checked`, which
        // reports the out-of-range read exactly where Java throws.
        self.values.value(self.docs.index())
    }
}

/// A `BINARY` field, whose values are cut out of the data region on demand.
struct LazyBinaryDocValues {
    docs: DocsCursor,
    bytes: RefCell<Box<dyn RandomAccessInput>>,
    region_length: i64,
    addresses: Option<DirectMonotonicReader>,
    fixed_length: i32,
    cost: i64,
}

impl LazyBinaryDocValues {
    fn new(data: &dyn IndexInput, max_doc: i32, entry: &BinaryEntry) -> Result<Self> {
        let docs = DocsCursor::new(
            data,
            max_doc,
            entry.docs_with_field_offset,
            entry.docs_with_field_length,
            entry.jump_table_entry_count,
            entry.dense_rank_power,
            i64::from(entry.num_docs_with_field),
        )?;
        if entry.min_length < 0 || entry.max_length < entry.min_length {
            return corrupt(format!(
                "binary lengths must be non-negative and ordered, got \
                 min={} max={}",
                entry.min_length, entry.max_length
            ));
        }
        check_region(data, entry.data_offset, entry.data_length)?;
        let bytes = data.random_access_slice(entry.data_offset, entry.data_length)?;
        let addresses = match entry.addresses_meta {
            Some(ref meta) => {
                let slice =
                    checked_random_access(data, entry.addresses_offset, entry.addresses_length)?;
                Some(DirectMonotonicReader::with_random_access(
                    meta.clone(),
                    slice,
                )?)
            }
            None => None,
        };
        Ok(Self {
            docs,
            bytes: RefCell::new(bytes),
            region_length: entry.data_length,
            addresses,
            fixed_length: entry.min_length,
            cost: i64::from(entry.num_docs_with_field),
        })
    }
}

impl DocIdSetIterator for LazyBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.docs.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.docs.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.docs.advance(target)
    }

    fn cost(&self) -> i64 {
        self.docs.cost(self.cost)
    }
}

impl DocValuesIterator for LazyBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.docs.advance_exact(target)
    }
}

impl BinaryDocValues for LazyBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        let index = self.docs.index();
        if index < 0 {
            return corrupt("the cursor is not positioned on a document".to_string());
        }
        let (start, end) = match self.addresses {
            Some(ref addresses) => (
                addresses.get_checked(index)?,
                addresses.get_checked(index.wrapping_add(1))?,
            ),
            None => {
                let start = index.saturating_mul(i64::from(self.fixed_length));
                (start, start.saturating_add(i64::from(self.fixed_length)))
            }
        };
        // Every bound here is decoded from the file, so the region is checked
        // before a byte of it is read: Java raises rather than handing back
        // bytes that belong to another document.
        if start < 0 || end < start || end > self.region_length {
            return corrupt(format!(
                "binary value spans [{start}, {end}), which the \
                 {}-byte region does not hold",
                self.region_length
            ));
        }
        let length = (end - start) as usize;
        let mut value = try_zeroed(length)?;
        self.bytes
            .borrow_mut()
            .read_bytes_at(start, &mut value, 0, length)?;
        Ok(BytesRef::new(value))
    }
}

/// A cursor over one term dictionary, seeking by ordinal.
///
/// Ports `Lucene90DocValuesProducer.TermsDict`: the terms of each 64-term block
/// are LZ4-compressed against the block's first term, which is stored raw, so
/// reaching an ordinal means seeking to its block, decompressing it once, and
/// walking forward. Only the block a lookup lands in is ever decompressed.
struct TermsDict {
    bytes: Box<dyn IndexInput>,
    block_addresses: DirectMonotonicReader,
    size: i64,
    data_length: i64,
    ord: i64,
    term: Vec<u8>,
    /// The block's first term followed by its decompressed suffixes.
    block: Vec<u8>,
    block_pos: usize,
}

impl TermsDict {
    fn new(data: &dyn IndexInput, entry: &TermsDictEntry) -> Result<Self> {
        let addresses = checked_random_access(
            data,
            entry.terms_addresses_offset,
            entry.terms_addresses_length,
        )?;
        let block_addresses = DirectMonotonicReader::with_random_access(
            entry.terms_addresses_meta.clone(),
            addresses,
        )?;
        let bytes = checked_slice(
            data,
            "terms",
            entry.terms_data_offset,
            entry.terms_data_length,
        )?;
        Ok(Self {
            bytes,
            block_addresses,
            size: entry.terms_dict_size,
            data_length: entry.terms_data_length,
            ord: -1,
            term: Vec::new(),
            block: Vec::new(),
            block_pos: 0,
        })
    }

    fn block_byte(&mut self) -> Result<u8> {
        let byte = *self.block.get(self.block_pos).ok_or_else(|| {
            LuceneError::CorruptIndex("a terms block ended before its terms did".to_string())
        })?;
        self.block_pos += 1;
        Ok(byte)
    }

    fn block_v_int(&mut self) -> Result<i32> {
        let mut value = 0i32;
        let mut shift = 0u32;
        loop {
            let byte = self.block_byte()?;
            value |= i32::from(byte & 0x7F).wrapping_shl(shift);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift = shift.wrapping_add(7);
            if shift > 28 {
                return corrupt("a terms block holds a malformed VInt".to_string());
            }
        }
    }

    /// Reads the first term of the block the cursor sits on and decompresses
    /// the rest of it against that term.
    fn decompress_block(&mut self) -> Result<()> {
        let term_length = self.bytes.read_v_int()?;
        if term_length < 0 {
            return corrupt(format!("negative term length {term_length}"));
        }
        let term_length = term_length as usize;
        let mut term = try_zeroed(term_length)?;
        self.bytes.read_bytes(&mut term, 0, term_length)?;
        self.term = term;
        self.block_pos = 0;
        self.block.clear();

        // The final block of a dictionary whose last block holds one term has
        // nothing compressed after it (`TermsDict.decompressBlock`).
        let offset = self.bytes.file_pointer();
        if offset < self.data_length - 1 {
            let uncompressed_length = self.bytes.read_v_int()?;
            if uncompressed_length < 0 {
                return corrupt(format!("negative terms block length {uncompressed_length}"));
            }
            let uncompressed_length = uncompressed_length as usize;
            let mut block = try_zeroed(term_length + uncompressed_length)?;
            block[..term_length].copy_from_slice(&self.term);
            Lz4::decompress(
                &mut *self.bytes,
                uncompressed_length,
                &mut block,
                term_length,
            )?;
            self.block = block;
            self.block_pos = term_length;
        }
        Ok(())
    }

    /// Advances to the next term, exactly as `TermsDict.next` does.
    fn next(&mut self) -> Result<()> {
        self.ord = self.ord.wrapping_add(1);
        if self.ord >= self.size {
            return corrupt("the term dictionary ended before the ordinal sought".to_string());
        }
        if self.ord & TERMS_DICT_BLOCK_LZ4_MASK as i64 == 0 {
            return self.decompress_block();
        }
        let token = usize::from(self.block_byte()?);
        let mut prefix_length = token & 0x0F;
        let mut suffix_length = 1 + (token >> 4);
        if prefix_length == 15 {
            prefix_length += self.block_v_int()? as usize;
        }
        if suffix_length == 16 {
            suffix_length += self.block_v_int()? as usize;
        }
        // The prefix is shared with the term before it, so a prefix longer than
        // that term names bytes the block never wrote.
        if prefix_length > self.term.len() {
            return corrupt(format!(
                "a term shares a {prefix_length}-byte prefix with a term of {} bytes",
                self.term.len()
            ));
        }
        let available = self.block.len().saturating_sub(self.block_pos);
        if suffix_length > available {
            return corrupt(format!(
                "a term suffix of {suffix_length} bytes runs past the {available} the block holds"
            ));
        }
        self.term.truncate(prefix_length);
        self.term
            .extend_from_slice(&self.block[self.block_pos..self.block_pos + suffix_length]);
        self.block_pos += suffix_length;
        Ok(())
    }

    fn seek_exact(&mut self, ord: i64) -> Result<&[u8]> {
        if ord < 0 || ord >= self.size {
            return corrupt(format!(
                "ordinal {ord} is outside the {}-term dictionary",
                self.size
            ));
        }
        // Signed shift, because `ord` is -1 while the cursor is unpositioned.
        let current_block = self.ord >> TERMS_DICT_BLOCK_LZ4_SHIFT;
        let block = ord >> TERMS_DICT_BLOCK_LZ4_SHIFT;
        if ord < self.ord || block != current_block {
            let address = self.block_addresses.get_checked(block)?;
            if address < 0 || address > self.data_length {
                return corrupt(format!(
                    "terms block {block} starts at {address}, outside the \
                     {}-byte dictionary",
                    self.data_length
                ));
            }
            self.bytes.seek(address)?;
            self.ord = (block << TERMS_DICT_BLOCK_LZ4_SHIFT) - 1;
        }
        while self.ord < ord {
            self.next()?;
        }
        Ok(&self.term)
    }
}

/// A `SORTED` field: an ordinal per document, plus the dictionary they index.
/// Rejects an ordinals entry that a single packed block encodes with a GCD, an
/// offset or a table, none of which an ordinal stream may use.
///
/// Equivalent to the check `getSorted` and `getSortedSet` make before
/// specialising "the common case for ordinals: single block of packed
/// integers" (`Lucene90DocValuesProducer.java:1319-1324, 2064-2067`).
///
/// # Errors
///
/// Returns [`LuceneError::IllegalState`] with Java's message.
fn check_ordinals_entry(ords: &NumericEntry) -> Result<()> {
    if ords.block_shift < 0
        && ords.bits_per_value > 0
        && (ords.gcd != 1 || ords.min_value != 0 || ords.table.is_some())
    {
        return Err(LuceneError::IllegalState(
            "Ordinals shouldn't use GCD, offset or table compression".to_string(),
        ));
    }
    Ok(())
}

struct LazySortedDocValues {
    ords: LazyNumericDocValues,
    terms: RefCell<TermsDict>,
    value_count: i64,
}

impl LazySortedDocValues {
    fn new(data: &dyn IndexInput, max_doc: i32, entry: &SortedEntry) -> Result<Self> {
        check_ordinals_entry(&entry.ords)?;
        Ok(Self {
            ords: LazyNumericDocValues::new(data, max_doc, &entry.ords)?,
            terms: RefCell::new(TermsDict::new(data, &entry.terms_dict)?),
            value_count: entry.terms_dict.terms_dict_size,
        })
    }
}

impl DocIdSetIterator for LazySortedDocValues {
    fn doc_id(&self) -> i32 {
        self.ords.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.ords.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.ords.advance(target)
    }

    fn cost(&self) -> i64 {
        self.ords.cost()
    }
}

impl DocValuesIterator for LazySortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.ords.advance_exact(target)
    }
}

impl SortedDocValues for LazySortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        Ok(self.ords.long_value()? as i32)
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(self.value_count as i32)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        let term = self.terms.borrow_mut().seek_exact(i64::from(ord))?.to_vec();
        Ok(BytesRef::new(term))
    }
}

/// A `SORTED_NUMERIC` field, or the ordinals of a multi-valued `SORTED_SET`.
///
/// When `numValues == numDocsWithField` the consumer writes no address table
/// and Lucene serves the entry through `DocValues.singleton`
/// (`Lucene90DocValuesProducer.java:437-450`); that is the `addresses == None`
/// case here, where a document's single value sits at its own index.
struct LazySortedNumericDocValues {
    docs: DocsCursor,
    values: NumericValueSource,
    addresses: Option<DirectMonotonicReader>,
    max_doc: i32,
    /// The fixed number of values every document holds, or `-1`.
    ///
    /// Equivalent to the `denseFixedCardinality` local of `getSortedNumeric`
    /// (`Lucene90DocValuesProducer.java:1843`).
    dense_fixed_cardinality: i32,
    cost: i64,
    start: i64,
    end: i64,
    cursor: i64,
}

impl LazySortedNumericDocValues {
    fn new(
        data: &dyn IndexInput,
        max_doc: i32,
        entry: &SortedNumericEntry,
        dense_fixed_cardinality: i32,
    ) -> Result<Self> {
        let docs = DocsCursor::new(
            data,
            max_doc,
            entry.numeric.docs_with_field_offset,
            entry.numeric.docs_with_field_length,
            entry.numeric.jump_table_entry_count,
            entry.numeric.dense_rank_power,
            i64::from(entry.num_docs_with_field),
        )?;
        let addresses = match entry.addresses_meta {
            Some(ref meta) => {
                let slice =
                    checked_random_access(data, entry.addresses_offset, entry.addresses_length)?;
                Some(DirectMonotonicReader::with_random_access(
                    meta.clone(),
                    slice,
                )?)
            }
            None => None,
        };
        Ok(Self {
            docs,
            values: NumericValueSource::new(data, &entry.numeric)?,
            addresses,
            max_doc,
            dense_fixed_cardinality,
            cost: i64::from(entry.num_docs_with_field),
            start: 0,
            end: 0,
            cursor: 0,
        })
    }

    /// Positions the value window on the document the cursor just reached.
    fn position(&mut self) -> Result<()> {
        let index = self.docs.index();
        if index < 0 {
            self.start = 0;
            self.end = 0;
            self.cursor = 0;
            return Ok(());
        }
        match self.addresses {
            Some(ref addresses) => {
                self.start = addresses.get_checked(index)?;
                self.end = addresses.get_checked(index.wrapping_add(1))?;
            }
            None => {
                self.start = index;
                self.end = index.wrapping_add(1);
            }
        }
        if self.end < self.start {
            return corrupt(format!(
                "a document claims values [{}, {})",
                self.start, self.end
            ));
        }
        self.cursor = self.start;
        Ok(())
    }
}

impl DocIdSetIterator for LazySortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.docs.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.docs.next_doc()?;
        if doc != NO_MORE_DOCS {
            self.position()?;
        }
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.docs.advance(target)?;
        if doc != NO_MORE_DOCS {
            self.position()?;
        }
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.docs.cost(self.cost)
    }
}

impl DocValuesIterator for LazySortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let found = self.docs.advance_exact(target)?;
        if found {
            self.position()?;
        }
        Ok(found)
    }
}

impl SortedNumericDocValues for LazySortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        if self.cursor >= self.end {
            return corrupt(format!(
                "the document holds values [{}, {}) and one more was asked for",
                self.start, self.end
            ));
        }
        let value = self.values.value(self.cursor)?;
        self.cursor += 1;
        Ok(value)
    }

    fn doc_value_count(&self) -> Result<i32> {
        // Java computes `(int) (end - start)` and lets both the subtraction and
        // the narrowing wrap; a corrupt address table has to produce the value
        // Java produces, not an overflow panic.
        Ok(self.end.wrapping_sub(self.start) as i32)
    }

    /// Equivalent to the `rangeIntoBitSet` overrides of the multi-valued
    /// views `getSortedNumeric` returns (`Lucene90DocValuesProducer.java:1898-1925`
    /// dense, `1997-2028` sparse): a constant field sets the whole run, a dense
    /// field of fixed cardinality addresses its values by multiplication, and
    /// every other document is decided by its first value at or above
    /// `min_value`. A single-valued field is served by `DocValues.singleton`,
    /// which keeps `SortedNumericDocValues.rangeIntoBitSet`.
    fn range_into_bit_set(
        &mut self,
        from_doc: i32,
        to_doc: i32,
        min_value: i64,
        max_value: i64,
        bit_set: &mut crate::util::FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        let Some(addresses) = self.addresses.as_ref() else {
            // `SortedNumericDocValues.rangeIntoBitSet`.
            for doc in from_doc..to_doc {
                if self.advance_exact(doc)? {
                    let count = self.doc_value_count()?;
                    for _ in 0..count {
                        let value = self.next_value()?;
                        if value >= min_value {
                            if value <= max_value {
                                bit_set.set((doc - offset) as usize);
                            }
                            break;
                        }
                    }
                }
            }
            return Ok(());
        };
        let end_doc = to_doc.min(self.max_doc);
        if from_doc >= end_doc {
            return Ok(());
        }
        let values = &self.values;
        let read = |index: i64| values.value(index);
        match &mut self.docs {
            DocsCursor::Dense { .. } => {
                if let NumericValueSource::Constant(constant) = values {
                    if *constant >= min_value && *constant <= max_value {
                        bit_set
                            .set_range((from_doc - offset) as usize, (end_doc - offset) as usize);
                    }
                    return Ok(());
                }
                let cardinality = self.dense_fixed_cardinality;
                if cardinality > 1 {
                    return Lucene90DocValuesProducer::sorted_numeric_scalar_range_into_bit_set(
                        &read,
                        from_doc,
                        end_doc,
                        cardinality,
                        min_value,
                        max_value,
                        bit_set,
                        offset,
                    );
                }
                for current_doc in from_doc..end_doc {
                    let start_offset = addresses.get_checked(i64::from(current_doc))?;
                    let end_offset = addresses.get_checked(i64::from(current_doc) + 1)?;
                    if Lucene90DocValuesProducer::sorted_numeric_matches_range(
                        &read,
                        start_offset,
                        end_offset,
                        min_value,
                        max_value,
                    )? {
                        bit_set.set((current_doc - offset) as usize);
                    }
                }
                Ok(())
            }
            DocsCursor::Sparse { disi } => {
                let mut current_doc = disi.doc_id();
                if current_doc < from_doc {
                    current_doc = disi.advance(from_doc)?;
                }
                if current_doc < end_doc {
                    if let NumericValueSource::Constant(constant) = values {
                        if *constant >= min_value && *constant <= max_value {
                            disi.into_bit_set(end_doc, bit_set, offset)?;
                        }
                    } else {
                        while current_doc < end_doc {
                            let index = i64::from(disi.index());
                            let start_offset = addresses.get_checked(index)?;
                            let end_offset = addresses.get_checked(index + 1)?;
                            if Lucene90DocValuesProducer::sorted_numeric_matches_range(
                                &read,
                                start_offset,
                                end_offset,
                                min_value,
                                max_value,
                            )? {
                                bit_set.set((current_doc - offset) as usize);
                            }
                            current_doc = disi.next_doc()?;
                        }
                    }
                }
                // Java resets `set`, so the value window follows the iterator
                // the next time it is read; this port positions eagerly.
                if self.docs.doc_id() != NO_MORE_DOCS {
                    self.position()?;
                }
                Ok(())
            }
            DocsCursor::Empty { .. } => Ok(()),
        }
    }
}

/// A `SORTED_SET` field, in either of its two layouts.
///
/// The single-valued one is a `SORTED` entry behind a `multiValued == 0` byte,
/// which Lucene serves through `DocValues.singleton`; both are driven here by
/// the same ordinal cursor, so only the construction differs.
struct LazySortedSetDocValues {
    ords: LazySortedNumericDocValues,
    terms: RefCell<TermsDict>,
    value_count: i64,
}

impl LazySortedSetDocValues {
    fn new(data: &dyn IndexInput, max_doc: i32, entry: &SortedSetEntry) -> Result<Self> {
        // `getSortedSet` checks the single-valued entry through `getSorted`
        // and the multi-valued one itself (`Lucene90DocValuesProducer.java:2059-2067`).
        match entry {
            SortedSetEntry::Single(single) => check_ordinals_entry(&single.ords)?,
            SortedSetEntry::Multi { ords, .. } => check_ordinals_entry(&ords.numeric)?,
        }
        let (ords_entry, terms_dict) = match entry {
            SortedSetEntry::Single(single) => (
                SortedNumericEntry {
                    numeric: single.ords.clone(),
                    // No address table: one ordinal per document, at its own
                    // index, which is what `numValues == numDocsWithField`
                    // means for the shared cursor.
                    num_docs_with_field: single.ords.num_values as i32,
                    addresses_offset: 0,
                    addresses_meta: None,
                    addresses_length: 0,
                },
                &single.terms_dict,
            ),
            SortedSetEntry::Multi { ords, terms_dict } => (ords.clone(), terms_dict),
        };
        Ok(Self {
            ords: LazySortedNumericDocValues::new(data, max_doc, &ords_entry, -1)?,
            terms: RefCell::new(TermsDict::new(data, terms_dict)?),
            value_count: terms_dict.terms_dict_size,
        })
    }
}

impl DocIdSetIterator for LazySortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.ords.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.ords.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.ords.advance(target)
    }

    fn cost(&self) -> i64 {
        self.ords.cost()
    }
}

impl DocValuesIterator for LazySortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.ords.advance_exact(target)
    }
}

impl SortedSetDocValues for LazySortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        self.ords.next_value()
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.ords.doc_value_count()
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        let term = self.terms.borrow_mut().seek_exact(ord)?.to_vec();
        Ok(BytesRef::new(term))
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(self.value_count)
    }
}

// -----------------------------------------------------------------------------
// The producer's public surface
// -----------------------------------------------------------------------------

impl DocValuesProducer for Lucene90DocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> = {
            match self.entries.get(&field.number) {
                Some(FieldEntry::Numeric(entry)) => Ok(Box::new(LazyNumericDocValues::new(
                    &*self.data,
                    self.max_doc,
                    entry,
                )?)),
                _ => Ok(Box::new(EmptyNumericDocValues::new())),
            }
        };
        Ok(Some(values?))
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> = {
            match self.entries.get(&field.number) {
                // A field no document has a value for: `DocValues.emptyBinary()`,
                // before anything else of the entry is looked at
                // (`Lucene90DocValuesProducer.java:1210-1215`). Its lengths are
                // the consumer's initial `Integer.MAX_VALUE` and `0`
                // (`Lucene90DocValuesConsumer.java:616-617, 651-652`), which describe
                // no value.
                Some(FieldEntry::Binary(entry)) if entry.docs_with_field_offset == -2 => {
                    Ok(Box::new(EmptyBinaryDocValues::new()))
                }
                Some(FieldEntry::Binary(entry)) => Ok(Box::new(LazyBinaryDocValues::new(
                    &*self.data,
                    self.max_doc,
                    entry,
                )?)),
                _ => Ok(Box::new(EmptyBinaryDocValues::new())),
            }
        };
        Ok(Some(values?))
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let values: Result<Box<dyn SortedDocValues>> = {
            match self.entries.get(&field.number) {
                Some(FieldEntry::Sorted(entry)) => Ok(Box::new(LazySortedDocValues::new(
                    &*self.data,
                    self.max_doc,
                    entry,
                )?)),
                _ => Ok(Box::new(EmptySortedDocValues::new())),
            }
        };
        Ok(Some(values?))
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> = {
            match self.entries.get(&field.number) {
                Some(FieldEntry::SortedNumeric(entry)) => {
                    // `Lucene90DocValuesProducer.java:1823,1843`.
                    let dense_fixed_cardinality = Self::fixed_cardinality(
                        entry.numeric.num_values,
                        entry.num_docs_with_field,
                        self.skippers
                            .get(&field.number)
                            .map(|skipper| skipper.max_value_count),
                    );
                    Ok(Box::new(LazySortedNumericDocValues::new(
                        &*self.data,
                        self.max_doc,
                        entry,
                        dense_fixed_cardinality,
                    )?))
                }
                _ => Ok(Box::new(EmptySortedNumericDocValues::new())),
            }
        };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let values: Result<Box<dyn SortedSetDocValues>> =
            {
                match self.entries.get(&field.number) {
                    Some(FieldEntry::SortedSet(entry)) => Ok(Box::new(
                        LazySortedSetDocValues::new(&*self.data, self.max_doc, entry)?,
                    )),
                    _ => Ok(Box::new(EmptySortedSetDocValues::new())),
                }
            };
        Ok(Some(values?))
    }

    /// Equivalent to `Lucene90DocValuesProducer.getSkipper(FieldInfo)`
    /// (`Lucene90DocValuesProducer.java:2368-2470`): a cursor over the field's
    /// slice of the skip-index file (or of the data file, for a segment written
    /// before [`VERSION_SKIPPER_SEPARATE_FILE`]).
    ///
    /// # Errors
    ///
    /// A field with no skip-index entry returns [`LuceneError::IllegalState`].
    /// Java reads `skippers.get(field.number)` and dereferences the result
    /// unchecked (`Lucene90DocValuesProducer.java:2370-2373`), so it throws
    /// `NullPointerException`; its callers never reach that case, because
    /// `CodecReader.getDocValuesSkipper` returns `null` first when the field's
    /// `docValuesSkipIndexType()` is `NONE` (`index/CodecReader.java:198-206`).
    /// The port fails the same call, with an error instead of a panic.
    fn get_skipper(&self, field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        let Some(entry) = self.skippers.get(&field.number).copied() else {
            return Err(LuceneError::IllegalState(format!(
                "field \"{}\" has no doc-values skip-index entry",
                field.name
            )));
        };
        let skipper_source: &dyn IndexInput = match &self.skip_index_data {
            Some(skip_index_data) => skip_index_data.as_ref(),
            None => self.data.as_ref(),
        };
        let input = skipper_source.slice("doc value skipper", entry.offset, entry.length)?;
        Ok(Some(Box::new(Lucene90DocValuesSkipper::new(entry, input))))
    }

    fn check_integrity(&self) -> Result<()> {
        // `Lucene90DocValuesProducer.java:2285-2291`.
        let mut data = self.data.clone_input()?;
        checksum_entire_file(&mut *data)?;
        if let Some(skip_index_data) = &self.skip_index_data {
            let mut skip = skip_index_data.clone_input()?;
            checksum_entire_file(&mut *skip)?;
        }
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Self {
            data: self.data.clone_input()?,
            max_doc: self.max_doc,
            entries: self.entries.clone(),
            skippers: self.skippers.clone(),
            skip_index_data: match &self.skip_index_data {
                Some(skip_index_data) => Some(skip_index_data.clone_input()?),
                None => None,
            },
            merging: true,
        }))
    }

    /// Equivalent to `IOUtils.close(data, skipIndexData)`: both are closed,
    /// and the first error is reported.
    fn close(&mut self) -> Result<()> {
        let data = self.data.close();
        let skip = match self.skip_index_data.as_mut() {
            Some(skip_index_data) => skip_index_data.close(),
            None => Ok(()),
        };
        data.and(skip)
    }
}
#[cfg(test)]
mod tests {
    /// Round 2, item 1: `getSorted`/`getSortedSet` refuse a single-block
    /// ordinals entry with a GCD, an offset or a table
    /// (`Lucene90DocValuesProducer.java:1319-1324, 2064-2067`), and accept
    /// the other shapes untouched.
    #[test]
    fn ordinals_entries_refuse_gcd_offset_or_table() {
        let base = NumericEntry {
            docs_with_field_offset: -1,
            docs_with_field_length: 0,
            jump_table_entry_count: -1,
            dense_rank_power: -1,
            num_values: 4,
            table: None,
            block_shift: -1,
            bits_per_value: 2,
            min_value: 0,
            gcd: 1,
            values_offset: 0,
            values_length: 1,
            value_jump_table_offset: -1,
        };
        assert!(super::check_ordinals_entry(&base).is_ok());
        let refused = [
            NumericEntry {
                gcd: 2,
                ..base.clone()
            },
            NumericEntry {
                min_value: 1,
                ..base.clone()
            },
            NumericEntry {
                table: Some(vec![0, 1]),
                ..base.clone()
            },
        ];
        for entry in refused {
            match super::check_ordinals_entry(&entry) {
                Err(LuceneError::IllegalState(message)) => assert_eq!(
                    message,
                    "Ordinals shouldn't use GCD, offset or table compression"
                ),
                other => panic!("expected Java's IllegalStateException, got {other:?}"),
            }
        }
        // A constant (width 0) or per-block entry takes the general path.
        assert!(super::check_ordinals_entry(&NumericEntry {
            gcd: 0,
            bits_per_value: 0,
            ..base.clone()
        })
        .is_ok());
        assert!(super::check_ordinals_entry(&NumericEntry {
            gcd: 2,
            block_shift: 14,
            ..base
        })
        .is_ok());
    }

    use super::*;
    use crate::codecs::stub::{BufferedUpdates, SegmentInfo};
    use crate::codecs::tests::DummyCodec;
    use crate::index::FieldInfos;
    use crate::search::Sort;
    use crate::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
    use crate::util::default_info_stream;
    use crate::util::string_helper::ID_LENGTH;
    use crate::util::Version;
    use std::collections::HashMap;
    use std::sync::Arc;

    type NumericDocsValues = (Vec<i32>, Vec<i64>);
    type BinaryDocsValues = (Vec<i32>, Vec<Vec<u8>>);
    type SortedDocsValues = (Vec<i32>, Vec<i32>, Vec<Vec<u8>>);
    type SortedNumericDocsValues = (Vec<i32>, Vec<Vec<i64>>);
    type SortedSetDocsValues = (Vec<i32>, Vec<Vec<i64>>, Vec<Vec<u8>>);

    #[derive(Debug, Default, Clone)]
    struct TestDocValuesProducer {
        numeric: HashMap<i32, NumericDocsValues>,
        binary: HashMap<i32, BinaryDocsValues>,
        sorted: HashMap<i32, SortedDocsValues>,
        sorted_numeric: HashMap<i32, SortedNumericDocsValues>,
        sorted_set: HashMap<i32, SortedSetDocsValues>,
    }

    impl TestDocValuesProducer {
        fn with_numeric(field: i32, docs: Vec<i32>, values: Vec<i64>) -> Self {
            let mut this = Self::default();
            this.numeric.insert(field, (docs, values));
            this
        }

        fn with_binary(field: i32, docs: Vec<i32>, values: Vec<Vec<u8>>) -> Self {
            let mut this = Self::default();
            this.binary.insert(field, (docs, values));
            this
        }

        fn with_sorted(field: i32, docs: Vec<i32>, ords: Vec<i32>, terms: Vec<Vec<u8>>) -> Self {
            let mut this = Self::default();
            this.sorted.insert(field, (docs, ords, terms));
            this
        }

        fn with_sorted_numeric(field: i32, docs: Vec<i32>, values: Vec<Vec<i64>>) -> Self {
            let mut this = Self::default();
            this.sorted_numeric.insert(field, (docs, values));
            this
        }

        fn with_sorted_set(
            field: i32,
            docs: Vec<i32>,
            ords: Vec<Vec<i64>>,
            terms: Vec<Vec<u8>>,
        ) -> Self {
            let mut this = Self::default();
            this.sorted_set.insert(field, (docs, ords, terms));
            this
        }
    }

    // -------------------------------------------------------------------------
    // In-memory doc values, used only to feed the consumer in these tests
    // -------------------------------------------------------------------------

    /// Walks a fixed document list, exposing the ordinal of the current one.
    #[derive(Debug, Clone, Default)]
    struct DocCursor {
        docs: Vec<i32>,
        pos: usize,
        doc: i32,
    }

    impl DocCursor {
        fn new(docs: Vec<i32>) -> Self {
            Self {
                docs,
                pos: usize::MAX,
                doc: -1,
            }
        }

        fn next_doc(&mut self) -> i32 {
            let next = self.pos.wrapping_add(1);
            self.doc = if next < self.docs.len() {
                self.pos = next;
                self.docs[next]
            } else {
                NO_MORE_DOCS
            };
            self.doc
        }

        fn advance(&mut self, target: i32) -> i32 {
            let from = self.pos.wrapping_add(1).min(self.docs.len());
            let found = self.docs[from..].iter().position(|doc| *doc >= target);
            self.doc = match found {
                Some(offset) => {
                    self.pos = from + offset;
                    self.docs[self.pos]
                }
                None => {
                    self.pos = self.docs.len();
                    NO_MORE_DOCS
                }
            };
            self.doc
        }

        fn advance_exact(&mut self, target: i32) -> bool {
            match self.docs.binary_search(&target) {
                Ok(pos) => {
                    self.pos = pos;
                    self.doc = target;
                    true
                }
                Err(_) => false,
            }
        }
    }

    macro_rules! delegate_cursor {
        ($ty:ty) => {
            impl DocIdSetIterator for $ty {
                fn doc_id(&self) -> i32 {
                    self.cursor.doc
                }

                fn next_doc(&mut self) -> Result<i32> {
                    Ok(self.cursor.next_doc())
                }

                fn advance(&mut self, target: i32) -> Result<i32> {
                    Ok(self.cursor.advance(target))
                }

                fn cost(&self) -> i64 {
                    self.cursor.docs.len() as i64
                }
            }

            impl DocValuesIterator for $ty {
                fn advance_exact(&mut self, target: i32) -> Result<bool> {
                    Ok(self.cursor.advance_exact(target))
                }
            }
        };
    }

    struct VecNumeric {
        cursor: DocCursor,
        values: Vec<i64>,
    }
    delegate_cursor!(VecNumeric);
    impl NumericDocValues for VecNumeric {
        fn long_value(&self) -> Result<i64> {
            Ok(self.values[self.cursor.pos])
        }
    }

    struct VecBinary {
        cursor: DocCursor,
        values: Vec<Vec<u8>>,
    }
    delegate_cursor!(VecBinary);
    impl BinaryDocValues for VecBinary {
        fn binary_value(&self) -> Result<BytesRef> {
            Ok(BytesRef::new(self.values[self.cursor.pos].clone()))
        }
    }

    struct VecSorted {
        cursor: DocCursor,
        ords: Vec<i32>,
        terms: Vec<Vec<u8>>,
    }
    delegate_cursor!(VecSorted);
    impl SortedDocValues for VecSorted {
        fn ord_value(&self) -> Result<i32> {
            Ok(self.ords[self.cursor.pos])
        }

        fn get_value_count(&self) -> Result<i32> {
            Ok(self.terms.len() as i32)
        }

        fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
            Ok(BytesRef::new(self.terms[ord as usize].clone()))
        }
    }

    struct VecSortedNumeric {
        cursor: DocCursor,
        values: Vec<Vec<i64>>,
        index: usize,
    }
    impl DocIdSetIterator for VecSortedNumeric {
        fn doc_id(&self) -> i32 {
            self.cursor.doc
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.index = 0;
            Ok(self.cursor.next_doc())
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.index = 0;
            Ok(self.cursor.advance(target))
        }

        fn cost(&self) -> i64 {
            self.cursor.docs.len() as i64
        }
    }
    impl DocValuesIterator for VecSortedNumeric {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.index = 0;
            Ok(self.cursor.advance_exact(target))
        }
    }
    impl SortedNumericDocValues for VecSortedNumeric {
        fn next_value(&mut self) -> Result<i64> {
            let value = self.values[self.cursor.pos][self.index];
            self.index += 1;
            Ok(value)
        }

        fn doc_value_count(&self) -> Result<i32> {
            Ok(self.values[self.cursor.pos].len() as i32)
        }
    }

    struct VecSortedSet {
        cursor: DocCursor,
        ords: Vec<Vec<i64>>,
        terms: Vec<Vec<u8>>,
        index: usize,
    }
    impl DocIdSetIterator for VecSortedSet {
        fn doc_id(&self) -> i32 {
            self.cursor.doc
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.index = 0;
            Ok(self.cursor.next_doc())
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.index = 0;
            Ok(self.cursor.advance(target))
        }

        fn cost(&self) -> i64 {
            self.cursor.docs.len() as i64
        }
    }
    impl DocValuesIterator for VecSortedSet {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.index = 0;
            Ok(self.cursor.advance_exact(target))
        }
    }
    impl SortedSetDocValues for VecSortedSet {
        fn next_ord(&mut self) -> Result<i64> {
            let ord = self.ords[self.cursor.pos][self.index];
            self.index += 1;
            Ok(ord)
        }

        fn doc_value_count(&self) -> Result<i32> {
            Ok(self.ords[self.cursor.pos].len() as i32)
        }

        fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
            Ok(BytesRef::new(self.terms[ord as usize].clone()))
        }

        fn get_value_count(&self) -> Result<i64> {
            Ok(self.terms.len() as i64)
        }
    }

    impl DocValuesProducer for TestDocValuesProducer {
        fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
            let values: Result<Box<dyn NumericDocValues>> = {
                match self.numeric.get(&field.number) {
                    Some((docs, values)) => Ok(Box::new(VecNumeric {
                        cursor: DocCursor::new(docs.clone()),
                        values: values.clone(),
                    })),
                    None => Ok(Box::new(EmptyNumericDocValues::new())),
                }
            };
            Ok(Some(values?))
        }

        fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
            let values: Result<Box<dyn BinaryDocValues>> = {
                match self.binary.get(&field.number) {
                    Some((docs, values)) => Ok(Box::new(VecBinary {
                        cursor: DocCursor::new(docs.clone()),
                        values: values.clone(),
                    })),
                    None => Ok(Box::new(EmptyBinaryDocValues::new())),
                }
            };
            Ok(Some(values?))
        }

        fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
            let values: Result<Box<dyn SortedDocValues>> = {
                match self.sorted.get(&field.number) {
                    Some((docs, ords, terms)) => Ok(Box::new(VecSorted {
                        cursor: DocCursor::new(docs.clone()),
                        ords: ords.clone(),
                        terms: terms.clone(),
                    })),
                    None => Ok(Box::new(EmptySortedDocValues::new())),
                }
            };
            Ok(Some(values?))
        }

        fn get_sorted_numeric(
            &self,
            field: &FieldInfo,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            let values: Result<Box<dyn SortedNumericDocValues>> = {
                match self.sorted_numeric.get(&field.number) {
                    Some((docs, values)) => Ok(Box::new(VecSortedNumeric {
                        cursor: DocCursor::new(docs.clone()),
                        values: values.clone(),
                        index: 0,
                    })),
                    None => Ok(Box::new(EmptySortedNumericDocValues::new())),
                }
            };
            Ok(Some(values?))
        }

        fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            let values: Result<Box<dyn SortedSetDocValues>> = {
                match self.sorted_set.get(&field.number) {
                    Some((docs, ords, terms)) => Ok(Box::new(VecSortedSet {
                        cursor: DocCursor::new(docs.clone()),
                        ords: ords.clone(),
                        terms: terms.clone(),
                        index: 0,
                    })),
                    None => Ok(Box::new(EmptySortedSetDocValues::new())),
                }
            };
            Ok(Some(values?))
        }

        fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            Err(LuceneError::UnsupportedOperation(
                "the test producer does not provide a doc-values skipper".to_string(),
            ))
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
            Ok(Box::new(self.clone()))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn make_segment_info(dir: Arc<dyn Directory>, name: &str, max_doc: i32) -> SegmentInfo {
        SegmentInfo::new(
            dir,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            false,
            false,
            Arc::new(DummyCodec::new("Dummy")),
            HashMap::new(),
            [0u8; ID_LENGTH],
            HashMap::new(),
            Sort::default(),
        )
        .unwrap()
    }

    fn write_only<F>(
        max_doc: i32,
        field: &FieldInfo,
        write_field: F,
    ) -> (Arc<dyn Directory>, SegmentInfo, FieldInfos)
    where
        F: FnOnce(&mut dyn DocValuesConsumer),
    {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();
        let segment_info = make_segment_info(Arc::clone(&dir), "_0", max_doc);
        let seg_updates = BufferedUpdates::default();
        let write_state = SegmentWriteState::new(
            default_info_stream(),
            Arc::clone(&dir),
            &segment_info,
            &field_infos,
            &seg_updates,
            &*DEFAULT_IO_CONTEXT,
        );
        let format = Lucene90DocValuesFormat::new();
        {
            let mut consumer = format.fields_consumer(&write_state).unwrap();
            write_field(consumer.as_mut());
            consumer.close().unwrap();
        }
        (dir, segment_info, field_infos)
    }

    /// Round 2, item 5: the consumer opens its three outputs in the segment's
    /// directory when it is created and writes through them, as Java's
    /// constructor does (`Lucene90DocValuesConsumer.java:73-125`), instead of
    /// buffering the files until `close`.
    #[test]
    fn the_consumer_creates_its_files_when_it_opens() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = FieldInfos::default();
        let segment_info = make_segment_info(Arc::clone(&dir), "_0", 1);
        let write_state = SegmentWriteState::new(
            default_info_stream(),
            Arc::clone(&dir),
            &segment_info,
            &field_infos,
            Box::leak(Box::new(BufferedUpdates::default())),
            &*DEFAULT_IO_CONTEXT,
        );
        let mut consumer = Lucene90DocValuesFormat::new()
            .fields_consumer(&write_state)
            .unwrap();
        let mut files = dir.list_all().unwrap();
        files.sort();
        assert_eq!(files, vec!["_0.dvd", "_0.dvm", "_0.dvs"]);
        // Still being written: listed, but empty, as `ByteBuffersDirectory`
        // reports it.
        assert_eq!(dir.file_length("_0.dvm").unwrap(), 0);
        consumer.close().unwrap();
        // Closing twice is a no-op, as Java's nulled outputs make it.
        consumer.close().unwrap();
        let meta = dir.file_length("_0.dvm").unwrap();
        assert!(meta > 0);
    }

    fn write_and_read<F>(
        max_doc: i32,
        field: &FieldInfo,
        write_field: F,
    ) -> Lucene90DocValuesProducer
    where
        F: FnOnce(&mut dyn DocValuesConsumer),
    {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();
        let segment_info = make_segment_info(Arc::clone(&dir), "_0", max_doc);
        let seg_updates = BufferedUpdates::default();
        let write_state = SegmentWriteState::new(
            default_info_stream(),
            Arc::clone(&dir),
            &segment_info,
            &field_infos,
            &seg_updates,
            &*DEFAULT_IO_CONTEXT,
        );
        let format = Lucene90DocValuesFormat::new();
        {
            let mut consumer = format.fields_consumer(&write_state).unwrap();
            write_field(consumer.as_mut());
            consumer.close().unwrap();
        }
        let read_state = SegmentReadState::new(
            dir.as_ref(),
            &segment_info,
            &field_infos,
            &*DEFAULT_IO_CONTEXT,
        );
        Lucene90DocValuesProducer::new(&read_state).unwrap()
    }

    #[test]
    fn format_name_is_lucene90() {
        assert_eq!(Lucene90DocValuesFormat::new().name(), "Lucene90");
    }

    #[test]
    fn round_trip_numeric_sparse() {
        let field = FieldInfo::new("num", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_numeric(0, vec![0, 2, 4], vec![1, 3, 5]);
            consumer.add_numeric_field(&field, &values).unwrap();
        });
        let mut numeric = producer.get_numeric(&field).unwrap().unwrap();
        assert_eq!(numeric.next_doc().unwrap(), 0);
        assert_eq!(numeric.long_value().unwrap(), 1);
        assert_eq!(numeric.next_doc().unwrap(), 2);
        assert_eq!(numeric.long_value().unwrap(), 3);
        assert_eq!(numeric.next_doc().unwrap(), 4);
        assert_eq!(numeric.long_value().unwrap(), 5);
        assert_eq!(numeric.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_numeric_table_compression() {
        let field = FieldInfo::new("num", 0);
        let producer = write_and_read(4, &field, |consumer| {
            let values =
                TestDocValuesProducer::with_numeric(0, vec![0, 1, 2, 3], vec![0, 1, 2, 100]);
            consumer.add_numeric_field(&field, &values).unwrap();
        });
        let mut numeric = producer.get_numeric(&field).unwrap().unwrap();
        assert_eq!(numeric.next_doc().unwrap(), 0);
        assert_eq!(numeric.long_value().unwrap(), 0);
        assert_eq!(numeric.next_doc().unwrap(), 1);
        assert_eq!(numeric.long_value().unwrap(), 1);
        assert_eq!(numeric.next_doc().unwrap(), 2);
        assert_eq!(numeric.long_value().unwrap(), 2);
        assert_eq!(numeric.next_doc().unwrap(), 3);
        assert_eq!(numeric.long_value().unwrap(), 100);
        assert_eq!(numeric.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_binary_fixed() {
        let field = FieldInfo::new("bin", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_binary(
                0,
                vec![0, 2, 4],
                vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            );
            consumer.add_binary_field(&field, &values).unwrap();
        });
        let mut binary = producer.get_binary(&field).unwrap().unwrap();
        assert_eq!(binary.next_doc().unwrap(), 0);
        assert_eq!(binary.binary_value().unwrap().bytes, b"a");
        assert_eq!(binary.next_doc().unwrap(), 2);
        assert_eq!(binary.binary_value().unwrap().bytes, b"b");
        assert_eq!(binary.next_doc().unwrap(), 4);
        assert_eq!(binary.binary_value().unwrap().bytes, b"c");
        assert_eq!(binary.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_binary_variable() {
        let field = FieldInfo::new("bin", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_binary(
                0,
                vec![0, 2, 4],
                vec![b"alpha".to_vec(), b"beta".to_vec(), b"gamma".to_vec()],
            );
            consumer.add_binary_field(&field, &values).unwrap();
        });
        let mut binary = producer.get_binary(&field).unwrap().unwrap();
        assert_eq!(binary.next_doc().unwrap(), 0);
        assert_eq!(binary.binary_value().unwrap().bytes, b"alpha");
        assert_eq!(binary.next_doc().unwrap(), 2);
        assert_eq!(binary.binary_value().unwrap().bytes, b"beta");
        assert_eq!(binary.next_doc().unwrap(), 4);
        assert_eq!(binary.binary_value().unwrap().bytes, b"gamma");
        assert_eq!(binary.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_sorted() {
        let field = FieldInfo::new("sorted", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_sorted(
                0,
                vec![0, 1, 2, 4],
                vec![2, 0, 1, 2],
                vec![b"bar".to_vec(), b"baz".to_vec(), b"foo".to_vec()],
            );
            consumer.add_sorted_field(&field, &values).unwrap();
        });
        let mut sorted = producer.get_sorted(&field).unwrap().unwrap();
        assert_eq!(sorted.next_doc().unwrap(), 0);
        assert_eq!(sorted.ord_value().unwrap(), 2);
        assert_eq!(sorted.next_doc().unwrap(), 1);
        assert_eq!(sorted.ord_value().unwrap(), 0);
        assert_eq!(sorted.next_doc().unwrap(), 2);
        assert_eq!(sorted.ord_value().unwrap(), 1);
        assert_eq!(sorted.next_doc().unwrap(), 4);
        assert_eq!(sorted.ord_value().unwrap(), 2);
        assert_eq!(sorted.next_doc().unwrap(), NO_MORE_DOCS);

        assert_eq!(sorted.get_value_count().unwrap(), 3);
        assert_eq!(sorted.lookup_ord(0).unwrap().bytes, b"bar");
        assert_eq!(sorted.lookup_ord(1).unwrap().bytes, b"baz");
        assert_eq!(sorted.lookup_ord(2).unwrap().bytes, b"foo");
    }

    #[test]
    fn round_trip_sorted_numeric() {
        let field = FieldInfo::new("sorted_num", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_sorted_numeric(
                0,
                vec![0, 1, 3],
                vec![vec![1, 2], vec![3], vec![4, 5, 6]],
            );
            consumer.add_sorted_numeric_field(&field, &values).unwrap();
        });
        let mut sorted_numeric = producer.get_sorted_numeric(&field).unwrap().unwrap();
        assert_eq!(sorted_numeric.next_doc().unwrap(), 0);
        assert_eq!(sorted_numeric.doc_value_count().unwrap(), 2);
        assert_eq!(sorted_numeric.next_value().unwrap(), 1);
        assert_eq!(sorted_numeric.next_value().unwrap(), 2);
        assert_eq!(sorted_numeric.next_doc().unwrap(), 1);
        assert_eq!(sorted_numeric.doc_value_count().unwrap(), 1);
        assert_eq!(sorted_numeric.next_value().unwrap(), 3);
        assert_eq!(sorted_numeric.next_doc().unwrap(), 3);
        assert_eq!(sorted_numeric.doc_value_count().unwrap(), 3);
        assert_eq!(sorted_numeric.next_value().unwrap(), 4);
        assert_eq!(sorted_numeric.next_value().unwrap(), 5);
        assert_eq!(sorted_numeric.next_value().unwrap(), 6);
        assert_eq!(sorted_numeric.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_sorted_set() {
        let field = FieldInfo::new("sorted_set", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_sorted_set(
                0,
                vec![0, 2],
                vec![vec![0, 2], vec![1]],
                vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            );
            consumer.add_sorted_set_field(&field, &values).unwrap();
        });
        let mut sorted_set = producer.get_sorted_set(&field).unwrap().unwrap();
        assert_eq!(sorted_set.next_doc().unwrap(), 0);
        assert_eq!(sorted_set.doc_value_count().unwrap(), 2);
        assert_eq!(sorted_set.next_ord().unwrap(), 0);
        assert_eq!(sorted_set.next_ord().unwrap(), 2);
        assert_eq!(sorted_set.next_doc().unwrap(), 2);
        assert_eq!(sorted_set.doc_value_count().unwrap(), 1);
        assert_eq!(sorted_set.next_ord().unwrap(), 1);
        assert_eq!(sorted_set.next_doc().unwrap(), NO_MORE_DOCS);

        assert_eq!(sorted_set.get_value_count().unwrap(), 3);
        assert_eq!(sorted_set.lookup_ord(0).unwrap().bytes, b"a");
        assert_eq!(sorted_set.lookup_ord(1).unwrap().bytes, b"b");
        assert_eq!(sorted_set.lookup_ord(2).unwrap().bytes, b"c");
    }

    fn capture_doc_values_bytes(
        max_doc: i32,
        field: &FieldInfo,
        write_field: &dyn Fn(&mut dyn DocValuesConsumer),
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let (dir, _segment_info, _field_infos) = write_only(max_doc, field, |consumer| {
            write_field(consumer);
        });
        let mut dvd = vec![];
        let mut dvm = vec![];
        let mut dvs = vec![];
        {
            let mut input = dir.open_input("_0.dvd", &*DEFAULT_IO_CONTEXT).unwrap();
            let len = input.length() as usize;
            dvd.resize(len, 0);
            input.read_bytes(&mut dvd, 0, len).unwrap();
        }
        {
            let mut input = dir.open_input("_0.dvm", &*DEFAULT_IO_CONTEXT).unwrap();
            let len = input.length() as usize;
            dvm.resize(len, 0);
            input.read_bytes(&mut dvm, 0, len).unwrap();
        }
        {
            let mut input = dir.open_input("_0.dvs", &*DEFAULT_IO_CONTEXT).unwrap();
            let len = input.length() as usize;
            dvs.resize(len, 0);
            input.read_bytes(&mut dvs, 0, len).unwrap();
        }
        (dvd, dvm, dvs)
    }

    #[test]
    fn deterministic_bytes_numeric() {
        let field = FieldInfo::new("num", 0);
        let write = |consumer: &mut dyn DocValuesConsumer| {
            let values = TestDocValuesProducer::with_numeric(0, vec![0, 2, 4], vec![1, 3, 5]);
            consumer.add_numeric_field(&field, &values).unwrap();
        };
        let (dvd1, dvm1, dvs1) = capture_doc_values_bytes(5, &field, &write);
        let (dvd2, dvm2, dvs2) = capture_doc_values_bytes(5, &field, &write);
        assert_eq!(dvd1, dvd2);
        assert_eq!(dvm1, dvm2);
        assert_eq!(dvs1, dvs2);
    }

    #[test]
    fn deterministic_bytes_sorted_set() {
        let field = FieldInfo::new("sorted_set", 0);
        let write = |consumer: &mut dyn DocValuesConsumer| {
            let values = TestDocValuesProducer::with_sorted_set(
                0,
                vec![0, 2],
                vec![vec![0, 2], vec![1]],
                vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            );
            consumer.add_sorted_set_field(&field, &values).unwrap();
        };
        let (dvd1, dvm1, dvs1) = capture_doc_values_bytes(5, &field, &write);
        let (dvd2, dvm2, dvs2) = capture_doc_values_bytes(5, &field, &write);
        assert_eq!(dvd1, dvd2);
        assert_eq!(dvm1, dvm2);
        assert_eq!(dvs1, dvs2);
    }

    #[test]
    fn deterministic_bytes_binary_variable() {
        let field = FieldInfo::new("bin", 0);
        let write = |consumer: &mut dyn DocValuesConsumer| {
            let values = TestDocValuesProducer::with_binary(
                0,
                vec![0, 2, 4],
                vec![b"alpha".to_vec(), b"beta".to_vec(), b"gamma".to_vec()],
            );
            consumer.add_binary_field(&field, &values).unwrap();
        };
        let (dvd1, dvm1, dvs1) = capture_doc_values_bytes(5, &field, &write);
        let (dvd2, dvm2, dvs2) = capture_doc_values_bytes(5, &field, &write);
        assert_eq!(dvd1, dvd2);
        assert_eq!(dvm1, dvm2);
        assert_eq!(dvs1, dvs2);
    }

    #[test]
    fn deterministic_bytes_sorted_numeric() {
        let field = FieldInfo::new("sorted_num", 0);
        let write = |consumer: &mut dyn DocValuesConsumer| {
            let values = TestDocValuesProducer::with_sorted_numeric(
                0,
                vec![0, 1, 3],
                vec![vec![1, 2], vec![3], vec![4, 5, 6]],
            );
            consumer.add_sorted_numeric_field(&field, &values).unwrap();
        };
        let (dvd1, dvm1, dvs1) = capture_doc_values_bytes(5, &field, &write);
        let (dvd2, dvm2, dvs2) = capture_doc_values_bytes(5, &field, &write);
        assert_eq!(dvd1, dvd2);
        assert_eq!(dvm1, dvm2);
        assert_eq!(dvs1, dvs2);
    }

    /// A `.dvs` interval whose level byte is outside `[0, 4]` fails as Java
    /// fails (`Lucene90DocValuesProducer.java:2402-2420`): above the maximum,
    /// `maxDocID[levels - 1]` is out of bounds; below zero, the level
    /// adjustment's `maxDocID[levels]` is. The port reports a corrupt index.
    #[test]
    fn skipper_refuses_out_of_range_level_byte() {
        for level in [5u8, 0x7f, 0xff, 0x80] {
            let dir = RamDirectory::new();
            {
                let mut out = dir
                    .create_output("crafted.dvs", &*DEFAULT_IO_CONTEXT)
                    .unwrap();
                out.write_byte(level).unwrap();
                // Enough bytes for any interval the level could claim.
                for _ in 0..(4 * SKIP_INDEX_INTERVAL_BYTES) {
                    out.write_byte(0).unwrap();
                }
                out.close().unwrap();
            }
            let input = dir.open_input("crafted.dvs", &*DEFAULT_IO_CONTEXT).unwrap();
            let length = input.length();
            let entry = DocValuesSkipperEntry {
                offset: 0,
                length,
                min_value: 0,
                max_value: 0,
                doc_count: 1,
                max_doc_id: 10,
                max_value_count: 1,
            };
            let mut skipper = Lucene90DocValuesSkipper::new(entry, input);
            match skipper.advance(0) {
                Err(LuceneError::CorruptIndex(message)) => assert!(
                    message.contains(&format!("level out of range [{}]", level as i8)),
                    "{message}"
                ),
                other => panic!("level byte {level:#x}: expected a corrupt index, got {other:?}"),
            }
        }
    }
}

/// Presents a numeric field as the single-valued sorted-numeric field the skip
/// index is built from.
///
/// Equivalent to the anonymous `EmptyDocValuesProducer` in
/// `Lucene90DocValuesConsumer.addNumericField`, whose `getSortedNumeric`
/// answers `DocValues.singleton(valuesProducer.getNumeric(field))`.
#[derive(Debug)]
struct SingletonNumericProducer<'a> {
    inner: &'a dyn DocValuesProducer,
    field: FieldInfo,
}

impl DocValuesProducer for SingletonNumericProducer<'_> {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.inner.get_numeric(field)
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> =
            { corrupt("a singleton numeric producer has no binary values".to_string()) };
        Ok(Some(values?))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let values: Result<Box<dyn SortedDocValues>> =
            { corrupt("a singleton numeric producer has no sorted values".to_string()) };
        Ok(Some(values?))
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> = {
            Ok(Box::new(crate::index::DocValues::singleton_numeric(
                crate::codecs::doc_values::non_null(self.inner.get_numeric(field)?, field)?,
            )))
        };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let values: Result<Box<dyn SortedSetDocValues>> =
            { corrupt("a singleton numeric producer has no sorted set values".to_string()) };
        Ok(Some(values?))
    }

    /// Inherited from `EmptyDocValuesProducer.getSkipper`
    /// (`index/EmptyDocValuesProducer.java:54-57`): unsupported.
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(LuceneError::UnsupportedOperation(
            "a singleton numeric producer does not provide a doc-values skipper".to_string(),
        ))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        corrupt("a singleton numeric producer has no merge instance".to_string())
    }

    fn close(&mut self) -> Result<()> {
        let _ = &self.field;
        Ok(())
    }
}

/// Presents a sorted-set field's ordinals as the sorted-numeric values the skip
/// index is built from.
///
/// Equivalent to the anonymous `EmptyDocValuesProducer` in
/// `Lucene90DocValuesConsumer.addSortedSetField`, whose `getSortedNumeric`
/// serves the ordinals of each document. A skip index over a dictionary field
/// bounds ordinals, not terms — which is what makes it comparable at all.
#[derive(Debug)]
struct OrdinalsAsSortedNumericProducer<'a> {
    inner: &'a dyn DocValuesProducer,
}

impl DocValuesProducer for OrdinalsAsSortedNumericProducer<'_> {
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> =
            { corrupt("an ordinals producer has no numeric values".to_string()) };
        Ok(Some(values?))
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> =
            { corrupt("an ordinals producer has no binary values".to_string()) };
        Ok(Some(values?))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let values: Result<Box<dyn SortedDocValues>> =
            { corrupt("an ordinals producer has no sorted values".to_string()) };
        Ok(Some(values?))
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> = {
            Ok(Box::new(SortedSetOrdinals {
                values: crate::codecs::doc_values::non_null(
                    self.inner.get_sorted_set(field)?,
                    field,
                )?,
            }))
        };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.inner.get_sorted_set(field)
    }

    /// Inherited from `EmptyDocValuesProducer.getSkipper`
    /// (`index/EmptyDocValuesProducer.java:54-57`): unsupported.
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(LuceneError::UnsupportedOperation(
            "an ordinals producer does not provide a doc-values skipper".to_string(),
        ))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        corrupt("an ordinals producer has no merge instance".to_string())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Serves a sorted field's ordinals as single-valued sorted-numeric values.
///
/// Equivalent to the anonymous `EmptyDocValuesProducer` in
/// `Lucene90DocValuesConsumer.doAddSortedField`, whose `getSortedNumeric`
/// returns `DocValues.singleton` over a `NumericDocValues` view of the field's
/// `SortedDocValues.ordValue()` (`Lucene90DocValuesConsumer.java:684-733`).
#[derive(Debug)]
struct SortedOrdinalsProducer<'a> {
    inner: &'a dyn DocValuesProducer,
}

impl DocValuesProducer for SortedOrdinalsProducer<'_> {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> = {
            Ok(Box::new(SortedOrdinals {
                values: crate::codecs::doc_values::non_null(self.inner.get_sorted(field)?, field)?,
            }))
        };
        Ok(Some(values?))
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> =
            { corrupt("a sorted-ordinals producer has no binary values".to_string()) };
        Ok(Some(values?))
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.inner.get_sorted(field)
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> = {
            Ok(Box::new(
                crate::index::doc_values::DocValues::singleton_numeric(Box::new(SortedOrdinals {
                    values: crate::codecs::doc_values::non_null(
                        self.inner.get_sorted(field)?,
                        field,
                    )?,
                })),
            ))
        };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let values: Result<Box<dyn SortedSetDocValues>> =
            { corrupt("a sorted-ordinals producer has no sorted-set values".to_string()) };
        Ok(Some(values?))
    }

    /// Inherited from `EmptyDocValuesProducer.getSkipper`
    /// (`index/EmptyDocValuesProducer.java:54-57`): unsupported.
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(LuceneError::UnsupportedOperation(
            "a sorted-ordinals producer does not provide a doc-values skipper".to_string(),
        ))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        corrupt("a sorted-ordinals producer has no merge instance".to_string())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A sorted field's ordinals, seen as numeric values.
///
/// Equivalent to the anonymous `NumericDocValues` that
/// `Lucene90DocValuesConsumer.doAddSortedField` builds over the field's
/// `SortedDocValues` (`Lucene90DocValuesConsumer.java:687-731`).
struct SortedOrdinals {
    values: Box<dyn SortedDocValues>,
}

impl DocIdSetIterator for SortedOrdinals {
    fn doc_id(&self) -> i32 {
        self.values.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.values.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.values.advance(target)
    }

    fn cost(&self) -> i64 {
        self.values.cost()
    }
}

impl DocValuesIterator for SortedOrdinals {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.values.advance_exact(target)
    }
}

impl NumericDocValues for SortedOrdinals {
    fn long_value(&self) -> Result<i64> {
        Ok(i64::from(self.values.ord_value()?))
    }
}

/// Serves the minimum ordinal of every document of a sorted-set field as a
/// sorted field.
///
/// Equivalent to the anonymous `EmptyDocValuesProducer` in
/// `Lucene90DocValuesConsumer.addSortedSetField`, whose `getSorted` returns
/// `SortedSetSelector.wrap(valuesProducer.getSortedSet(field),
/// SortedSetSelector.Type.MIN)` (`Lucene90DocValuesConsumer.java:964-973`).
#[derive(Debug)]
struct MinOrdinalOfSortedSetProducer<'a> {
    inner: &'a dyn DocValuesProducer,
}

impl DocValuesProducer for MinOrdinalOfSortedSetProducer<'_> {
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> =
            { corrupt("a minimum-ordinal producer has no numeric values".to_string()) };
        Ok(Some(values?))
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> =
            { corrupt("a minimum-ordinal producer has no binary values".to_string()) };
        Ok(Some(values?))
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let values: Result<Box<dyn SortedDocValues>> = {
            SortedSetSelector::wrap(
                crate::codecs::doc_values::non_null(self.inner.get_sorted_set(field)?, field)?,
                SortedSetSelectorType::MIN,
            )
        };
        Ok(Some(values?))
    }

    fn get_sorted_numeric(
        &self,
        _field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> =
            { corrupt("a minimum-ordinal producer has no sorted-numeric values".to_string()) };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.inner.get_sorted_set(field)
    }

    /// Inherited from `EmptyDocValuesProducer.getSkipper`
    /// (`index/EmptyDocValuesProducer.java:54-57`): unsupported.
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(LuceneError::UnsupportedOperation(
            "a minimum-ordinal producer does not provide a doc-values skipper".to_string(),
        ))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        corrupt("a minimum-ordinal producer has no merge instance".to_string())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A sorted-set field's ordinals, seen as sorted-numeric values.
struct SortedSetOrdinals {
    values: Box<dyn SortedSetDocValues>,
}

impl DocIdSetIterator for SortedSetOrdinals {
    fn doc_id(&self) -> i32 {
        self.values.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.values.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.values.advance(target)
    }

    fn cost(&self) -> i64 {
        self.values.cost()
    }
}

impl DocValuesIterator for SortedSetOrdinals {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.values.advance_exact(target)
    }
}

impl SortedNumericDocValues for SortedSetOrdinals {
    fn next_value(&mut self) -> Result<i64> {
        self.values.next_ord()
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.values.doc_value_count()
    }
}
