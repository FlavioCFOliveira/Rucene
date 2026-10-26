//! Lucene 9.0 term-vectors format implementation.
//!
//! Ports `Lucene90TermVectorsFormat` and the underlying
//! `Lucene90CompressingTermVectorsFormat` / `Lucene90CompressingTermVectorsReader` /
//! `Lucene90CompressingTermVectorsWriter` classes from Apache Lucene Core 10.5.0.
//!
//! The format writes three files per segment:
//!
//! * `.tvd` – compressed term-vector chunks.
//! * `.tvx` – index mapping documents to chunks.
//! * `.tvm` – metadata (offsets into `.tvx`, number of chunks, etc.).
//!
//! Lucene Core equivalents:
//! * `org.apache.lucene.codecs.lucene90.Lucene90TermVectorsFormat`
//! * `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsFormat`
//! * `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsReader`
//! * `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsWriter`

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::AtomicI32;
use std::sync::{Arc, Mutex};

use crate::codecs::codec_util::{
    check_footer, check_index_header, checksum_entire_file, retrieve_checksum, write_footer,
    write_index_header,
};
use crate::codecs::compressing::{CompressionMode, Compressor, Decompressor, MatchingReaders};
use crate::codecs::lucene90::stored_fields::{FieldsIndex, FieldsIndexReader, FieldsIndexWriter};
use crate::codecs::stub::{FieldInfo, FieldInfos, SegmentInfo};
use crate::codecs::term_vectors::{TermVectorsFormat, TermVectorsReader, TermVectorsWriter};
use crate::error::{LuceneError, Result};
use crate::index::leaf_reader::TermVectors;
use crate::index::postings_enum::{Impacts, ImpactsEnum, ImpactsSource, PostingsEnum};
use crate::index::terms::{Fields, SeekStatus, TermState, Terms, TermsEnum};
use crate::index::{segment_file_name, DocIDMerger, DocIDMergerSub, DocMap, MergeState};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{
    ByteArrayDataInput, ByteBuffersDataOutput, DataInput, DataOutput, Directory, IOContext,
    IndexInput, IndexOutput,
};
use crate::util::attribute::AttributeSource;
use crate::util::byte_block_pool::MAX_TERM_LENGTH;
use crate::util::packed::{
    read_packed_ints_no_header, write_packed_ints_no_header, BlockPackedReaderIterator,
    BlockPackedWriter, DirectReader, DirectWriter, PackedInts,
};
use crate::util::string_helper::StringHelper;
use crate::util::{Accountable, ArrayUtil, BytesRef};

// -----------------------------------------------------------------------------
// Format constants
// -----------------------------------------------------------------------------

/// Extension of the term-vectors data file (`.tvd`).
pub const VECTORS_EXTENSION: &str = "tvd";
/// Extension of the term-vectors index file (`.tvx`).
pub const INDEX_EXTENSION: &str = "tvx";
/// Extension of the term-vectors metadata file (`.tvm`).
pub const META_EXTENSION: &str = "tvm";

/// Codec name written into the `.tvd` header.
pub const VECTORS_CODEC_NAME: &str = "Lucene90TermVectorsData";
/// Codec name written into the `.tvx` header.
pub const INDEX_CODEC_NAME: &str = "Lucene90TermVectorsIndex";
/// Codec name written into the `.tvm` header.
pub const META_CODEC_NAME: &str = "Lucene90TermVectorsMeta";

/// Initial term-vectors format version.
pub const VERSION_START: i32 = 0;
/// Current term-vectors format version.
pub const VERSION_CURRENT: i32 = VERSION_START;

/// Packed block size used by the block-packed integer sequences.
const PACKED_BLOCK_SIZE: usize = 64;

/// Flag: positions are stored.
const POSITIONS: i32 = 0x01;
/// Flag: offsets are stored.
const OFFSETS: i32 = 0x02;
/// Flag: payloads are stored.
const PAYLOADS: i32 = 0x04;

/// Number of bits required to represent the flags.
///
/// Matches `DirectWriter.bitsRequired(POSITIONS | OFFSETS | PAYLOADS)`, which
/// rounds `3` up to the nearest supported packed width (`4`).
const FLAGS_BITS: i32 = 4;

/// Default chunk size in bytes.
const DEFAULT_CHUNK_SIZE: i32 = 1 << 12;
/// Default maximum number of documents per chunk.
const DEFAULT_MAX_DOCS_PER_CHUNK: i32 = 128;
/// Default block shift for the chunk index.
const DEFAULT_BLOCK_SHIFT: i32 = 10;

// -----------------------------------------------------------------------------
// Term vectors format
// -----------------------------------------------------------------------------

/// Lucene 9.0 term-vectors format.
///
/// Lucene Core equivalent:
/// `org.apache.lucene.codecs.lucene90.Lucene90TermVectorsFormat`.
#[derive(Debug, Default, Clone)]
pub struct Lucene90TermVectorsFormat;

impl Lucene90TermVectorsFormat {
    /// Creates the format.
    pub fn new() -> Self {
        Self
    }
}

impl TermVectorsFormat for Lucene90TermVectorsFormat {
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn vectors_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>> {
        Lucene90CompressingTermVectorsFormat::new(
            VECTORS_CODEC_NAME,
            "",
            CompressionMode::FAST,
            DEFAULT_CHUNK_SIZE,
            DEFAULT_MAX_DOCS_PER_CHUNK,
            DEFAULT_BLOCK_SHIFT,
        )?
        .vectors_reader(directory, segment_info, field_infos, context)
    }

    fn vectors_writer(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>> {
        Lucene90CompressingTermVectorsFormat::new(
            VECTORS_CODEC_NAME,
            "",
            CompressionMode::FAST,
            DEFAULT_CHUNK_SIZE,
            DEFAULT_MAX_DOCS_PER_CHUNK,
            DEFAULT_BLOCK_SHIFT,
        )?
        .vectors_writer(directory, segment_info, context)
    }
}

// -----------------------------------------------------------------------------
// Compressing term vectors format
// -----------------------------------------------------------------------------

/// Compressing term-vectors format used by `Lucene90TermVectorsFormat`.
///
/// Lucene Core equivalent:
/// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsFormat`.
#[derive(Debug, Clone)]
pub struct Lucene90CompressingTermVectorsFormat {
    format_name: String,
    segment_suffix: String,
    compression_mode: CompressionMode,
    chunk_size: i32,
    max_docs_per_chunk: i32,
    block_shift: i32,
}

impl Lucene90CompressingTermVectorsFormat {
    /// Creates the format with the given compression mode and chunk size.
    pub fn new(
        format_name: &str,
        segment_suffix: &str,
        compression_mode: CompressionMode,
        chunk_size: i32,
        max_docs_per_chunk: i32,
        block_shift: i32,
    ) -> Result<Self> {
        if chunk_size < 1 {
            return Err(LuceneError::IllegalArgument(
                "chunkSize must be >= 1".to_string(),
            ));
        }
        if max_docs_per_chunk < 1 {
            return Err(LuceneError::IllegalArgument(
                "maxDocsPerChunk must be >= 1".to_string(),
            ));
        }
        if block_shift < 1 {
            return Err(LuceneError::IllegalArgument(
                "blockShift must be >= 1".to_string(),
            ));
        }
        Ok(Self {
            format_name: format_name.to_string(),
            segment_suffix: segment_suffix.to_string(),
            compression_mode,
            chunk_size,
            max_docs_per_chunk,
            block_shift,
        })
    }
}

impl TermVectorsFormat for Lucene90CompressingTermVectorsFormat {
    fn name(&self) -> &str {
        "Lucene90TermVectors"
    }

    fn vectors_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>> {
        Ok(Box::new(Lucene90CompressingTermVectorsReader::new(
            directory,
            segment_info,
            &self.segment_suffix,
            field_infos,
            context,
            &self.format_name,
            self.compression_mode,
            self.chunk_size,
            self.block_shift,
        )?))
    }

    fn vectors_writer(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>> {
        Ok(Box::new(Lucene90CompressingTermVectorsWriter::new(
            directory,
            segment_info,
            &self.segment_suffix,
            context,
            &self.format_name,
            self.compression_mode,
            self.chunk_size,
            self.max_docs_per_chunk,
            self.block_shift,
        )?))
    }
}

// -----------------------------------------------------------------------------
// Writer: in-memory buffered field/term/position state
// -----------------------------------------------------------------------------

struct FieldData {
    field_num: i32,
    num_terms: i32,
    has_positions: bool,
    has_offsets: bool,
    has_payloads: bool,
    flags: i32,
    freqs: Vec<i32>,
    prefix_lengths: Vec<i32>,
    suffix_lengths: Vec<i32>,
    pos_start: i32,
    off_start: i32,
    pay_start: i32,
    total_positions: i32,
    ord: i32,
}

impl FieldData {
    #[allow(clippy::too_many_arguments)]
    fn new(
        field_num: i32,
        num_terms: i32,
        has_positions: bool,
        has_offsets: bool,
        has_payloads: bool,
        pos_start: i32,
        off_start: i32,
        pay_start: i32,
    ) -> Self {
        Self {
            field_num,
            num_terms,
            has_positions,
            has_offsets,
            has_payloads,
            flags: (if has_positions { POSITIONS } else { 0 })
                | (if has_offsets { OFFSETS } else { 0 })
                | (if has_payloads { PAYLOADS } else { 0 }),
            freqs: vec![0; num_terms as usize],
            prefix_lengths: vec![0; num_terms as usize],
            suffix_lengths: vec![0; num_terms as usize],
            pos_start,
            off_start,
            pay_start,
            total_positions: 0,
            ord: 0,
        }
    }

    fn add_term(&mut self, freq: i32, prefix_length: i32, suffix_length: i32) {
        self.freqs[self.ord as usize] = freq;
        self.prefix_lengths[self.ord as usize] = prefix_length;
        self.suffix_lengths[self.ord as usize] = suffix_length;
        self.ord += 1;
    }
}

struct DocData {
    num_fields: i32,
    fields: Vec<FieldData>,
    pos_start: i32,
    off_start: i32,
    pay_start: i32,
}

impl DocData {
    fn new(num_fields: i32, pos_start: i32, off_start: i32, pay_start: i32) -> Self {
        Self {
            num_fields,
            fields: Vec::with_capacity(num_fields as usize),
            pos_start,
            off_start,
            pay_start,
        }
    }

    fn add_field(
        &mut self,
        field_num: i32,
        num_terms: i32,
        has_positions: bool,
        has_offsets: bool,
        has_payloads: bool,
    ) -> &mut FieldData {
        let pos_start = if let Some(last) = self.fields.last() {
            last.pos_start
                + (if last.has_positions {
                    last.total_positions
                } else {
                    0
                })
        } else {
            self.pos_start
        };
        let off_start = if let Some(last) = self.fields.last() {
            last.off_start
                + (if last.has_offsets {
                    last.total_positions
                } else {
                    0
                })
        } else {
            self.off_start
        };
        let pay_start = if let Some(last) = self.fields.last() {
            last.pay_start
                + (if last.has_payloads {
                    last.total_positions
                } else {
                    0
                })
        } else {
            self.pay_start
        };
        self.fields.push(FieldData::new(
            field_num,
            num_terms,
            has_positions,
            has_offsets,
            has_payloads,
            pos_start,
            off_start,
            pay_start,
        ));
        self.fields.last_mut().unwrap()
    }
}

// -----------------------------------------------------------------------------
// Term vectors writer
// -----------------------------------------------------------------------------

/// Writer for the compressing term-vectors format.
///
/// Lucene Core equivalent:
/// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsWriter`.
pub struct Lucene90CompressingTermVectorsWriter {
    segment: String,
    index_writer: FieldsIndexWriter,
    meta_stream: Box<dyn IndexOutput>,
    vectors_stream: Box<dyn IndexOutput>,
    compressor: Box<dyn Compressor>,
    compression_mode: CompressionMode,
    chunk_size: usize,
    max_docs_per_chunk: usize,
    num_docs: i32,
    pending_docs: Vec<DocData>,
    cur_doc: Option<DocData>,
    cur_field: Option<usize>,
    term_suffixes: ByteBuffersDataOutput,
    payload_bytes: ByteBuffersDataOutput,
    /// The buffer every packed block is assembled in before being copied to the
    /// `.tvd` stream.
    ///
    /// Equivalent to Lucene's `private final ByteBuffersDataOutput
    /// scratchBuffer = ByteBuffersDataOutput.newResettableInstance()`: one
    /// instance, reset before each use, so its blocks are recycled instead of
    /// reallocated. It is also one of the terms of
    /// [`ram_bytes_used`](Accountable::ram_bytes_used).
    scratch_buffer: ByteBuffersDataOutput,
    last_term: BytesRef,
    positions_buf: Vec<i32>,
    start_offsets_buf: Vec<i32>,
    lengths_buf: Vec<i32>,
    payload_lengths_buf: Vec<i32>,
    num_chunks: i64,
    num_dirty_chunks: i64,
    num_dirty_docs: i64,
    finished: bool,
}

// bulk merge is scary: it has caused corruption bugs in the past.
// we try to be extra safe with this impl, but keep an escape hatch to work
// around undiscovered bugs.
//
// Divergence: Lucene reads the switch from the system property named by
// `BULK_MERGE_ENABLED_SYSPROP`
// (`Lucene90CompressingTermVectorsWriter.java:795-807`). Rust has no system
// properties, and reading an environment variable at first use would give the
// constant a different lifetime than Java's static initialiser, so the escape
// hatch is a compile-time constant with Java's default value.
const BULK_MERGE_ENABLED: bool = true;

/// One source segment of a term-vectors merge, together with whether its chunks
/// can be copied verbatim.
///
/// Equivalent to the private class
/// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsWriter.CompressingTermVectorsSub`.
struct CompressingTermVectorsSub {
    doc_map: DocMap,
    max_doc: i32,
    reader_index: usize,
    can_perform_bulk_merge: bool,
    doc_id: i32,
    mapped_doc_id: i32,
}

impl DocIDMergerSub for CompressingTermVectorsSub {
    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id += 1;
        if self.doc_id == self.max_doc {
            Ok(NO_MORE_DOCS)
        } else {
            Ok(self.doc_id)
        }
    }

    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }

    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }

    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// What one step of `DocIDMerger` tells the merge loop.
///
/// Java holds the `CompressingTermVectorsSub` that `DocIDMerger.next()` returns
/// and compares consecutive steps with `==`, that is by object identity, to
/// find the run of documents that one segment contributes in a row
/// (`Lucene90CompressingTermVectorsWriter.java:918-928`). A `&mut` reference
/// cannot be held across the next call here, so the loop keeps the sub's
/// position in the merger — which is exactly its identity — together with the
/// fields it reads.
#[derive(Debug, Clone, Copy)]
struct TermVectorsMergeStep {
    /// Position of the sub inside the merger; stands in for Java's identity
    /// comparison of the sub itself.
    sub_index: usize,
    reader_index: usize,
    doc_id: i32,
    can_perform_bulk_merge: bool,
    mapped_doc_id: i32,
}

/// Advances the merger one document and reports what it landed on, or `None`
/// once every source segment is exhausted.
///
/// Equivalent to `DocIDMerger.next()` as
/// `Lucene90CompressingTermVectorsWriter#merge` uses it
/// (`Lucene90CompressingTermVectorsWriter.java:915`, `922`, `934`).
fn next_term_vectors_step(
    merger: &mut DocIDMerger<CompressingTermVectorsSub>,
) -> Result<Option<TermVectorsMergeStep>> {
    if merger.next_sub()?.is_none() {
        return Ok(None);
    }
    let sub_index = merger.current_index().ok_or_else(|| {
        LuceneError::IllegalState("DocIDMerger returned a sub it cannot locate".to_string())
    })?;
    let sub = merger.sub(sub_index);
    Ok(Some(TermVectorsMergeStep {
        sub_index,
        reader_index: sub.reader_index,
        doc_id: sub.doc_id,
        can_perform_bulk_merge: sub.can_perform_bulk_merge,
        mapped_doc_id: sub.mapped_doc_id,
    }))
}

impl Lucene90CompressingTermVectorsWriter {
    #[allow(clippy::too_many_arguments)]
    fn new(
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        context: &dyn IOContext,
        format_name: &str,
        compression_mode: CompressionMode,
        chunk_size: i32,
        max_docs_per_chunk: i32,
        block_shift: i32,
    ) -> Result<Self> {
        let segment = segment_info.name.clone();
        let id = segment_info.id();

        let meta_file = segment_file_name(&segment, segment_suffix, META_EXTENSION);
        let mut meta_stream = directory.create_output(&meta_file, context)?;
        write_index_header(
            meta_stream.as_mut(),
            &format!("{INDEX_CODEC_NAME}Meta"),
            VERSION_CURRENT,
            &id,
            segment_suffix,
        )?;

        let vectors_file = segment_file_name(&segment, segment_suffix, VECTORS_EXTENSION);
        let mut vectors_stream = directory.create_output(&vectors_file, context)?;
        write_index_header(
            vectors_stream.as_mut(),
            format_name,
            VERSION_CURRENT,
            &id,
            segment_suffix,
        )?;

        let index_writer = FieldsIndexWriter::new(
            directory,
            &segment,
            segment_suffix,
            INDEX_EXTENSION,
            INDEX_CODEC_NAME,
            id,
            block_shift,
            context,
        )?;

        meta_stream.write_v_int(PackedInts::VERSION_CURRENT)?;
        meta_stream.write_v_int(chunk_size)?;

        Ok(Self {
            segment,
            index_writer,
            meta_stream,
            vectors_stream,
            compressor: compression_mode.new_compressor(),
            compression_mode,
            chunk_size: chunk_size as usize,
            max_docs_per_chunk: max_docs_per_chunk as usize,
            num_docs: 0,
            pending_docs: Vec::new(),
            cur_doc: None,
            cur_field: None,
            term_suffixes: ByteBuffersDataOutput::new_resettable_instance(),
            payload_bytes: ByteBuffersDataOutput::new_resettable_instance(),
            scratch_buffer: ByteBuffersDataOutput::new_resettable_instance(),
            // Java is `new BytesRef(ArrayUtil.oversize(30, 1))`: a buffer of
            // that many bytes with `length` zero, not a capacity hint. The
            // buffer length is what `ramBytesUsed` reports, so it has to be a
            // real allocation of the same size.
            last_term: BytesRef {
                bytes: vec![0u8; ArrayUtil::oversize(30, 1)],
                offset: 0,
                length: 0,
            },
            positions_buf: vec![0; 1024],
            start_offsets_buf: vec![0; 1024],
            lengths_buf: vec![0; 1024],
            payload_lengths_buf: vec![0; 1024],
            num_chunks: 0,
            num_dirty_chunks: 0,
            num_dirty_docs: 0,
            finished: false,
        })
    }

    fn trigger_flush(&self) -> bool {
        self.term_suffixes.size() >= self.chunk_size
            || self.pending_docs.len() >= self.max_docs_per_chunk
    }

    fn add_doc_data(&mut self, num_vector_fields: i32) -> &mut DocData {
        let (pos_start, off_start, pay_start) = self
            .pending_docs
            .iter()
            .rev()
            .find_map(|doc| {
                doc.fields.last().map(|last| {
                    (
                        last.pos_start
                            + (if last.has_positions {
                                last.total_positions
                            } else {
                                0
                            }),
                        last.off_start
                            + (if last.has_offsets {
                                last.total_positions
                            } else {
                                0
                            }),
                        last.pay_start
                            + (if last.has_payloads {
                                last.total_positions
                            } else {
                                0
                            }),
                    )
                })
            })
            .unwrap_or((0, 0, 0));
        self.pending_docs.push(DocData::new(
            num_vector_fields,
            pos_start,
            off_start,
            pay_start,
        ));
        self.pending_docs.last_mut().unwrap()
    }

    /// Returns whether `candidate` should be recompressed rather than
    /// bulk-merged.
    ///
    /// The last chunk written for a segment is typically incomplete, so without
    /// recompressing, in some worst-case situations (frequent reopens with tiny
    /// flushes) the compression ratio degrades over time. A segment counts as
    /// dirty only when it has enough dirty documents to fill a whole block
    /// *and* more than one per cent of its blocks are dirty.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsWriter#tooDirty`.
    fn too_dirty(&self, candidate: &Lucene90CompressingTermVectorsReader) -> bool {
        candidate.num_dirty_docs() > self.max_docs_per_chunk as i64
            && candidate.num_dirty_chunks() * 100 > candidate.num_chunks()
    }

    /// Returns whether one source segment's term vectors can be copied chunk by
    /// chunk instead of being decoded and re-encoded.
    ///
    /// That is only possible when the reader is one of this format's own, was
    /// written with the same version, compression mode and chunk size, carries
    /// no deletions, and is not so fragmented that recompressing it is worth
    /// the cost.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsWriter#canPerformBulkMerge`.
    fn can_perform_bulk_merge(
        &self,
        merge_state: &MergeState,
        matching_readers: &MatchingReaders,
        reader_index: usize,
    ) -> bool {
        let candidate = merge_state.term_vectors_readers[reader_index]
            .as_ref()
            .and_then(|reader| reader.as_any())
            .and_then(|any| any.downcast_ref::<Lucene90CompressingTermVectorsReader>());
        match candidate {
            Some(reader) => {
                BULK_MERGE_ENABLED
                    && matching_readers.matching_readers[reader_index]
                    && reader.compression_mode() == self.compression_mode
                    && reader.chunk_size() == self.chunk_size as i32
                    && reader.version() == VERSION_CURRENT
                    && reader.packed_ints_version() == PackedInts::VERSION_CURRENT
                    && merge_state.live_docs[reader_index].is_none()
                    && !self.too_dirty(reader)
            }
            None => false,
        }
    }

