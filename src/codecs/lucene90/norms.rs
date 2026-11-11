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
use std::sync::{Arc, Mutex, PoisonError};

use crate::codecs::codec_util;
use crate::codecs::lucene90::indexed_disi::{
    create_block_slice, create_jump_table, write_bit_set, IndexedDISI, DEFAULT_DENSE_RANK_POWER,
};
use crate::codecs::norms::{NormsConsumer, NormsFormat, NormsProducer};
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::doc_values::{DocValues, DocValuesIterator, NumericDocValues};
use crate::index::index_file_names::{segment_file_name, NORMS_EXTENSION, NORMS_META_EXTENSION};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{DataInput, IndexInput, IndexOutput, RandomAccessInput};
use crate::util::packed::{
    lock_random_access_input, shared_random_access_input, SharedRandomAccessInput,
};
use crate::util::IOUtils;

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
    /// Set once [`close`](NormsConsumer::close) ran: Java nulls `meta` and
    /// `data` in its `finally` (`Lucene90NormsConsumer.java:86`), so a second
    /// close does nothing.
    closed: bool,
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
    /// Equivalent to `Lucene90NormsConsumer(SegmentWriteState, String, String,
    /// String, String)` (`Lucene90NormsConsumer.java:38-66`): when it fails,
    /// `IOUtils.closeWhileHandlingException(this)` runs `close()` over the
    /// outputs created so far.
    fn new(state: &SegmentWriteState) -> Result<Self> {
        let mut data: Option<Box<dyn IndexOutput>> = None;
        let mut meta: Option<Box<dyn IndexOutput>> = None;
        let opened = (|| -> Result<i32> {
            let data_name = segment_file_name(
                &state.segment_info.name,
                &state.segment_suffix,
                NORMS_EXTENSION,
            );
            let data = data.insert(state.directory.create_output(&data_name, state.context)?);
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
            let meta = meta.insert(state.directory.create_output(&meta_name, state.context)?);
            codec_util::write_index_header(
                meta.as_mut(),
                METADATA_CODEC,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            state.segment_info.max_doc()
        })();
        let max_doc = match opened {
            Ok(max_doc) => max_doc,
            Err(error) => {
                // `IOUtils.closeWhileHandlingException(this)`
                // (`Lucene90NormsConsumer.java:61-65`).
                IOUtils::close_while_handling_exception([Some(|| {
                    Self::close_outputs(data.as_mut(), meta.as_mut())
                })])?;
                return Err(error);
            }
        };
        let expect = "INVARIANT: the constructor created both outputs";
        Ok(Self {
            data: data.expect(expect),
            meta: meta.expect(expect),
            max_doc,
            closed: false,
        })
    }

    /// The body of `Lucene90NormsConsumer.close()`
    /// (`Lucene90NormsConsumer.java:68-88`) over the outputs that are not
    /// null: the EOF marker and the footers, then `IOUtils.close(data, meta)`,
    /// or `IOUtils.closeWhileHandlingException(data, meta)` when writing them
    /// failed.
    fn close_outputs(
        mut data: Option<&mut Box<dyn IndexOutput>>,
        mut meta: Option<&mut Box<dyn IndexOutput>>,
    ) -> Result<()> {
        let written = (|| -> Result<()> {
            if let Some(meta) = meta.as_deref_mut() {
                meta.write_int(-1)?; // write EOF marker
                codec_util::write_footer(meta.as_mut())?; // write checksum
            }
            if let Some(data) = data.as_deref_mut() {
                codec_util::write_footer(data.as_mut())?; // write checksum
            }
            Ok(())
        })();
        let closeables = [data, meta].map(|o| o.map(|o| move || o.close()));
        match written {
            Ok(()) => IOUtils::close(closeables),
            Err(error) => {
                IOUtils::close_while_handling_exception(closeables)?;
                Err(error)
            }
        }
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
            let mut it = crate::codecs::doc_values::non_null(values.get_norms(field)?, field)?;
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
            let mut it = crate::codecs::doc_values::non_null(values.get_norms(field)?, field)?;
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
            let mut it = crate::codecs::doc_values::non_null(values.get_norms(field)?, field)?;
            write_norm_values(it.as_mut(), bytes_per_norm, self.data.as_mut())?;
        }

        Ok(())
    }

    /// Equivalent to `Lucene90NormsConsumer.close()`
    /// (`Lucene90NormsConsumer.java:68-88`): the outputs are nulled in the
    /// `finally` (`:86`), whatever the outcome, so a second call finds them
    /// null and does nothing.
    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        Self::close_outputs(Some(&mut self.data), Some(&mut self.meta))
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
    data: crate::store::SharedIndexInput,
    /// `merging`: set on the instance `getMergeInstance()` returns.
    merging: bool,
    /// `disiInputs`: the docs-with-field inputs a merge instance opened, by
    /// field number.
    disi_inputs: Mutex<HashMap<i32, SharedIndexInput>>,
    /// `disiJumpTables`: the jump tables a merge instance opened, by field
    /// number.
    disi_jump_tables: Mutex<HashMap<i32, SharedRandomAccessInput>>,
    /// `dataInputs`: the values slices a merge instance opened, by field
    /// number.
    data_inputs: Mutex<HashMap<i32, SharedRandomAccessInput>>,
}

