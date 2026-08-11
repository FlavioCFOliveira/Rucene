//! Lucene 9.0 score-normalization format.
//!
//! Equivalent to `org.apache.lucene.codecs.lucene90.Lucene90NormsFormat`.
//!
//! This codec writes two files per segment:
//!
//! * `.nvd` — norms data (docs-with-field bit set + packed norm values);
//! * `.nvm` — norms metadata (per-field offsets, byte widths and counts).
//!
//! # File layout
//!
//! The `.nvm` file starts with an index header and ends with a codec footer.
//! For each norms field it stores, in order:
//!
//! * `field_number` (`i32`) — `-1` marks the end of metadata;
//! * `docs_with_field_offset` (`i64`) — `-2` if no docs have a value, `-1` if
//!   every doc has a value, otherwise the offset in `.nvd` of the bit set;
//! * `docs_with_field_length` (`i64`) — byte length of the bit set in `.nvd`
//!   (`0` for empty/dense);
//! * `jump_table_entry_count` (`i16`) — `-1` because this implementation uses a
//!   simple fixed bit set rather than `IndexedDISI`;
//! * `dense_rank_power` (`i8`) — `-1` for the same reason;
//! * `num_docs_with_field` (`i32`);
//! * `bytes_per_norm` (`i8`) — `0`, `1`, `2`, `4` or `8`;
//! * `norms_offset` (`i64`) — for `bytes_per_norm == 0` this is the singleton
//!   value, otherwise the offset in `.nvd` of the packed values.
//!
//! The `.nvd` file starts with an index header and ends with a codec footer.
//! For each sparse field it stores the docs-with-field bit set as a sequence of
//! little-endian `i64` words covering `maxDoc` bits, followed by the packed norm
//! values for only the documents that have a value.
//!
//! # Compatibility note
//!
//! The bit-set serialization used here is a plain `FixedBitSet` dump, not the
//! `IndexedDISI` format employed by the Java reference. This keeps the
//! implementation self-contained while still supporting correct round-trips
//! and the full metadata envelope described above. A future iteration can
//! replace the bit-set reader/writer with `IndexedDISI` to achieve byte-for-byte
//! index-file compatibility with Apache Lucene Core 10.5.0.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::fmt;

use crate::codecs::codec_util;
use crate::codecs::norms::{NormsConsumer, NormsFormat, NormsProducer};
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::doc_values::{DocValues, DocValuesIterator, NumericDocValues};
use crate::index::index_file_names::{segment_file_name, NORMS_EXTENSION, NORMS_META_EXTENSION};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{IndexInput, IndexOutput, RandomAccessInput};
use crate::util::FixedBitSet;

const DATA_CODEC: &str = "Lucene90NormsData";
const METADATA_CODEC: &str = "Lucene90NormsMetadata";
const VERSION_START: i32 = 0;
const VERSION_CURRENT: i32 = VERSION_START;

/// Sentinel written as the metadata `jump_table_entry_count` to indicate that
/// the field uses the simple fixed-bit-set representation.
const NO_JUMP_TABLE: i16 = -1;

/// Sentinel written as the metadata `dense_rank_power` for the same reason.
const NO_DENSE_RANK_POWER: i8 = -1;

// -----------------------------------------------------------------------------
// Format
// -----------------------------------------------------------------------------

/// Lucene 9.0 score-normalization format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.Lucene90NormsFormat`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Lucene90NormsFormat;

impl Lucene90NormsFormat {
    /// Creates a new norms format instance.
    pub fn new() -> Self {
        Self
    }
}

impl NormsFormat for Lucene90NormsFormat {
    fn name(&self) -> &str {
        "Lucene90Norms"
    }

    fn norms_consumer(&self, state: &SegmentWriteState) -> Result<Box<dyn NormsConsumer>> {
        Ok(Box::new(Lucene90NormsConsumer::new(state)?))
    }

    fn norms_producer(&self, state: &SegmentReadState) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(Lucene90NormsProducer::new(state)?))
    }
}

// -----------------------------------------------------------------------------
// Consumer
// -----------------------------------------------------------------------------