    /// Copies documents `[from_doc_id, to_doc_id)` of one source segment,
    /// moving whole compressed chunks byte for byte wherever a chunk boundary
    /// allows it and re-encoding the documents at either end that do not line
    /// up with one.
    ///
    /// Only the chunk header is rewritten, to rebase the document numbers onto
    /// the merged segment; the compressed payload is copied verbatim, which is
    /// what makes a bulk merge cheap.
    ///
    /// The documents are read through the merge state's own reader, never a
    /// clone, exactly as Java casts `mergeState.termVectorsReaders[readerIndex]`
    /// (`Lucene90CompressingTermVectorsWriter.java:815-816`): the chunk that
    /// reader last decoded ([`is_loaded`](Lucene90CompressingTermVectorsReader::is_loaded))
    /// must survive from one call to the next. An index-sorted merge calls this
    /// once per run of consecutive documents, and a run that starts inside the
    /// chunk the previous run ended in must copy those documents one by one; a
    /// fresh clone has forgotten that chunk, seeks to its start and fails the
    /// `base != docID` check.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when a chunk header disagrees with
    /// the term-vectors index, and propagates any I/O error.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsWriter#copyChunks`
    /// (`Lucene90CompressingTermVectorsWriter.java:809-896`).
    fn copy_chunks(
        &mut self,
        merge_state: &MergeState,
        sub: &CompressingTermVectorsSub,
        from_doc_id: i32,
        to_doc_id: i32,
    ) -> Result<()> {
        let reader = merge_state.term_vectors_readers[sub.reader_index]
            .as_ref()
            .and_then(|reader| reader.as_any())
            .and_then(|any| any.downcast_ref::<Lucene90CompressingTermVectorsReader>())
            .ok_or_else(|| {
                LuceneError::IllegalState(
                    "bulk merge needs a Lucene90CompressingTermVectorsReader".to_string(),
                )
            })?;
        debug_assert_eq!(reader.version(), VERSION_CURRENT);
        debug_assert_eq!(reader.chunk_size(), self.chunk_size as i32);
        debug_assert_eq!(reader.compression_mode(), self.compression_mode);
        debug_assert!(!self.too_dirty(reader));
        debug_assert!(merge_state.live_docs[sub.reader_index].is_none());

        let mut doc_id = from_doc_id;

        // copy docs that belong to the previous chunk
        while doc_id < to_doc_id && reader.is_loaded(doc_id) {
            let vectors = reader.get(doc_id)?;
            self.add_all_doc_vectors(vectors.as_deref(), &merge_state.merge_field_infos)?;
            doc_id += 1;
        }

        if doc_id >= to_doc_id {
            return Ok(());
        }
        // copy chunks
        let mut from_pointer = reader.index_reader().get_start_pointer(doc_id)?;
        let to_pointer = if to_doc_id == sub.max_doc {
            reader.max_pointer()
        } else {
            reader.index_reader().get_start_pointer(to_doc_id)?
        };
        if from_pointer < to_pointer {
            // flush any pending chunks
            if !self.pending_docs.is_empty() {
                self.flush(true)?;
            }
            let mut raw_docs = reader.vectors_stream()?;
            raw_docs.seek(from_pointer)?;
            loop {
                // iterate over each chunk. we use the vectors index to find chunk
                // boundaries, read the docstart + doccount from the chunk header (we
                // write a new header, since doc numbers will change), and just copy the
                // bytes directly.
                // read header
                let base = raw_docs.read_v_int()?;
                if base != doc_id {
                    return Err(LuceneError::CorruptIndex(format!(
                        "invalid state: base={base}, docID={doc_id}"
                    )));
                }

                let code = raw_docs.read_v_int()?;
                let buffered_docs = ((code as u32) >> 1) as i32;

                // write a new index entry and new header for this chunk.
                self.index_writer
                    .write_index(buffered_docs, self.vectors_stream.file_pointer())?;
                self.vectors_stream.write_v_int(self.num_docs)?; // rebase
                self.vectors_stream.write_v_int(code)?;
                doc_id += buffered_docs;
                self.num_docs += buffered_docs;
                if doc_id > to_doc_id {
                    return Err(LuceneError::CorruptIndex(format!(
                        "invalid state: base={base}, count={buffered_docs}, toDocID={to_doc_id}"
                    )));
                }

                // copy bytes until the next chunk boundary (or end of chunk data).
                // using the term vectors index for this isn't the most efficient, but
                // fast enough and is a source of redundancy for detecting bad things.
                let end = if doc_id == sub.max_doc {
                    reader.max_pointer()
                } else {
                    reader.index_reader().get_start_pointer(doc_id)?
                };
                let to_copy = end - raw_docs.file_pointer();
                self.vectors_stream.copy_bytes(raw_docs.as_mut(), to_copy)?;
                self.num_chunks += 1;
                let dirty_chunk = (code & 1) != 0;
                if dirty_chunk {
                    self.num_dirty_chunks += 1;
                    self.num_dirty_docs += i64::from(buffered_docs);
                }
                from_pointer = end;
                if from_pointer >= to_pointer {
                    break;
                }
            }
        }
        // copy leftover docs that don't form a complete chunk
        debug_assert!(!reader.is_loaded(doc_id));
        while doc_id < to_doc_id {
            let vectors = reader.get(doc_id)?;
            self.add_all_doc_vectors(vectors.as_deref(), &merge_state.merge_field_infos)?;
            doc_id += 1;
        }
        Ok(())
    }

    fn flush(&mut self, force: bool) -> Result<()> {
        debug_assert!(force || self.trigger_flush());
        let chunk_docs = self.pending_docs.len();
        debug_assert!(chunk_docs > 0);
        self.num_chunks += 1;
        if force {
            self.num_dirty_chunks += 1;
            self.num_dirty_docs += chunk_docs as i64;
        }

        let start_pointer = self.vectors_stream.file_pointer();
        self.index_writer
            .write_index(chunk_docs as i32, start_pointer)?;

        let doc_base = self.num_docs - chunk_docs as i32;
        self.vectors_stream.write_v_int(doc_base)?;
        let dirty_bit = if force { 1 } else { 0 };
        self.vectors_stream
            .write_v_int(((chunk_docs as i32) << 1) | dirty_bit)?;

        let total_fields = self.flush_num_fields(chunk_docs as i32)?;

        if total_fields > 0 {
            let field_nums = self.flush_field_nums()?;
            self.flush_fields(total_fields, &field_nums)?;
            self.flush_flags(total_fields, &field_nums)?;
            self.flush_num_terms(total_fields)?;
            self.flush_term_lengths()?;
            self.flush_term_freqs()?;
            self.flush_positions()?;
            self.flush_offsets(&field_nums)?;
            self.flush_payload_lengths()?;

            self.compressor
                .compress(&self.term_suffixes, self.vectors_stream.as_mut())?;
        }

        self.pending_docs.clear();
        self.cur_doc = None;
        self.cur_field = None;
        self.term_suffixes.reset();
        Ok(())
    }

    fn flush_num_fields(&mut self, chunk_docs: i32) -> Result<i32> {
        if chunk_docs == 1 {
            let num_fields = self.pending_docs[0].num_fields;
            self.vectors_stream.write_v_int(num_fields)?;
            Ok(num_fields)
        } else {
            let mut writer =
                BlockPackedWriter::new(self.vectors_stream.as_mut(), PACKED_BLOCK_SIZE)?;
            let mut total_fields = 0i32;
            for doc in &self.pending_docs {
                writer.add(doc.num_fields as i64)?;
                total_fields += doc.num_fields;
            }
            writer.finish()?;
            Ok(total_fields)
        }
    }

    fn flush_field_nums(&mut self) -> Result<Vec<i32>> {
        let mut field_nums_set = HashSet::new();
        for doc in &self.pending_docs {
            for field in &doc.fields {
                field_nums_set.insert(field.field_num);
            }
        }
        let mut field_nums: Vec<i32> = field_nums_set.into_iter().collect();
        field_nums.sort();

        let num_distinct_fields = field_nums.len();
        debug_assert!(num_distinct_fields > 0);
        // `PackedInts.bitsRequired`, not `DirectWriter.bitsRequired`
        // (`Lucene90CompressingTermVectorsWriter.java:467`): the field numbers
        // are written with `PackedInts.Format.PACKED`, which accepts any width,
        // so Lucene stores the exact number of bits. `DirectWriter` rounds up
        // to its own supported set — 3 becomes 4 — which still round-trips but
        // writes different bytes and a longer file than Lucene's.
        let bits_required = PackedInts::bits_required(field_nums[num_distinct_fields - 1] as i64)?;
        let token = (((num_distinct_fields - 1).min(0x07) as i32) << 5) | bits_required;
        self.vectors_stream.write_byte(token as u8)?;
        if num_distinct_fields > 0x07 {
            self.vectors_stream
                .write_v_int((num_distinct_fields - 1 - 0x07) as i32)?;
        }

        write_packed_ints_no_header(
            self.vectors_stream.as_mut(),
            &field_nums.iter().map(|&v| v as i64).collect::<Vec<_>>(),
            bits_required,
        )?;

        Ok(field_nums)
    }

    fn flush_fields(&mut self, total_fields: i32, field_nums: &[i32]) -> Result<()> {
        self.scratch_buffer.reset();
        let bits_per_off = DirectWriter::bits_required((field_nums.len() - 1) as i64);
        let mut writer =
            DirectWriter::new(&mut self.scratch_buffer, total_fields as i64, bits_per_off)?;
        for doc in &self.pending_docs {
            for field in &doc.fields {
                let idx = field_nums.binary_search(&field.field_num).unwrap();
                writer.add(idx as i64)?;
            }
        }
        writer.finish()?;
        let bytes = self.scratch_buffer.to_array_copy();
        self.vectors_stream.write_v_long(bytes.len() as i64)?;
        self.vectors_stream.write_bytes(&bytes, 0, bytes.len())?;
        Ok(())
    }

    fn flush_flags(&mut self, total_fields: i32, field_nums: &[i32]) -> Result<()> {
        let mut field_flags = vec![-1i32; field_nums.len()];
        let mut non_changing_flags = true;
        'outer: for doc in &self.pending_docs {
            for field in &doc.fields {
                let idx = field_nums.binary_search(&field.field_num).unwrap();
                if field_flags[idx] == -1 {
                    field_flags[idx] = field.flags;
                } else if field_flags[idx] != field.flags {
                    non_changing_flags = false;
                    break 'outer;
                }
            }
        }

        if non_changing_flags {
            self.vectors_stream.write_v_int(0)?;
            self.scratch_buffer.reset();
            let mut writer = DirectWriter::new(
                &mut self.scratch_buffer,
                field_flags.len() as i64,
                FLAGS_BITS,
            )?;
            for flags in &field_flags {
                writer.add(*flags as i64)?;
            }
            writer.finish()?;
            let bytes = self.scratch_buffer.to_array_copy();
            self.vectors_stream.write_v_int(bytes.len() as i32)?;
            self.vectors_stream.write_bytes(&bytes, 0, bytes.len())?;
        } else {
            self.vectors_stream.write_v_int(1)?;
            self.scratch_buffer.reset();
            let mut writer =
                DirectWriter::new(&mut self.scratch_buffer, total_fields as i64, FLAGS_BITS)?;
            for doc in &self.pending_docs {
                for field in &doc.fields {
                    writer.add(field.flags as i64)?;
                }
            }
            writer.finish()?;
            let bytes = self.scratch_buffer.to_array_copy();
            self.vectors_stream.write_v_int(bytes.len() as i32)?;
            self.vectors_stream.write_bytes(&bytes, 0, bytes.len())?;
        }
        Ok(())
    }

    fn flush_num_terms(&mut self, total_fields: i32) -> Result<()> {
        let mut max_num_terms = 0i32;
        for doc in &self.pending_docs {
            for field in &doc.fields {
                max_num_terms |= field.num_terms;
            }
        }
        let bits_required = DirectWriter::bits_required(max_num_terms as i64);
        self.vectors_stream.write_v_int(bits_required)?;

        self.scratch_buffer.reset();
        let mut writer =
            DirectWriter::new(&mut self.scratch_buffer, total_fields as i64, bits_required)?;
        for doc in &self.pending_docs {
            for field in &doc.fields {
                writer.add(field.num_terms as i64)?;
            }
        }
        writer.finish()?;
        let bytes = self.scratch_buffer.to_array_copy();
        self.vectors_stream.write_v_int(bytes.len() as i32)?;
        self.vectors_stream.write_bytes(&bytes, 0, bytes.len())?;
        Ok(())
    }

    fn flush_term_lengths(&mut self) -> Result<()> {
        let mut writer = BlockPackedWriter::new(self.vectors_stream.as_mut(), PACKED_BLOCK_SIZE)?;
        for doc in &self.pending_docs {
            for field in &doc.fields {
                for i in 0..field.num_terms as usize {
                    writer.add(field.prefix_lengths[i] as i64)?;
                }
            }
        }
        writer.finish()?;

        let mut writer = BlockPackedWriter::new(self.vectors_stream.as_mut(), PACKED_BLOCK_SIZE)?;
        for doc in &self.pending_docs {
            for field in &doc.fields {
                for i in 0..field.num_terms as usize {
                    writer.add(field.suffix_lengths[i] as i64)?;
                }
            }
        }
        writer.finish()?;
        Ok(())
    }

    fn flush_term_freqs(&mut self) -> Result<()> {
        let _total_terms: i32 = self
            .pending_docs
            .iter()
            .map(|doc| doc.fields.iter().map(|field| field.num_terms).sum::<i32>())
            .sum();
        let mut writer = BlockPackedWriter::new(self.vectors_stream.as_mut(), PACKED_BLOCK_SIZE)?;
        for doc in &self.pending_docs {
            for field in &doc.fields {
                for i in 0..field.num_terms as usize {
                    writer.add((field.freqs[i] - 1) as i64)?;
                }
            }
        }
        writer.finish()?;
        Ok(())
    }

    fn flush_positions(&mut self) -> Result<()> {
        let mut writer = BlockPackedWriter::new(self.vectors_stream.as_mut(), PACKED_BLOCK_SIZE)?;
        for doc in &self.pending_docs {
            for field in &doc.fields {
                if field.has_positions {
                    let mut pos = 0i32;
                    for i in 0..field.num_terms as usize {
                        let mut previous_position = 0i64;
                        for _ in 0..field.freqs[i] {
                            let position = self.positions_buf[(field.pos_start + pos) as usize];
                            writer.add((position - previous_position as i32) as i64)?;
                            previous_position = position as i64;
                            pos += 1;
                        }
                    }
                    debug_assert_eq!(pos, field.total_positions);
                }
            }
        }
        writer.finish()?;
        Ok(())
    }

    fn flush_offsets(&mut self, field_nums: &[i32]) -> Result<()> {
        let mut has_offsets = false;
        let mut sum_pos = vec![0i64; field_nums.len()];
        let mut sum_offsets = vec![0i64; field_nums.len()];
        for doc in &self.pending_docs {
            for field in &doc.fields {
                has_offsets |= field.has_offsets;
                if field.has_offsets && field.has_positions {
                    let idx = field_nums.binary_search(&field.field_num).unwrap();
                    let mut pos = 0i32;
                    for i in 0..field.num_terms as usize {
                        sum_pos[idx] += self.positions_buf
                            [(field.pos_start + field.freqs[i] - 1 + pos) as usize]
                            as i64;
                        sum_offsets[idx] += self.start_offsets_buf
                            [(field.off_start + field.freqs[i] - 1 + pos) as usize]
                            as i64;
                        pos += field.freqs[i];
                    }
                    debug_assert_eq!(pos, field.total_positions);
                }
            }
        }

        if !has_offsets {
            return Ok(());
        }

        let mut chars_per_term = vec![0.0f32; field_nums.len()];
        for i in 0..field_nums.len() {
            chars_per_term[i] = if sum_pos[i] <= 0 || sum_offsets[i] <= 0 {
                0.0
            } else {
                (sum_offsets[i] as f64 / sum_pos[i] as f64) as f32
            };
            self.vectors_stream
                .write_int(f32::to_bits(chars_per_term[i]) as i32)?;
        }

        let mut writer = BlockPackedWriter::new(self.vectors_stream.as_mut(), PACKED_BLOCK_SIZE)?;
        for doc in &self.pending_docs {
            for field in &doc.fields {
                if (field.flags & OFFSETS) != 0 {
                    let idx = field_nums.binary_search(&field.field_num).unwrap();
                    let cpt = chars_per_term[idx];
                    let mut pos = 0i32;
                    for i in 0..field.num_terms as usize {
                        let mut previous_pos = 0i32;
                        let mut previous_off = 0i32;
                        for _ in 0..field.freqs[i] {
                            let position = if field.has_positions {
                                self.positions_buf[(field.pos_start + pos) as usize]
                            } else {
                                0
                            };
                            let start_offset =
                                self.start_offsets_buf[(field.off_start + pos) as usize];
                            writer.add(
                                (start_offset
                                    - previous_off
                                    - (cpt * (position - previous_pos) as f32) as i32)
                                    as i64,
                            )?;
                            previous_pos = position;
                            previous_off = start_offset;
                            pos += 1;
                        }
                    }
                }
            }
        }
        writer.finish()?;

        let mut writer = BlockPackedWriter::new(self.vectors_stream.as_mut(), PACKED_BLOCK_SIZE)?;
        for doc in &self.pending_docs {
            for field in &doc.fields {
                if (field.flags & OFFSETS) != 0 {
                    let mut pos = 0i32;
                    for i in 0..field.num_terms as usize {
                        for _ in 0..field.freqs[i] {
                            writer.add(
                                (self.lengths_buf[(field.off_start + pos) as usize]
                                    - field.prefix_lengths[i]
                                    - field.suffix_lengths[i])
                                    as i64,
                            )?;
                            pos += 1;
                        }
                    }
                    debug_assert_eq!(pos, field.total_positions);
                }
            }
        }
        writer.finish()?;
        Ok(())
    }

    fn flush_payload_lengths(&mut self) -> Result<()> {
        let mut writer = BlockPackedWriter::new(self.vectors_stream.as_mut(), PACKED_BLOCK_SIZE)?;
        for doc in &self.pending_docs {
            for field in &doc.fields {
                if field.has_payloads {
                    for i in 0..field.total_positions as usize {
                        writer.add(
                            self.payload_lengths_buf[(field.pay_start + i as i32) as usize] as i64,
                        )?;
                    }
                }
            }
        }
        writer.finish()?;
        Ok(())
    }
}

impl fmt::Debug for Lucene90CompressingTermVectorsWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90CompressingTermVectorsWriter")
            .field("segment", &self.segment)
            .field("compression_mode", &format!("{}", self.compression_mode))
            .field("chunk_size", &self.chunk_size)
            .field("max_docs_per_chunk", &self.max_docs_per_chunk)
            .field("num_docs", &self.num_docs)
            .field("pending_docs", &self.pending_docs.len())
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

/// Accounts for the buffers this writer holds between flushes.
///
/// Equivalent to `Lucene90CompressingTermVectorsWriter.ramBytesUsed()` and
/// `getChildResources()`. Note that Java adds the four `int[]` buffers by their
/// **element count**, not their byte size — `positionsBuf.length` and friends —
/// so a fresh writer reports `4 * 1024 + lastTerm.bytes.length`. Measured
/// against `lucene-core-10.5.0.jar`: a freshly created
/// `Lucene90CompressingTermVectorsWriter` answers 4136, which is
/// `4 * 1024 + ArrayUtil.oversize(30, 1)`.
impl Accountable for Lucene90CompressingTermVectorsWriter {
    fn ram_bytes_used(&self) -> i64 {
        self.positions_buf.len() as i64
            + self.start_offsets_buf.len() as i64
            + self.lengths_buf.len() as i64
            + self.payload_lengths_buf.len() as i64
            + self.term_suffixes.ram_bytes_used()
            + self.payload_bytes.ram_bytes_used()
            + self.last_term.bytes.len() as i64
            + self.scratch_buffer.ram_bytes_used()
    }

    fn child_resources(&self) -> Vec<&dyn Accountable> {
        vec![&self.term_suffixes, &self.payload_bytes]
    }
}

impl TermVectorsWriter for Lucene90CompressingTermVectorsWriter {
    fn start_document(&mut self, num_vector_fields: i32) -> Result<()> {
        self.cur_doc = None;
        self.cur_field = None;
        let _doc = self.add_doc_data(num_vector_fields);
        // Move the DocData out into self.cur_doc so we can mutate it through the option.
        let taken = self.pending_docs.pop().unwrap();
        self.cur_doc = Some(taken);
        Ok(())
    }

    fn finish_document(&mut self) -> Result<()> {
        // Append payload bytes of the doc after its terms.
        let payload_len = self.payload_bytes.size();
        if payload_len > 0 {
            let bytes = self.payload_bytes.to_array_copy();
            self.term_suffixes.write_bytes(&bytes, 0, payload_len)?;
        }
        self.payload_bytes.reset();
        self.last_term.length = 0;

        if let Some(doc) = self.cur_doc.take() {
            self.num_docs += 1;
            self.pending_docs.push(doc);
            if self.trigger_flush() {
                self.flush(false)?;
            }
        }
        self.cur_field = None;
        Ok(())
    }

    fn start_field(
        &mut self,
        field_info: &FieldInfo,
        num_terms: i32,
        positions: bool,
        offsets: bool,
        payloads: bool,
    ) -> Result<()> {
        let doc = self.cur_doc.as_mut().ok_or_else(|| {
            LuceneError::IllegalState("start_field called without active document".to_string())
        })?;
        let _field = doc.add_field(field_info.number, num_terms, positions, offsets, payloads);
        self.cur_field = Some(doc.fields.len() - 1);
        self.last_term.length = 0;
        Ok(())
    }

    fn finish_field(&mut self) -> Result<()> {
        self.cur_field = None;
        Ok(())
    }