/// An `IndexInput` several readers of a merge instance share (Java shares the
/// object; a Rust input reads through `&mut self`, hence the lock).
type SharedIndexInput = Arc<Mutex<Box<dyn IndexInput>>>;

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
        let parsed = codec_util::check_footer_after(meta.as_mut(), parsed);
        // The try-with-resources closes `in` on every exit.
        let version = crate::store::close_after(meta.as_mut(), parsed)?;

        let data_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            NORMS_EXTENSION,
        );
        let mut data = state.directory.open_input(
            &data_name,
            /* withHints, `Lucene90NormsProducer.java:85` */
            state
                .context
                .with_hints(&[std::sync::Arc::new(crate::store::FileTypeHint::Data)])
                .as_ref(),
        )?;
        let checked = (|| -> Result<()> {
            let version2 = codec_util::check_index_header(
                data.as_mut(),
                DATA_CODEC,
                VERSION_START,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            if version != version2 {
                return Err(LuceneError::from(LuceneErrorKind::CorruptIndex(format!(
                    "Format versions mismatch: meta={version}, data={version2}"
                ))));
            }
            // Cheap structural check of the data footer; the actual checksum is
            // verified by `check_integrity` on demand.
            codec_util::retrieve_checksum(data.as_mut())?;
            Ok(())
        })();
        if let Err(error) = checked {
            // `IOUtils.closeWhileHandlingException(this.data)`
            // (`Lucene90NormsProducer.java:108-112`).
            IOUtils::close_while_handling_exception([Some(|| data.close())])?;
            return Err(error);
        }

        Ok(Self {
            norms,
            max_doc,
            data: crate::store::SharedIndexInput::new(data),
            merging: false,
            disi_inputs: Mutex::new(HashMap::new()),
            disi_jump_tables: Mutex::new(HashMap::new()),
            data_inputs: Mutex::new(HashMap::new()),
        })
    }

    /// The instance `getMergeInstance()` returns
    /// (`Lucene90NormsProducer.java:116-130`): a clone sharing the entries,
    /// over a clone of the data input, with empty caches and `merging` set.
    fn merge_instance(&self) -> Result<Self> {
        Ok(Self {
            norms: self.norms.clone(),
            max_doc: self.max_doc,
            data: crate::store::SharedIndexInput::new(self.data.clone_input()?),
            merging: true,
            disi_inputs: Mutex::new(HashMap::new()),
            disi_jump_tables: Mutex::new(HashMap::new()),
            data_inputs: Mutex::new(HashMap::new()),
        })
    }

    /// Opens (or, on a merge instance, reuses) the packed-values slice of
    /// one field.
    ///
    /// Equivalent to `Lucene90NormsProducer.getDataInput`
    /// (`Lucene90NormsProducer.java:246-265`): a merge instance keeps the
    /// slice of each field and hands it out again; a new slice prefetches its
    /// first page.
    fn get_data_input(
        &self,
        field: &FieldInfo,
        entry: &NormsEntry,
    ) -> Result<Box<dyn RandomAccessInput>> {
        let mut slice: Option<SharedRandomAccessInput> = None;
        if self.merging {
            slice = lock(&self.data_inputs).get(&field.number).cloned();
        }
        let slice = match slice {
            Some(slice) => slice,
            None => {
                let slice = self.data_input(entry)?;
                // Prefetch the first page of data. Following pages are
                // expected to get prefetched through read-ahead.
                if slice.length() > 0 {
                    slice.prefetch(0, 1)?;
                }
                if !self.merging {
                    return Ok(slice);
                }
                let slice = shared_random_access_input(slice);
                lock(&self.data_inputs).insert(field.number, Arc::clone(&slice));
                slice
            }
        };
        Ok(Box::new(SharedRandomAccessSlice::new(slice)))
    }

    /// Opens the packed-values slice of one field:
    /// `data.randomAccessSlice(entry.normsOffset, entry.numDocsWithField *
    /// (long) entry.bytesPerNorm)`.
    fn data_input(&self, entry: &NormsEntry) -> Result<Box<dyn RandomAccessInput>> {
        // `read_fields` has already restricted `bytes_per_norm` to 0, 1, 2, 4
        // or 8, so the product of an `i32` and it cannot leave `i64`; what can
        // still arrive off disk is a negative `num_docs_with_field`, which
        // would ask for a slice of negative length. Java lets
        // `randomAccessSlice` refuse that; it is refused here with the numbers
        // that caused it.
        let length = entry.num_docs_with_field as i64 * entry.bytes_per_norm as i64;
        if length < 0 {
            return Err(LuceneError::from(LuceneErrorKind::CorruptIndex(format!(
                "invalid norms slice: numDocsWithField={}, bytesPerNorm={}",
                entry.num_docs_with_field, entry.bytes_per_norm
            ))));
        }
        self.data
            .read()
            .random_access_slice(entry.norms_offset, length)
    }

    /// Opens the data-blocks slice of one sparse field's docs-with-field
    /// stream.
    ///
    /// A merge instance keeps the slice of each field and hands out an
    /// offset-tracking wrapper over it ([`MergingDisiInput`]), so that two
    /// norms instances pulled in parallel do not fight over its file pointer
    /// (`Lucene90NormsProducer.java:267-356`).
    ///
    /// # Errors
    ///
    /// Propagates the error the input raises for a range it cannot serve.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.Lucene90NormsProducer#getDisiInput`.
    fn get_disi_input(&self, field: &FieldInfo, entry: &NormsEntry) -> Result<Box<dyn IndexInput>> {
        if !self.merging {
            return create_block_slice(
                &**self.data.read(),
                "docs",
                entry.docs_with_field_offset,
                entry.docs_with_field_length,
                entry.jump_table_entry_count as i32,
            );
        }

        let mut disi_inputs = lock(&self.disi_inputs);
        let r#in = match disi_inputs.get(&field.number) {
            Some(r#in) => Arc::clone(r#in),
            None => {
                let r#in = Arc::new(Mutex::new(create_block_slice(
                    &**self.data.read(),
                    "docs",
                    entry.docs_with_field_offset,
                    entry.docs_with_field_length,
                    entry.jump_table_entry_count as i32,
                )?));
                disi_inputs.insert(field.number, Arc::clone(&r#in));
                r#in
            }
        };

        // Wrap so that reads can be interleaved from the same thread if two
        // norms instances are pulled and consumed in parallel. Merging usually
        // doesn't need this feature but CheckIndex might, plus we need merge
        // instances to behave well and not be trappy.
        Ok(Box::new(MergingDisiInput { r#in, offset: 0 }))
    }

    /// Opens (or, on a merge instance, reuses) the jump table of one sparse
    /// field's docs-with-field stream, or `None` when it was written without
    /// one (`Lucene90NormsProducer.java:358-375`).
    ///
    /// # Errors
    ///
    /// Propagates the error the input raises for a range it cannot serve.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.Lucene90NormsProducer#getDisiJumpTable`.
    fn get_disi_jump_table(
        &self,
        field: &FieldInfo,
        entry: &NormsEntry,
    ) -> Result<Option<Box<dyn RandomAccessInput>>> {
        let mut jump_table: Option<SharedRandomAccessInput> = None;
        if self.merging {
            jump_table = lock(&self.disi_jump_tables).get(&field.number).cloned();
        }
        let jump_table = match jump_table {
            Some(jump_table) => jump_table,
            None => {
                let Some(jump_table) = create_jump_table(
                    &**self.data.read(),
                    entry.docs_with_field_offset,
                    entry.docs_with_field_length,
                    entry.jump_table_entry_count as i32,
                )?
                else {
                    return Ok(None);
                };
                if !self.merging {
                    return Ok(Some(jump_table));
                }
                let jump_table = shared_random_access_input(jump_table);
                lock(&self.disi_jump_tables).insert(field.number, Arc::clone(&jump_table));
                jump_table
            }
        };
        Ok(Some(Box::new(SharedRandomAccessSlice::new(jump_table))))
    }

    /// Opens the docs-with-field iterator of one sparse field.
    ///
    /// Equivalent to the `IndexedDISI` that `Lucene90NormsProducer.getNorms`
    /// builds on the sparse branch (`Lucene90NormsProducer.java:435-444`), out
    /// of the two slices [`get_disi_input`](Self::get_disi_input) and
    /// [`get_disi_jump_table`](Self::get_disi_jump_table) return.
    fn disi(&self, field: &FieldInfo, entry: &NormsEntry) -> Result<IndexedDISI> {
        let disi_input = self.get_disi_input(field, entry)?;
        let disi_jump_table = self.get_disi_jump_table(field, entry)?;
        IndexedDISI::with_slices(
            disi_input,
            disi_jump_table,
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
            LuceneError::from(LuceneErrorKind::CorruptIndex(format!(
                "Invalid field number: {field_number}"
            )))
        })?;
        if !info.has_norms() {
            return Err(LuceneError::from(LuceneErrorKind::CorruptIndex(format!(
                "Invalid field: {} (no norms)",
                info.name
            ))));
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
                return Err(LuceneError::from(LuceneErrorKind::CorruptIndex(format!(
                    "Invalid bytesPerNorm: {}, field: {}",
                    entry.bytes_per_norm, info.name
                ))))
            }
        }
        entry.norms_offset = meta.read_long()?;
        norms.insert(info.number, entry);
    }
    Ok(())
}

