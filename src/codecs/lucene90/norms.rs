//! Lucene 9.0 score-normalization format.
//!
//! Equivalent to `org.apache.lucene.codecs.lucene90.Lucene90NormsFormat`.
//!
//! This codec writes two files per segment:
//!
//! * `.nvd` — norms data (docs-with-field set + packed norm values);
//! * `.nvm` — norms metadata (per-field offsets, byte widths and counts).
//!
//! # File layout
//!
//! The `.nvm` file starts with an index header and ends with a codec footer.
//! For each norms field it stores, in order:
//!
//! * `field_number` (`i32`) — `-1` marks the end of metadata;
//! * `docs_with_field_offset` (`i64`) — `-2` if no docs have a value, `-1` if
//!   every doc has a value, otherwise the offset in `.nvd` of the
//!   [`IndexedDISI`] block stream;
//! * `docs_with_field_length` (`i64`) — byte length of that stream in `.nvd`
//!   (`0` for the empty and all-documents cases);
//! * `jump_table_entry_count` (`i16`) — the value
//!   `IndexedDISI::write_bit_set` returned, or `-1` when no stream was
//!   written;
//! * `dense_rank_power` (`i8`) — [`DEFAULT_DENSE_RANK_POWER`], or `-1` when no
//!   stream was written;
//! * `num_docs_with_field` (`i32`);
//! * `bytes_per_norm` (`i8`) — `0`, `1`, `2`, `4` or `8`;
//! * `norms_offset` (`i64`) — for `bytes_per_norm == 0` this is the singleton
//!   value itself, otherwise the offset in `.nvd` of the packed values.
//!
//! The `.nvd` file starts with an index header and ends with a codec footer.
//! For each field whose documents are neither all present nor all absent it
//! stores the docs-with-field set in the `IndexedDISI` encoding, followed by the
//! packed norm values for only the documents that have a value. The layout is
//! byte-for-byte that of Apache Lucene Core 10.5.0; see
//! `codecs/lucene90/Lucene90NormsConsumer.java:88-127`.
//!
//! # Norms are signed
//!
//! `Similarity.computeNorm` returns a `byte` widened to `long`, so a norm is in
//! `[-128, 127]` (see [`crate::search::similarities`]). `bytes_per_norm` is
//! derived from the signed minimum and maximum of exactly those values, which
//! is why a whole segment of norms normally fits in one byte per document.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::fmt;

use crate::codecs::codec_util;
use crate::codecs::lucene90::indexed_disi::{write_bit_set, IndexedDISI, DEFAULT_DENSE_RANK_POWER};
use crate::codecs::norms::{NormsConsumer, NormsFormat, NormsProducer};
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::doc_values::{DocValues, DocValuesIterator, NumericDocValues};
use crate::index::index_file_names::{segment_file_name, NORMS_EXTENSION, NORMS_META_EXTENSION};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{IndexInput, IndexOutput, RandomAccessInput};

const DATA_CODEC: &str = "Lucene90NormsData";
const METADATA_CODEC: &str = "Lucene90NormsMetadata";
const VERSION_START: i32 = 0;
const VERSION_CURRENT: i32 = VERSION_START;

/// Written as `jump_table_entry_count` when the field has no docs-with-field
/// stream at all, i.e. when every document has a value or none has.
///
/// Java writes the literal `(short) -1` in both branches
/// (`Lucene90NormsConsumer.java:103`, `:109`).
const NO_JUMP_TABLE: i16 = -1;