    fn start_term(&mut self, term: &BytesRef, freq: i32) -> Result<()> {
        if freq < 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "freq must be >= 1, got {freq}"
            )));
        }
        let cur_field_idx = self.cur_field.ok_or_else(|| {
            LuceneError::IllegalState("start_term called without active field".to_string())
        })?;
        let doc = self.cur_doc.as_mut().unwrap();
        let field = &mut doc.fields[cur_field_idx];

        let prefix = if self.last_term.length == 0 {
            0
        } else {
            StringHelper::bytes_difference(&self.last_term, term)?
        };
        let prefix_usize = prefix as usize;
        let suffix_length = term.length - prefix_usize;
        field.add_term(freq, prefix, suffix_length as i32);
        self.term_suffixes
            .write_bytes(&term.bytes, term.offset + prefix_usize, suffix_length)?;

        if self.last_term.bytes.len() < term.length {
            // Java replaces the array with `new byte[ArrayUtil.oversize(term.length, 1)]`,
            // whose length is what `ramBytesUsed` reports; `len * 2` would
            // report a different number for the same input.
            self.last_term.bytes = vec![0u8; ArrayUtil::oversize(term.length, 1)];
        }
        self.last_term.offset = 0;
        self.last_term.length = term.length;
        self.last_term.bytes[..term.length]
            .copy_from_slice(&term.bytes[term.offset..term.offset + term.length]);
        Ok(())
    }

    fn add_position(
        &mut self,
        position: i32,
        start_offset: i32,
        end_offset: i32,
        payload: Option<&BytesRef>,
    ) -> Result<()> {
        let cur_field_idx = self.cur_field.ok_or_else(|| {
            LuceneError::IllegalState("add_position called without active field".to_string())
        })?;
        let doc = self.cur_doc.as_mut().unwrap();
        let field = &doc.fields[cur_field_idx];

        let payload_length = if let Some(p) = payload {
            if !field.has_payloads {
                return Err(LuceneError::IllegalState(
                    "payload provided but field does not store payloads".to_string(),
                ));
            }
            self.payload_bytes
                .write_bytes(&p.bytes, p.offset, p.length)?;
            p.length as i32
        } else {
            0
        };

        if field.has_positions {
            let pos_start = (field.pos_start + field.total_positions) as usize;
            if pos_start + 1 > self.positions_buf.len() {
                self.positions_buf.resize(pos_start + 1, 0);
            }
            self.positions_buf[pos_start] = position;
        }
        if field.has_offsets {
            let off_start = (field.off_start + field.total_positions) as usize;
            if off_start + 1 > self.start_offsets_buf.len() {
                let new_len = (off_start + 1).max(self.start_offsets_buf.len() * 2);
                self.start_offsets_buf.resize(new_len, 0);
                self.lengths_buf.resize(new_len, 0);
            }
            self.start_offsets_buf[off_start] = start_offset;
            self.lengths_buf[off_start] = end_offset - start_offset;
        }
        if field.has_payloads {
            let pay_start = (field.pay_start + field.total_positions) as usize;
            if pay_start + 1 > self.payload_lengths_buf.len() {
                self.payload_lengths_buf.resize(pay_start + 1, 0);
            }
            self.payload_lengths_buf[pay_start] = payload_length;
        }

        let field = &mut doc.fields[cur_field_idx];
        field.total_positions += 1;
        Ok(())
    }

    fn finish_term(&mut self) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self, num_docs: i32) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "Lucene90CompressingTermVectorsWriter already finished".to_string(),
            ));
        }
        if !self.pending_docs.is_empty() || self.cur_doc.is_some() {
            // Finish any in-progress document first.
            if self.cur_doc.is_some() {
                self.finish_document()?;
            }
            self.flush(true)?;
        }
        if num_docs != self.num_docs {
            return Err(LuceneError::IllegalState(format!(
                "Wrote {} docs, finish called with numDocs={num_docs}",
                self.num_docs
            )));
        }
        let max_pointer = self.vectors_stream.file_pointer();
        self.index_writer
            .finish(num_docs, max_pointer, self.meta_stream.as_mut())?;
        self.meta_stream.write_v_long(self.num_chunks)?;
        self.meta_stream.write_v_long(self.num_dirty_chunks)?;
        self.meta_stream.write_v_long(self.num_dirty_docs)?;
        write_footer(self.meta_stream.as_mut())?;
        write_footer(self.vectors_stream.as_mut())?;
        self.finished = true;
        Ok(())
    }

    /// Copies every document's term vectors from the segments being merged,
    /// copying whole compressed chunks for the segments that allow it.
    ///
    /// [`can_perform_bulk_merge`](Self::can_perform_bulk_merge) makes that
    /// choice once per segment. When it holds, a run of consecutive documents
    /// from that segment is gathered first — a chunk can only be copied as a
    /// whole — and handed to [`copy_chunks`](Self::copy_chunks), which moves
    /// the compressed bytes verbatim. Otherwise the document's vectors are
    /// decoded and written again through
    /// [`add_all_doc_vectors`](TermVectorsWriter::add_all_doc_vectors), which
    /// is what the trait's own [`TermVectorsWriter::merge`] does for every
    /// document.
    ///
    /// A segment with no term-vectors reader contributes empty vectors, exactly
    /// as it does in Java.
    ///
    /// # Errors
    ///
    /// Propagates every I/O and corruption error raised while reading the
    /// source segments.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsWriter#merge`
    /// (`Lucene90CompressingTermVectorsWriter.java:898-939`).
    fn merge(&mut self, merge_state: &MergeState) -> Result<i32> {
        let num_readers = merge_state.term_vectors_readers.len();
        let matching_readers = MatchingReaders::new(merge_state);
        let mut subs: Vec<CompressingTermVectorsSub> = Vec::with_capacity(num_readers);
        for i in 0..num_readers {
            if let Some(reader) = merge_state.term_vectors_readers[i].as_ref() {
                merge_state.check_aborted()?;
                reader.check_integrity()?;
            }
            let can_perform_bulk_merge =
                self.can_perform_bulk_merge(merge_state, &matching_readers, i);
            subs.push(CompressingTermVectorsSub {
                doc_map: Arc::clone(&merge_state.doc_maps[i]),
                max_doc: merge_state.max_docs[i],
                reader_index: i,
                can_perform_bulk_merge,
                doc_id: -1,
                mapped_doc_id: -1,
            });
        }

        let mut doc_count = 0i32;
        let mut merger = DocIDMerger::new(subs, merge_state.needs_index_sort)?;
        let mut step = next_term_vectors_step(&mut merger)?;
        while let Some(current) = step {
            debug_assert_eq!(current.mapped_doc_id, doc_count);
            if current.can_perform_bulk_merge {
                // Gather the run of documents this segment contributes in a
                // row: chunks can only be copied whole.
                let from_doc_id = current.doc_id;
                let mut to_doc_id = from_doc_id;
                loop {
                    step = next_term_vectors_step(&mut merger)?;
                    match step {
                        Some(next) if next.sub_index == current.sub_index => {
                            to_doc_id += 1;
                            debug_assert_eq!(next.doc_id, to_doc_id);
                        }
                        _ => break,
                    }
                }
                to_doc_id += 1; // exclusive bound
                self.copy_chunks(
                    merge_state,
                    merger.sub(current.sub_index),
                    from_doc_id,
                    to_doc_id,
                )?;
                doc_count += to_doc_id - from_doc_id;
            } else {
                // Decode and re-encode one document; a segment without a
                // term-vectors reader contributes an empty document
                // (`Lucene90CompressingTermVectorsWriter.java:929-935`).
                let vectors = match merge_state.term_vectors_readers[current.reader_index].as_ref()
                {
                    Some(reader) => reader.get(current.doc_id)?,
                    None => None,
                };
                self.add_all_doc_vectors(vectors.as_deref(), &merge_state.merge_field_infos)?;
                doc_count += 1;
                step = next_term_vectors_step(&mut merger)?;
            }
        }
        self.finish(doc_count)?;
        Ok(doc_count)
    }

    fn close(&mut self) -> Result<()> {
        self.meta_stream.close()?;
        self.vectors_stream.close()?;
        self.compressor.close()
    }
}

/// Upper bound on how many packed values one byte of a term-vector chunk can
/// possibly encode.
///
/// `BlockPackedWriter` spends at least one header byte per block of
/// [`PACKED_BLOCK_SIZE`] values, and `DirectWriter`/`PackedInts` spend at least
/// one bit per value, so no encoding this format uses can pack more than 64
/// values into a byte. A count read off disk that exceeds the remaining bytes
/// times this factor cannot be honest, and honouring it would mean allocating
/// gigabytes for a file that holds none.
const MAX_VALUES_PER_BYTE: i64 = PACKED_BLOCK_SIZE as i64;

/// Returns how many bytes of `input` are still unread.
fn remaining(input: &dyn IndexInput) -> i64 {
    (input.length() - input.file_pointer()).max(0)
}

/// Validates a length that came off disk before it is used to allocate.
///
/// Java would answer a negative length with a `NegativeArraySizeException` and
/// an absurd one with an `OutOfMemoryError`, both of which a caller can catch.
/// Rust cannot catch an allocation failure, so a length no honest file could
/// carry is rejected up front.
fn checked_block_len(what: &str, length: i64, input: &dyn IndexInput) -> Result<usize> {
    let available = remaining(input);
    if length < 0 || length > available {
        return Err(LuceneError::CorruptIndex(format!(
            "{what} claims {length} bytes, but only {available} remain in the term-vectors stream"
        )));
    }
    Ok(length as usize)
}

/// Validates a value count that came off disk before it is used to allocate.
///
/// `available` is how many bytes of the stream are still unread; it is taken
/// before the caller borrows the stream to decode, because the reader that
/// produces `count` holds that borrow.
fn checked_count(what: &str, count: i64, available: i64) -> Result<usize> {
    // Every count in this format is a Java `int`, so it is also capped there.
    let ceiling = available
        .saturating_mul(MAX_VALUES_PER_BYTE)
        .min(i64::from(i32::MAX));
    if count < 0 || count > ceiling {
        return Err(LuceneError::CorruptIndex(format!(
            "{what} claims {count} values, which the {available} remaining bytes of the \
             term-vectors stream cannot encode"
        )));
    }
    Ok(count as usize)
}

/// Reads `length` bytes into a fresh buffer, refusing a length the stream
/// cannot satisfy.
/// Reads `length` bytes out of `input` into an owned buffer, which the packed
/// readers then index at random.
///
/// Equivalent to the private
/// `Lucene90CompressingTermVectorsReader.slice(IndexInput)`, minus the length:
/// Java's helper reads the block length itself with `readVInt()` and wraps the
/// bytes in a `ByteBuffersDataInput`, whereas the callers here read the length
/// (the first of them as a `vLong`, which is what the writer emits at
/// `Lucene90CompressingTermVectorsWriter.java:497`) and hand it in, because
/// `DirectReader::new` takes the bytes by value rather than a
/// `RandomAccessInput`.
fn read_block(what: &str, input: &mut dyn IndexInput, length: i64) -> Result<Vec<u8>> {
    let length = checked_block_len(what, length, input)?;
    let mut bytes = vec![0u8; length];
    input.read_bytes(&mut bytes, 0, length)?;
    Ok(bytes)
}

/// Returns `values[index]`, reporting an index the file invented rather than
/// panicking.
fn at(what: &str, values: &[i32], index: i32) -> Result<i32> {
    usize::try_from(index)
        .ok()
        .and_then(|index| values.get(index).copied())
        .ok_or_else(|| {
            LuceneError::CorruptIndex(format!(
                "{what} index {index} is outside the {} values the chunk holds",
                values.len()
            ))
        })
}

// -----------------------------------------------------------------------------
// Reader
// -----------------------------------------------------------------------------

/// Number of block ids the reader remembers having prefetched.
///
/// Equivalent to `Lucene90CompressingTermVectorsReader.PREFETCH_CACHE_SIZE`.
const PREFETCH_CACHE_SIZE: usize = 1 << 4;

/// Mask that turns the running prefetch counter into a cache slot.
///
/// Equivalent to `Lucene90CompressingTermVectorsReader.PREFETCH_CACHE_MASK`.
const PREFETCH_CACHE_MASK: i32 = PREFETCH_CACHE_SIZE as i32 - 1;

/// Reader for the compressing term-vectors format.
///
/// Lucene Core equivalent:
/// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsReader`.
pub struct Lucene90CompressingTermVectorsReader {
    field_infos: FieldInfos,
    index_reader: Box<dyn FieldsIndex>,
    version: i32,
    packed_ints_version: i32,
    compression_mode: CompressionMode,
    decompressor: Mutex<Box<dyn Decompressor>>,
    chunk_size: i32,
    num_docs: i32,
    num_chunks: i64,
    num_dirty_chunks: i64,
    num_dirty_docs: i64,
    max_pointer: i64,
    /// The `.tvd` file, kept open exactly as Lucene keeps `vectorsStream`.
    /// Every read clones it, so the reader works on whatever directory it was
    /// opened from — including a compound-file directory, whose files no
    /// `SegmentInfo` can point at.
    vectors_stream: Box<dyn IndexInput>,
    /// The chunk the last [`get`](TermVectorsReader::get) decoded, so that a
    /// read inside the same chunk skips the index lookup and a bulk merge can
    /// tell whether a document's chunk is already in flight.
    block_state: Mutex<BlockState>,
    /// The last [`PREFETCH_CACHE_SIZE`] blocks
    /// [`prefetch`](TermVectorsReader::prefetch) asked the `.tvd` input for,
    /// overwritten round-robin; `-1` marks a free slot.
    ///
    /// Equivalent to Lucene's `long[] prefetchedBlockIDCache`
    /// (`Lucene90CompressingTermVectorsReader.java:104`). `prefetch` takes
    /// `&self`, so the array sits behind a lock.
    prefetched_block_id_cache: Mutex<[i64; PREFETCH_CACHE_SIZE]>,
    /// How many blocks have been prefetched, which picks the next slot.
    ///
    /// Equivalent to Lucene's `int prefetchedBlockIDCacheIndex`
    /// (`Lucene90CompressingTermVectorsReader.java:105`); it wraps as Java's
    /// `int` does.
    prefetched_block_id_cache_index: AtomicI32,
    /// Whether [`close`](TermVectorsReader::close) has already run, so that a
    /// later read reports `AlreadyClosed` instead of touching a closed file.
    ///
    /// Equivalent to Lucene's `private boolean closed`.
    closed: bool,
}

/// Which chunk the reader last decoded, and where it starts.
///
/// Equivalent to the private record
/// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsReader.BlockState`.
#[derive(Debug, Clone, Copy)]
struct BlockState {
    start_pointer: i64,
    doc_base: i32,
    chunk_docs: i32,
}

impl BlockState {
    /// The state a freshly opened reader starts in: no chunk decoded yet.
    fn new() -> Self {
        Self {
            start_pointer: -1,
            doc_base: -1,
            chunk_docs: 0,
        }
    }
}

impl Lucene90CompressingTermVectorsReader {
    #[allow(clippy::too_many_arguments)]
    fn new(
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
        format_name: &str,
        compression_mode: CompressionMode,
        _chunk_size: i32,
        _block_shift: i32,
    ) -> Result<Self> {
        let segment = segment_info.name.clone();
        let id = segment_info.id();
        let num_docs = segment_info.max_doc()?;

        let vectors_file = segment_file_name(&segment, segment_suffix, VECTORS_EXTENSION);
        let mut vectors_stream = directory.open_input(&vectors_file, context)?;
        let version = check_index_header(
            vectors_stream.as_mut(),
            format_name,
            VERSION_START,
            VERSION_CURRENT,
            &id,
            segment_suffix,
        )?;
        retrieve_checksum(vectors_stream.as_mut())?;

        let meta_file = segment_file_name(&segment, segment_suffix, META_EXTENSION);
        let mut meta_in = directory.open_checksum_input(&meta_file)?;
        check_index_header(
            meta_in.as_mut(),
            &format!("{INDEX_CODEC_NAME}Meta"),
            VERSION_START,
            version,
            &id,
            segment_suffix,
        )?;
        let packed_ints_version = meta_in.read_v_int()?;
        let chunk_size = meta_in.read_v_int()?;

        let index_reader = FieldsIndexReader::new(
            directory,
            &segment,
            segment_suffix,
            INDEX_EXTENSION,
            INDEX_CODEC_NAME,
            id,
            meta_in.as_mut(),
            context,
        )?;
        let max_pointer = index_reader.max_pointer();

        let num_chunks = meta_in.read_v_long()?;
        let num_dirty_chunks = meta_in.read_v_long()?;
        let num_dirty_docs = meta_in.read_v_long()?;

        if num_chunks < num_dirty_chunks {
            return Err(LuceneError::CorruptIndex(format!(
                "Cannot have more dirty chunks than chunks: numChunks={num_chunks}, numDirtyChunks={num_dirty_chunks}"
            )));
        }
        if (num_dirty_chunks == 0) != (num_dirty_docs == 0) {
            return Err(LuceneError::CorruptIndex(
                "Cannot have dirty chunks without dirty docs or vice-versa".to_string(),
            ));
        }
        if num_dirty_docs < num_dirty_chunks {
            return Err(LuceneError::CorruptIndex(format!(
                "Cannot have more dirty chunks than documents within dirty chunks: numDirtyChunks={num_dirty_chunks}, numDirtyDocs={num_dirty_docs}"
            )));
        }

        check_footer(meta_in.as_mut())?;
        meta_in.close()?;

        Ok(Self {
            field_infos: FieldInfos::new(field_infos.iter().cloned().collect())?,
            index_reader: index_reader.clone_index()?,
            version,
            packed_ints_version,
            compression_mode,
            decompressor: Mutex::new(compression_mode.new_decompressor()),
            chunk_size,
            num_docs,
            num_chunks,
            num_dirty_chunks,
            num_dirty_docs,
            max_pointer,
            vectors_stream,
            block_state: Mutex::new(BlockState::new()),
            prefetched_block_id_cache: Mutex::new([-1; PREFETCH_CACHE_SIZE]),
            prefetched_block_id_cache_index: AtomicI32::new(0),
            closed: false,
        })
    }
}

impl fmt::Debug for Lucene90CompressingTermVectorsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90CompressingTermVectorsReader")
            .field("version", &self.version)
            .field("compression_mode", &format!("{}", self.compression_mode))
            .field("chunk_size", &self.chunk_size)
            .field("num_docs", &self.num_docs)
            .field("num_chunks", &self.num_chunks)
            .field("max_pointer", &self.max_pointer)
            .finish_non_exhaustive()
    }
}

impl Clone for Lucene90CompressingTermVectorsReader {
    fn clone(&self) -> Self {
        Self {
            field_infos: FieldInfos::new(self.field_infos.iter().cloned().collect())
                .expect("clone field infos"),
            index_reader: self.index_reader.clone_index().expect("clone index"),
            version: self.version,
            packed_ints_version: self.packed_ints_version,
            compression_mode: self.compression_mode,
            decompressor: Mutex::new(self.compression_mode.new_decompressor()),
            chunk_size: self.chunk_size,
            num_docs: self.num_docs,
            num_chunks: self.num_chunks,
            num_dirty_chunks: self.num_dirty_chunks,
            num_dirty_docs: self.num_dirty_docs,
            max_pointer: self.max_pointer,
            vectors_stream: self
                .vectors_stream
                .clone_input()
                .expect("INVARIANT: cloning an open IndexInput never fails"),
            block_state: Mutex::new(BlockState::new()),
            prefetched_block_id_cache: Mutex::new([-1; PREFETCH_CACHE_SIZE]),
            prefetched_block_id_cache_index: AtomicI32::new(0),
            closed: false,
        }
    }
}

impl TermVectorsReader for Lucene90CompressingTermVectorsReader {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn check_integrity(&self) -> Result<()> {
        self.index_reader.check_integrity()?;
        let mut input = self.vectors_stream.clone_input()?;
        input.seek(0)?;
        checksum_entire_file(input.as_mut())?;
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn TermVectorsReader> {
        Box::new(self.clone())
    }

    fn get_merge_instance(&self) -> Box<dyn TermVectorsReader> {
        Box::new(self.clone())
    }

    /// Closes the fields index and the `.tvd` stream, once.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.close()`: both are
    /// closed even when the first fails, the first failure is the one reported,
    /// and the reader is only marked closed when the close succeeded — so a
    /// failed close can be retried, exactly as Java's `if (!closed)` allows.
    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        let index = self.index_reader.close();
        let stream = self.vectors_stream.close();
        index.and(stream)?;
        self.closed = true;
        Ok(())
    }
}

impl TermVectors for Lucene90CompressingTermVectorsReader {
    #[allow(clippy::needless_range_loop)]
    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        self.ensure_open()?;

        // seek to the right place
        let start_pointer = if self.is_loaded(doc) {
            // avoid searching the start pointer
            self.block_state.lock().unwrap().start_pointer
        } else {
            self.index_reader.get_start_pointer(doc)?
        };
        let mut input = self.vectors_stream.clone_input()?;
        input.seek(start_pointer)?;

        let doc_base = input.read_v_int()?;
        // Every one of these comes off disk, so the bounds check itself must
        // not overflow: Java's `int` arithmetic wraps here and simply fails the
        // comparison, while Rust would abort a debug build before it got that
        // far.
        let chunk_docs = i64::from(input.read_v_int()?) >> 1;
        let chunk_end = i64::from(doc_base) + chunk_docs;
        if i64::from(doc) < i64::from(doc_base)
            || i64::from(doc) >= chunk_end
            || chunk_end > i64::from(self.num_docs)
        {
            return Err(LuceneError::CorruptIndex(format!(
                "docBase={doc_base}, chunkDocs={chunk_docs}, doc={doc}, numDocs={}",
                self.num_docs
            )));
        }
        let chunk_docs = chunk_docs as i32;
        *self.block_state.lock().unwrap() = BlockState {
            start_pointer,
            doc_base,
            chunk_docs,
        };
        // Taken before the block-packed reader borrows the stream, because
        // every count it decodes is bounded by what is still left to read.
        let available = remaining(input.as_ref());
        let skip;
        let num_fields;
        let total_fields;
        if chunk_docs == 1 {
            skip = 0;
            num_fields = input.read_v_int()?;
            total_fields = num_fields;
        } else {
            let mut reader = BlockPackedReaderIterator::new(
                input.as_mut(),
                self.packed_ints_version,
                PACKED_BLOCK_SIZE,
                chunk_docs as i64,
            )?;
            // The per-document field counts are summed in 64 bits and validated
            // once, rather than accumulated in a wrapping `int` as Java does.
            // A corrupt block can decode to any `i64` at all, so the running
            // sum saturates rather than overflowing; `checked_count` then
            // rejects it.
            let mut sum = 0i64;
            for _ in doc_base..doc {
                sum = sum.saturating_add(reader.next()?);
            }
            skip = checked_count("the fields skipped in the chunk", sum, available)? as i32;
            let fields_here = reader.next()?;
            num_fields =
                checked_count("the fields of the document", fields_here, available)? as i32;
            sum = sum.saturating_add(fields_here);
            for _ in doc + 1..doc_base + chunk_docs {
                sum = sum.saturating_add(reader.next()?);
            }
            total_fields = checked_count("the fields of the chunk", sum, available)? as i32;
        }