impl Lucene90NormsProducer {
    /// The body of `Lucene90NormsProducer.getNorms(FieldInfo)`.
    fn norms_of(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        let entry = self.norms.get(&field.number).ok_or_else(|| {
            LuceneError::illegal_argument(format!("no norms for field {}", field.name))
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
            let slice = self.get_data_input(field, entry)?;
            return Ok(Box::new(DenseNormsIterator::new(
                self.max_doc,
                slice,
                entry.bytes_per_norm,
            )));
        }

        // Sparse: the docs-with-field set is an `IndexedDISI` stream.
        let disi = self.disi(field, entry)?;
        if entry.bytes_per_norm == 0 {
            return Ok(Box::new(SparseConstantNormsIterator::new(
                disi,
                entry.norms_offset,
            )));
        }
        let slice = self.get_data_input(field, entry)?;
        Ok(Box::new(SparseNormsIterator::new(
            disi,
            slice,
            entry.bytes_per_norm,
        )))
    }
}

impl NormsProducer for Lucene90NormsProducer {
    fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        // Java's body never answers `null`.
        self.norms_of(field).map(Some)
    }

    fn check_integrity(&self) -> Result<()> {
        let mut clone = self.data.clone_input()?;
        codec_util::checksum_entire_file(clone.as_mut())?;
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(self.merge_instance()?))
    }

    fn close(&self) -> Result<()> {
        self.data.close()
    }
}