/// Writer for [`Lucene90NormsFormat`].
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.Lucene90NormsConsumer`.
pub struct Lucene90NormsConsumer {
    data: Box<dyn IndexOutput>,
    meta: Box<dyn IndexOutput>,
    max_doc: i32,
}

impl fmt::Debug for Lucene90NormsConsumer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90NormsConsumer")
            .field("max_doc", &self.max_doc)
            .field("data_fp", &self.data.file_pointer())
            .field("meta_fp", &self.meta.file_pointer())
            .finish_non_exhaustive()
    }
}

impl Lucene90NormsConsumer {
    fn new(state: &SegmentWriteState) -> Result<Self> {
        let data_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            NORMS_EXTENSION,
        );
        let mut data = state.directory.create_output(&data_name, state.context)?;
        codec_util::write_index_header(
            data.as_mut(),
            DATA_CODEC,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let meta_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            NORMS_META_EXTENSION,
        );
        let mut meta = state.directory.create_output(&meta_name, state.context)?;
        codec_util::write_index_header(
            meta.as_mut(),
            METADATA_CODEC,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let max_doc = state.segment_info.max_doc()?;
        Ok(Self {
            data,
            meta,
            max_doc,
        })
    }
}

impl NormsConsumer for Lucene90NormsConsumer {
    fn add_norms_field(&mut self, field: &FieldInfo, values: &dyn NormsProducer) -> Result<()> {
        // First pass: count docs with a value and determine value range.
        let mut num_docs_with_value = 0i32;
        let mut min = i64::MAX;
        let mut max = i64::MIN;
        {
            let mut it = values.get_norms(field)?;
            loop {
                let doc = it.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                num_docs_with_value += 1;
                let v = it.long_value()?;
                if v < min {
                    min = v;
                }
                if v > max {
                    max = v;
                }
            }
        }
        debug_assert!(num_docs_with_value <= self.max_doc);

        self.meta.write_int(field.number)?;

        if num_docs_with_value == 0 {
            self.meta.write_long(-2)?; // docsWithFieldOffset
            self.meta.write_long(0)?; // docsWithFieldLength
            self.meta.write_short(NO_JUMP_TABLE)?;
            self.meta.write_byte(NO_DENSE_RANK_POWER as u8)?;
        } else if num_docs_with_value == self.max_doc {
            self.meta.write_long(-1)?; // docsWithFieldOffset
            self.meta.write_long(0)?; // docsWithFieldLength
            self.meta.write_short(NO_JUMP_TABLE)?;
            self.meta.write_byte(NO_DENSE_RANK_POWER as u8)?;
        } else {
            let offset = self.data.file_pointer();
            self.meta.write_long(offset)?;
            let mut it = values.get_norms(field)?;
            write_fixed_bit_set(self.max_doc as usize, &mut it, self.data.as_mut())?;
            let length = self.data.file_pointer() - offset;
            self.meta.write_long(length)?;
            self.meta.write_short(NO_JUMP_TABLE)?;
            self.meta.write_byte(NO_DENSE_RANK_POWER as u8)?;
        }

        self.meta.write_int(num_docs_with_value)?;
        let bytes_per_norm = num_bytes_per_value(min, max);
        self.meta.write_byte(bytes_per_norm as u8)?;

        if bytes_per_norm == 0 {
            // Empty fields store the (only possible) singleton value here.
            self.meta.write_long(min)?;
        } else {
            let norms_offset = self.data.file_pointer();
            self.meta.write_long(norms_offset)?;
            let mut it = values.get_norms(field)?;
            write_norm_values(&mut it, bytes_per_norm, self.data.as_mut())?;
        }

        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.meta.write_int(-1)?; // EOF marker
        codec_util::write_footer(self.meta.as_mut())?;
        codec_util::write_footer(self.data.as_mut())?;
        self.meta.close()?;
        self.data.close()?;
        Ok(())
    }
}

/// Returns the minimum number of bytes needed to represent every value in
/// `[min, max]`.
fn num_bytes_per_value(min: i64, max: i64) -> i32 {
    if min >= max {
        0
    } else if min >= i8::MIN as i64 && max <= i8::MAX as i64 {
        1
    } else if min >= i16::MIN as i64 && max <= i16::MAX as i64 {
        2
    } else if min >= i32::MIN as i64 && max <= i32::MAX as i64 {
        4
    } else {
        8
    }
}

/// Writes a `max_doc`-bit `FixedBitSet` built from the documents reported by
/// `values`.
fn write_fixed_bit_set(
    max_doc: usize,
    values: &mut dyn NumericDocValues,
    out: &mut dyn IndexOutput,
) -> Result<()> {
    let mut bits = FixedBitSet::new(max_doc);
    loop {
        let doc = values.next_doc()?;
        if doc == NO_MORE_DOCS {
            break;
        }
        bits.set(doc as usize);
    }
    for &word in bits.get_bits() {
        out.write_long(word as i64)?;
    }
    Ok(())
}

/// Writes the norm values for the documents reported by `values` using the
/// given byte width.
fn write_norm_values(
    values: &mut dyn NumericDocValues,
    bytes_per_norm: i32,
    out: &mut dyn IndexOutput,
) -> Result<()> {
    loop {
        let doc = values.next_doc()?;
        if doc == NO_MORE_DOCS {
            break;
        }
        let value = values.long_value()?;
        match bytes_per_norm {
            1 => out.write_byte(value as i8 as u8)?,
            2 => out.write_short(value as i16)?,
            4 => out.write_int(value as i32)?,
            8 => out.write_long(value)?,
            _ => unreachable!("validated bytes_per_norm: {bytes_per_norm}"),
        }
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Producer
// -----------------------------------------------------------------------------

/// Per-field metadata parsed from the `.nvm` file.
#[derive(Clone, Copy, Debug, Default)]
struct NormsEntry {
    dense_rank_power: i8,
    bytes_per_norm: i8,
    docs_with_field_offset: i64,
    docs_with_field_length: i64,
    jump_table_entry_count: i16,
    num_docs_with_field: i32,
    norms_offset: i64,
}

/// Reader for [`Lucene90NormsFormat`].
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.Lucene90NormsProducer`.
pub struct Lucene90NormsProducer {
    norms: HashMap<i32, NormsEntry>,
    max_doc: i32,
    data: Box<dyn IndexInput>,
}