        if num_fields == 0 {
            return Ok(None);
        }
        if num_fields < 0 || skip < 0 || total_fields < skip + num_fields {
            return Err(LuceneError::CorruptIndex(format!(
                "the chunk claims {total_fields} fields but the document at {doc} needs \
                 {skip} + {num_fields}"
            )));
        }

        // Read field numbers.
        let token = input.read_byte()? as i32;
        let bits_per_field_num = token & 0x1F;
        let mut total_distinct_fields = i64::from(token >> 5);
        if total_distinct_fields == 0x07 {
            total_distinct_fields += i64::from(input.read_v_int()?);
        }
        total_distinct_fields += 1;
        let total_distinct_fields = checked_count(
            "the distinct fields of the chunk",
            total_distinct_fields,
            remaining(input.as_ref()),
        )?;
        let field_nums = read_packed_ints_no_header(
            input.as_mut(),
            total_distinct_fields as i64,
            bits_per_field_num,
        )?;
        let field_nums: Vec<i32> = field_nums.into_iter().map(|v| v as i32).collect();
        if field_nums.is_empty() {
            return Err(LuceneError::CorruptIndex(
                "a term-vector chunk with fields must name at least one of them".to_string(),
            ));
        }

        // Read field num offsets and flags.
        let field_num_offs_len = input.read_v_long()?;
        let field_num_offs_bytes = read_block(
            "the field-number offsets",
            input.as_mut(),
            field_num_offs_len,
        )?;
        let bits_per_off = DirectWriter::bits_required((field_nums.len() - 1) as i64);
        let all_field_num_offs_reader = DirectReader::new(field_num_offs_bytes, bits_per_off)?;
        let mut all_field_num_offs_vec = Vec::with_capacity(total_fields as usize);
        for i in 0..total_fields as i64 {
            let off = all_field_num_offs_reader.get_checked(i)?;
            // Validated once, here, so that every later use — the flags, the
            // per-field `charsPerTerm` — is provably in range.
            if off < 0 || off >= field_nums.len() as i64 {
                return Err(LuceneError::CorruptIndex(format!(
                    "field {i} of the chunk points at field-number slot {off} of {}",
                    field_nums.len()
                )));
            }
            all_field_num_offs_vec.push(off as i32);
        }

        let mut field_num_offs = Vec::with_capacity(num_fields as usize);
        for i in 0..num_fields {
            field_num_offs.push(at(
                "the field-number offset",
                &all_field_num_offs_vec,
                skip + i,
            )?);
        }

        let flags_mode = input.read_v_int()?;
        let mut field_flags_values = vec![0i32; total_fields as usize];
        match flags_mode {
            0 => {
                let flags_len = i64::from(input.read_v_int()?);
                let flags_bytes = read_block("the per-field flags", input.as_mut(), flags_len)?;
                let field_flags_reader = DirectReader::new(flags_bytes, FLAGS_BITS)?;
                let mut distinct_flags = Vec::with_capacity(field_nums.len());
                for idx in 0..field_nums.len() as i64 {
                    distinct_flags.push(field_flags_reader.get_checked(idx)? as i32);
                }
                for i in 0..total_fields as usize {
                    field_flags_values[i] =
                        at("the flags", &distinct_flags, all_field_num_offs_vec[i])?;
                }
            }
            1 => {
                let flags_len = i64::from(input.read_v_int()?);
                let flags_bytes = read_block("the per-instance flags", input.as_mut(), flags_len)?;
                let all_flags = DirectReader::new(flags_bytes, FLAGS_BITS)?;
                for i in 0..total_fields as usize {
                    field_flags_values[i] = all_flags.get_checked(i as i64)? as i32;
                }
            }
            _ => {
                return Err(LuceneError::CorruptIndex(format!(
                    "Invalid flags mode: {flags_mode}"
                )))
            }
        };
        let flags = |idx: i32| -> i32 {
            // Every index handed to this closure is below `total_fields`, which
            // is exactly the length of the vector.
            field_flags_values[idx as usize]
        };

        // Number of terms per field.
        let num_terms_bits = input.read_v_int()?;
        let num_terms_len = i64::from(input.read_v_int()?);
        let num_terms_bytes = read_block("the term counts", input.as_mut(), num_terms_len)?;
        let num_terms_reader = DirectReader::new(num_terms_bytes, num_terms_bits)?;
        let mut num_terms_per_field = Vec::with_capacity(total_fields as usize);
        let mut total_terms = 0i64;
        for i in 0..total_fields as i64 {
            let count = num_terms_reader.get_checked(i)?;
            if count < 0 || count > i64::from(i32::MAX) {
                return Err(LuceneError::CorruptIndex(format!(
                    "field {i} of the chunk claims {count} terms"
                )));
            }
            num_terms_per_field.push(count as i32);
            total_terms = total_terms.saturating_add(count);
        }
        let total_terms = checked_count(
            "the terms of the chunk",
            total_terms,
            remaining(input.as_ref()),
        )? as i32;
        let num_terms_at = |index: i64| -> i64 { i64::from(num_terms_per_field[index as usize]) };

        // Term lengths.
        let mut all_prefix_lengths = vec![0i32; total_terms as usize];
        let mut all_suffix_lengths = vec![0i32; total_terms as usize];
        {
            let mut reader = BlockPackedReaderIterator::new(
                input.as_mut(),
                self.packed_ints_version,
                PACKED_BLOCK_SIZE,
                total_terms as i64,
            )?;
            for i in 0..total_terms as usize {
                all_prefix_lengths[i] = reader.next()? as i32;
            }
        }
        {
            let mut reader = BlockPackedReaderIterator::new(
                input.as_mut(),
                self.packed_ints_version,
                PACKED_BLOCK_SIZE,
                total_terms as i64,
            )?;
            for i in 0..total_terms as usize {
                all_suffix_lengths[i] = reader.next()? as i32;
            }
        }

        let mut skip_terms = 0i32;
        for i in 0..skip as usize {
            skip_terms = skip_terms.wrapping_add(num_terms_at(i as i64) as i32);
        }
        let mut prefix_lengths = vec![Vec::new(); num_fields as usize];
        let mut suffix_lengths = vec![Vec::new(); num_fields as usize];
        let mut field_lengths = vec![0i32; num_fields as usize];
        let mut doc_off = 0i32;
        let mut doc_len = 0i32;
        for i in 0..skip_terms as usize {
            doc_off = doc_off.wrapping_add(all_suffix_lengths[i]);
        }
        let mut off = skip_terms;
        for i in 0..num_fields as usize {
            let term_count = num_terms_at((skip + i as i32) as i64) as usize;
            prefix_lengths[i] =
                all_prefix_lengths[off as usize..off as usize + term_count].to_vec();
            suffix_lengths[i] =
                all_suffix_lengths[off as usize..off as usize + term_count].to_vec();
            field_lengths[i] = Self::sum(&suffix_lengths[i]);
            doc_len = doc_len.wrapping_add(field_lengths[i]);
            off = off.wrapping_add(term_count as i32);
        }
        let chunk_total_len: i32 = Self::sum(&all_suffix_lengths);

        // Term freqs.
        let mut term_freqs = vec![0i32; total_terms as usize];
        {
            let mut reader = BlockPackedReaderIterator::new(
                input.as_mut(),
                self.packed_ints_version,
                PACKED_BLOCK_SIZE,
                total_terms as i64,
            )?;
            // Java stores `1 + next()` and lets a corrupt block produce a
            // frequency of zero or less, which then walks off the end of the
            // position arrays. A frequency below one cannot describe a term
            // that is in the vector at all, so it is rejected here; the total
            // is capped for the same reason every other count is.
            let mut total_freqs = 0i64;
            for i in 0..total_terms as usize {
                let freq = reader.next()?.saturating_add(1);
                if freq < 1 {
                    return Err(LuceneError::CorruptIndex(format!(
                        "term {i} of the chunk claims a frequency of {freq}"
                    )));
                }
                total_freqs = total_freqs.saturating_add(freq);
                term_freqs[i] = checked_count("a term frequency", freq, available)? as i32;
            }
            checked_count("the term occurrences of the chunk", total_freqs, available)?;
        }

        // Compute total positions/offsets/payloads and positionIndex.
        let mut total_positions = 0i32;
        let mut total_offsets = 0i32;
        let mut total_payloads = 0i32;
        let mut term_index = 0i32;
        for i in 0..total_fields as usize {
            let f = flags(i as i32);
            let term_count = num_terms_at(i as i64) as i32;
            for j in 0..term_count as usize {
                let freq = term_freqs[(term_index + j as i32) as usize];
                if (f & POSITIONS) != 0 {
                    total_positions = total_positions.wrapping_add(freq);
                }
                if (f & OFFSETS) != 0 {
                    total_offsets = total_offsets.wrapping_add(freq);
                }
                if (f & PAYLOADS) != 0 {
                    total_payloads = total_payloads.wrapping_add(freq);
                }
            }
            term_index = term_index.wrapping_add(term_count);
        }

        let position_index = Self::position_index(skip, num_fields, &num_terms_at, &term_freqs);

        let mut positions = vec![None; num_fields as usize];
        let mut start_offsets = vec![None; num_fields as usize];
        let mut lengths = vec![None; num_fields as usize];

        if total_positions > 0 {
            let raw = Self::read_positions(
                input.as_mut(),
                self.packed_ints_version,
                skip,
                num_fields,
                &num_terms_at,
                &term_freqs,
                &flags,
                POSITIONS,
                total_positions,
                &position_index,
            )?;
            // The positions stay *delta-encoded* here, on purpose: the offsets
            // block below patches every start offset with
            // `charsPerTerm * positionDelta`, which is exactly what the writer
            // subtracted (`Lucene90CompressingTermVectorsWriter.flushOffsets`
            // uses `position - previousPos`). Java therefore delta-decodes the
            // positions only after the offsets are done
            // (`Lucene90CompressingTermVectorsReader.java:585-645`), and so
            // does this port.
            positions[..num_fields as usize].clone_from_slice(&raw[..num_fields as usize]);
        }

        if total_offsets > 0 {
            let mut chars_per_term = vec![0.0f32; field_nums.len()];
            for cpt in &mut chars_per_term {
                *cpt = f32::from_bits(input.read_int()? as u32);
            }
            let raw_start = Self::read_positions(
                input.as_mut(),
                self.packed_ints_version,
                skip,
                num_fields,
                &num_terms_at,
                &term_freqs,
                &flags,
                OFFSETS,
                total_offsets,
                &position_index,
            )?;
            let raw_lengths = Self::read_positions(
                input.as_mut(),
                self.packed_ints_version,
                skip,
                num_fields,
                &num_terms_at,
                &term_freqs,
                &flags,
                OFFSETS,
                total_offsets,
                &position_index,
            )?;

            for i in 0..num_fields as usize {
                let f = flags(skip + i as i32);
                if (f & OFFSETS) != 0 {
                    let term_count = num_terms_at((skip + i as i32) as i64) as usize;
                    let mut f_start_offsets = raw_start[i].clone().unwrap();
                    let f_positions = positions[i].as_ref();
                    let cpt = chars_per_term[field_num_offs[i] as usize];
                    if let Some(fp) = f_positions {
                        for j in 0..f_start_offsets.len() {
                            f_start_offsets[j] =
                                f_start_offsets[j].wrapping_add((cpt * fp[j] as f32) as i32);
                        }
                    }
                    let mut f_lengths = raw_lengths[i].clone().unwrap();
                    let f_prefix = &prefix_lengths[i];
                    let f_suffix = &suffix_lengths[i];
                    for j in 0..term_count {
                        let term_length = f_prefix[j] + f_suffix[j];
                        let slot = position_index[i][j] as usize;
                        f_lengths[slot] = f_lengths[slot].wrapping_add(term_length);
                        for k in position_index[i][j] + 1..position_index[i][j + 1] {
                            f_start_offsets[k as usize] = f_start_offsets[k as usize]
                                .wrapping_add(f_start_offsets[(k - 1) as usize]);
                            f_lengths[k as usize] = f_lengths[k as usize].wrapping_add(term_length);
                        }
                    }
                    start_offsets[i] = Some(f_start_offsets);
                    lengths[i] = Some(f_lengths);
                }
            }
        }

        // Only now are the positions themselves delta-decoded, mirroring the
        // `if (totalPositions > 0)` block Java runs after the offsets.
        if total_positions > 0 {
            for i in 0..num_fields as usize {
                let Some(field_positions) = positions[i].as_mut() else {
                    continue;
                };
                let term_count = num_terms_at((skip + i as i32) as i64) as usize;
                for j in 0..term_count {
                    for k in position_index[i][j] + 1..position_index[i][j + 1] {
                        field_positions[k as usize] = field_positions[k as usize]
                            .wrapping_add(field_positions[(k - 1) as usize]);
                    }
                }
            }
        }

        // Payload lengths.
        let mut payload_index = vec![None; num_fields as usize];
        let mut payload_off = 0i32;
        let mut payload_len = 0i32;
        let mut chunk_total_payload_length = 0i32;
        if total_payloads > 0 {
            let mut all_payload_lengths = vec![0i32; total_payloads as usize];
            {
                let mut reader = BlockPackedReaderIterator::new(
                    input.as_mut(),
                    self.packed_ints_version,
                    PACKED_BLOCK_SIZE,
                    total_payloads as i64,
                )?;
                for i in 0..total_payloads as usize {
                    all_payload_lengths[i] = reader.next()? as i32;
                }
            }

            let mut offset = 0usize;
            let mut term_idx = 0i32;
            for i in 0..skip as usize {
                let f = flags(i as i32);
                let term_count = num_terms_at(i as i64) as i32;
                if (f & PAYLOADS) != 0 {
                    for j in 0..term_count as usize {
                        let freq = term_freqs[(term_idx + j as i32) as usize];
                        for _ in 0..freq {
                            payload_off = payload_off.wrapping_add(all_payload_lengths[offset]);
                            offset += 1;
                        }
                    }
                }
                term_idx = term_idx.wrapping_add(term_count);
            }
            for i in 0..num_fields as usize {
                let f = flags(skip + i as i32);
                let term_count = num_terms_at((skip + i as i32) as i64) as i32;
                if (f & PAYLOADS) != 0 {
                    let total_freq = position_index[i][term_count as usize];
                    let mut idx = vec![0; total_freq as usize + 1];
                    // The index is an offset into the *document's* payload
                    // buffer, which every payload-bearing field of the document
                    // shares, so it starts where the previous field ended —
                    // `payloadIndex[i][0] = payloadLen` in
                    // `Lucene90CompressingTermVectorsReader.java:675`. Starting
                    // at zero makes the first payload of the second and every
                    // later field of a document begin at the buffer's start and
                    // run far too long.
                    idx[0] = payload_len;
                    for pos_idx in 0..total_freq as usize {
                        let pl = all_payload_lengths[offset + pos_idx];
                        payload_len = payload_len.wrapping_add(pl);
                        idx[pos_idx + 1] = payload_len;
                    }
                    offset += total_freq as usize;
                    payload_index[i] = Some(idx);
                }
            }
            // Java also sums the payloads of the fields that follow this
            // document inside the same chunk
            // (`Lucene90CompressingTermVectorsReader.java:688-700`); the total
            // is the decompressor's `originalLength`, which bounds the buffer
            // and is checked against what the stream actually holds. Every
            // remaining entry belongs to a payload-bearing field of a later
            // document, so summing the tail of the array is that third term.
            chunk_total_payload_length = all_payload_lengths[offset..]
                .iter()
                .fold(payload_off.wrapping_add(payload_len), |sum, value| {
                    sum.wrapping_add(*value)
                });
        }

        // Decompress term and payload bytes. Java asserts
        // `offset + length <= originalLength` inside the decompressor
        // (`CompressionMode.java`), which a production JVM skips; here the three
        // figures are all sums of values that came off disk, so they are checked
        // rather than trusted — a negative one would become an enormous `usize`
        // and ask the allocator for it.
        let original_length = i64::from(chunk_total_len) + i64::from(chunk_total_payload_length);
        let block_offset = i64::from(doc_off) + i64::from(payload_off);
        let block_length = i64::from(doc_len) + i64::from(payload_len);
        // Each of the four has to be checked on its own, not only in the two
        // sums: `doc_len` and `payload_len` slice the decompressed block apart
        // below, so a negative one that a positive partner hides in the sum
        // would still become an enormous `usize` there. `doc_len` is a wrapping
        // sum of suffix lengths and `payload_len` of payload lengths, both
        // decoded from the chunk, so both can be negative.
        if chunk_total_len < 0
            || chunk_total_payload_length < 0
            || doc_off < 0
            || doc_len < 0
            || payload_off < 0
            || payload_len < 0
            || block_offset + block_length > original_length
        {
            return Err(LuceneError::CorruptIndex(format!(
                "the chunk claims {doc_len} term bytes at {doc_off} and {payload_len} payload \
                 bytes at {payload_off} of a {original_length}-byte block"
            )));
        }
        // `doc_len` is a wrapping sum of `field_lengths`, so checking it alone
        // is not enough: a negative entry hidden by a positive sibling keeps the
        // total non-negative and reaches `TVFields::terms`, which walks the
        // per-field prefix sum to place each field's window inside the
        // decompressed block. The same masking pattern, one level down.
        if let Some(bad) = field_lengths.iter().position(|length| *length < 0) {
            return Err(LuceneError::CorruptIndex(format!(
                "field {bad} of the document claims {} term bytes",
                field_lengths[bad]
            )));
        }
        let mut suffix_bytes = BytesRef::default();
        let mut decompressor = self.decompressor.lock().unwrap();
        decompressor.decompress(
            input.as_mut(),
            original_length as usize,
            block_offset as usize,
            block_length as usize,
            &mut suffix_bytes,
        )?;
        let payload_len_usize = payload_len as usize;
        let payload_bytes = BytesRef {
            bytes: suffix_bytes.bytes.clone(),
            offset: suffix_bytes.offset + doc_len as usize,
            length: payload_len_usize,
        };
        suffix_bytes.length = doc_len as usize;
        drop(decompressor);

        let mut field_flags = vec![0i32; num_fields as usize];
        let mut field_num_terms = vec![0i32; num_fields as usize];
        let mut field_term_freqs: Vec<Vec<i32>> = vec![Vec::new(); num_fields as usize];
        term_index = 0;
        for i in 0..skip as usize {
            term_index = term_index.wrapping_add(num_terms_at(i as i64) as i32);
        }
        for i in 0..num_fields as usize {
            field_flags[i] = flags(skip + i as i32);
            let term_count = num_terms_at((skip + i as i32) as i64) as i32;
            field_num_terms[i] = term_count;
            field_term_freqs[i] = vec![0; term_count as usize];
            for j in 0..term_count as usize {
                field_term_freqs[i][j] = term_freqs[(term_index + j as i32) as usize];
            }
            term_index = term_index.wrapping_add(term_count);
        }

        Ok(Some(Box::new(TVFields {
            field_infos: self.field_infos.clone(),
            field_nums,
            field_flags,
            field_num_offs,
            field_num_terms,
            field_lengths,
            prefix_lengths,
            suffix_lengths,
            field_term_freqs,
            position_index,
            positions,
            start_offsets,
            lengths,
            payload_bytes,
            payload_index,
            suffix_bytes,
        })))
    }

    /// Asks the `.tvd` input to load the block that holds `doc_id`, unless it
    /// was one of the last sixteen blocks prefetched.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.prefetch(int)`
    /// (`Lucene90CompressingTermVectorsReader.java:347-361`).
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        let block_id = self.index_reader.get_block_id(doc_id)?;

        let mut prefetched_block_id_cache = self.prefetched_block_id_cache.lock().unwrap();
        for &prefetched_block_id in prefetched_block_id_cache.iter() {
            if prefetched_block_id == block_id {
                return Ok(());
            }
        }

        let block_start_pointer = self.index_reader.get_block_start_pointer(block_id)?;
        let block_length = self.index_reader.get_block_length(block_id)?;
        self.vectors_stream
            .prefetch(block_start_pointer, block_length)?;

        let index = self
            .prefetched_block_id_cache_index
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        prefetched_block_id_cache[(index & PREFETCH_CACHE_MASK) as usize] = block_id;
        Ok(())
    }
}

impl Lucene90CompressingTermVectorsReader {
    /// Returns the number of documents this reader covers, which is the
    /// segment's `maxDoc`.
    ///
    /// Equivalent to the package-private `Lucene90CompressingTermVectorsReader#getNumDocs`
    /// (`Lucene90CompressingTermVectorsReader.java:301-303`).
    pub fn get_num_docs(&self) -> i32 {
        self.num_docs
    }

    /// Adds up the entries of `arr`.
    ///
    /// Equivalent to the private
    /// `Lucene90CompressingTermVectorsReader.sum(int[])`. Java's `int`
    /// arithmetic wraps on overflow, so this wraps too: the values come off
    /// disk and a corrupt chunk must fail the length checks that follow rather
    /// than panic here.
    fn sum(arr: &[i32]) -> i32 {
        arr.iter().fold(0i32, |sum, value| sum.wrapping_add(*value))
    }