/// Written as `dense_rank_power` in the same two branches
/// (`Lucene90NormsConsumer.java:104`, `:110`).
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
        // First pass: count docs with a value and determine the value range.
        // Java walks the producer three times over; so does this, because the
        // producer is the buffer the indexing chain filled and replaying it is
        // cheaper than materialising it here.
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
            let mut docs = NormsDocs::new(it.as_mut());
            let jump_table_entry_count =
                write_bit_set(&mut docs, self.data.as_mut(), DEFAULT_DENSE_RANK_POWER)?;
            self.meta.write_long(self.data.file_pointer() - offset)?;
            self.meta.write_short(jump_table_entry_count)?;
            self.meta.write_byte(DEFAULT_DENSE_RANK_POWER as u8)?;
        }

        self.meta.write_int(num_docs_with_value)?;
        let bytes_per_norm = num_bytes_per_value(min, max);
        self.meta.write_byte(bytes_per_norm as u8)?;

        if bytes_per_norm == 0 {
            // A field whose norms are all equal stores the value in the
            // metadata instead of the data file. For a field with no documents
            // at all `min` is still `Long.MAX_VALUE`, which is exactly what
            // Java writes (`Lucene90NormsConsumer.java:123`); nothing ever
            // reads it back, because `docs_with_field_offset` is `-2`.
            self.meta.write_long(min)?;
        } else {
            let norms_offset = self.data.file_pointer();
            self.meta.write_long(norms_offset)?;
            let mut it = values.get_norms(field)?;
            write_norm_values(it.as_mut(), bytes_per_norm, self.data.as_mut())?;
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

/// Views the documents of a [`NumericDocValues`] as a plain
/// [`DocIdSetIterator`].
///
/// `write_bit_set` takes a `&mut dyn DocIdSetIterator`, and this crate's MSRV
/// (1.80) predates trait upcasting, so a `&mut dyn NumericDocValues` cannot be
/// coerced to its supertrait object. Delegating through a thin adapter keeps
/// the streaming behaviour of Java's
/// `IndexedDISI.writeBitSet(values, data, ...)` without buffering the doc ids.
struct NormsDocs<'a> {
    values: &'a mut dyn NumericDocValues,
}

impl<'a> NormsDocs<'a> {
    fn new(values: &'a mut dyn NumericDocValues) -> Self {
        Self { values }
    }
}

impl DocIdSetIterator for NormsDocs<'_> {
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

/// Returns the minimum number of bytes needed to represent every value in
/// `[min, max]`.
///
/// Equivalent to `Lucene90NormsConsumer.numBytesPerValue(long, long)`.
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

/// Writes the norm values for the documents reported by `values` using the
/// given byte width.
///
/// Equivalent to `Lucene90NormsConsumer.writeValues`.
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
///
/// Equivalent to `Lucene90NormsProducer.NormsEntry`.
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
        let mut norms = HashMap::new();
        // Java puts **both** the header check and the entry parse inside one
        // `try`, and verifies the footer in the matching `finally` with
        // whatever they threw (`Lucene90NormsProducer.java:49-64`). Rust has no
        // `finally`: the two are run into one outcome, which is then matched
        // on, and `check_footer_with_prior` makes the same choice Java's
        // two-argument `checkFooter` does about which of the two failures
        // explains the file.
        let parsed = match codec_util::check_index_header(
            meta.as_mut(),
            METADATA_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        ) {
            Ok(version) => {
                read_fields(meta.as_mut(), state.field_infos, &mut norms).map(|()| version)
            }
            Err(error) => Err(error),
        };
        let version = match parsed {
            Ok(version) => {
                codec_util::check_footer(meta.as_mut())?;
                version
            }
            Err(prior) => {
                return Err(codec_util::check_footer_with_prior(meta.as_mut(), prior));
            }
        };
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

    /// Opens the packed-values slice of one field.
    ///
    /// Equivalent to `Lucene90NormsProducer.getDataInput`, without the
    /// merge-instance cache: Rucene's merge instance clones the whole input, so
    /// there is no shared slice to reuse.
    fn data_input(&self, entry: &NormsEntry) -> Result<Box<dyn RandomAccessInput>> {
        // `read_fields` has already restricted `bytes_per_norm` to 0, 1, 2, 4
        // or 8, so the product of an `i32` and it cannot leave `i64`; what can
        // still arrive off disk is a negative `num_docs_with_field`, which
        // would ask for a slice of negative length. Java lets
        // `randomAccessSlice` refuse that; it is refused here with the numbers
        // that caused it.
        let length = entry.num_docs_with_field as i64 * entry.bytes_per_norm as i64;
        if length < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "invalid norms slice: numDocsWithField={}, bytesPerNorm={}",
                entry.num_docs_with_field, entry.bytes_per_norm
            )));
        }
        self.data.random_access_slice(entry.norms_offset, length)
    }

    /// Opens the docs-with-field iterator of one sparse field.
    ///
    /// Equivalent to `Lucene90NormsProducer.getDisiInput` plus
    /// `getDisiJumpTable`, which [`IndexedDISI::new`] performs together.
    fn disi(&self, entry: &NormsEntry) -> Result<IndexedDISI> {
        IndexedDISI::new(
            self.data.as_ref(),
            entry.docs_with_field_offset,
            entry.docs_with_field_length,
            entry.jump_table_entry_count as i32,
            entry.dense_rank_power,
            entry.num_docs_with_field as i64,
        )
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
        let mut entry = NormsEntry {
            docs_with_field_offset: meta.read_long()?,
            docs_with_field_length: meta.read_long()?,
            jump_table_entry_count: meta.read_short()?,
            dense_rank_power: meta.read_byte()? as i8,
            num_docs_with_field: meta.read_int()?,
            bytes_per_norm: meta.read_byte()? as i8,
            ..Default::default()
        };
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
            let slice = self.data_input(entry)?;
            return Ok(Box::new(DenseNormsIterator::new(
                self.max_doc,
                slice,
                entry.bytes_per_norm,
            )));
        }

        // Sparse: the docs-with-field set is an `IndexedDISI` stream.
        let disi = self.disi(entry)?;
        if entry.bytes_per_norm == 0 {
            return Ok(Box::new(SparseConstantNormsIterator::new(
                disi,
                entry.norms_offset,
            )));
        }
        let slice = self.data_input(entry)?;
        Ok(Box::new(SparseNormsIterator::new(
            disi,
            slice,
            entry.bytes_per_norm,
        )))
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