impl fmt::Debug for Lucene90NormsProducer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90NormsProducer")
            .field("fields", &self.norms.len())
            .field("max_doc", &self.max_doc)
            .finish_non_exhaustive()
    }
}

impl Lucene90NormsProducer {
    fn new(state: &SegmentReadState) -> Result<Self> {
        let max_doc = state.segment_info.max_doc()?;

        let meta_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            NORMS_META_EXTENSION,
        );
        let mut meta = state.directory.open_checksum_input(&meta_name)?;
        let version = codec_util::check_index_header(
            meta.as_mut(),
            METADATA_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let mut norms = HashMap::new();
        read_fields(meta.as_mut(), state.field_infos, &mut norms)?;
        codec_util::check_footer(meta.as_mut())?;
        drop(meta);

        let data_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            NORMS_EXTENSION,
        );
        let mut data = state.directory.open_input(&data_name, state.context)?;
        let version2 = codec_util::check_index_header(
            data.as_mut(),
            DATA_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        if version != version2 {
            return Err(LuceneError::CorruptIndex(format!(
                "Format versions mismatch: meta={version}, data={version2}"
            )));
        }
        // Cheap structural check of the data footer; the actual checksum is
        // verified by `check_integrity` on demand.
        codec_util::retrieve_checksum(data.as_mut())?;

        Ok(Self {
            norms,
            max_doc,
            data,
        })
    }

    /// Reads the bit set stored at `offset` of length `length` bytes.
    fn read_bitset(&self, offset: i64, length: i64) -> Result<FixedBitSet> {
        let num_words = FixedBitSet::bits2words(self.max_doc as usize);
        let expected_length = num_words as i64 * 8;
        if length != expected_length {
            return Err(LuceneError::CorruptIndex(format!(
                "bitset length mismatch: expected {expected_length}, got {length}"
            )));
        }
        let mut slice = self.data.slice("bitset", offset, length)?;
        let mut words = vec![0u64; num_words];
        for word in &mut words {
            *word = slice.read_long()? as u64;
        }
        Ok(FixedBitSet::from_bits(words, self.max_doc as usize))
    }
}

fn read_fields(
    meta: &mut dyn IndexInput,
    infos: &crate::index::FieldInfos,
    norms: &mut HashMap<i32, NormsEntry>,
) -> Result<()> {
    loop {
        let field_number = meta.read_int()?;
        if field_number == -1 {
            break;
        }
        let info = infos.field_info_by_number(field_number).ok_or_else(|| {
            LuceneError::CorruptIndex(format!("Invalid field number: {field_number}"))
        })?;
        if !info.has_norms() {
            return Err(LuceneError::CorruptIndex(format!(
                "Invalid field: {} (no norms)",
                info.name
            )));
        }
        let mut entry = NormsEntry::default();
        entry.docs_with_field_offset = meta.read_long()?;
        entry.docs_with_field_length = meta.read_long()?;
        entry.jump_table_entry_count = meta.read_short()?;
        entry.dense_rank_power = meta.read_byte()? as i8;
        entry.num_docs_with_field = meta.read_int()?;
        entry.bytes_per_norm = meta.read_byte()? as i8;
        match entry.bytes_per_norm {
            0 | 1 | 2 | 4 | 8 => {}
            _ => {
                return Err(LuceneError::CorruptIndex(format!(
                    "Invalid bytesPerNorm: {}, field: {}",
                    entry.bytes_per_norm, info.name
                )))
            }
        }
        entry.norms_offset = meta.read_long()?;
        norms.insert(info.number, entry);
    }
    Ok(())
}