    /// Fails when this reader has already been closed.
    ///
    /// Equivalent to the private
    /// `Lucene90CompressingTermVectorsReader.ensureOpen()`, including its
    /// message.
    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(LuceneError::AlreadyClosed(
                "this FieldsReader is closed".to_string(),
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::needless_range_loop)]
    /// Builds, per field of the document, the running offset at which each of
    /// its terms' positions (or offsets, or payloads) start inside that field's
    /// flat array — so entry `j + 1` is entry `j` plus the frequency of term
    /// `j`, and the last entry is the field's total number of occurrences.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsReader#positionIndex`.
    fn position_index(
        skip: i32,
        num_fields: i32,
        num_terms_at: &dyn Fn(i64) -> i64,
        term_freqs: &[i32],
    ) -> Vec<Vec<i32>> {
        let mut position_index = vec![Vec::new(); num_fields as usize];
        let mut term_index = 0i32;
        for i in 0..skip as usize {
            term_index = term_index.wrapping_add(num_terms_at(i as i64) as i32);
        }
        for i in 0..num_fields as usize {
            let term_count = num_terms_at((skip + i as i32) as i64) as usize;
            position_index[i] = vec![0; term_count + 1];
            for j in 0..term_count {
                let freq = term_freqs[(term_index + j as i32) as usize];
                position_index[i][j + 1] = position_index[i][j] + freq;
            }
            term_index = term_index.wrapping_add(term_count as i32);
        }
        position_index
    }

    /// Returns the format version this segment was written with.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.getVersion`.
    fn version(&self) -> i32 {
        self.version
    }

    /// Returns the packed-integers version the block-packed streams use.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.getPackedIntsVersion`.
    fn packed_ints_version(&self) -> i32 {
        self.packed_ints_version
    }

    /// Returns the uncompressed size of one chunk, in bytes.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.getChunkSize`.
    fn chunk_size(&self) -> i32 {
        self.chunk_size
    }

    /// Returns the compression mode the chunks were written with.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.getCompressionMode`.
    fn compression_mode(&self) -> CompressionMode {
        self.compression_mode
    }

    /// Returns the number of chunks in this segment.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.getNumChunks`.
    fn num_chunks(&self) -> i64 {
        self.num_chunks
    }

    /// Returns the number of chunks that were flushed before they were full.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.getNumDirtyChunks`.
    fn num_dirty_chunks(&self) -> i64 {
        self.num_dirty_chunks
    }

    /// Returns the number of documents held by those incomplete chunks.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.getNumDirtyDocs`.
    fn num_dirty_docs(&self) -> i64 {
        self.num_dirty_docs
    }

    /// Returns the offset just past the last chunk of the `.tvd` file.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.getMaxPointer`.
    fn max_pointer(&self) -> i64 {
        self.max_pointer
    }

    /// Returns the term-vectors index, which maps a document to the start of
    /// the chunk that holds it.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.getIndexReader`.
    fn index_reader(&self) -> &dyn FieldsIndex {
        self.index_reader.as_ref()
    }

    /// Opens a cursor over the `.tvd` file, positioned at its start.
    ///
    /// Divergence: Lucene hands out its own `vectorsStream` and lets the caller
    /// seek it (`Lucene90CompressingTermVectorsReader.java:333-335`). A `&self`
    /// method cannot hand out a seekable cursor here, so an independent clone
    /// of the same file is returned; a clone starts at offset zero and the
    /// caller seeks it exactly as Java does.
    ///
    /// # Errors
    ///
    /// Propagates the I/O error raised while cloning the input.
    fn vectors_stream(&self) -> Result<Box<dyn IndexInput>> {
        self.vectors_stream.clone_input()
    }

    /// Returns whether `doc_id` falls inside the chunk this reader last
    /// decoded, in which case its start pointer is already known.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.isLoaded`.
    fn is_loaded(&self, doc_id: i32) -> bool {
        let state = self.block_state.lock().unwrap();
        state.doc_base <= doc_id && doc_id < state.doc_base.wrapping_add(state.chunk_docs)
    }

    fn read_positions(
        input: &mut dyn DataInput,
        packed_ints_version: i32,
        skip: i32,
        num_fields: i32,
        num_terms_at: &dyn Fn(i64) -> i64,
        term_freqs: &[i32],
        flags: &dyn Fn(i32) -> i32,
        flag: i32,
        total_positions: i32,
        position_index: &[Vec<i32>],
    ) -> Result<Vec<Option<Vec<i32>>>> {
        let mut reader = BlockPackedReaderIterator::new(
            input,
            packed_ints_version,
            PACKED_BLOCK_SIZE,
            total_positions as i64,
        )?;
        let mut all_values = vec![0i32; total_positions as usize];
        for i in 0..total_positions as usize {
            all_values[i] = reader.next()? as i32;
        }

        let mut offset = 0usize;
        let mut term_index = 0i32;
        for i in 0..skip as usize {
            let f = flags(i as i32);
            let term_count = num_terms_at(i as i64) as i32;
            if (f & flag) != 0 {
                for j in 0..term_count as usize {
                    let freq = term_freqs[(term_index + j as i32) as usize];
                    offset += freq as usize;
                }
            }
            term_index += term_count;
        }

        let mut result = vec![None; num_fields as usize];
        for i in 0..num_fields as usize {
            let f = flags(skip + i as i32);
            let term_count = num_terms_at((skip + i as i32) as i64) as i32;
            if (f & flag) != 0 {
                let total_freq = position_index[i][term_count as usize];
                result[i] = Some(all_values[offset..offset + total_freq as usize].to_vec());
                offset += total_freq as usize;
            }
        }
        Ok(result)
    }
}

// -----------------------------------------------------------------------------
// TVFields / TVTerms / TVTermsEnum / TVPostingsEnum
// -----------------------------------------------------------------------------

struct TVFields {
    field_infos: FieldInfos,
    field_nums: Vec<i32>,
    field_flags: Vec<i32>,
    field_num_offs: Vec<i32>,
    field_num_terms: Vec<i32>,
    field_lengths: Vec<i32>,
    prefix_lengths: Vec<Vec<i32>>,
    suffix_lengths: Vec<Vec<i32>>,
    field_term_freqs: Vec<Vec<i32>>,
    position_index: Vec<Vec<i32>>,
    positions: Vec<Option<Vec<i32>>>,
    start_offsets: Vec<Option<Vec<i32>>>,
    lengths: Vec<Option<Vec<i32>>>,
    payload_bytes: BytesRef,
    payload_index: Vec<Option<Vec<i32>>>,
    suffix_bytes: BytesRef,
}

impl Fields for TVFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.field_num_offs.iter().map(|off| {
            let field_num = self.field_nums[*off as usize];
            self.field_infos
                .field_info_by_number(field_num)
                .map(|fi| fi.name.clone())
                .unwrap_or_default()
        }))
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let field_info = self.field_infos.field_info(field);
        if field_info.is_none() {
            return Ok(None);
        }
        let field_info = field_info.unwrap();
        let idx = self
            .field_num_offs
            .iter()
            .position(|off| self.field_nums[*off as usize] == field_info.number);
        if idx.is_none() || self.field_num_terms[idx.unwrap()] == 0 {
            return Ok(None);
        }
        let idx = idx.unwrap();
        // Every length here was decoded from the chunk. `Lucene90CompressingTermVectorsReader`
        // bounds the field's window with
        // `new ByteArrayDataInput(termBytes.bytes, termBytes.offset, termBytes.length)`
        // (`:1013`) and answers a window outside the block with a catchable
        // `ArrayIndexOutOfBoundsException`; unchecked arithmetic here would
        // overflow instead, and a `usize` cast would then place the window
        // anywhere at all.
        let mut field_off = 0i64;
        for i in 0..idx {
            field_off += i64::from(self.field_lengths[i]);
        }
        let field_len = i64::from(self.field_lengths[idx]);
        let block_len = self.suffix_bytes.length as i64;
        if field_off < 0 || field_len < 0 || field_off + field_len > block_len {
            return Err(LuceneError::CorruptIndex(format!(
                "field {field} claims {field_len} term bytes at {field_off} of a \
                 {block_len}-byte block"
            )));
        }
        Ok(Some(Box::new(TVTerms {
            num_terms: self.field_num_terms[idx],
            flags: self.field_flags[idx],
            prefix_lengths: self.prefix_lengths[idx].clone(),
            suffix_lengths: self.suffix_lengths[idx].clone(),
            term_freqs: self.field_term_freqs[idx].clone(),
            position_index: self.position_index[idx].clone(),
            positions: self.positions[idx].clone(),
            start_offsets: self.start_offsets[idx].clone(),
            lengths: self.lengths[idx].clone(),
            payload_index: self.payload_index[idx].clone(),
            payload_bytes: self.payload_bytes.clone(),
            term_bytes: BytesRef {
                bytes: self.suffix_bytes.bytes.clone(),
                offset: self.suffix_bytes.offset + field_off as usize,
                length: field_len as usize,
            },
        })))
    }

    fn size(&self) -> i32 {
        self.field_num_offs.len() as i32
    }
}

struct TVTerms {
    num_terms: i32,
    flags: i32,
    prefix_lengths: Vec<i32>,
    suffix_lengths: Vec<i32>,
    term_freqs: Vec<i32>,
    position_index: Vec<i32>,
    positions: Option<Vec<i32>>,
    start_offsets: Option<Vec<i32>>,
    lengths: Option<Vec<i32>>,
    payload_index: Option<Vec<i32>>,
    payload_bytes: BytesRef,
    term_bytes: BytesRef,
}

impl Terms for TVTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(TVTermsEnum::new(
            self.num_terms,
            self.flags,
            self.prefix_lengths.clone(),
            self.suffix_lengths.clone(),
            self.term_freqs.clone(),
            self.position_index.clone(),
            self.positions.clone(),
            self.start_offsets.clone(),
            self.lengths.clone(),
            self.payload_index.clone(),
            self.payload_bytes.clone(),
            self.term_bytes.clone(),
        )?))
    }

    fn size(&self) -> Result<i64> {
        Ok(self.num_terms as i64)
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Ok(self.term_freqs.iter().map(|&f| f as i64).sum())
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok(self.num_terms as i64)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(1)
    }

    fn has_freqs(&self) -> bool {
        true
    }

    fn has_offsets(&self) -> bool {
        (self.flags & OFFSETS) != 0
    }

    fn has_positions(&self) -> bool {
        (self.flags & POSITIONS) != 0
    }

    fn has_payloads(&self) -> bool {
        (self.flags & PAYLOADS) != 0
    }
}

struct TVTermsEnum {
    num_terms: i32,
    prefix_lengths: Vec<i32>,
    suffix_lengths: Vec<i32>,
    term_freqs: Vec<i32>,
    position_index: Vec<i32>,
    positions: Option<Vec<i32>>,
    start_offsets: Option<Vec<i32>>,
    lengths: Option<Vec<i32>>,
    payload_index: Option<Vec<i32>>,
    payload_bytes: BytesRef,
    // Java holds `ByteArrayDataInput in`, constructed over the field's window
    // inside the whole decompressed block
    // (`Lucene90CompressingTermVectorsReader.java:1013`, `:1069`). A struct that
    // holds both a buffer and a view of that buffer is a self-reference Rust
    // forbids, so the block stays here and the view's fields — position and
    // limit — sit beside it, with [`ByteArrayDataInput`] rebuilt around them for
    // each term.
    /// The whole decompressed suffix block this field's terms live in.
    term_bytes: BytesRef,
    /// Read position inside `term_bytes.bytes`.
    input_pos: usize,
    /// Where this field's window ends inside `term_bytes.bytes`.
    input_limit: usize,
    /// Where this field's window starts. Java's `TVTermsEnum.startPos`
    /// (`Lucene90CompressingTermVectorsReader.java:1060`, `:1101`).
    start_pos: usize,
    term: BytesRef,
    ord: i32,
    atts: AttributeSource,
}

impl TVTermsEnum {
    #[allow(clippy::too_many_arguments)]
    fn new(
        num_terms: i32,
        _flags: i32,
        prefix_lengths: Vec<i32>,
        suffix_lengths: Vec<i32>,
        term_freqs: Vec<i32>,
        position_index: Vec<i32>,
        positions: Option<Vec<i32>>,
        start_offsets: Option<Vec<i32>>,
        lengths: Option<Vec<i32>>,
        payload_index: Option<Vec<i32>>,
        payload_bytes: BytesRef,
        term_bytes: BytesRef,
    ) -> Result<Self> {
        // Java constructs the input over the field's own window —
        // `new ByteArrayDataInput(termBytes.bytes, termBytes.offset, termBytes.length)`
        // (`Lucene90CompressingTermVectorsReader.java:1013`) — which is exactly
        // what the position and limit kept below describe. That window is not an
        // enforced limit in Java: `ByteArrayDataInput.readBytes` is a bare
        // `System.arraycopy` (`ByteArrayDataInput.java:141-144`) and `limit` is
        // consulted only by `eof()` and `length()`, so a field that over-reads
        // silently takes the next field's bytes and throws only at the backing
        // array's end. This port keeps the window honest in `next()` instead,
        // where the remaining bytes are checked before the read, so an over-read
        // is a `CorruptIndexException` rather than a neighbouring field's
        // contents; the window itself is still validated against the block here.
        let end = term_bytes
            .offset
            .checked_add(term_bytes.length)
            .filter(|end| *end <= term_bytes.bytes.len())
            .ok_or_else(|| {
                LuceneError::CorruptIndex(format!(
                    "a field window of {} bytes at {} is outside the {}-byte decompressed block",
                    term_bytes.length,
                    term_bytes.offset,
                    term_bytes.bytes.len()
                ))
            })?;
        let start_pos = term_bytes.offset;
        Ok(Self {
            num_terms,
            prefix_lengths,
            suffix_lengths,
            term_freqs,
            position_index,
            positions,
            start_offsets,
            lengths,
            payload_index,
            payload_bytes,
            term_bytes,
            input_pos: start_pos,
            input_limit: end,
            start_pos,
            term: BytesRef::with_capacity(16),
            ord: -1,
            atts: AttributeSource::new(),
        })
    }

    fn reset(&mut self) {
        self.term.length = 0;
        // Java's `in.setPosition(startPos)`
        // (`Lucene90CompressingTermVectorsReader.java:1107`) — the start of the
        // field's window, not of the block.
        self.input_pos = self.start_pos;
        self.ord = -1;
    }
}

impl std::fmt::Display for TVTermsEnum {
    /// `TVTermsEnum` declares no `toString()` in Lucene 10.5.0, so
    /// `java.lang.Object`'s applies: the binary class name, `@`, and the
    /// identity hash. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(

            "org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsReader$TVTermsEnum",

            self,

        ))
    }
}

impl TermsEnum for TVTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.atts
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        match self.seek_ceil(text)? {
            SeekStatus::FOUND => Ok(true),
            _ => Ok(false),
        }
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        if self.ord >= 0 && self.ord < self.num_terms {
            let cmp = self.term.cmp(text);
            if cmp == Ordering::Equal {
                return Ok(SeekStatus::FOUND);
            } else if cmp == Ordering::Greater {
                self.reset();
            }
        }
        loop {
            match self.next()? {
                Some(term) => {
                    let cmp = term.cmp(text);
                    if cmp == Ordering::Equal {
                        return Ok(SeekStatus::FOUND);
                    } else if cmp == Ordering::Greater {
                        return Ok(SeekStatus::NOT_FOUND);
                    }
                }
                None => return Ok(SeekStatus::END),
            }
        }
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "seek_ord not supported by TVTermsEnum".to_string(),
        ))
    }

    fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "seek_term_state not supported by TVTermsEnum".to_string(),
        ))
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(self.term.clone())
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "ord not supported by TVTermsEnum".to_string(),
        ))
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Ok(1)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        if self.ord < 0 || self.ord >= self.num_terms {
            return Err(LuceneError::IllegalState(
                "total_term_freq called before first term".to_string(),
            ));
        }
        Ok(self.term_freqs[self.ord as usize] as i64)
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        let mut postings = TVPostingsEnum::new();
        if self.ord < 0 || self.ord >= self.num_terms {
            return Err(LuceneError::IllegalState(
                "postings called before first term".to_string(),
            ));
        }
        postings.reset(
            self.term_freqs[self.ord as usize],
            self.position_index[self.ord as usize],
            self.positions.clone(),
            self.start_offsets.clone(),
            self.lengths.clone(),
            self.payload_bytes.clone(),
            self.payload_index.clone(),
        );
        Ok(Box::new(postings))
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        let postings = self.postings(None, flags)?;
        Ok(Box::new(SlowImpactsEnum::new(postings)))
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Err(LuceneError::UnsupportedOperation(
            "term_state not supported by TVTermsEnum".to_string(),
        ))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.ord == self.num_terms - 1 {
            return Ok(None);
        }
        self.ord += 1;
        // Both lengths were decoded from the chunk, so both are untrusted. Java
        // reaches `ArrayUtil.grow(term.bytes, prefix + suffix)`, which answers a
        // negative sum with an `IllegalArgumentException` and an absurd one with
        // an `OutOfMemoryError` — catchable either way. In Rust a negative
        // `i32` cast to `usize` becomes about 1.8e19 and the `resize` aborts
        // the process, so the two lengths are validated instead.
        let prefix = self.prefix_lengths[self.ord as usize];
        let suffix = self.suffix_lengths[self.ord as usize];
        if prefix < 0 || suffix < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "term {} of the field has a prefix of {prefix} and a suffix of {suffix}",
                self.ord
            )));
        }
        let prefix = prefix as usize;
        let suffix = suffix as usize;
        let new_len = prefix + suffix;
        // The term bytes are a window into the decompressed suffix block, so no
        // term can be longer than what is left of it.
        let available = self.input_limit.saturating_sub(self.input_pos);
        if suffix > available {
            return Err(LuceneError::CorruptIndex(format!(
                "term {} of the field claims a {suffix}-byte suffix, but only {available} bytes \
                 of the decompressed block remain",
                self.ord
            )));
        }
        if new_len > MAX_TERM_LENGTH {
            return Err(LuceneError::CorruptIndex(format!(
                "term {} of the field is {new_len} bytes, above the {MAX_TERM_LENGTH}-byte maximum",
                self.ord
            )));
        }
        if self.term.bytes.len() < new_len {
            self.term.bytes.resize(new_len, 0);
        }
        let Self {
            term_bytes,
            input_pos,
            input_limit,
            term,
            ..
        } = self;
        let mut input = ByteArrayDataInput::with_range(&term_bytes.bytes, 0, *input_limit);
        input.set_position(*input_pos);
        let read = input.read_bytes(&mut term.bytes, prefix, suffix);
        *input_pos = input.position();
        read?;
        self.term.offset = 0;
        self.term.length = new_len;
        Ok(Some(self.term.clone()))
    }
}

#[derive(Clone)]
struct TVPostingsEnum {
    doc: i32,
    term_freq: i32,
    position_index: i32,
    positions: Option<Vec<i32>>,
    start_offsets: Option<Vec<i32>>,
    lengths: Option<Vec<i32>>,
    payload_bytes: BytesRef,
    payload_index: Option<Vec<i32>>,
    payload: BytesRef,
    i: i32,
}

impl TVPostingsEnum {
    fn new() -> Self {
        Self {
            doc: -1,
            term_freq: 0,
            position_index: 0,
            positions: None,
            start_offsets: None,
            lengths: None,
            payload_bytes: BytesRef::default(),
            payload_index: None,
            payload: BytesRef::default(),
            i: -1,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reset(
        &mut self,
        freq: i32,
        position_index: i32,
        positions: Option<Vec<i32>>,
        start_offsets: Option<Vec<i32>>,
        lengths: Option<Vec<i32>>,
        payload_bytes: BytesRef,
        payload_index: Option<Vec<i32>>,
    ) {
        self.term_freq = freq;
        self.position_index = position_index;
        self.positions = positions;
        self.start_offsets = start_offsets;
        self.lengths = lengths;
        self.payload_bytes = payload_bytes.clone();
        self.payload_index = payload_index;
        self.payload = BytesRef::default();
        self.payload.bytes = payload_bytes.bytes;
        self.doc = -1;
        self.i = -1;
    }
}

impl DocIdSetIterator for TVPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.doc == -1 {
            self.doc = 0;
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        self.next_doc()
    }

    fn cost(&self) -> i64 {
        1
    }
}

impl std::fmt::Display for TVPostingsEnum {
    /// `TVPostingsEnum` declares no `toString()` in Lucene 10.5.0, so
    /// `java.lang.Object`'s applies: the binary class name, `@`, and the
    /// identity hash. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(

            "org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsReader$TVPostingsEnum",

            self,

        ))
    }
}

impl PostingsEnum for TVPostingsEnum {
    fn freq(&self) -> Result<i32> {
        if self.doc == NO_MORE_DOCS || self.doc == -1 {
            return Err(LuceneError::IllegalState(
                "freq called on unpositioned postings enum".to_string(),
            ));
        }
        Ok(self.term_freq)
    }