/// Locks one of a merge instance's caches; a panic while it was held leaves
/// a map that is still whole.
fn lock<T>(cache: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    cache.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A slice a merge instance shares between the readers it hands out: every
/// read goes to the one shared input, as Java's readers share the one
/// `RandomAccessInput` object (`Lucene90NormsProducer.java:246-265, 358-375`).
struct SharedRandomAccessSlice {
    input: SharedRandomAccessInput,
    resource_description: String,
}

impl SharedRandomAccessSlice {
    fn new(input: SharedRandomAccessInput) -> Self {
        let resource_description = lock_random_access_input(&input)
            .resource_description()
            .to_string();
        Self {
            input,
            resource_description,
        }
    }
}

impl DataInput for SharedRandomAccessSlice {
    fn read_byte(&mut self) -> Result<u8> {
        lock_random_access_input(&self.input).read_byte()
    }

    fn to_java_string(&self) -> String {
        self.resource_description.clone()
    }

    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        lock_random_access_input(&self.input).read_bytes(b, offset, len)
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        lock_random_access_input(&self.input).skip_bytes(num_bytes)
    }
}

impl IndexInput for SharedRandomAccessSlice {
    fn close(&mut self) -> Result<()> {
        lock_random_access_input(&self.input).close()
    }

    fn file_pointer(&self) -> i64 {
        lock_random_access_input(&self.input).file_pointer()
    }

    fn length(&self) -> i64 {
        lock_random_access_input(&self.input).length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        lock_random_access_input(&self.input).seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        lock_random_access_input(&self.input).slice(slice_description, offset, length)
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        lock_random_access_input(&self.input).clone_input()
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
        lock_random_access_input(&self.input).prefetch(offset, length)
    }
}

impl RandomAccessInput for SharedRandomAccessSlice {
    fn read_byte_at(&mut self, pos: i64) -> Result<u8> {
        lock_random_access_input(&self.input).read_byte_at(pos)
    }

    fn read_bytes_at(
        &mut self,
        pos: i64,
        bytes: &mut [u8],
        offset: usize,
        len: usize,
    ) -> Result<()> {
        lock_random_access_input(&self.input).read_bytes_at(pos, bytes, offset, len)
    }

    fn read_short_at(&mut self, pos: i64) -> Result<i16> {
        lock_random_access_input(&self.input).read_short_at(pos)
    }

    fn read_int_at(&mut self, pos: i64) -> Result<i32> {
        lock_random_access_input(&self.input).read_int_at(pos)
    }

    fn read_long_at(&mut self, pos: i64) -> Result<i64> {
        lock_random_access_input(&self.input).read_long_at(pos)
    }
}