impl NormsProducer for Lucene90NormsProducer {
    fn get_norms(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        let entry = self.norms.get(&field.number).ok_or_else(|| {
            LuceneError::IllegalArgument(format!("no norms for field {}", field.name))
        })?;

        if entry.docs_with_field_offset == -2 {
            // No documents have a value.
            return Ok(Box::new(DocValues::empty_numeric()));
        }

        if entry.docs_with_field_offset == -1 {
            // Dense: every document has a value.
            if entry.bytes_per_norm == 0 {
                return Ok(Box::new(DenseConstantNormsIterator::new(
                    self.max_doc,
                    entry.norms_offset,
                )));
            }
            let mut slice = self.data.random_access_slice(
                entry.norms_offset,
                self.max_doc as i64 * entry.bytes_per_norm as i64,
            )?;
            let values =
                read_norm_values(&mut *slice, entry.bytes_per_norm, self.max_doc as usize)?;
            return Ok(Box::new(DenseNormsIterator::new(self.max_doc, values)));
        }

        // Sparse: read the docs-with-field bit set and build an iterator over it.
        let bits = self.read_bitset(entry.docs_with_field_offset, entry.docs_with_field_length)?;
        if entry.bytes_per_norm == 0 {
            return Ok(Box::new(SparseConstantNormsIterator::new(
                bits,
                entry.norms_offset,
            )));
        }
        let mut slice = self.data.random_access_slice(
            entry.norms_offset,
            entry.num_docs_with_field as i64 * entry.bytes_per_norm as i64,
        )?;
        let values = read_norm_values(
            &mut *slice,
            entry.bytes_per_norm,
            entry.num_docs_with_field as usize,
        )?;
        Ok(Box::new(SparseNormsIterator::new(bits, values)))
    }

    fn check_integrity(&self) -> Result<()> {
        let mut clone = self.data.clone_input()?;
        codec_util::checksum_entire_file(clone.as_mut())?;
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        let cloned_data = self.data.clone_input()?;
        Ok(Box::new(Self {
            norms: self.norms.clone(),
            max_doc: self.max_doc,
            data: cloned_data,
        }))
    }

    fn close(&mut self) -> Result<()> {
        self.data.close()
    }
}

// -----------------------------------------------------------------------------
// Iterators
// -----------------------------------------------------------------------------

/// Dense iterator over norms where every document has the same singleton value.
#[derive(Debug)]
struct DenseConstantNormsIterator {
    max_doc: i32,
    doc: i32,
    value: i64,
}

impl DenseConstantNormsIterator {
    fn new(max_doc: i32, value: i64) -> Self {
        Self {
            max_doc,
            doc: -1,
            value,
        }
    }
}

impl DocIdSetIterator for DenseConstantNormsIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target >= self.max_doc {
            self.doc = NO_MORE_DOCS;
        } else {
            self.doc = target.max(0);
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.max_doc as i64
    }
}

impl DocValuesIterator for DenseConstantNormsIterator {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        Ok(target >= 0 && target < self.max_doc)
    }
}

impl NumericDocValues for DenseConstantNormsIterator {
    fn long_value(&self) -> Result<i64> {
        if self.doc < 0 || self.doc >= self.max_doc {
            return Err(LuceneError::IllegalState(
                "long_value called with no current document".to_string(),
            ));
        }
        Ok(self.value)
    }
}

/// Dense iterator backed by a pre-read vector of norm values.
#[derive(Debug)]
struct DenseNormsIterator {
    max_doc: i32,
    doc: i32,
    values: Vec<i64>,
}

impl DenseNormsIterator {
    fn new(max_doc: i32, values: Vec<i64>) -> Self {
        Self {
            max_doc,
            doc: -1,
            values,
        }
    }
}

impl DocIdSetIterator for DenseNormsIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target >= self.max_doc {
            self.doc = NO_MORE_DOCS;
        } else {
            self.doc = target.max(0);
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.max_doc as i64
    }
}

impl DocValuesIterator for DenseNormsIterator {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        Ok(target >= 0 && target < self.max_doc)
    }
}

impl NumericDocValues for DenseNormsIterator {
    fn long_value(&self) -> Result<i64> {
        if self.doc < 0 || self.doc >= self.max_doc {
            return Err(LuceneError::IllegalState(
                "long_value called with no current document".to_string(),
            ));
        }
        Ok(self.values[self.doc as usize])
    }
}

/// Sparse iterator over norms backed by a `FixedBitSet` of docs with values and
/// a pre-read vector of the corresponding norm values.
#[derive(Debug)]
struct SparseNormsIterator {
    bits: FixedBitSet,
    doc: i32,
    index: i64,
    values: Vec<i64>,
}

impl SparseNormsIterator {
    fn new(bits: FixedBitSet, values: Vec<i64>) -> Self {
        Self {
            bits,
            doc: -1,
            index: -1,
            values,
        }
    }