    fn next_position(&mut self) -> Result<i32> {
        if self.doc != 0 {
            return Err(LuceneError::IllegalState(
                "next_position called on unpositioned postings enum".to_string(),
            ));
        }
        if self.i >= self.term_freq - 1 {
            return Err(LuceneError::IllegalState(
                "read past last position".to_string(),
            ));
        }
        self.i += 1;
        if let Some(ref idx) = self.payload_index {
            // Java assigns `payload.offset`/`payload.length` straight from the
            // index and hands back a nonsense `BytesRef`; the caller then trips
            // an `ArrayIndexOutOfBoundsException` when it reads. Rust would
            // instead panic inside `BytesRef::slice`, so the window is
            // validated against the buffer here, where the error can still be
            // reported.
            let slot = (self.position_index + self.i) as usize;
            let (Some(&from), Some(&to)) = (idx.get(slot), idx.get(slot + 1)) else {
                return Err(LuceneError::CorruptIndex(format!(
                    "payload {slot} is outside the {} entries of the payload index",
                    idx.len()
                )));
            };
            let base = self.payload_bytes.offset;
            let end = self.payload_bytes.offset + self.payload_bytes.length;
            if from < 0
                || to < from
                || base + to as usize > end
                || end > self.payload_bytes.bytes.len()
            {
                return Err(LuceneError::CorruptIndex(format!(
                    "payload {slot} spans [{from}, {to}) of a {}-byte payload buffer",
                    self.payload_bytes.length
                )));
            }
            self.payload.offset = base + from as usize;
            self.payload.length = (to - from) as usize;
        }
        match &self.positions {
            Some(positions) => Ok(positions[(self.position_index + self.i) as usize]),
            None => Ok(-1),
        }
    }

    fn start_offset(&self) -> i32 {
        if self.i < 0 || self.i >= self.term_freq {
            return -1;
        }
        match &self.start_offsets {
            Some(offsets) => offsets[(self.position_index + self.i) as usize],
            None => -1,
        }
    }

    fn end_offset(&self) -> i32 {
        if self.i < 0 || self.i >= self.term_freq {
            return -1;
        }
        match (&self.start_offsets, &self.lengths) {
            (Some(offsets), Some(lengths)) => {
                // `startOffsets[i] + lengths[i]` in Java, on `int`s decoded
                // from the chunk: it wraps there and must wrap here.
                offsets[(self.position_index + self.i) as usize]
                    .wrapping_add(lengths[(self.position_index + self.i) as usize])
            }
            _ => -1,
        }
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        if self.i < 0 || self.i >= self.term_freq {
            return Ok(None);
        }
        if self.payload.length == 0 {
            return Ok(None);
        }
        Ok(Some(self.payload.slice()))
    }
}

// SlowImpactsEnum wraps a PostingsEnum and reports a single impact level.
struct SlowImpactsEnum {
    postings: Box<dyn PostingsEnum>,
}

impl SlowImpactsEnum {
    fn new(postings: Box<dyn PostingsEnum>) -> Self {
        Self { postings }
    }
}

impl DocIdSetIterator for SlowImpactsEnum {
    fn doc_id(&self) -> i32 {
        self.postings.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.postings.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.postings.advance(target)
    }

    fn cost(&self) -> i64 {
        self.postings.cost()
    }
}

impl std::fmt::Display for SlowImpactsEnum {
    /// `org.apache.lucene.index.SlowImpactsEnum` declares no `toString()` in Lucene 10.5.0, so
    /// `java.lang.Object`'s applies: the binary class name, `@`, and the
    /// identity hash. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.SlowImpactsEnum",
            self,
        ))
    }
}

impl PostingsEnum for SlowImpactsEnum {
    fn freq(&self) -> Result<i32> {
        self.postings.freq()
    }

    fn next_position(&mut self) -> Result<i32> {
        self.postings.next_position()
    }

    fn start_offset(&self) -> i32 {
        self.postings.start_offset()
    }

    fn end_offset(&self) -> i32 {
        self.postings.end_offset()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.postings.get_payload()
    }
}

impl ImpactsSource for SlowImpactsEnum {
    fn advance_shallow(&mut self, _target: i32) -> Result<()> {
        Ok(())
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        let freq = self.postings.freq()?;
        Ok(Box::new(SingleImpact { freq }))
    }
}

impl ImpactsEnum for SlowImpactsEnum {}

struct SingleImpact {
    freq: i32,
}

impl Impacts for SingleImpact {
    fn num_levels(&self) -> i32 {
        1
    }

    fn doc_id_up_to(&self, _level: i32) -> i32 {
        NO_MORE_DOCS
    }