/// Reads the packed norm of the value at ordinal `index`.
///
/// Equivalent to the four `slice.readByte(...)`/`readShort(...)`/`readInt(...)`/
/// `readLong(...)` bodies of `Lucene90NormsProducer.getNorms`, whose positions
/// are `index << log2(bytesPerNorm)`. The shift is done in `i64` from an `i32`
/// ordinal, so it cannot overflow.
fn read_norm_at(slice: &mut dyn RandomAccessInput, index: i32, bytes_per_norm: i8) -> Result<i64> {
    let index = index as i64;
    match bytes_per_norm {
        1 => Ok(slice.read_byte_at(index)? as i8 as i64),
        2 => Ok(slice.read_short_at(index << 1)? as i64),
        4 => Ok(slice.read_int_at(index << 2)? as i64),
        8 => slice.read_long_at(index << 3),
        // `read_fields` rejects every other width before an entry is stored.
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid bytes per norm: {bytes_per_norm}"
        ))),
    }
}

/// Dense iterator over norms where every document has the same singleton value.
///
/// Equivalent to the `DenseNormsIterator` with `bytesPerNorm == 0` of
/// `Lucene90NormsProducer.getNorms`, which returns `entry.normsOffset` as the
/// value for every document.
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

/// Dense iterator that reads each norm from the `.nvd` slice on demand.
///
/// Equivalent to the `DenseNormsIterator` subclasses of
/// `Lucene90NormsProducer.getNorms`. Java reads inside `longValue()`; here the
/// read happens in the positioning methods, because [`NumericDocValues`] takes
/// `&self` in `long_value` while [`RandomAccessInput`] needs `&mut self`. The
/// two are observationally identical — nothing can change between positioning
/// and reading — and it keeps the reader lazy: a segment's norms are never
/// materialised in memory, so a corrupt `num_docs_with_field` cannot ask for a
/// multi-gigabyte allocation.
struct DenseNormsIterator {
    max_doc: i32,
    doc: i32,
    slice: Box<dyn RandomAccessInput>,
    bytes_per_norm: i8,
    value: i64,
}

impl fmt::Debug for DenseNormsIterator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DenseNormsIterator")
            .field("max_doc", &self.max_doc)
            .field("doc", &self.doc)
            .field("bytes_per_norm", &self.bytes_per_norm)
            .finish_non_exhaustive()
    }
}

impl DenseNormsIterator {
    fn new(max_doc: i32, slice: Box<dyn RandomAccessInput>, bytes_per_norm: i8) -> Self {
        Self {
            max_doc,
            doc: -1,
            slice,
            bytes_per_norm,
            value: 0,
        }
    }

    fn load(&mut self) -> Result<()> {
        self.value = read_norm_at(self.slice.as_mut(), self.doc, self.bytes_per_norm)?;
        Ok(())
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
            return Ok(self.doc);
        }
        self.doc = target.max(0);
        self.load()?;
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.max_doc as i64
    }
}

impl DocValuesIterator for DenseNormsIterator {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        if target < 0 || target >= self.max_doc {
            return Ok(false);
        }
        self.load()?;
        Ok(true)
    }
}