    /// Positions on the next set bit at or after `target`, updating `doc` and
    /// `index`.
    fn advance_internal(&mut self, target: i32) -> Result<i32> {
        let start = target.max(0) as usize;
        let len = self.bits.length();
        if start >= len {
            self.doc = NO_MORE_DOCS;
            return Ok(NO_MORE_DOCS);
        }

        let word_idx = start >> 6;
        let bit_idx = start & 0x3f;
        let bit_words = self.bits.get_bits();

        // Search the first (partial) word.
        let word = bit_words[word_idx] >> bit_idx;
        if word != 0 {
            let candidate = (word_idx << 6) + bit_idx + word.trailing_zeros() as usize;
            if candidate < len {
                self.doc = candidate as i32;
                self.index = popcount_up_to(&self.bits, candidate) as i64 - 1;
                return Ok(self.doc);
            }
        }

        // Search the remaining whole words.
        for i in (word_idx + 1)..bit_words.len() {
            let w = bit_words[i];
            if w != 0 {
                let candidate = (i << 6) + w.trailing_zeros() as usize;
                if candidate < len {
                    self.doc = candidate as i32;
                    self.index = popcount_up_to(&self.bits, candidate) as i64 - 1;
                    return Ok(self.doc);
                }
            }
        }

        self.doc = NO_MORE_DOCS;
        Ok(NO_MORE_DOCS)
    }
}

impl DocIdSetIterator for SparseNormsIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.advance_internal(target)
    }

    fn cost(&self) -> i64 {
        self.bits.cardinality() as i64
    }
}

impl DocValuesIterator for SparseNormsIterator {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        let len = self.bits.length();
        if target < 0 || target as usize >= len {
            return Ok(false);
        }
        let has_value = self.bits.get(target as usize);
        if has_value {
            self.index = popcount_up_to(&self.bits, target as usize) as i64 - 1;
        }
        Ok(has_value)
    }
}

impl NumericDocValues for SparseNormsIterator {
    fn long_value(&self) -> Result<i64> {
        if self.doc < 0
            || self.doc as usize >= self.bits.length()
            || !self.bits.get(self.doc as usize)
        {
            return Err(LuceneError::IllegalState(
                "long_value called with no current document".to_string(),
            ));
        }
        Ok(self.values[self.index as usize])
    }
}

/// Sparse iterator where every document with a value shares the same value.
#[derive(Debug)]
struct SparseConstantNormsIterator {
    bits: FixedBitSet,
    doc: i32,
    index: i64,
    value: i64,
}

impl SparseConstantNormsIterator {
    fn new(bits: FixedBitSet, value: i64) -> Self {
        Self {
            bits,
            doc: -1,
            index: -1,
            value,
        }
    }

    fn advance_internal(&mut self, target: i32) -> Result<i32> {
        let start = target.max(0) as usize;
        let len = self.bits.length();
        if start >= len {
            self.doc = NO_MORE_DOCS;
            return Ok(NO_MORE_DOCS);
        }

        let word_idx = start >> 6;
        let bit_idx = start & 0x3f;
        let bit_words = self.bits.get_bits();

        let word = bit_words[word_idx] >> bit_idx;
        if word != 0 {
            let candidate = (word_idx << 6) + bit_idx + word.trailing_zeros() as usize;
            if candidate < len {
                self.doc = candidate as i32;
                self.index = popcount_up_to(&self.bits, candidate) as i64 - 1;
                return Ok(self.doc);
            }
        }

        for i in (word_idx + 1)..bit_words.len() {
            let w = bit_words[i];
            if w != 0 {
                let candidate = (i << 6) + w.trailing_zeros() as usize;
                if candidate < len {
                    self.doc = candidate as i32;
                    self.index = popcount_up_to(&self.bits, candidate) as i64 - 1;
                    return Ok(self.doc);
                }
            }
        }

        self.doc = NO_MORE_DOCS;
        Ok(NO_MORE_DOCS)
    }
}

impl DocIdSetIterator for SparseConstantNormsIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.advance_internal(target)
    }

    fn cost(&self) -> i64 {
        self.bits.cardinality() as i64
    }
}

impl DocValuesIterator for SparseConstantNormsIterator {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        let len = self.bits.length();
        if target < 0 || target as usize >= len {
            return Ok(false);
        }
        let has_value = self.bits.get(target as usize);
        if has_value {
            self.index = popcount_up_to(&self.bits, target as usize) as i64 - 1;
        }
        Ok(has_value)
    }
}

impl NumericDocValues for SparseConstantNormsIterator {
    fn long_value(&self) -> Result<i64> {
        if self.doc < 0
            || self.doc as usize >= self.bits.length()
            || !self.bits.get(self.doc as usize)
        {
            return Err(LuceneError::IllegalState(
                "long_value called with no current document".to_string(),
            ));
        }
        Ok(self.value)
    }
}