    fn get_impacts(&self, _level: i32) -> crate::index::postings_enum::FreqAndNormBuffer {
        let mut buf = crate::index::postings_enum::FreqAndNormBuffer::new();
        buf.grow_no_copy(1);
        buf.freqs[0] = self.freq;
        buf.norms[0] = 1;
        buf.size = 1;
        buf
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::compound::CompoundFormat;
    use crate::index::postings_enum::{POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS};
    use crate::index::{FieldInfo as RealFieldInfo, IndexOptions};
    use crate::search::Sort;
    use crate::store::RamDirectory;
    use crate::store::DEFAULT_IO_CONTEXT;
    use crate::util::string_helper::ID_LENGTH;
    use crate::util::Bits;
    use crate::util::Version;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn test_segment_info(dir: Arc<dyn Directory>, name: &str, max_doc: i32) -> SegmentInfo {
        SegmentInfo::new(
            dir,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            false,
            false,
            Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            HashMap::new(),
            [0u8; ID_LENGTH],
            HashMap::new(),
            Sort::default(),
        )
        .unwrap()
    }

    fn make_field_infos() -> FieldInfos {
        let mut body = RealFieldInfo::new("body", 0);
        body.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS;
        let mut title = RealFieldInfo::new("title", 1);
        title.index_options = IndexOptions::DOCS_AND_FREQS;
        FieldInfos::new(vec![body, title]).unwrap()
    }

    fn write_read_round_trip(compression_mode: CompressionMode) {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = make_field_infos();
        let segment_info = test_segment_info(Arc::clone(&dir), "_0", 3);

        let format = Lucene90CompressingTermVectorsFormat::new(
            VECTORS_CODEC_NAME,
            "",
            compression_mode,
            1 << 12,
            128,
            10,
        )
        .unwrap();

        {
            let mut writer = format
                .vectors_writer(dir.as_ref(), &segment_info, &*DEFAULT_IO_CONTEXT)
                .unwrap();

            // Document 0: body with positions and offsets.
            writer.start_document(1).unwrap();
            writer
                .start_field(
                    field_infos.field_info("body").unwrap(),
                    2,
                    true,
                    true,
                    false,
                )
                .unwrap();
            writer
                .start_term(&BytesRef::new(b"hello".to_vec()), 1)
                .unwrap();
            writer.add_position(0, 0, 5, None).unwrap();
            writer.finish_term().unwrap();
            writer
                .start_term(&BytesRef::new(b"world".to_vec()), 1)
                .unwrap();
            writer.add_position(1, 6, 11, None).unwrap();
            writer.finish_term().unwrap();
            writer.finish_field().unwrap();
            writer.finish_document().unwrap();

            // Document 1: title without positions/offsets.
            writer.start_document(1).unwrap();
            writer
                .start_field(
                    field_infos.field_info("title").unwrap(),
                    2,
                    false,
                    false,
                    false,
                )
                .unwrap();
            writer
                .start_term(&BytesRef::new(b"lucene".to_vec()), 2)
                .unwrap();
            writer.finish_term().unwrap();
            writer
                .start_term(&BytesRef::new(b"rust".to_vec()), 1)
                .unwrap();
            writer.finish_term().unwrap();
            writer.finish_field().unwrap();
            writer.finish_document().unwrap();

            // Document 2: empty vectors.
            writer.start_document(0).unwrap();
            writer.finish_document().unwrap();

            writer.finish(3).unwrap();
            writer.close().unwrap();
        }

        let reader = format
            .vectors_reader(
                dir.as_ref(),
                &segment_info,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .unwrap();

        // Doc 0.
        let fields = match reader.get(0) {
            Ok(Some(f)) => f,
            Ok(None) => panic!("doc 0 has vectors"),
            Err(e) => panic!("reader.get(0) failed: {e:?}"),
        };
        assert_eq!(fields.size(), 1);
        let mut names: Vec<String> = fields.iterator().collect();
        names.sort();
        assert_eq!(names, vec!["body"]);
        let terms = fields.terms("body").unwrap().expect("body terms");
        assert!(terms.has_positions());
        assert!(terms.has_offsets());
        assert!(!terms.has_payloads());
        let mut it = terms.iterator().unwrap();
        let t1 = it.next().unwrap().expect("first term");
        assert_eq!(t1.slice(), b"hello");
        let mut postings = it.postings(None, POSTINGS_ENUM_POSITIONS).unwrap();
        assert_eq!(postings.next_doc().unwrap(), 0);
        assert_eq!(postings.freq().unwrap(), 1);
        assert_eq!(postings.next_position().unwrap(), 0);
        assert_eq!(postings.start_offset(), 0);
        assert_eq!(postings.end_offset(), 5);
        let t2 = it.next().unwrap().expect("second term");
        assert_eq!(t2.slice(), b"world");

        // Doc 1.
        let fields = reader.get(1).unwrap().expect("doc 1 has vectors");
        let terms = fields.terms("title").unwrap().expect("title terms");
        assert!(!terms.has_positions());
        let mut it = terms.iterator().unwrap();
        let t1 = it.next().unwrap().expect("first term");
        assert_eq!(
            t1.slice(),
            b"lucene",
            "expected 'lucene', got {:?}",
            String::from_utf8_lossy(t1.slice())
        );
        assert_eq!(it.total_term_freq().unwrap(), 2);
        let t2 = it.next().unwrap().expect("second term");
        assert_eq!(t2.slice(), b"rust");
        assert!(it.next().unwrap().is_none());

        // Doc 2.
        assert!(reader.get(2).unwrap().is_none());

        reader.check_integrity().unwrap();
        let _clone = reader.clone_reader();
        let _merge = reader.get_merge_instance();

        // Files are non-empty and have valid headers/footers.
        assert!(dir.file_length("_0.tvd").unwrap() > 0);
        assert!(dir.file_length("_0.tvx").unwrap() > 0);
        assert!(dir.file_length("_0.tvm").unwrap() > 0);
    }

    #[test]
    fn round_trip_fast() {
        write_read_round_trip(CompressionMode::FAST);
    }

    #[test]
    fn round_trip_high_compression() {
        write_read_round_trip(CompressionMode::HIGH_COMPRESSION);
    }

    #[test]
    fn round_trip_payloads() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut tags = RealFieldInfo::new("tags", 0);
        tags.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;
        let field_infos = FieldInfos::new(vec![tags]).unwrap();
        let segment_info = test_segment_info(Arc::clone(&dir), "_0", 1);

        let format = Lucene90CompressingTermVectorsFormat::new(
            VECTORS_CODEC_NAME,
            "",
            CompressionMode::FAST,
            1 << 12,
            128,
            10,
        )
        .unwrap();

        {
            let mut writer = format
                .vectors_writer(dir.as_ref(), &segment_info, &*DEFAULT_IO_CONTEXT)
                .unwrap();
            writer.start_document(1).unwrap();
            writer
                .start_field(
                    field_infos.field_info("tags").unwrap(),
                    2,
                    true,
                    false,
                    true,
                )
                .unwrap();
            writer
                .start_term(&BytesRef::new(b"alpha".to_vec()), 1)
                .unwrap();
            let p1 = BytesRef::new(b"P1".to_vec());
            writer.add_position(0, 0, 0, Some(&p1)).unwrap();
            writer.finish_term().unwrap();
            writer
                .start_term(&BytesRef::new(b"beta".to_vec()), 1)
                .unwrap();
            let p2 = BytesRef::new(b"P2".to_vec());
            writer.add_position(7, 0, 0, Some(&p2)).unwrap();
            writer.finish_term().unwrap();
            writer.finish_field().unwrap();
            writer.finish_document().unwrap();
            writer.finish(1).unwrap();
            writer.close().unwrap();
        }

        let reader = format
            .vectors_reader(
                dir.as_ref(),
                &segment_info,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .unwrap();
        let fields = reader.get(0).unwrap().expect("doc 0 has vectors");
        let terms = fields.terms("tags").unwrap().expect("tags terms");
        assert!(terms.has_positions());
        assert!(terms.has_payloads());
        assert!(!terms.has_offsets());
        let mut it = terms.iterator().unwrap();

        let t1 = it.next().unwrap().expect("first term");
        assert_eq!(t1.slice(), b"alpha");
        let mut postings = it.postings(None, POSTINGS_ENUM_POSITIONS).unwrap();
        assert_eq!(postings.next_doc().unwrap(), 0);
        assert_eq!(postings.freq().unwrap(), 1);
        assert_eq!(postings.next_position().unwrap(), 0);
        assert_eq!(postings.get_payload().unwrap().unwrap(), b"P1".as_slice());

        let t2 = it.next().unwrap().expect("second term");
        assert_eq!(t2.slice(), b"beta");
        let mut postings = it.postings(None, POSTINGS_ENUM_POSITIONS).unwrap();
        assert_eq!(postings.next_doc().unwrap(), 0);
        assert_eq!(postings.freq().unwrap(), 1);
        assert_eq!(postings.next_position().unwrap(), 7);
        assert_eq!(postings.get_payload().unwrap().unwrap(), b"P2".as_slice());
        assert!(it.next().unwrap().is_none());

        reader.check_integrity().unwrap();
    }

    #[test]
    fn term_vectors_format_name() {
        let format = Lucene90TermVectorsFormat::new();
        assert_eq!(format.name(), "Lucene90");
    }

    // -- Regressions -------------------------------------------------------

    /// One occurrence of a term: its position, its two offsets and its payload.
    type Occurrence<'a> = (i32, i32, i32, Option<&'a [u8]>);

    /// One term of a scripted field: its text and its occurrences.
    type ScriptedTerm<'a> = (&'a str, Vec<Occurrence<'a>>);

    /// One scripted field: its name, whether it stores positions, offsets and
    /// payloads, and its terms.
    type ScriptedField<'a> = (&'a str, bool, bool, bool, Vec<ScriptedTerm<'a>>);

    /// Writes one document made of `fields`, exactly as a
    /// `TermVectorsConsumer` would.
    fn write_document(
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        fields: &[ScriptedField<'_>],
    ) {
        let format = Lucene90TermVectorsFormat::new();
        let mut writer = format
            .vectors_writer(directory, segment_info, &*DEFAULT_IO_CONTEXT)
            .expect("writer");
        writer
            .start_document(fields.len() as i32)
            .expect("start document");
        for (name, positions, offsets, payloads, terms) in fields {
            writer
                .start_field(
                    field_infos.field_info(name).expect("field info"),
                    terms.len() as i32,
                    *positions,
                    *offsets,
                    *payloads,
                )
                .expect("start field");
            for (text, occurrences) in terms {
                writer
                    .start_term(
                        &BytesRef::new(text.as_bytes().to_vec()),
                        occurrences.len() as i32,
                    )
                    .expect("start term");
                for (position, start, end, payload) in occurrences {
                    let payload = payload.map(|bytes| BytesRef::new(bytes.to_vec()));
                    writer
                        .add_position(*position, *start, *end, payload.as_ref())
                        .expect("add position");
                }
                writer.finish_term().expect("finish term");
            }
            writer.finish_field().expect("finish field");
        }
        writer.finish_document().expect("finish document");
        writer.finish(1).expect("finish");
        writer.close().expect("close");
    }

    #[test]
    fn offsets_survive_a_term_that_occurs_more_than_twice() {
        // The writer encodes a start offset as
        // `startOffset - previousStart - (int) (charsPerTerm * (position - previousPosition))`,
        // so the reader has to add back `charsPerTerm * positionDelta` — the
        // *delta*, not the decoded position. Java therefore delta-decodes the
        // positions only after the offsets are reconstructed. Doing it earlier
        // leaves every occurrence but the first two shifted by one
        // `charsPerTerm`.
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = make_field_infos();
        let segment_info = test_segment_info(Arc::clone(&dir), "_0", 1);
        let occurrences: Vec<(i32, i32, i32, Option<&[u8]>)> = vec![
            (0, 0, 4, None),
            (1, 5, 9, None),
            (2, 10, 14, None),
            (3, 15, 19, None),
        ];
        write_document(
            dir.as_ref(),
            &segment_info,
            &field_infos,
            &[("body", true, true, false, vec![("zeta", occurrences)])],
        );

        let reader = Lucene90TermVectorsFormat::new()
            .vectors_reader(
                dir.as_ref(),
                &segment_info,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .expect("reader");
        let fields = reader.get(0).expect("get").expect("doc 0 has vectors");
        let terms = fields.terms("body").expect("terms").expect("body terms");
        let mut iterator = terms.iterator().expect("terms enum");
        iterator.next().expect("next").expect("zeta");
        let mut postings = iterator
            .postings(None, POSTINGS_ENUM_POSITIONS)
            .expect("postings");
        postings.next_doc().expect("next doc");
        let mut decoded = Vec::new();
        for _ in 0..4 {
            let position = postings.next_position().expect("next position");
            decoded.push((position, postings.start_offset(), postings.end_offset()));
        }
        assert_eq!(
            decoded,
            vec![(0, 0, 4), (1, 5, 9), (2, 10, 14), (3, 15, 19)]
        );
    }

    #[test]
    fn the_second_payload_bearing_field_of_a_document_keeps_its_own_slice() {
        // Every payload-bearing field of a document shares one payload buffer,
        // so a field's payload index starts where the previous field's ended.
        // Starting it at zero hands the second field the first field's bytes.
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut first = RealFieldInfo::new("aaa", 0);
        first.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;
        let mut second = RealFieldInfo::new("bbb", 1);
        second.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;
        let field_infos = FieldInfos::new(vec![first, second]).unwrap();
        let segment_info = test_segment_info(Arc::clone(&dir), "_0", 1);

        write_document(
            dir.as_ref(),
            &segment_info,
            &field_infos,
            &[
                (
                    "aaa",
                    true,
                    false,
                    true,
                    vec![("alpha", vec![(0, 0, 0, Some(&[1, 2, 3, 4]))])],
                ),
                (
                    "bbb",
                    true,
                    false,
                    true,
                    vec![("alpha", vec![(0, 0, 0, Some(&[9]))])],
                ),
            ],
        );

        let reader = Lucene90TermVectorsFormat::new()
            .vectors_reader(
                dir.as_ref(),
                &segment_info,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .expect("reader");
        let fields = reader.get(0).expect("get").expect("doc 0 has vectors");
        for (name, expected) in [("aaa", &[1u8, 2, 3, 4][..]), ("bbb", &[9u8][..])] {
            let terms = fields.terms(name).expect("terms").expect("terms");
            let mut iterator = terms.iterator().expect("terms enum");
            iterator.next().expect("next").expect("alpha");
            let mut postings = iterator
                .postings(None, POSTINGS_ENUM_PAYLOADS)
                .expect("postings");
            postings.next_doc().expect("next doc");
            postings.next_position().expect("next position");
            assert_eq!(
                postings.get_payload().expect("payload"),
                Some(expected),
                "field {name}"
            );
        }
    }

    #[test]
    fn the_field_numbers_are_packed_with_the_exact_bit_width_lucene_uses() {
        // `flushFieldNums` writes the field numbers with
        // `PackedInts.Format.PACKED`, whose width is unconstrained, so Lucene
        // stores `PackedInts.bitsRequired(maxFieldNum)`. Rounding that up to a
        // width `DirectWriter` supports — 3 becomes 4 — still round-trips but
        // writes bytes Lucene never would.
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let infos: Vec<RealFieldInfo> = (0..6)
            .map(|number| {
                let mut info = RealFieldInfo::new(format!("f{number}"), number);
                info.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;
                info
            })
            .collect();
        let field_infos = FieldInfos::new(infos).unwrap();
        let segment_info = test_segment_info(Arc::clone(&dir), "_0", 1);

        let fields: Vec<ScriptedField<'_>> = vec![
            (
                "f0",
                false,
                false,
                false,
                vec![("t", vec![(0, 0, 0, None)])],
            ),
            (
                "f1",
                false,
                false,
                false,
                vec![("t", vec![(0, 0, 0, None)])],
            ),
            (
                "f2",
                false,
                false,
                false,
                vec![("t", vec![(0, 0, 0, None)])],
            ),
            (
                "f3",
                false,
                false,
                false,
                vec![("t", vec![(0, 0, 0, None)])],
            ),
            (
                "f4",
                false,
                false,
                false,
                vec![("t", vec![(0, 0, 0, None)])],
            ),
            (
                "f5",
                false,
                false,
                false,
                vec![("t", vec![(0, 0, 0, None)])],
            ),
        ];
        write_document(dir.as_ref(), &segment_info, &field_infos, &fields);

        let mut input = dir
            .open_input("_0.tvd", &*DEFAULT_IO_CONTEXT)
            .expect("open .tvd");
        let mut header = vec![0u8; 53];
        input.read_bytes(&mut header, 0, 53).expect("read header");
        // The index header is 4 magic bytes, the codec name as a
        // length-prefixed string, a four-byte version, the sixteen-byte segment
        // id and an empty suffix: 4 + 1 + 23 + 4 + 16 + 1 = 49 bytes. The chunk
        // then opens with `docBase`, `(chunkDocs << 1) | dirty` and, for a
        // single-document chunk, the field count — one byte each.
        let token = header[49 + 3];
        assert_eq!(
            token, 0xA3,
            "the token packs (numDistinctFields - 1) in its top three bits and \\
             PackedInts.bitsRequired(5) = 3 in its low five, so it must be 0xA3"
        );
    }

    #[test]
    fn a_document_reads_its_payloads_when_a_later_document_of_the_chunk_has_some() {
        // The decompressor is told the *chunk's* total length, which includes
        // the payload bytes of every document of the chunk — including the ones
        // after the requested document
        // (`Lucene90CompressingTermVectorsReader.java:688-700`). Leaving that
        // third term out understates the buffer the codec is allowed to
        // produce.
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut body = RealFieldInfo::new("body", 0);
        body.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;
        let field_infos = FieldInfos::new(vec![body]).unwrap();
        let segment_info = test_segment_info(Arc::clone(&dir), "_0", 2);

        for mode in [CompressionMode::FAST, CompressionMode::HIGH_COMPRESSION] {
            let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            let format = Lucene90CompressingTermVectorsFormat::new(
                VECTORS_CODEC_NAME,
                "",
                mode,
                1 << 12,
                128,
                10,
            )
            .unwrap();
            {
                let mut writer = format
                    .vectors_writer(dir.as_ref(), &segment_info, &*DEFAULT_IO_CONTEXT)
                    .unwrap();
                for doc in 0..2u8 {
                    writer.start_document(1).unwrap();
                    writer
                        .start_field(
                            field_infos.field_info("body").unwrap(),
                            1,
                            true,
                            false,
                            true,
                        )
                        .unwrap();
                    writer
                        .start_term(&BytesRef::new(b"alpha".to_vec()), 1)
                        .unwrap();
                    let payload = BytesRef::new(vec![doc; 32]);
                    writer.add_position(0, 0, 0, Some(&payload)).unwrap();
                    writer.finish_term().unwrap();
                    writer.finish_field().unwrap();
                    writer.finish_document().unwrap();
                }
                writer.finish(2).unwrap();
                writer.close().unwrap();
            }

            let reader = format
                .vectors_reader(
                    dir.as_ref(),
                    &segment_info,
                    &field_infos,
                    &*DEFAULT_IO_CONTEXT,
                )
                .unwrap();
            for doc in 0..2u8 {
                let fields = reader
                    .get(doc as i32)
                    .unwrap_or_else(|error| panic!("{mode} doc {doc}: {error}"))
                    .expect("vectors");
                let terms = fields.terms("body").unwrap().expect("body terms");
                let mut iterator = terms.iterator().unwrap();
                iterator.next().unwrap().expect("alpha");
                let mut postings = iterator.postings(None, POSTINGS_ENUM_PAYLOADS).unwrap();
                postings.next_doc().unwrap();
                postings.next_position().unwrap();
                assert_eq!(
                    postings.get_payload().unwrap(),
                    Some(&vec![doc; 32][..]),
                    "{mode} doc {doc}"
                );
            }
        }
    }

    // -- The directory the reader is handed ---------------------------------

    /// Writes one document's term vector into `directory` and returns the field
    /// infos the reader needs.
    fn write_one_document(directory: &dyn Directory, segment_info: &SegmentInfo) -> FieldInfos {
        let field_infos = make_field_infos();
        let format = Lucene90TermVectorsFormat::new();
        let mut writer = format
            .vectors_writer(directory, segment_info, &*DEFAULT_IO_CONTEXT)
            .expect("writer");
        writer.start_document(1).expect("start document");
        writer
            .start_field(
                field_infos.field_info("body").unwrap(),
                1,
                true,
                true,
                false,
            )
            .expect("start field");
        writer
            .start_term(&BytesRef::new(b"hello".to_vec()), 1)
            .expect("start term");
        writer.add_position(0, 0, 5, None).expect("add position");
        writer.finish_term().expect("finish term");
        writer.finish_field().expect("finish field");
        writer.finish_document().expect("finish document");
        writer.finish(1).expect("finish");
        writer.close().expect("close");
        field_infos
    }

    fn assert_reads_hello(reader: &dyn TermVectorsReader) {
        reader.check_integrity().expect("integrity");
        let fields = reader.get(0).expect("get").expect("doc 0 has vectors");
        let terms = fields.terms("body").expect("terms").expect("body terms");
        let mut iterator = terms.iterator().expect("terms enum");
        let term = iterator.next().expect("next").expect("first term");
        assert_eq!(term.slice(), b"hello");
        let mut postings = iterator
            .postings(None, POSTINGS_ENUM_POSITIONS)
            .expect("postings");
        assert_eq!(postings.next_doc().expect("next doc"), 0);
        assert_eq!(postings.freq().expect("freq"), 1);
        assert_eq!(postings.next_position().expect("next position"), 0);
        assert_eq!(postings.start_offset(), 0);
        assert_eq!(postings.end_offset(), 5);
    }

    #[test]
    fn the_reader_and_writer_use_the_directory_they_are_handed() {
        // `SegmentInfo.directory` is not usable as a fallback: the Lucene 9.9
        // segment-info format attaches an empty placeholder directory when it
        // parses a `.si`, and a compound segment's files exist only inside the
        // `.cfs`. Both halves of the format must therefore go through their
        // `directory` argument.
        let files: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let placeholder: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let segment_info = test_segment_info(Arc::clone(&placeholder), "_0", 1);

        let field_infos = write_one_document(files.as_ref(), &segment_info);
        assert!(
            placeholder.list_all().expect("list").is_empty(),
            "the writer must not create files in SegmentInfo.directory"
        );
        assert!(files.file_length("_0.tvd").expect("length") > 0);

        let reader = Lucene90TermVectorsFormat::new()
            .vectors_reader(
                files.as_ref(),
                &segment_info,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .expect("the reader must open the files from the directory it was given");
        assert_reads_hello(reader.as_ref());
    }

    #[test]
    fn term_vectors_are_readable_from_a_compound_file_segment() {
        let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let segment_info = test_segment_info(Arc::clone(&directory), "_0", 1);
        let field_infos = write_one_document(directory.as_ref(), &segment_info);

        let loose: Vec<String> = ["_0.tvd", "_0.tvx", "_0.tvm"]
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        segment_info.set_files(loose.iter().cloned().collect::<Vec<String>>());
        let compound_format = crate::codecs::lucene90::compound::Lucene90CompoundFormat;
        compound_format
            .write(directory.as_ref(), &segment_info, &*DEFAULT_IO_CONTEXT)
            .expect("compound write");
        // A compound segment keeps no loose copy; the reader has nowhere else
        // to look.
        for name in &loose {
            directory.delete_file(name).expect("delete loose file");
        }

        let compound = compound_format
            .get_compound_reader(directory.as_ref(), &segment_info)
            .expect("compound reader");
        let reader = Lucene90TermVectorsFormat::new()
            .vectors_reader(
                compound.as_ref(),
                &segment_info,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .expect("the reader must open the files from the compound directory");
        assert_reads_hello(reader.as_ref());

        // A clone must keep reading from the same compound stream.
        assert_reads_hello(reader.clone_reader().as_ref());
        assert_reads_hello(reader.get_merge_instance().as_ref());
    }

    // -----------------------------------------------------------------------
    // Bulk merge — Lucene90CompressingTermVectorsWriter#merge
    // -----------------------------------------------------------------------

    /// The format the merge tests use: chunks of four documents, so that a
    /// handful of tiny documents already makes several chunks and a merge has
    /// real chunk boundaries to copy. The production format holds 128
    /// documents per chunk, which would take that many documents to exercise.
    const MERGE_TEST_CHUNK_SIZE: i32 = 1 << 12;
    const MERGE_TEST_DOCS_PER_CHUNK: i32 = 4;

    fn merge_test_format() -> Lucene90CompressingTermVectorsFormat {
        Lucene90CompressingTermVectorsFormat::new(
            VECTORS_CODEC_NAME,
            "",
            CompressionMode::FAST,
            MERGE_TEST_CHUNK_SIZE,
            MERGE_TEST_DOCS_PER_CHUNK,
            10,
        )
        .unwrap()
    }

    /// A writer of the merge-test format with the given compression
    /// parameters, writing segment `name` into `dir`.
    fn merge_test_writer(
        dir: &Arc<dyn Directory>,
        name: &str,
        compression_mode: CompressionMode,
        chunk_size: i32,
    ) -> Lucene90CompressingTermVectorsWriter {
        Lucene90CompressingTermVectorsWriter::new(
            dir.as_ref(),
            &test_segment_info(Arc::clone(dir), name, 0),
            "",
            &*DEFAULT_IO_CONTEXT,
            VECTORS_CODEC_NAME,
            compression_mode,
            chunk_size,
            MERGE_TEST_DOCS_PER_CHUNK,
            10,
        )
        .unwrap()
    }

    /// One occurrence of a term: position, start offset, end offset, payload.
    type TvPosition = (i32, i32, i32, Option<Vec<u8>>);

    /// One term of a field's vector, with every occurrence the field records.
    #[derive(Debug, Clone, PartialEq)]
    struct TvTerm {
        text: Vec<u8>,
        freq: i64,
        occurrences: Vec<TvPosition>,
    }

    /// One field's vector, with the flags its terms were written with.
    #[derive(Debug, Clone, PartialEq)]
    struct TvField {
        name: String,
        positions: bool,
        offsets: bool,
        payloads: bool,
        terms: Vec<TvTerm>,
    }

    /// A document's term vectors, `None` when it has none.
    type TvDoc = Option<Vec<TvField>>;

    /// The term vectors of merge-test document `n`.
    ///
    /// Every fifth document has none; the others carry a `body` vector with
    /// positions and offsets — with payloads on odd documents, one occurrence
    /// in them left without one — and two documents in three also carry a
    /// `title` vector of bare term frequencies. So comparing two merged
    /// segments document by document compares every part a vector can hold.
    /// The documents stay far below [`MERGE_TEST_CHUNK_SIZE`], so a chunk
    /// closes on the document count alone.
    fn tv_merge_doc(n: i32) -> TvDoc {
        if n.rem_euclid(5) == 0 {
            return None;
        }
        let payloads = n % 2 == 1;
        let payload = |tag: u8| payloads.then(|| vec![(n & 0xff) as u8, tag]);
        let mut fields = vec![TvField {
            name: "body".to_string(),
            positions: true,
            offsets: true,
            payloads,
            terms: vec![
                TvTerm {
                    text: b"alpha".to_vec(),
                    freq: 1,
                    occurrences: vec![(0, 0, 5, None)],
                },
                TvTerm {
                    text: format!("t{n:04}").into_bytes(),
                    freq: 2,
                    occurrences: vec![(1, 6, 11, payload(1)), (3, 18, 23, payload(2))],
                },
            ],
        }];
        if n.rem_euclid(3) != 0 {
            fields.push(TvField {
                name: "title".to_string(),
                positions: false,
                offsets: false,
                payloads: false,
                terms: vec![
                    TvTerm {
                        text: b"common".to_vec(),
                        freq: 2,
                        occurrences: Vec::new(),
                    },
                    TvTerm {
                        text: format!("x{n}").into_bytes(),
                        freq: 1,
                        occurrences: Vec::new(),
                    },
                ],
            });
        }
        Some(fields)
    }

    /// The merge-test documents numbered by `numbers`, in that order.
    fn tv_merge_docs(numbers: impl IntoIterator<Item = i32>) -> Vec<TvDoc> {
        numbers.into_iter().map(tv_merge_doc).collect()
    }

    /// Writes one source segment holding merge-test documents
    /// `first..first + count`, and returns its segment info.
    fn write_tv_merge_source(
        dir: &Arc<dyn Directory>,
        name: &str,
        first: i32,
        count: i32,
    ) -> SegmentInfo {
        let field_infos = make_field_infos();
        let segment_info = test_segment_info(Arc::clone(dir), name, count);
        let mut writer = merge_test_format()
            .vectors_writer(dir.as_ref(), &segment_info, &*DEFAULT_IO_CONTEXT)
            .unwrap();
        for n in first..first + count {
            let fields = tv_merge_doc(n).unwrap_or_default();
            writer.start_document(fields.len() as i32).unwrap();
            for field in &fields {
                writer
                    .start_field(
                        field_infos.field_info(&field.name).unwrap(),
                        field.terms.len() as i32,
                        field.positions,
                        field.offsets,
                        field.payloads,
                    )
                    .unwrap();
                for term in &field.terms {
                    writer
                        .start_term(&BytesRef::new(term.text.clone()), term.freq as i32)
                        .unwrap();
                    for (position, start, end, payload) in &term.occurrences {
                        let payload = payload.as_ref().map(|bytes| BytesRef::new(bytes.clone()));
                        writer
                            .add_position(*position, *start, *end, payload.as_ref())
                            .unwrap();
                    }
                    writer.finish_term().unwrap();
                }
                writer.finish_field().unwrap();
            }
            writer.finish_document().unwrap();
        }
        writer.finish(count).unwrap();
        writer.close().unwrap();
        segment_info
    }

    /// Reads back every part of document `doc_id`'s term vectors.
    fn read_tv_doc(reader: &dyn TermVectorsReader, doc_id: i32) -> TvDoc {
        let fields = reader.get(doc_id).unwrap()?;
        let mut decoded = Vec::new();
        for name in fields.iterator() {
            let terms = fields
                .terms(&name)
                .unwrap()
                .unwrap_or_else(|| panic!("doc {doc_id} lists field {name} without terms"));
            let (positions, offsets, payloads) = (
                terms.has_positions(),
                terms.has_offsets(),
                terms.has_payloads(),
            );
            let mut decoded_terms = Vec::new();
            let mut it = terms.iterator().unwrap();
            while let Some(term) = it.next().unwrap() {
                let freq = it.total_term_freq().unwrap();
                let mut occurrences = Vec::new();
                if positions || offsets {
                    let mut postings = it
                        .postings(None, crate::index::postings_enum::POSTINGS_ENUM_ALL)
                        .unwrap();
                    assert_eq!(postings.next_doc().unwrap(), 0);
                    for _ in 0..freq {
                        let position = postings.next_position().unwrap();
                        occurrences.push((
                            position,
                            postings.start_offset(),
                            postings.end_offset(),
                            postings.get_payload().unwrap().map(<[u8]>::to_vec),
                        ));
                    }
                }
                decoded_terms.push(TvTerm {
                    text: term.slice().to_vec(),
                    freq,
                    occurrences,
                });
            }
            decoded.push(TvField {
                name,
                positions,
                offsets,
                payloads,
                terms: decoded_terms,
            });
        }
        Some(decoded)
    }

    /// Opens one source segment for merging.
    fn open_tv_merge_source(
        dir: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
    ) -> Box<dyn TermVectorsReader> {
        merge_test_format()
            .vectors_reader(
                dir.as_ref(),
                segment_info,
                &make_field_infos(),
                &*DEFAULT_IO_CONTEXT,
            )
            .unwrap()
    }

    /// Opens one source segment as this format's concrete reader, whose
    /// recorded metadata a test may then alter.
    fn concrete_tv_merge_source(
        dir: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
    ) -> Lucene90CompressingTermVectorsReader {
        open_tv_merge_source(dir, segment_info)
            .as_any()
            .and_then(|any| any.downcast_ref::<Lucene90CompressingTermVectorsReader>())
            .expect("the segment is written by this format")
            .clone()
    }

    /// `getNumDocs` is the segment's `maxDoc` — the constructor's
    /// `numDocs = si.maxDoc()` — and a merge instance, which Java builds with
    /// the copy constructor's `this.numDocs = reader.numDocs`, reports the same
    /// (`Lucene90CompressingTermVectorsReader.java:116,143,301-303`).
    #[test]
    fn get_num_docs_is_the_segment_max_doc() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        for (name, count) in [("_t1", 1), ("_t3", 3), ("_t9", 9)] {
            let source = write_tv_merge_source(&dir, name, 0, count);
            let reader = concrete_tv_merge_source(&dir, &source);
            assert_eq!(reader.get_num_docs(), count, "{name}");
            let merge_box = reader.get_merge_instance();
            let merge = merge_box
                .as_any()
                .and_then(|any| any.downcast_ref::<Lucene90CompressingTermVectorsReader>())
                .expect("the merge instance is this format's reader");
            assert_eq!(merge.get_num_docs(), count, "{name} merge instance");
        }
    }

    /// `(numChunks, numDirtyChunks, numDirtyDocs)` of a term-vectors segment,
    /// as its own reader reports them.
    fn tv_chunk_counters(dir: &Arc<dyn Directory>, segment_info: &SegmentInfo) -> (i64, i64, i64) {
        let reader = concrete_tv_merge_source(dir, segment_info);
        (
            reader.num_chunks(),
            reader.num_dirty_chunks(),
            reader.num_dirty_docs(),
        )
    }

    /// A term-vectors reader that hides the concrete type it wraps.
    ///
    /// `can_perform_bulk_merge` recognises this format's own reader through
    /// [`TermVectorsReader::as_any`], whose default answer is `None`; a reader
    /// that keeps the default therefore takes the route Java takes for a
    /// foreign codec's reader, decoding and re-encoding every vector
    /// (`Lucene90CompressingTermVectorsWriter.java:956-972`). That is what
    /// makes the merge routes comparable over the very same documents.
    #[derive(Debug)]
    struct OpaqueTermVectorsReader(Box<dyn TermVectorsReader>);

    impl TermVectorsReader for OpaqueTermVectorsReader {
        fn check_integrity(&self) -> Result<()> {
            self.0.check_integrity()
        }

        fn clone_reader(&self) -> Box<dyn TermVectorsReader> {
            Box::new(Self(self.0.clone_reader()))
        }

        fn close(&mut self) -> Result<()> {
            self.0.close()
        }
    }

    impl TermVectors for OpaqueTermVectorsReader {
        fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
            self.0.get(doc)
        }
    }

    /// Hides `reader`'s concrete type, forcing the re-encoding route for it.
    fn opaque_tv(reader: Box<dyn TermVectorsReader>) -> Box<dyn TermVectorsReader> {
        Box::new(OpaqueTermVectorsReader(reader))
    }

    /// The live documents of a `max_doc`-document segment whose `deleted`
    /// documents are deleted.
    fn tv_live_docs_without(max_doc: i32, deleted: &[i32]) -> Box<dyn Bits> {
        let mut live = crate::util::FixedBitSet::new(max_doc as usize);
        for doc in 0..max_doc {
            if !deleted.contains(&doc) {
                live.set(doc as usize);
            }
        }
        Box::new(live)
    }

    /// The doc map of a `max_doc`-document segment placed at `doc_base` in the
    /// merged segment, its `deleted` documents dropped and the rest packed.
    fn tv_doc_map_without(max_doc: i32, doc_base: i32, deleted: &[i32]) -> DocMap {
        let mut mapping = Vec::with_capacity(max_doc as usize);
        let mut next = doc_base;
        for doc in 0..max_doc {
            if deleted.contains(&doc) {
                mapping.push(-1);
            } else {
                mapping.push(next);
                next += 1;
            }
        }
        Arc::new(move |doc: i32| mapping[doc as usize])
    }

    /// The doc map that sends each `(from, to, merged)` run of a segment's
    /// documents `from..to` to `merged..`, as an index-sorted merge does when
    /// it interleaves segments.
    fn tv_interleaving_doc_map(runs: &[(i32, i32, i32)]) -> DocMap {
        let runs = runs.to_vec();
        Arc::new(move |doc: i32| {
            runs.iter()
                .find(|&&(from, to, _)| from <= doc && doc < to)
                .map_or(-1, |&(from, _, merged)| merged + doc - from)
        })
    }

    /// Builds the merge state for the given source readers.
    fn term_vectors_merge_state_with(
        readers: Vec<Box<dyn TermVectorsReader>>,
        doc_maps: Vec<DocMap>,
        max_docs: Vec<i32>,
        live_docs: Vec<Option<Box<dyn Bits>>>,
        needs_index_sort: bool,
    ) -> MergeState {
        let n = readers.len();
        let field_infos = make_field_infos();
        let merged_info =
            test_segment_info(Arc::new(RamDirectory::new()), "_m", max_docs.iter().sum());
        MergeState::new(
            doc_maps,
            merged_info,
            field_infos.clone(),
            (0..n).map(|_| None).collect(),
            readers.into_iter().map(Some).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| field_infos.clone()).collect(),
            live_docs,
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            max_docs,
            needs_index_sort,
        )
    }

    /// Builds the merge state of a plain, unsorted merge of segments without
    /// deletions, laid end to end.
    fn term_vectors_merge_state(
        readers: Vec<Box<dyn TermVectorsReader>>,
        max_docs: Vec<i32>,
    ) -> MergeState {
        let mut doc_maps = Vec::with_capacity(max_docs.len());
        let mut doc_base = 0;
        for &max_doc in &max_docs {
            doc_maps.push(crate::index::identity_doc_map(max_doc, doc_base));
            doc_base += max_doc;
        }
        let live_docs = max_docs.iter().map(|_| None).collect();
        term_vectors_merge_state_with(readers, doc_maps, max_docs, live_docs, false)
    }

    /// What one term-vectors merge produced.
    struct TvMergeOutcome {
        /// The directory holding the merged segment `_m`.
        dir: Arc<dyn Directory>,
        /// The merged segment's info.
        info: SegmentInfo,
        /// Every file of the merged segment, by name, sorted.
        files: Vec<(String, Vec<u8>)>,
        /// Every part of every merged document's term vectors, in order.
        documents: Vec<TvDoc>,
        /// `(numChunks, numDirtyChunks, numDirtyDocs)` as the merged segment
        /// records them in its `.tvm`.
        counters: (i64, i64, i64),
    }

    /// Runs one merge into a fresh directory and reports everything about it.
    fn run_term_vectors_merge(merge_state: &MergeState) -> TvMergeOutcome {
        let total_docs: i32 = merge_state
            .max_docs
            .iter()
            .zip(&merge_state.live_docs)
            .map(|(&max_doc, live)| match live {
                None => max_doc,
                Some(live) => (0..max_doc).filter(|&doc| live.get(doc as usize)).count() as i32,
            })
            .sum();
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let info = test_segment_info(Arc::clone(&dir), "_m", total_docs);
        {
            let mut writer = merge_test_format()
                .vectors_writer(dir.as_ref(), &info, &*DEFAULT_IO_CONTEXT)
                .unwrap();
            assert_eq!(
                writer.merge(merge_state).unwrap(),
                total_docs,
                "the merge must write every live document of every source segment"
            );
            writer.close().unwrap();
        }

        let reader = concrete_tv_merge_source(&dir, &info);
        let documents = (0..total_docs)
            .map(|doc_id| read_tv_doc(&reader, doc_id))
            .collect();
        let counters = (
            reader.num_chunks(),
            reader.num_dirty_chunks(),
            reader.num_dirty_docs(),
        );

        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for name in dir.list_all().unwrap() {
            let mut input = dir.open_input(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            let length = input.length() as usize;
            let mut bytes = vec![0u8; length];
            input.read_bytes(&mut bytes, 0, length).unwrap();
            files.push((name, bytes));
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));

        TvMergeOutcome {
            dir,
            info,
            files,
            documents,
            counters,
        }
    }

    /// `canPerformBulkMerge` allows the copy only for this format's own reader,
    /// whose fields the merged segment numbers exactly as the source did,
    /// written with the writer's compression mode and chunk size, at the
    /// current format and packed-ints versions, carrying no deletions and not
    /// too dirty (`Lucene90CompressingTermVectorsWriter.java:956-972`,
    /// `949-954`).
    #[test]
    fn the_term_vectors_bulk_merge_conditions_are_lucenes() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let source = write_tv_merge_source(&dir, "_0", 0, 10);
        let out: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = merge_test_writer(&out, "_w0", CompressionMode::FAST, MERGE_TEST_CHUNK_SIZE);
        let bulk_for = |writer: &Lucene90CompressingTermVectorsWriter,
                        reader: Box<dyn TermVectorsReader>,
                        live_docs: Option<Box<dyn Bits>>| {
            let state = term_vectors_merge_state_with(
                vec![reader],
                vec![crate::index::identity_doc_map(10, 0)],
                vec![10],
                vec![live_docs],
                false,
            );
            writer.can_perform_bulk_merge(&state, &MatchingReaders::new(&state), 0)
        };

        assert!(
            bulk_for(&writer, open_tv_merge_source(&dir, &source), None),
            "a matching, undeleted, clean segment of this very format is bulk merged"
        );

        assert!(
            !bulk_for(
                &writer,
                open_tv_merge_source(&dir, &source),
                Some(tv_live_docs_without(10, &[3]))
            ),
            "a segment with deletions must be decoded and re-encoded"
        );
        let other_mode = merge_test_writer(
            &out,
            "_w1",
            CompressionMode::HIGH_COMPRESSION,
            MERGE_TEST_CHUNK_SIZE,
        );
        assert!(
            !bulk_for(&other_mode, open_tv_merge_source(&dir, &source), None),
            "chunks compressed another way cannot be copied"
        );
        let other_chunk_size = merge_test_writer(
            &out,
            "_w2",
            CompressionMode::FAST,
            MERGE_TEST_CHUNK_SIZE * 2,
        );
        assert!(
            !bulk_for(&other_chunk_size, open_tv_merge_source(&dir, &source), None),
            "chunks of another size cannot be copied"
        );
        assert!(
            !bulk_for(
                &writer,
                opaque_tv(open_tv_merge_source(&dir, &source)),
                None
            ),
            "a foreign reader has no chunks this writer may copy"
        );

        // The only format and packed-ints versions there are today are the
        // current ones, so those checks are reached by altering what the
        // reader recorded.
        let mut old_version = concrete_tv_merge_source(&dir, &source);
        old_version.version = VERSION_CURRENT - 1;
        assert!(
            !bulk_for(&writer, Box::new(old_version), None),
            "a segment of another format version is re-encoded"
        );
        let mut old_packed_ints = concrete_tv_merge_source(&dir, &source);
        old_packed_ints.packed_ints_version = PackedInts::VERSION_CURRENT - 1;
        assert!(
            !bulk_for(&writer, Box::new(old_packed_ints), None),
            "a segment of another packed-ints version is re-encoded"
        );

        // tooDirty: more dirty documents than one chunk holds AND more than one
        // per cent of the chunks dirty; each boundary on its own is not enough.
        for (dirty_docs, dirty_chunks, chunks, too_dirty) in [
            (i64::from(MERGE_TEST_DOCS_PER_CHUNK), 100, 100, false),
            (i64::from(MERGE_TEST_DOCS_PER_CHUNK) + 1, 1, 100, false),
            (i64::from(MERGE_TEST_DOCS_PER_CHUNK) + 1, 2, 100, true),
        ] {
            let mut reader = concrete_tv_merge_source(&dir, &source);
            reader.num_dirty_docs = dirty_docs;
            reader.num_dirty_chunks = dirty_chunks;
            reader.num_chunks = chunks;
            assert_eq!(writer.too_dirty(&reader), too_dirty);
            assert_eq!(
                bulk_for(&writer, Box::new(reader), None),
                !too_dirty,
                "dirty docs {dirty_docs}, dirty chunks {dirty_chunks} of {chunks}"
            );
        }

        let mut renumbered = RealFieldInfo::new("title", 7);
        renumbered.index_options = IndexOptions::DOCS_AND_FREQS;
        let mut state =
            term_vectors_merge_state(vec![open_tv_merge_source(&dir, &source)], vec![10]);
        state.field_infos = vec![FieldInfos::new(vec![renumbered]).unwrap()];
        let matching = MatchingReaders::new(&state);
        assert!(!matching.matching_readers[0]);
        assert!(
            !writer.can_perform_bulk_merge(&state, &matching, 0),
            "a segment whose fields are numbered differently has to be remapped"
        );
    }

    /// The bulk route is the one `merge` actually takes, and it leaves a mark
    /// in the merged segment's `.tvm` that re-encoding cannot leave.
    ///
    /// Copying chunks verbatim preserves the *source* chunk boundaries
    /// (`Lucene90CompressingTermVectorsWriter.java:845-889`): two segments of
    /// ten and six documents, chunked four at a time, hold three and two
    /// chunks, the last of each a partial one the writer had to force out. A
    /// bulk merge therefore ends with five chunks, two of them dirty.
    /// Re-encoding the same sixteen documents packs them four at a time with
    /// nothing left over: four chunks, none dirty.
    ///
    /// The two merged segments differ on disk for that reason, yet must hold
    /// exactly the same vectors.
    #[test]
    fn a_bulk_term_vectors_merge_copies_the_source_chunks_verbatim() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let first = write_tv_merge_source(&dir, "_0", 0, 10);
        let second = write_tv_merge_source(&dir, "_1", 100, 6);
        assert_eq!(
            tv_chunk_counters(&dir, &first),
            (3, 1, 2),
            "ten documents chunked four at a time make three chunks, the last forced out"
        );
        assert_eq!(tv_chunk_counters(&dir, &second), (2, 1, 2));

        let bulk = run_term_vectors_merge(&term_vectors_merge_state(
            vec![
                open_tv_merge_source(&dir, &first),
                open_tv_merge_source(&dir, &second),
            ],
            vec![10, 6],
        ));
        assert_eq!(
            bulk.counters,
            (5, 2, 4),
            "the bulk merge must carry over both source segments' chunk boundaries"
        );

        let reencoded = run_term_vectors_merge(&term_vectors_merge_state(
            vec![
                opaque_tv(open_tv_merge_source(&dir, &first)),
                opaque_tv(open_tv_merge_source(&dir, &second)),
            ],
            vec![10, 6],
        ));
        assert_eq!(
            reencoded.counters,
            (4, 0, 0),
            "re-encoding the vectors repacks them into full chunks"
        );

        let expected = tv_merge_docs((0..10).chain(100..106));
        assert_eq!(bulk.documents, expected);
        assert_eq!(reencoded.documents, expected);
    }