impl NumericDocValues for DenseNormsIterator {
    fn long_value(&self) -> Result<i64> {
        if self.doc < 0 || self.doc >= self.max_doc {
            return Err(LuceneError::IllegalState(
                "long_value called with no current document".to_string(),
            ));
        }
        Ok(self.value)
    }
}

/// Sparse iterator where every document with a value shares the same value.
///
/// Equivalent to the `SparseNormsIterator` with `bytesPerNorm == 0` of
/// `Lucene90NormsProducer.getNorms`.
struct SparseConstantNormsIterator {
    disi: IndexedDISI,
    value: i64,
    positioned: bool,
}

impl fmt::Debug for SparseConstantNormsIterator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SparseConstantNormsIterator")
            .field("doc", &self.disi.doc_id())
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

impl SparseConstantNormsIterator {
    fn new(disi: IndexedDISI, value: i64) -> Self {
        Self {
            disi,
            value,
            positioned: false,
        }
    }
}

impl DocIdSetIterator for SparseConstantNormsIterator {
    fn doc_id(&self) -> i32 {
        self.disi.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.disi.next_doc()?;
        self.positioned = doc != NO_MORE_DOCS;
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.disi.advance(target)?;
        self.positioned = doc != NO_MORE_DOCS;
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.disi.cost()
    }
}

impl DocValuesIterator for SparseConstantNormsIterator {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.positioned = self.disi.advance_exact(target)?;
        Ok(self.positioned)
    }
}

impl NumericDocValues for SparseConstantNormsIterator {
    fn long_value(&self) -> Result<i64> {
        // `IndexedDISI.advanceExact` sets its current document to the target
        // whether or not that document has a value, so whether this iterator is
        // on a document that has a norm has to be remembered rather than read
        // back off the iterator.
        if !self.positioned {
            return Err(LuceneError::IllegalState(
                "long_value called with no current document".to_string(),
            ));
        }
        Ok(self.value)
    }
}

/// Sparse iterator that reads each norm from the `.nvd` slice on demand,
/// indexed by the ordinal the [`IndexedDISI`] reports.
///
/// Equivalent to the `SparseNormsIterator` subclasses of
/// `Lucene90NormsProducer.getNorms`. See [`DenseNormsIterator`] for why the
/// read happens while positioning.
struct SparseNormsIterator {
    disi: IndexedDISI,
    slice: Box<dyn RandomAccessInput>,
    bytes_per_norm: i8,
    value: i64,
    positioned: bool,
}

impl fmt::Debug for SparseNormsIterator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SparseNormsIterator")
            .field("doc", &self.disi.doc_id())
            .field("bytes_per_norm", &self.bytes_per_norm)
            .finish_non_exhaustive()
    }
}

impl SparseNormsIterator {
    fn new(disi: IndexedDISI, slice: Box<dyn RandomAccessInput>, bytes_per_norm: i8) -> Self {
        Self {
            disi,
            slice,
            bytes_per_norm,
            value: 0,
            positioned: false,
        }
    }

    fn load(&mut self, doc: i32) -> Result<i32> {
        self.positioned = false;
        if doc != NO_MORE_DOCS {
            self.value = read_norm_at(self.slice.as_mut(), self.disi.index(), self.bytes_per_norm)?;
            self.positioned = true;
        }
        Ok(doc)
    }
}

impl DocIdSetIterator for SparseNormsIterator {
    fn doc_id(&self) -> i32 {
        self.disi.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.disi.next_doc()?;
        self.load(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.disi.advance(target)?;
        self.load(doc)
    }

    fn cost(&self) -> i64 {
        self.disi.cost()
    }
}

impl DocValuesIterator for SparseNormsIterator {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.positioned = false;
        if !self.disi.advance_exact(target)? {
            return Ok(false);
        }
        self.value = read_norm_at(self.slice.as_mut(), self.disi.index(), self.bytes_per_norm)?;
        self.positioned = true;
        Ok(true)
    }
}