/// Counts set bits in `bits` in `[0, doc]` inclusive.
fn popcount_up_to(bits: &FixedBitSet, doc: usize) -> usize {
    let len = bits.length();
    if doc >= len {
        return bits.cardinality();
    }
    let word_idx = doc >> 6;
    let bit_idx = doc & 0x3f;
    let bit_words = bits.get_bits();
    let mut count = 0usize;
    for i in 0..word_idx {
        count += bit_words[i].count_ones() as usize;
    }
    if word_idx < bit_words.len() {
        let mask = (1u64 << (bit_idx + 1)) - 1;
        count += (bit_words[word_idx] & mask).count_ones() as usize;
    }
    count
}

/// Reads `count` packed norm values from `slice` using `bytes_per_norm` per
/// value.
fn read_norm_values(
    slice: &mut dyn RandomAccessInput,
    bytes_per_norm: i8,
    count: usize,
) -> Result<Vec<i64>> {
    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        values.push(read_value_at(
            slice,
            i as i64 * bytes_per_norm as i64,
            bytes_per_norm,
        )?);
    }
    Ok(values)
}

/// Reads a packed norm value of `bytes_per_norm` bytes at `pos` from `slice`.
fn read_value_at(slice: &mut dyn RandomAccessInput, pos: i64, bytes_per_norm: i8) -> Result<i64> {
    match bytes_per_norm {
        1 => Ok(slice.read_byte_at(pos)? as i8 as i64),
        2 => Ok(slice.read_short_at(pos)? as i64),
        4 => Ok(slice.read_int_at(pos)? as i64),
        8 => Ok(slice.read_long_at(pos)?),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid bytes per norm: {bytes_per_norm}"
        ))),
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::state::{SegmentReadState, SegmentWriteState};
    use crate::codecs::stub::BufferedUpdates;
    use crate::error::LuceneError;
    use crate::index::{DocValuesType, FieldInfo, FieldInfos, IndexOptions};
    use crate::search::DocIdSetIterator;
    use crate::store::{Directory, RamDirectory};
    use crate::util::Version;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn test_codec() -> Arc<dyn crate::codecs::Codec> {
        Arc::new(crate::codecs::FilterCodec::new(
            "TestCodec",
            Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
        ))
    }

    fn test_segment_info(name: &str, max_doc: i32) -> crate::index::SegmentInfo {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        crate::index::SegmentInfo::new(
            dir,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            false,
            false,
            test_codec(),
            HashMap::new(),
            [0u8; crate::util::string_helper::ID_LENGTH],
            HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap()
    }

    fn norm_field(name: &str, number: i32) -> FieldInfo {
        let mut fi = FieldInfo::new(name, number);
        fi.index_options = IndexOptions::DOCS_AND_FREQS;
        fi.doc_values_type = DocValuesType::NONE;
        fi
    }

    fn field_infos(fields: Vec<FieldInfo>) -> FieldInfos {
        FieldInfos::new(fields).unwrap()
    }

    fn write_state<'a>(
        dir: &'a dyn Directory,
        info: &'a crate::index::SegmentInfo,
        fis: &'a FieldInfos,
    ) -> SegmentWriteState<'a> {
        SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir,
            info,
            fis,
            &BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        )
    }

    fn read_state<'a>(
        dir: &'a dyn Directory,
        info: &'a crate::index::SegmentInfo,
        fis: &'a FieldInfos,
    ) -> SegmentReadState<'a> {
        SegmentReadState::new(dir, info, fis, &*crate::store::DEFAULT_IO_CONTEXT)
    }

    /// In-memory numeric doc-values iterator used only for tests.
    struct VecNumericDocValues {
        docs: Vec<(i32, i64)>,
        idx: i32,
    }

    impl VecNumericDocValues {
        fn new(docs: Vec<(i32, i64)>) -> Self {
            Self { docs, idx: -1 }
        }
    }

    impl DocIdSetIterator for VecNumericDocValues {
        fn doc_id(&self) -> i32 {
            if self.idx < 0 {
                -1
            } else if self.idx as usize >= self.docs.len() {
                NO_MORE_DOCS
            } else {
                self.docs[self.idx as usize].0
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.idx += 1;
            if self.idx as usize >= self.docs.len() {
                self.idx = self.docs.len() as i32;
                Ok(NO_MORE_DOCS)
            } else {
                Ok(self.docs[self.idx as usize].0)
            }
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.idx = self
                .docs
                .iter()
                .position(|(d, _)| *d >= target)
                .map(|p| p as i32)
                .unwrap_or(self.docs.len() as i32);
            Ok(self.doc_id())
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    impl DocValuesIterator for VecNumericDocValues {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.idx = self
                .docs
                .iter()
                .position(|(d, _)| *d == target)
                .map(|p| p as i32)
                .unwrap_or(-1);
            Ok(self.idx >= 0)
        }
    }

    impl NumericDocValues for VecNumericDocValues {
        fn long_value(&self) -> Result<i64> {
            if self.idx < 0 || self.idx as usize >= self.docs.len() {
                return Err(LuceneError::IllegalState(
                    "long_value called with no current document".to_string(),
                ));
            }
            Ok(self.docs[self.idx as usize].1)
        }
    }

    /// Norms producer that serves a fixed set of values per field.
    #[derive(Debug)]
    struct VecNormsProducer {
        fields: HashMap<i32, Vec<(i32, i64)>>,
    }

    impl VecNormsProducer {
        fn new(fields: HashMap<i32, Vec<(i32, i64)>>) -> Self {
            Self { fields }
        }
    }

    impl NormsProducer for VecNormsProducer {
        fn get_norms(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
            let docs = self.fields.get(&field.number).cloned().unwrap_or_default();
            Ok(Box::new(VecNumericDocValues::new(docs)))
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
            Ok(Box::new(Self {
                fields: self.fields.clone(),
            }))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn collect_values(values: &mut dyn NumericDocValues, max_doc: i32) -> Vec<Option<i64>> {
        let mut out = vec![None; max_doc as usize];
        while let Ok(doc) = values.next_doc() {
            if doc == NO_MORE_DOCS {
                break;
            }
            out[doc as usize] = Some(values.long_value().unwrap());
        }
        out
    }

    #[test]
    fn format_name_is_lucene90_norms() {
        assert_eq!(Lucene90NormsFormat::new().name(), "Lucene90Norms");
    }

    #[test]
    fn round_trip_dense_field() {
        let dir = RamDirectory::default();
        let info = test_segment_info("_0", 5);
        let body = norm_field("body", 0);
        let fis = field_infos(vec![body.clone()]);

        let values = VecNormsProducer::new(HashMap::from([(
            0,
            vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)],
        )]));

        {
            let mut consumer = Lucene90NormsFormat::new()
                .norms_consumer(&write_state(&dir, &info, &fis))
                .unwrap();
            consumer.add_norms_field(&body, &values).unwrap();
            consumer.close().unwrap();
        }

        let mut producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();
        let mut values = producer.get_norms(&body).unwrap();
        let collected = collect_values(&mut *values, 5);
        assert_eq!(collected, vec![Some(1), Some(2), Some(3), Some(4), Some(5)]);
        producer.check_integrity().unwrap();
        let _merge = producer.get_merge_instance().unwrap();
        producer.close().unwrap();
    }

    #[test]
    fn round_trip_sparse_field() {
        let dir = RamDirectory::default();
        let info = test_segment_info("_0", 10);
        let body = norm_field("body", 0);
        let fis = field_infos(vec![body.clone()]);

        let values =
            VecNormsProducer::new(HashMap::from([(0, vec![(1, 100), (3, 200), (7, 300)])]));

        {
            let mut consumer = Lucene90NormsFormat::new()
                .norms_consumer(&write_state(&dir, &info, &fis))
                .unwrap();
            consumer.add_norms_field(&body, &values).unwrap();
            consumer.close().unwrap();
        }

        let mut producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();
        let mut values = producer.get_norms(&body).unwrap();
        let collected = collect_values(&mut *values, 10);
        let expected: Vec<Option<i64>> = (0..10)
            .map(|d| match d {
                1 => Some(100),
                3 => Some(200),
                7 => Some(300),
                _ => None,
            })
            .collect();
        assert_eq!(collected, expected);
    }

    #[test]
    fn round_trip_empty_field() {
        let dir = RamDirectory::default();
        let info = test_segment_info("_0", 4);
        let body = norm_field("body", 0);
        let fis = field_infos(vec![body.clone()]);

        let values = VecNormsProducer::new(HashMap::new());

        {
            let mut consumer = Lucene90NormsFormat::new()
                .norms_consumer(&write_state(&dir, &info, &fis))
                .unwrap();
            consumer.add_norms_field(&body, &values).unwrap();
            consumer.close().unwrap();
        }

        let mut producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();
        let mut values = producer.get_norms(&body).unwrap();
        assert_eq!(values.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_constant_value() {
        let dir = RamDirectory::default();
        let info = test_segment_info("_0", 6);
        let body = norm_field("body", 0);
        let fis = field_infos(vec![body.clone()]);

        let values = VecNormsProducer::new(HashMap::from([(0, vec![(0, 42), (2, 42), (4, 42)])]));

        {
            let mut consumer = Lucene90NormsFormat::new()
                .norms_consumer(&write_state(&dir, &info, &fis))
                .unwrap();
            consumer.add_norms_field(&body, &values).unwrap();
            consumer.close().unwrap();
        }

        let mut producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();
        let mut values = producer.get_norms(&body).unwrap();
        let collected = collect_values(&mut *values, 6);
        let expected: Vec<Option<i64>> = (0..6)
            .map(|d| if d % 2 == 0 { Some(42) } else { None })
            .collect();
        assert_eq!(collected, expected);
    }

    #[test]
    fn round_trip_multiple_fields() {
        let dir = RamDirectory::default();
        let info = test_segment_info("_0", 5);
        let body = norm_field("body", 0);
        let title = norm_field("title", 1);
        let fis = field_infos(vec![body.clone(), title.clone()]);

        let values = VecNormsProducer::new(HashMap::from([
            (0, vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)]),
            (1, vec![(1, 10), (3, 20)]),
        ]));

        {
            let mut consumer = Lucene90NormsFormat::new()
                .norms_consumer(&write_state(&dir, &info, &fis))
                .unwrap();
            consumer.add_norms_field(&body, &values).unwrap();
            consumer.add_norms_field(&title, &values).unwrap();
            consumer.close().unwrap();
        }

        let mut producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();

        let mut body_values = producer.get_norms(&body).unwrap();
        assert_eq!(
            collect_values(&mut *body_values, 5),
            vec![Some(1), Some(2), Some(3), Some(4), Some(5)]
        );

        let mut title_values = producer.get_norms(&title).unwrap();
        let title_collected = collect_values(&mut *title_values, 5);
        let expected: Vec<Option<i64>> = (0..5)
            .map(|d| match d {
                1 => Some(10),
                3 => Some(20),
                _ => None,
            })
            .collect();
        assert_eq!(title_collected, expected);
    }

    #[test]
    fn round_trip_byte_widths() {
        let dir = RamDirectory::default();
        let info = test_segment_info("_0", 4);
        let fields = vec![
            (0, "byte", vec![(0, 100), (1, -100), (2, 50), (3, -50)]),
            (1, "short", vec![(0, 30_000), (1, -30_000), (2, 0), (3, 1)]),
            (
                2,
                "int",
                vec![(0, i32::MAX as i64), (1, i32::MIN as i64), (2, 0), (3, -1)],
            ),
            (
                3,
                "long",
                vec![(0, i64::MAX), (1, i64::MIN), (2, 0), (3, -1)],
            ),
        ];

        let mut field_infos_fields = Vec::new();
        let mut values_map = HashMap::new();
        for (number, name, docs) in &fields {
            let fi = norm_field(name, *number);
            field_infos_fields.push(fi);
            values_map.insert(*number, docs.clone());
        }
        let fis = field_infos(field_infos_fields);
        let values = VecNormsProducer::new(values_map);

        {
            let mut consumer = Lucene90NormsFormat::new()
                .norms_consumer(&write_state(&dir, &info, &fis))
                .unwrap();
            for (number, _, _) in &fields {
                consumer
                    .add_norms_field(fis.field_info_by_number(*number).unwrap(), &values)
                    .unwrap();
            }
            consumer.close().unwrap();
        }

        let mut producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();
        for (number, _, expected) in &fields {
            let field = fis.field_info_by_number(*number).unwrap();
            let mut it = producer.get_norms(field).unwrap();
            let collected = collect_values(&mut *it, 4);
            assert_eq!(
                collected,
                expected.iter().map(|(_, v)| Some(*v)).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn rejects_missing_field_number() {
        let dir = RamDirectory::default();
        let info = test_segment_info("_0", 3);
        let body = norm_field("body", 0);
        let fis_with_body = field_infos(vec![body.clone()]);
        let values = VecNormsProducer::new(HashMap::from([(0, vec![(0, 1)])]));

        {
            let mut consumer = Lucene90NormsFormat::new()
                .norms_consumer(&write_state(&dir, &info, &fis_with_body))
                .unwrap();
            consumer.add_norms_field(&body, &values).unwrap();
            consumer.close().unwrap();
        }

        let empty_fis = field_infos(vec![]);
        let err = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &empty_fis))
            .unwrap_err();
        assert!(matches!(err, LuceneError::CorruptIndex(_)));
    }

    #[test]
    fn rejects_field_without_norms() {
        let dir = RamDirectory::default();
        let info = test_segment_info("_0", 3);
        let mut body = FieldInfo::new("body", 0);
        body.index_options = IndexOptions::DOCS_AND_FREQS;
        body.doc_values_type = DocValuesType::NONE;
        // Explicitly omit norms.
        body.set_omits_norms().unwrap();
        let fis = field_infos(vec![body.clone()]);
        let values = VecNormsProducer::new(HashMap::from([(0, vec![(0, 1)])]));

        {
            let mut consumer = Lucene90NormsFormat::new()
                .norms_consumer(&write_state(&dir, &info, &fis))
                .unwrap();
            // The writer does not enforce has_norms; the reader does.
            consumer.add_norms_field(&body, &values).unwrap();
            consumer.close().unwrap();
        }

        let err = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap_err();
        assert!(matches!(err, LuceneError::CorruptIndex(_)));
    }
}