    /// Where the writer would have drawn the same chunk boundaries anyway, the
    /// bulk and the re-encoding route must produce the same index, byte for
    /// byte — the property Lucene's own comment says bulk merge broke in the
    /// past (`Lucene90CompressingTermVectorsWriter.java:791-793`).
    #[test]
    fn bulk_and_reencoded_term_vectors_merges_produce_the_same_index() {
        for sources in [vec![(0i32, 10i32)], vec![(0i32, 8i32), (100i32, 8i32)]] {
            let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            let infos: Vec<SegmentInfo> = sources
                .iter()
                .enumerate()
                .map(|(i, &(first, count))| {
                    write_tv_merge_source(&dir, &format!("_{i}"), first, count)
                })
                .collect();
            let max_docs: Vec<i32> = sources.iter().map(|&(_, count)| count).collect();
            let expected = tv_merge_docs(
                sources
                    .iter()
                    .flat_map(|&(first, count)| first..first + count),
            );

            // The comparison only means something if the first merge really
            // copies chunks, so every source must be eligible for it.
            let bulk_state = term_vectors_merge_state(
                infos
                    .iter()
                    .map(|info| open_tv_merge_source(&dir, info))
                    .collect(),
                max_docs.clone(),
            );
            let out: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            let writer =
                merge_test_writer(&out, "_w", CompressionMode::FAST, MERGE_TEST_CHUNK_SIZE);
            let matching = MatchingReaders::new(&bulk_state);
            for i in 0..infos.len() {
                assert!(
                    writer.can_perform_bulk_merge(&bulk_state, &matching, i),
                    "sources {sources:?}: source {i} must be bulk merged"
                );
            }

            let bulk = run_term_vectors_merge(&bulk_state);
            let reencoded = run_term_vectors_merge(&term_vectors_merge_state(
                infos
                    .iter()
                    .map(|info| opaque_tv(open_tv_merge_source(&dir, info)))
                    .collect(),
                max_docs.clone(),
            ));

            assert_eq!(bulk.documents, expected, "sources {sources:?}");
            assert_eq!(reencoded.documents, expected, "sources {sources:?}");
            assert_eq!(
                bulk.counters, reencoded.counters,
                "sources {sources:?}: both routes must draw the same chunk boundaries here"
            );
            assert_eq!(
                bulk.files.iter().map(|(name, _)| name).collect::<Vec<_>>(),
                reencoded
                    .files
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
                "sources {sources:?}: both routes must write the same files"
            );
            for ((name, bulk_bytes), (_, reencoded_bytes)) in
                bulk.files.iter().zip(reencoded.files.iter())
            {
                assert_eq!(
                    bulk_bytes, reencoded_bytes,
                    "sources {sources:?}: {name} differs between the two merge routes"
                );
            }
        }
    }

    /// `merge` decides per source segment whether its chunks can be copied,
    /// and mixes both routes in one merge
    /// (`Lucene90CompressingTermVectorsWriter.java:903-936`).
    ///
    /// Four segments: a clean one (copied), one with a deleted document and
    /// one behind a foreign reader (both re-encoded), and another clean one
    /// (copied). The first copy keeps its three chunks `[4][4][2]`; the five
    /// live documents of the second and the six of the third are re-chunked as
    /// `[4][4]` with three left over, which the last copy forces out as a dirty
    /// chunk before carrying over its own `[4][2]`
    /// (`Lucene90CompressingTermVectorsWriter.java:839-842`): eight chunks,
    /// three of them dirty, holding 2 + 3 + 2 documents.
    #[test]
    fn a_term_vectors_merge_decides_the_route_per_segment() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let a = write_tv_merge_source(&dir, "_0", 0, 10);
        let b = write_tv_merge_source(&dir, "_1", 100, 6);
        let c = write_tv_merge_source(&dir, "_2", 200, 6);
        let d = write_tv_merge_source(&dir, "_3", 300, 6);
        let state = || {
            term_vectors_merge_state_with(
                vec![
                    open_tv_merge_source(&dir, &a),
                    open_tv_merge_source(&dir, &b),
                    opaque_tv(open_tv_merge_source(&dir, &c)),
                    open_tv_merge_source(&dir, &d),
                ],
                vec![
                    crate::index::identity_doc_map(10, 0),
                    tv_doc_map_without(6, 10, &[2]),
                    crate::index::identity_doc_map(6, 15),
                    crate::index::identity_doc_map(6, 21),
                ],
                vec![10, 6, 6, 6],
                vec![None, Some(tv_live_docs_without(6, &[2])), None, None],
                false,
            )
        };

        let out: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = merge_test_writer(&out, "_w", CompressionMode::FAST, MERGE_TEST_CHUNK_SIZE);
        let probe = state();
        let matching = MatchingReaders::new(&probe);
        assert_eq!(
            (0..4)
                .map(|i| writer.can_perform_bulk_merge(&probe, &matching, i))
                .collect::<Vec<_>>(),
            vec![true, false, false, true]
        );

        let merged = run_term_vectors_merge(&state());
        assert_eq!(merged.counters, (8, 3, 7));
        assert_eq!(
            merged.documents,
            tv_merge_docs(
                (0..10)
                    .chain([100, 101, 103, 104, 105])
                    .chain(200..206)
                    .chain(300..306)
            )
        );
    }

    /// A segment that is too dirty is re-encoded instead of copied, so that a
    /// run of tiny flushes cannot degrade the compression ratio for ever
    /// (`Lucene90CompressingTermVectorsWriter.java:941-954`).
    ///
    /// Bulk-merging three segments of two documents each carries over three
    /// dirty chunks: six dirty documents, more than one chunk holds, in every
    /// one of the chunks. Merging that segment again must re-encode it, which
    /// repacks the six documents as `[4][2]`.
    #[test]
    fn a_too_dirty_term_vectors_segment_is_recompressed() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let sources: Vec<SegmentInfo> = (0..3)
            .map(|i| write_tv_merge_source(&dir, &format!("_{i}"), 10 * i + 1, 2))
            .collect();
        let expected = tv_merge_docs([1, 2, 11, 12, 21, 22]);

        let dirty = run_term_vectors_merge(&term_vectors_merge_state(
            sources
                .iter()
                .map(|info| open_tv_merge_source(&dir, info))
                .collect(),
            vec![2, 2, 2],
        ));
        assert_eq!(dirty.counters, (3, 3, 6), "each tiny chunk is carried over");
        assert_eq!(dirty.documents, expected);
        let out: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = merge_test_writer(&out, "_w", CompressionMode::FAST, MERGE_TEST_CHUNK_SIZE);
        assert!(writer.too_dirty(&concrete_tv_merge_source(&dirty.dir, &dirty.info)));

        let recompressed = run_term_vectors_merge(&term_vectors_merge_state(
            vec![open_tv_merge_source(&dirty.dir, &dirty.info)],
            vec![6],
        ));
        assert_eq!(
            recompressed.counters,
            (2, 1, 2),
            "a too dirty segment must be repacked, not copied"
        );
        assert_eq!(recompressed.documents, expected);
    }

    /// An index-sorted merge interleaves the source segments, so the bulk
    /// route copies each segment in several runs, and a run may start inside
    /// the chunk the previous run of the same segment ended in. Those documents
    /// are re-encoded one by one until the next chunk boundary, then whole
    /// chunks are copied, then the leftovers re-encoded one by one
    /// (`Lucene90CompressingTermVectorsWriter.java:826-895`); that relies on
    /// the merge state's reader remembering the chunk it last decoded.
    ///
    /// Regression test: `copy_chunks` used to read through a fresh clone of the
    /// reader, which had forgotten that chunk, sought to the start of the chunk
    /// holding the run's first document and failed with "invalid state:
    /// base=0, docID=3".
    ///
    /// Segments A and B of ten documents each, chunked `[4][4][2]`, are merged
    /// as A0-2, B0-2, A3-8, B3-9, A9. The chunks the bulk route writes are
    /// `[A0 A1 A2 B0]`, `[B1 B2 A3]` (forced), `[A4-A7]` (copied), `[A8 B3]`
    /// (forced), `[B4-B7]` and `[B8 B9]` (both copied, the latter dirty) and
    /// `[A9]` (forced at the end): seven chunks, four dirty, eight dirty
    /// documents. Re-encoding packs the twenty documents into five full
    /// chunks.
    #[test]
    fn an_index_sorted_term_vectors_merge_copies_interleaved_runs() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let a = write_tv_merge_source(&dir, "_0", 1, 10);
        let b = write_tv_merge_source(&dir, "_1", 101, 10);
        let state = |wrap: fn(Box<dyn TermVectorsReader>) -> Box<dyn TermVectorsReader>| {
            term_vectors_merge_state_with(
                vec![
                    wrap(open_tv_merge_source(&dir, &a)),
                    wrap(open_tv_merge_source(&dir, &b)),
                ],
                vec![
                    tv_interleaving_doc_map(&[(0, 3, 0), (3, 9, 6), (9, 10, 19)]),
                    tv_interleaving_doc_map(&[(0, 3, 3), (3, 10, 12)]),
                ],
                vec![10, 10],
                vec![None, None],
                true,
            )
        };
        let expected = tv_merge_docs(
            (1..4)
                .chain(101..104)
                .chain(4..10)
                .chain(104..111)
                .chain(10..11),
        );

        let bulk = run_term_vectors_merge(&state(|reader| reader));
        assert_eq!(bulk.documents, expected);
        assert_eq!(
            bulk.counters,
            (7, 4, 8),
            "the whole chunks between the runs' ends must be copied verbatim"
        );

        let reencoded = run_term_vectors_merge(&state(opaque_tv));
        assert_eq!(reencoded.documents, expected);
        assert_eq!(reencoded.counters, (5, 0, 0));
    }

    /// An input that records every `prefetch` request and otherwise reads
    /// through to the input it wraps.
    struct PrefetchRecordingInput {
        inner: Box<dyn IndexInput>,
        requests: Arc<Mutex<Vec<(i64, i64)>>>,
    }

    impl fmt::Debug for PrefetchRecordingInput {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PrefetchRecordingInput")
                .finish_non_exhaustive()
        }
    }

    impl DataInput for PrefetchRecordingInput {
        fn read_byte(&mut self) -> Result<u8> {
            self.inner.read_byte()
        }

        fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
            self.inner.read_bytes(b, offset, len)
        }

        fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
            self.inner.skip_bytes(num_bytes)
        }
    }

    impl IndexInput for PrefetchRecordingInput {
        fn close(&mut self) -> Result<()> {
            self.inner.close()
        }

        fn file_pointer(&self) -> i64 {
            self.inner.file_pointer()
        }

        fn length(&self) -> i64 {
            self.inner.length()
        }

        fn seek(&mut self, pos: i64) -> Result<()> {
            self.inner.seek(pos)
        }

        fn slice(
            &self,
            slice_description: &str,
            offset: i64,
            length: i64,
        ) -> Result<Box<dyn IndexInput>> {
            self.inner.slice(slice_description, offset, length)
        }

        fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
            self.inner.clone_input()
        }

        fn resource_description(&self) -> &str {
            self.inner.resource_description()
        }

        fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
            self.requests.lock().unwrap().push((offset, length));
            self.inner.prefetch(offset, length)
        }
    }

    /// A `.tvm` whose fields index declares no chunks for a non-empty segment
    /// opens, `prefetch` answers from the cache, and reading the document's
    /// vectors is a corrupt-index error rather than a panic.
    ///
    /// Lucene 10.5.0 shares `FieldsIndexReader` between both formats:
    /// `getBlockID` answers `-1` (`FieldsIndexReader.java:119-127`), the
    /// `-1`-filled prefetch cache holds it
    /// (`Lucene90CompressingTermVectorsReader.java:125, 347-361`), and `get(0)`
    /// throws from `getBlockStartPointer(-1)`.
    #[test]
    fn a_term_vectors_index_without_chunks_is_a_corrupt_index_not_a_panic() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let source = write_tv_merge_source(&dir, "_h", 0, 3);
        // Positive control: the intact index serves the same calls.
        {
            let mut reader = concrete_tv_merge_source(&dir, &source);
            reader.prefetch(0).unwrap();
            reader.get(0).unwrap();
        }

        crate::codecs::lucene90::stored_fields::declare_no_fields_index_chunks(
            dir.as_ref(),
            "_h.tvm",
            2,
        );
        let mut reader = concrete_tv_merge_source(&dir, &source);
        assert_eq!(reader.index_reader.get_block_id(0).unwrap(), -1);
        reader.prefetch(0).unwrap();
        assert!(matches!(reader.get(0), Err(LuceneError::CorruptIndex(_))));
    }

    /// `prefetch` asks the `.tvd` input for the whole block that holds the
    /// document — from `getBlockStartPointer` for `getBlockLength` bytes, the
    /// last block ending at `maxPointer` — and asks once while it is cached.
    ///
    /// Equivalent to `Lucene90CompressingTermVectorsReader.prefetch(int)`
    /// (`Lucene90CompressingTermVectorsReader.java:347-361`).
    #[test]
    fn term_vectors_prefetch_asks_for_the_whole_block_once_while_it_is_cached() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        // Two full blocks and a last partial one.
        let source = write_tv_merge_source(&dir, "_p", 0, 2 * MERGE_TEST_DOCS_PER_CHUNK + 1);
        let mut reader = concrete_tv_merge_source(&dir, &source);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let inner = reader.vectors_stream.clone_input().unwrap();
        reader.vectors_stream = Box::new(PrefetchRecordingInput {
            inner,
            requests: Arc::clone(&requests),
        });

        let block = |b: i64| {
            let start = reader.index_reader.get_block_start_pointer(b).unwrap();
            let end = if b == 2 {
                reader.max_pointer
            } else {
                reader.index_reader.get_block_start_pointer(b + 1).unwrap()
            };
            assert_eq!(
                reader.index_reader.get_block_length(b).unwrap(),
                end - start
            );
            (start, end - start)
        };
        let expected = vec![block(0), block(2), block(1)];

        reader.prefetch(1).unwrap();
        reader.prefetch(0).unwrap(); // same block: cached
        reader.prefetch(2 * MERGE_TEST_DOCS_PER_CHUNK).unwrap(); // the last block
        reader.prefetch(MERGE_TEST_DOCS_PER_CHUNK).unwrap();
        reader.prefetch(2 * MERGE_TEST_DOCS_PER_CHUNK).unwrap(); // cached

        assert_eq!(*requests.lock().unwrap(), expected);
    }
}

impl fmt::Display for Lucene90CompressingTermVectorsFormat {
    /// Port of `Lucene90CompressingTermVectorsFormat.toString()`, whose last
    /// field is called `blockSize` although the value is the block *shift* —
    /// Java prints the `blockSize` constructor parameter, which it stores in
    /// `blockShift`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Lucene90CompressingTermVectorsFormat(compressionMode={}, chunkSize={}, maxDocsPerChunk={}, blockSize={})",
            self.compression_mode, self.chunk_size, self.max_docs_per_chunk, self.block_shift
        )
    }
}

impl fmt::Display for Lucene90CompressingTermVectorsReader {
    /// Port of `Lucene90CompressingTermVectorsReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Lucene90CompressingTermVectorsReader(mode={},chunksize={})",
            self.compression_mode, self.chunk_size
        )
    }
}