impl NumericDocValues for SparseNormsIterator {
    fn long_value(&self) -> Result<i64> {
        if !self.positioned {
            return Err(LuceneError::IllegalState(
                "long_value called with no current document".to_string(),
            ));
        }
        Ok(self.value)
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

        let producer = Lucene90NormsFormat::new()
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

        let producer = Lucene90NormsFormat::new()
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

        let producer = Lucene90NormsFormat::new()
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

        let producer = Lucene90NormsFormat::new()
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

        let producer = Lucene90NormsFormat::new()
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
    // -----------------------------------------------------------------------
    // Regression tests
    // -----------------------------------------------------------------------

    /// Reads the metadata entry of the first field out of a written `.nvm`.
    ///
    /// The layout is the one `Lucene90NormsConsumer.addNormsField` writes:
    /// an index header, then `field_number` and seven words per field.
    fn read_first_entry(dir: &RamDirectory, max_doc: i32) -> NormsEntry {
        use crate::store::DataInput;
        let mut input = dir
            .open_input("_0.nvm", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let length = input.length() as usize;
        let mut bytes = vec![0u8; length];
        input.read_bytes(&mut bytes, 0, length).unwrap();
        // Entries end four bytes (the `-1` marker) before the sixteen-byte
        // footer; there is exactly one, thirty-six bytes long.
        let entry_start = length - 16 - 4 - 36;
        let mut reader = crate::store::ByteArrayDataInput::new(bytes[entry_start..].to_vec());
        let _ = max_doc;
        let _field_number = reader.read_int().unwrap();
        NormsEntry {
            docs_with_field_offset: reader.read_long().unwrap(),
            docs_with_field_length: reader.read_long().unwrap(),
            jump_table_entry_count: reader.read_short().unwrap(),
            dense_rank_power: reader.read_byte().unwrap() as i8,
            num_docs_with_field: reader.read_int().unwrap(),
            bytes_per_norm: reader.read_byte().unwrap() as i8,
            norms_offset: reader.read_long().unwrap(),
        }
    }

    fn write_one_field(dir: &RamDirectory, max_doc: i32, values: Vec<(i32, i64)>) -> FieldInfo {
        let info = test_segment_info("_0", max_doc);
        let body = norm_field("body", 0);
        let fis = field_infos(vec![body.clone()]);
        let producer = VecNormsProducer::new(HashMap::from([(0, values)]));
        let mut consumer = Lucene90NormsFormat::new()
            .norms_consumer(&write_state(dir, &info, &fis))
            .unwrap();
        consumer.add_norms_field(&body, &producer).unwrap();
        consumer.close().unwrap();
        body
    }

    #[test]
    fn a_sparse_field_uses_the_indexed_disi_encoding_lucene_expects() {
        // Regression: the first port of this format wrote the docs-with-field
        // set as a plain `FixedBitSet` dump and recorded `jumpTableEntryCount`
        // and `denseRankPower` as `-1`. Both the writer and the reader agreed
        // with each other, so every round-trip test passed — while the files
        // were unreadable by Lucene and Lucene's were unreadable here. The
        // metadata is asserted directly, because it is what names the encoding.
        let dir = RamDirectory::default();
        write_one_field(
            &dir,
            100,
            (0..100).filter(|d| d % 3 == 0).map(|d| (d, 1i64)).collect(),
        );
        let entry = read_first_entry(&dir, 100);
        assert!(
            entry.docs_with_field_offset >= 0,
            "a sparse field must carry a docs-with-field stream"
        );
        assert_eq!(
            entry.dense_rank_power, DEFAULT_DENSE_RANK_POWER,
            "the stream must be written with Lucene's default rank power"
        );
        // `IndexedDISI` skips the jump table when the set spans a single
        // block, so zero is the count Lucene writes here; the point is that it
        // is not the `-1` that means "there is no stream at all".
        assert_eq!(entry.jump_table_entry_count, 0);
        assert_ne!(entry.jump_table_entry_count, NO_JUMP_TABLE);
    }

    #[test]
    fn an_all_documents_field_writes_no_docs_with_field_stream() {
        let dir = RamDirectory::default();
        write_one_field(&dir, 20, (0..20).map(|d| (d, 3i64)).collect());
        let entry = read_first_entry(&dir, 20);
        assert_eq!(entry.docs_with_field_offset, -1);
        assert_eq!(entry.docs_with_field_length, 0);
        assert_eq!(entry.jump_table_entry_count, NO_JUMP_TABLE);
        assert_eq!(entry.dense_rank_power, NO_DENSE_RANK_POWER);
        assert_eq!(entry.num_docs_with_field, 20);
    }

    #[test]
    fn a_field_with_no_documents_writes_the_empty_sentinel() {
        let dir = RamDirectory::default();
        write_one_field(&dir, 20, Vec::new());
        let entry = read_first_entry(&dir, 20);
        assert_eq!(entry.docs_with_field_offset, -2);
        assert_eq!(entry.num_docs_with_field, 0);
        assert_eq!(entry.bytes_per_norm, 0);
        // Java writes the untouched `min`, which is `Long.MAX_VALUE`; nothing
        // reads it, but the byte must be there.
        assert_eq!(entry.norms_offset, i64::MAX);
    }

    #[test]
    fn a_sparse_document_on_a_word_boundary_reads_back() {
        // Regression: the first port derived the ordinal of a sparse document
        // with a popcount that built its mask as `(1 << (bit + 1)) - 1`. For a
        // document whose index is 63 modulo 64 that shift leaves the width of a
        // `u64`, which panics in a debug build and silently yields a mask of
        // zero — and so the wrong value — in a release one. Every document in
        // the segment is placed on such a boundary in turn.
        for boundary in [63, 64, 127, 128, 191, 255] {
            let dir = RamDirectory::default();
            let body = write_one_field(
                &dir,
                300,
                vec![(boundary - 1, 11), (boundary, 22), (boundary + 1, 33)],
            );
            let info = test_segment_info("_0", 300);
            let fis = field_infos(vec![body.clone()]);
            let producer = Lucene90NormsFormat::new()
                .norms_producer(&read_state(&dir, &info, &fis))
                .unwrap();
            let mut values = producer.get_norms(&body).unwrap();
            assert_eq!(
                collect_values(&mut *values, 300)[boundary as usize - 1],
                Some(11)
            );
            let mut values = producer.get_norms(&body).unwrap();
            assert_eq!(
                collect_values(&mut *values, 300)[boundary as usize],
                Some(22)
            );
            let mut values = producer.get_norms(&body).unwrap();
            assert_eq!(
                collect_values(&mut *values, 300)[boundary as usize + 1],
                Some(33)
            );
        }
    }

    #[test]
    fn a_sparse_field_supports_random_access() {
        let dir = RamDirectory::default();
        let body = write_one_field(&dir, 200, vec![(5, -7), (63, 8), (64, 9), (199, -1)]);
        let info = test_segment_info("_0", 200);
        let fis = field_infos(vec![body.clone()]);
        let producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();

        let mut values = producer.get_norms(&body).unwrap();
        assert_eq!(values.advance(6).unwrap(), 63);
        assert_eq!(values.long_value().unwrap(), 8);
        assert_eq!(values.advance(199).unwrap(), 199);
        assert_eq!(values.long_value().unwrap(), -1);
        assert_eq!(values.advance(200).unwrap(), NO_MORE_DOCS);
        assert!(values.long_value().is_err());

        let mut values = producer.get_norms(&body).unwrap();
        assert!(values.advance_exact(5).unwrap());
        assert_eq!(values.long_value().unwrap(), -7);
        assert!(!values.advance_exact(6).unwrap());
        assert!(values.long_value().is_err());
        assert!(values.advance_exact(64).unwrap());
        assert_eq!(values.long_value().unwrap(), 9);
    }

    #[test]
    fn a_dense_field_supports_random_access() {
        let dir = RamDirectory::default();
        let body = write_one_field(&dir, 8, (0..8).map(|d| (d, d as i64 - 4)).collect());
        let info = test_segment_info("_0", 8);
        let fis = field_infos(vec![body.clone()]);
        let producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();

        let mut values = producer.get_norms(&body).unwrap();
        assert_eq!(values.advance(3).unwrap(), 3);
        assert_eq!(values.long_value().unwrap(), -1);
        assert_eq!(values.advance(8).unwrap(), NO_MORE_DOCS);
        assert!(values.long_value().is_err());

        let mut values = producer.get_norms(&body).unwrap();
        assert!(values.advance_exact(7).unwrap());
        assert_eq!(values.long_value().unwrap(), 3);
        assert!(!values.advance_exact(8).unwrap());
    }

    #[test]
    fn a_field_whose_documents_fill_one_indexed_disi_block_reads_back() {
        // More than the 4095 entries `IndexedDISI` stores as shorts, so the
        // block is written as a bitmap plus a rank table. This is the encoding a
        // plain bit-set dump would pass its own round-trip on while being
        // unreadable by Lucene.
        const MAX_DOC: i32 = 4_300;
        let expected: Vec<(i32, i64)> = (0..MAX_DOC)
            .filter(|d| d % 43 != 0)
            .map(|d| (d, 1 + (d as i64 % 11)))
            .collect();
        let dir = RamDirectory::default();
        let body = write_one_field(&dir, MAX_DOC, expected.clone());
        let entry = read_first_entry(&dir, MAX_DOC);
        assert_eq!(entry.num_docs_with_field, expected.len() as i32);
        assert_eq!(entry.dense_rank_power, DEFAULT_DENSE_RANK_POWER);

        let info = test_segment_info("_0", MAX_DOC);
        let fis = field_infos(vec![body.clone()]);
        let producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();
        let mut values = producer.get_norms(&body).unwrap();
        let collected = collect_values(&mut *values, MAX_DOC);
        let mut want = vec![None; MAX_DOC as usize];
        for (doc, value) in &expected {
            want[*doc as usize] = Some(*value);
        }
        assert_eq!(collected, want);
    }

    #[test]
    fn the_merge_instance_reads_the_same_values() {
        let dir = RamDirectory::default();
        let body = write_one_field(
            &dir,
            50,
            (0..50)
                .filter(|d| d % 5 == 1)
                .map(|d| (d, d as i64))
                .collect(),
        );
        let info = test_segment_info("_0", 50);
        let fis = field_infos(vec![body.clone()]);
        let producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();
        let mut direct = producer.get_norms(&body).unwrap();
        let expected = collect_values(&mut *direct, 50);

        let merge = producer.get_merge_instance().unwrap();
        let mut merged = merge.get_norms(&body).unwrap();
        assert_eq!(collect_values(&mut *merged, 50), expected);
    }

    #[test]
    fn the_value_width_follows_the_signed_range_of_the_norms() {
        // `numBytesPerValue` is derived from the signed minimum and maximum, so
        // the norms a real similarity produces — a sign-extended byte — always
        // fit in one byte. Widening only happens when a custom similarity asks
        // for it.
        for (values, expected_width) in [
            (vec![(0, 5i64), (1, 5)], 0),
            (vec![(0, -128i64), (1, 127)], 1),
            (vec![(0, -129i64), (1, 127)], 2),
            (vec![(0, 0i64), (1, 40_000)], 4),
            (vec![(0, 0i64), (1, i64::MAX)], 8),
        ] {
            let dir = RamDirectory::default();
            let body = write_one_field(&dir, 2, values.clone());
            let entry = read_first_entry(&dir, 2);
            assert_eq!(
                entry.bytes_per_norm, expected_width,
                "values {values:?} must need {expected_width} bytes each"
            );
            let info = test_segment_info("_0", 2);
            let fis = field_infos(vec![body.clone()]);
            let producer = Lucene90NormsFormat::new()
                .norms_producer(&read_state(&dir, &info, &fis))
                .unwrap();
            let mut read = producer.get_norms(&body).unwrap();
            let collected = collect_values(&mut *read, 2);
            let mut want = vec![None; 2];
            for (doc, value) in &values {
                want[*doc as usize] = Some(*value);
            }
            assert_eq!(collected, want, "values {values:?} must round-trip");
        }
    }
    /// Rewrites `_0.nvm` with `patch` applied to its body, optionally recomputing
    /// the footer so that the file still passes its checksum.
    fn patch_metadata(
        dir: &RamDirectory,
        resign: bool,
        patch: impl FnOnce(&mut Vec<u8>),
    ) -> RamDirectory {
        let read = |name: &str| {
            let mut input = dir
                .open_input(name, &*crate::store::DEFAULT_IO_CONTEXT)
                .unwrap();
            let length = input.length() as usize;
            let mut bytes = vec![0u8; length];
            input.read_bytes(&mut bytes, 0, length).unwrap();
            bytes
        };
        let data = read("_0.nvd");
        let meta = read("_0.nvm");
        let mut body = meta[..meta.len() - 16].to_vec();
        patch(&mut body);

        let out_dir = RamDirectory::default();
        {
            let mut out = out_dir
                .create_output("_0.nvd", &*crate::store::DEFAULT_IO_CONTEXT)
                .unwrap();
            out.write_bytes(&data, 0, data.len()).unwrap();
            out.close().unwrap();
        }
        {
            let mut out = out_dir
                .create_output("_0.nvm", &*crate::store::DEFAULT_IO_CONTEXT)
                .unwrap();
            out.write_bytes(&body, 0, body.len()).unwrap();
            if resign {
                codec_util::write_footer(out.as_mut()).unwrap();
            } else {
                let footer = &meta[meta.len() - 16..];
                out.write_bytes(footer, 0, footer.len()).unwrap();
            }
            out.close().unwrap();
        }
        out_dir
    }

    /// Byte offset of the `bytes_per_norm` of the only entry, within the body of
    /// a single-field `.nvm`.
    fn bytes_per_norm_offset(body_len: usize) -> usize {
        // `header || field_number(4) || ...(19) || bytes_per_norm(1) || offset(8) || -1(4)`
        body_len - 4 - 8 - 1
    }

    #[test]
    fn an_intact_metadata_file_with_a_wrong_entry_reports_the_entry() {
        // `CodecUtil.checkFooter(in, priorE)` reports the *prior* failure when
        // the checksum passes: the file is exactly what was written, so it is
        // the entry that is wrong, and saying "footer mismatch" would send the
        // operator looking for disk corruption that is not there.
        let dir = RamDirectory::default();
        let body = write_one_field(&dir, 4, vec![(0, 1), (2, 2)]);
        let patched = patch_metadata(&dir, true, |body| {
            let at = bytes_per_norm_offset(body.len());
            body[at] = 3; // not one of 0, 1, 2, 4, 8
        });
        let info = test_segment_info("_0", 4);
        let fis = field_infos(vec![body]);
        let error = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&patched, &info, &fis))
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("Invalid bytesPerNorm"),
            "the entry failure must be the one reported, got: {message}"
        );
    }

    #[test]
    fn a_metadata_file_whose_checksum_fails_reports_the_corruption() {
        // With the footer left alone the same patch makes the file genuinely
        // corrupt, and then the corruption is what explains it — with the entry
        // failure folded in rather than lost.
        let dir = RamDirectory::default();
        let body = write_one_field(&dir, 4, vec![(0, 1), (2, 2)]);
        let patched = patch_metadata(&dir, false, |body| {
            let at = bytes_per_norm_offset(body.len());
            body[at] = 3;
        });
        let info = test_segment_info("_0", 4);
        let fis = field_infos(vec![body]);
        let error = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&patched, &info, &fis))
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("checksum failed"),
            "the corruption must be the one reported, got: {message}"
        );
        assert!(
            message.contains("Invalid bytesPerNorm"),
            "the entry failure must be folded into the message, got: {message}"
        );
    }

    #[test]
    fn a_metadata_file_truncated_into_its_footer_is_indeterminate() {
        // When the entry decoder has already read past the start of the footer
        // there is nothing left to verify, which Java reports as an
        // indeterminate checksum rather than pretending either way.
        let dir = RamDirectory::default();
        let body = write_one_field(&dir, 4, vec![(0, 1), (2, 2)]);
        // Dropping the end-of-metadata marker makes the decoder read on into
        // the footer looking for the next field number.
        let patched = patch_metadata(&dir, true, |body| {
            let len = body.len();
            body.truncate(len - 4);
        });
        let info = test_segment_info("_0", 4);
        let fis = field_infos(vec![body]);
        let error = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&patched, &info, &fis))
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("indeterminate") || message.contains("Invalid field number"),
            "unexpected error: {message}"
        );
    }
    #[test]
    fn an_intact_metadata_file_with_a_wrong_header_reports_the_header() {
        // Java checks the header inside the same `try` as the entries, so a
        // file whose header does not belong to this codec is reported as such
        // rather than as a footer problem.
        let dir = RamDirectory::default();
        let body = write_one_field(&dir, 4, vec![(0, 1), (2, 2)]);
        let patched = patch_metadata(&dir, true, |body| {
            // The codec name follows the four-byte magic and a one-byte length.
            body[9] ^= 0x20;
        });
        let info = test_segment_info("_0", 4);
        let fis = field_infos(vec![body]);
        let error = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&patched, &info, &fis))
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("codec mismatch") || message.contains("codec"),
            "the header failure must be the one reported, got: {message}"
        );
        assert!(
            !message.contains("misplaced codec footer"),
            "the footer must not be blamed, got: {message}"
        );
    }
}