/// The anonymous `IndexInput("docs")` of
/// `Lucene90NormsProducer.getDisiInput` on a merge instance
/// (`Lucene90NormsProducer.java:290-355`): keeps its own offset over the
/// shared docs-with-field input, seeking it before each read, and supports
/// only what `IndexedDISI` uses.
#[derive(Clone)]
struct MergingDisiInput {
    /// `inF`.
    r#in: SharedIndexInput,
    offset: i64,
}

/// `new UnsupportedOperationException("Unused by IndexedDISI")`.
fn unused_by_indexed_disi<T>() -> Result<T> {
    Err(LuceneError::unsupported_operation(
        "Unused by IndexedDISI".to_string(),
    ))
}

impl MergingDisiInput {
    fn shared(&self) -> std::sync::MutexGuard<'_, Box<dyn IndexInput>> {
        self.r#in.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `inF.seek(offset); offset += n;` then `read` on `inF`.
    fn read_at<T>(
        &mut self,
        n: i64,
        read: impl FnOnce(&mut dyn IndexInput) -> Result<T>,
    ) -> Result<T> {
        let offset = self.offset;
        let shared = Arc::clone(&self.r#in);
        let mut r#in = shared.lock().unwrap_or_else(PoisonError::into_inner);
        r#in.seek(offset)?;
        self.offset += n;
        read(r#in.as_mut())
    }
}

impl DataInput for MergingDisiInput {
    fn read_bytes(&mut self, b: Option<&mut [u8]>, off: usize, len: usize) -> Result<()> {
        self.read_at(len as i64, |r#in| r#in.read_bytes(b, off, len))
    }

    fn read_byte(&mut self) -> Result<u8> {
        unused_by_indexed_disi()
    }

    fn to_java_string(&self) -> String {
        "docs".to_string()
    }

    /// `IndexInput.skipBytes`: `seek(getFilePointer() + numBytes)`.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let skip_to = self.offset + num_bytes;
        self.seek(skip_to)
    }

    fn read_short(&mut self) -> Result<i16> {
        self.read_at(2, |r#in| r#in.read_short())
    }

    fn read_long(&mut self) -> Result<i64> {
        self.read_at(8, |r#in| r#in.read_long())
    }
}

impl IndexInput for MergingDisiInput {
    fn close(&mut self) -> Result<()> {
        unused_by_indexed_disi()
    }

    fn file_pointer(&self) -> i64 {
        self.offset
    }

    fn length(&self) -> i64 {
        self.shared().length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.offset = pos;
        Ok(())
    }

    fn slice(
        &self,
        _slice_description: &str,
        _offset: i64,
        _length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        unused_by_indexed_disi()
    }

    /// Java's `IndexInput.clone()` (`Object.clone()`): a shallow copy over
    /// the same shared input, at the same offset.
    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(self.clone()))
    }

    fn resource_description(&self) -> &str {
        "docs"
    }

    fn prefetch(&self, _offset: i64, _length: i64) -> Result<()> {
        // Not delegating to the wrapped instance on purpose. This is only used for merging.
        Ok(())
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
        _ => Err(LuceneError::from(LuceneErrorKind::CorruptIndex(format!(
            "invalid bytes per norm: {bytes_per_norm}"
        )))),
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
            return Err(LuceneError::illegal_state(
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
            return Err(LuceneError::illegal_state(
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
            return Err(LuceneError::illegal_state(
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
            return Err(LuceneError::illegal_state(
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
    use crate::error::{LuceneError, LuceneErrorKind};
    use crate::index::{DocValuesType, FieldInfo, FieldInfos, IndexOptions};
    use crate::search::DocIdSetIterator;
    use crate::store::{ByteBuffersDirectory, Directory};
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        dir: &'a Arc<dyn Directory>,
        info: &'a crate::index::SegmentInfo,
        fis: &'a FieldInfos,
    ) -> SegmentWriteState<'a> {
        SegmentWriteState::new(
            &crate::util::NoOutputInfoStream,
            Arc::clone(dir),
            info,
            fis,
            Box::leak(Box::new(BufferedUpdates::default())),
            &*crate::store::DEFAULT_IO_CONTEXT,
        )
    }

    fn read_state<'a>(
        dir: &'a Arc<dyn Directory>,
        info: &'a crate::index::SegmentInfo,
        fis: &'a FieldInfos,
    ) -> SegmentReadState<'a> {
        SegmentReadState::new(&**dir, info, fis, &*crate::store::DEFAULT_IO_CONTEXT)
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
                return Err(LuceneError::illegal_state(
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
        fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
            let values: Result<Box<dyn NumericDocValues>> = {
                let docs = self.fields.get(&field.number).cloned().unwrap_or_default();
                Ok(Box::new(VecNumericDocValues::new(docs)))
            };
            Ok(Some(values?))
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
            Ok(Box::new(Self {
                fields: self.fields.clone(),
            }))
        }

        fn close(&self) -> Result<()> {
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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

        let producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();
        let mut values = producer.get_norms(&body).unwrap().unwrap();
        let collected = collect_values(&mut *values, 5);
        assert_eq!(collected, vec![Some(1), Some(2), Some(3), Some(4), Some(5)]);
        producer.check_integrity().unwrap();
        let _merge = producer.get_merge_instance().unwrap();
        producer.close().unwrap();
    }

    /// `close()` nulls `meta` and `data` in its `finally`
    /// (`Lucene90NormsConsumer.java:86`), so the second close of
    /// `AssertingNormsConsumer.close` (`AssertingNormsFormat.java:78-79`) does
    /// nothing: no error, and both files keep the bytes of the first close.
    #[test]
    fn close_twice_is_a_no_op() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let info = test_segment_info("_0", 2);
        let body = norm_field("body", 0);
        let fis = field_infos(vec![body.clone()]);
        let values = VecNormsProducer::new(HashMap::from([(0, vec![(0, 3), (1, 4)])]));
        let mut consumer = Lucene90NormsFormat::new()
            .norms_consumer(&write_state(&dir, &info, &fis))
            .unwrap();
        consumer.add_norms_field(&body, &values).unwrap();
        consumer.close().unwrap();
        let lengths = |dir: &Arc<dyn Directory>| {
            ["_0.nvd", "_0.nvm"].map(|name| dir.file_length(name).unwrap())
        };
        let first = lengths(&dir);
        consumer.close().unwrap();
        assert_eq!(lengths(&dir), first);
        let producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();
        producer.check_integrity().unwrap();
        producer.close().unwrap();
    }

    #[test]
    fn round_trip_sparse_field() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        let mut values = producer.get_norms(&body).unwrap().unwrap();
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        let mut values = producer.get_norms(&body).unwrap().unwrap();
        assert_eq!(values.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_constant_value() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        let mut values = producer.get_norms(&body).unwrap().unwrap();
        let collected = collect_values(&mut *values, 6);
        let expected: Vec<Option<i64>> = (0..6)
            .map(|d| if d % 2 == 0 { Some(42) } else { None })
            .collect();
        assert_eq!(collected, expected);
    }

    #[test]
    fn round_trip_multiple_fields() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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

        let mut body_values = producer.get_norms(&body).unwrap().unwrap();
        assert_eq!(
            collect_values(&mut *body_values, 5),
            vec![Some(1), Some(2), Some(3), Some(4), Some(5)]
        );

        let mut title_values = producer.get_norms(&title).unwrap().unwrap();
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
            let mut it = producer.get_norms(field).unwrap().unwrap();
            let collected = collect_values(&mut *it, 4);
            assert_eq!(
                collected,
                expected.iter().map(|(_, v)| Some(*v)).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn rejects_missing_field_number() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        assert!(matches!(
            err,
            LuceneError {
                kind: LuceneErrorKind::CorruptIndex(_),
                ..
            }
        ));
    }

    #[test]
    fn rejects_field_without_norms() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        assert!(matches!(
            err,
            LuceneError {
                kind: LuceneErrorKind::CorruptIndex(_),
                ..
            }
        ));
    }
    // -----------------------------------------------------------------------
    // Regression tests
    // -----------------------------------------------------------------------

    /// Reads the metadata entry of the first field out of a written `.nvm`.
    ///
    /// The layout is the one `Lucene90NormsConsumer.addNormsField` writes:
    /// an index header, then `field_number` and seven words per field.
    fn read_first_entry(dir: &Arc<dyn Directory>, max_doc: i32) -> NormsEntry {
        use crate::store::DataInput;
        let mut input = dir
            .open_input("_0.nvm", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let length = input.length() as usize;
        let mut bytes = vec![0u8; length];
        input.read_bytes(Some(&mut bytes), 0, length).unwrap();
        // Entries end four bytes (the `-1` marker) before the sixteen-byte
        // footer; there is exactly one, thirty-six bytes long.
        let entry_start = length - 16 - 4 - 36;
        let reader_src = bytes[entry_start..].to_vec();
        let mut reader = crate::store::ByteArrayDataInput::new(&reader_src);
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

    fn write_one_field(
        dir: &Arc<dyn Directory>,
        max_doc: i32,
        values: Vec<(i32, i64)>,
    ) -> FieldInfo {
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
            let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
            let mut values = producer.get_norms(&body).unwrap().unwrap();
            assert_eq!(
                collect_values(&mut *values, 300)[boundary as usize - 1],
                Some(11)
            );
            let mut values = producer.get_norms(&body).unwrap().unwrap();
            assert_eq!(
                collect_values(&mut *values, 300)[boundary as usize],
                Some(22)
            );
            let mut values = producer.get_norms(&body).unwrap().unwrap();
            assert_eq!(
                collect_values(&mut *values, 300)[boundary as usize + 1],
                Some(33)
            );
        }
    }

    #[test]
    fn a_sparse_field_supports_random_access() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let body = write_one_field(&dir, 200, vec![(5, -7), (63, 8), (64, 9), (199, -1)]);
        let info = test_segment_info("_0", 200);
        let fis = field_infos(vec![body.clone()]);
        let producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();

        let mut values = producer.get_norms(&body).unwrap().unwrap();
        assert_eq!(values.advance(6).unwrap(), 63);
        assert_eq!(values.long_value().unwrap(), 8);
        assert_eq!(values.advance(199).unwrap(), 199);
        assert_eq!(values.long_value().unwrap(), -1);
        assert_eq!(values.advance(200).unwrap(), NO_MORE_DOCS);
        assert!(values.long_value().is_err());

        let mut values = producer.get_norms(&body).unwrap().unwrap();
        assert!(values.advance_exact(5).unwrap());
        assert_eq!(values.long_value().unwrap(), -7);
        assert!(!values.advance_exact(6).unwrap());
        assert!(values.long_value().is_err());
        assert!(values.advance_exact(64).unwrap());
        assert_eq!(values.long_value().unwrap(), 9);
    }

    #[test]
    fn a_dense_field_supports_random_access() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let body = write_one_field(&dir, 8, (0..8).map(|d| (d, d as i64 - 4)).collect());
        let info = test_segment_info("_0", 8);
        let fis = field_infos(vec![body.clone()]);
        let producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();

        let mut values = producer.get_norms(&body).unwrap().unwrap();
        assert_eq!(values.advance(3).unwrap(), 3);
        assert_eq!(values.long_value().unwrap(), -1);
        assert_eq!(values.advance(8).unwrap(), NO_MORE_DOCS);
        assert!(values.long_value().is_err());

        let mut values = producer.get_norms(&body).unwrap().unwrap();
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let body = write_one_field(&dir, MAX_DOC, expected.clone());
        let entry = read_first_entry(&dir, MAX_DOC);
        assert_eq!(entry.num_docs_with_field, expected.len() as i32);
        assert_eq!(entry.dense_rank_power, DEFAULT_DENSE_RANK_POWER);

        let info = test_segment_info("_0", MAX_DOC);
        let fis = field_infos(vec![body.clone()]);
        let producer = Lucene90NormsFormat::new()
            .norms_producer(&read_state(&dir, &info, &fis))
            .unwrap();
        let mut values = producer.get_norms(&body).unwrap().unwrap();
        let collected = collect_values(&mut *values, MAX_DOC);
        let mut want = vec![None; MAX_DOC as usize];
        for (doc, value) in &expected {
            want[*doc as usize] = Some(*value);
        }
        assert_eq!(collected, want);
    }

    #[test]
    fn the_merge_instance_reads_the_same_values() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        let mut direct = producer.get_norms(&body).unwrap().unwrap();
        let expected = collect_values(&mut *direct, 50);

        let merge = producer.get_merge_instance().unwrap();
        let mut merged = merge.get_norms(&body).unwrap().unwrap();
        assert_eq!(collect_values(&mut *merged, 50), expected);
    }

    /// rmp #232 (d): the merge instance runs in Java's `merging` mode
    /// (`Lucene90NormsProducer.java:116-130, 246-375`): it opens the
    /// docs-with-field input and the values slice of a field once and serves
    /// every later `getNorms` from them, two iterators pulled in parallel
    /// reading their own documents through the offset-tracking wrapper; the
    /// search instance (the positive control) caches nothing.
    #[test]
    fn the_merge_instance_caches_the_inputs_of_each_field() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let body = write_one_field(
            &dir,
            50,
            (0..50)
                .filter(|d| d % 5 == 1)
                .map(|d| (d, 300 + d as i64))
                .collect(),
        );
        let info = test_segment_info("_0", 50);
        let fis = field_infos(vec![body.clone()]);
        let producer = Lucene90NormsProducer::new(&read_state(&dir, &info, &fis)).unwrap();
        let mut direct = producer.get_norms(&body).unwrap().unwrap();
        let expected = collect_values(&mut *direct, 50);
        assert!(!producer.merging);
        assert!(lock(&producer.disi_inputs).is_empty());
        assert!(lock(&producer.data_inputs).is_empty());

        let merge = producer.merge_instance().unwrap();
        assert!(merge.merging);
        let mut first = merge.get_norms(&body).unwrap().unwrap();
        let mut second = merge.get_norms(&body).unwrap().unwrap();
        assert_eq!(lock(&merge.disi_inputs).len(), 1);
        assert_eq!(lock(&merge.data_inputs).len(), 1);
        // Interleaved: each iterator keeps its own position over the shared
        // inputs.
        assert_eq!(first.next_doc().unwrap(), 1);
        assert_eq!(second.next_doc().unwrap(), 1);
        assert_eq!(second.next_doc().unwrap(), 6);
        assert_eq!(first.long_value().unwrap(), 301);
        assert_eq!(second.long_value().unwrap(), 306);
        assert_eq!(
            collect_values(&mut *merge.get_norms(&body).unwrap().unwrap(), 50),
            expected
        );
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
            let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
            let mut read = producer.get_norms(&body).unwrap().unwrap();
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
        dir: &Arc<dyn Directory>,
        resign: bool,
        patch: impl FnOnce(&mut Vec<u8>),
    ) -> Arc<dyn Directory> {
        let read = |name: &str| {
            let mut input = dir
                .open_input(name, &*crate::store::DEFAULT_IO_CONTEXT)
                .unwrap();
            let length = input.length() as usize;
            let mut bytes = vec![0u8; length];
            input.read_bytes(Some(&mut bytes), 0, length).unwrap();
            bytes
        };
        let data = read("_0.nvd");
        let meta = read("_0.nvm");
        let mut body = meta[..meta.len() - 16].to_vec();
        patch(&mut body);

        let out_dir = ByteBuffersDirectory::default();
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
        Arc::new(out_dir)
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        // corrupt, and then the corruption is what explains it; the entry
        // failure is only suppressed into it (`CodecUtil.java:506-508`), so it
        // is not part of the message.
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
            !message.contains("Invalid bytesPerNorm"),
            "the entry failure is suppressed, not in the message: {message}"
        );
    }

    #[test]
    fn a_metadata_file_truncated_into_its_footer_is_indeterminate() {
        // When the entry decoder has already read past the start of the footer
        // there is nothing left to verify, which Java reports as an
        // indeterminate checksum rather than pretending either way.
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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

impl fmt::Display for Lucene90NormsProducer {
    /// Port of `Lucene90NormsProducer.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Lucene90NormsProducer(fields={})", self.norms.len())
    }
}

/// The `IOUtils` close family at the norms call sites (#308).
#[cfg(test)]
mod close_family_tests {
    use super::{Lucene90NormsConsumer, Lucene90NormsProducer};
    use crate::codecs::norms::NormsConsumer;
    use crate::codecs::tests::{
        read_state, rewrite_file, test_segment_info, tracking_directory, write_state,
    };
    use crate::index::FieldInfos;

    /// A constructor that fails runs `IOUtils.closeWhileHandlingException(this)`
    /// (`Lucene90NormsConsumer.java:61-65`), closing the `.nvd` it created.
    #[test]
    fn a_failed_consumer_constructor_closes_the_data_output() {
        let (dir, _tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        let fis = FieldInfos::default();
        log.fail_open("_0.nvm");
        Lucene90NormsConsumer::new(&write_state(&dir, &info, &fis)).unwrap_err();
        assert_eq!(log.closed(), ["_0.nvd"]);
    }

    /// `close()` ends with `IOUtils.close(data, meta)`
    /// (`Lucene90NormsConsumer.java:80-84`): data first, then meta, whose
    /// failure is raised.
    #[test]
    fn consumer_close_closes_data_then_meta() {
        let (dir, _tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        let fis = FieldInfos::default();
        let mut consumer = Lucene90NormsConsumer::new(&write_state(&dir, &info, &fis)).unwrap();
        log.fail_close("_0.nvm");
        let error = consumer.close().unwrap_err();
        assert!(format!("{error}").contains("_0.nvm"), "{error}");
        assert_eq!(log.closed(), ["_0.nvd", "_0.nvm"]);
    }

    /// A bad `.nvd` header closes the data input:
    /// `IOUtils.closeWhileHandlingException(this.data)`
    /// (`Lucene90NormsProducer.java:108-112`).
    #[test]
    fn a_bad_data_header_closes_the_data_input() {
        let (dir, tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        let fis = FieldInfos::default();
        Lucene90NormsConsumer::new(&write_state(&dir, &info, &fis))
            .unwrap()
            .close()
            .unwrap();
        let before = log.closed().len();
        rewrite_file(&tracking, "_0.nvd", |bytes| bytes[6] ^= 0xff);
        Lucene90NormsProducer::new(&read_state(&*dir, &info, &fis)).unwrap_err();
        // The metadata input is closed first, by its try-with-resources.
        assert_eq!(log.closed()[before..], ["_0.nvm", "_0.nvd"]);
    }
}
