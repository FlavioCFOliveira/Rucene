//! Lucene 10.4 postings writer.
//!
//! Port of `org.apache.lucene.codecs.lucene104.Lucene104PostingsWriter`. This
//! writer produces the `.psm` (metadata), `.doc`, `.pos` and `.pay` files for
//! the Lucene 10.4 postings format.
//!
//! # Current simplification
//!
//! The Java implementation reads per-document norms from the
//! `NumericDocValues` passed to `startTerm` in order to build competitive
//! `(freq, norm)` impact lists. The current Rust push API only passes that
//! reference for the duration of `start_term`, so the writer cannot safely
//! retain it across `start_doc` calls without either an API change or unsafe
//! lifetime extension. As a documented simplification, norms are currently
//! treated as the constant value `1` for impact accumulation, which matches the
//! Java output for fields that have no norms. Fields that actually store norms
//! will produce impact bytes that differ from Java until norm plumbing is
//! wired through the push API.

#![deny(unsafe_code)]

use crate::codecs::codec_util::{write_footer, write_index_header};
use crate::codecs::lucene104::{
    for_util::{ForUtil, BLOCK_SIZE as FOR_BLOCK_SIZE},
    p_for_util::PForUtil,
    postings_format::TERMS_CODEC,
    postings_util::write_v_int_block,
    BLOCK_SIZE, DOC_CODEC, LEVEL1_MASK, META_CODEC, PAY_CODEC, POS_CODEC, VERSION_CURRENT,
};
use crate::codecs::postings::{NumericDocValues, PushPostingsWriterBase, PushPostingsWriterState};
use crate::codecs::state::SegmentWriteState;
use crate::codecs::term_state::{BlockTermState, CompetitiveImpactAccumulator, Impact};
use crate::error::{LuceneError, Result};
use crate::index::{index_file_names::segment_file_name, FieldInfo, IndexOptions};
use crate::store::{DataOutput, IndexOutput};
use crate::util::{BitUtil, FixedBitSet};

/// Maximum allowed position value (mirrors `IndexWriter.MAX_POSITION`).
const MAX_POSITION: i32 = i32::MAX - 128;

// -----------------------------------------------------------------------------
// Resettable in-memory output used for skip/impact scratch buffers.
// -----------------------------------------------------------------------------

/// A tiny `DataOutput` backed by a `Vec<u8>` that can be reset and copied to
/// another output. Equivalent to Lucene's `ByteBuffersDataOutput` for the
/// limited needs of this writer.
#[derive(Debug, Default, Clone)]
struct ResettableOutput {
    bytes: Vec<u8>,
}

impl ResettableOutput {
    fn new() -> Self {
        Self::default()
    }

    fn len(&self) -> i64 {
        self.bytes.len() as i64
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn clear(&mut self) {
        self.bytes.clear();
    }

    fn copy_to(&self, out: &mut dyn DataOutput) -> Result<()> {
        if !self.bytes.is_empty() {
            out.write_bytes(&self.bytes, 0, self.bytes.len())?;
        }
        Ok(())
    }
}

impl DataOutput for ResettableOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.bytes.push(b);
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.bytes.extend_from_slice(&b[offset..offset + len]);
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// VInt15 / VLong15 helpers
// -----------------------------------------------------------------------------

/// Special two-byte vint for values that fit in 15 bits.
fn write_v_int15(out: &mut dyn DataOutput, v: i32) -> Result<()> {
    debug_assert!(v >= 0);
    write_v_long15(out, v as i64)
}

/// Special two-byte vlong for values that fit in 15 bits.
fn write_v_long15(out: &mut dyn DataOutput, v: i64) -> Result<()> {
    debug_assert!(v >= 0);
    if (v & !0x7FFFi64) == 0 {
        out.write_short(v as i16)?;
    } else {
        out.write_short((0x8000 | (v & 0x7FFF)) as i16)?;
        out.write_v_long(v >> 15)?;
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Impact encoding helper (also used by the reader-side tests in this module).
// -----------------------------------------------------------------------------

/// Writes a competitive-impact list using the Lucene 10.4 delta encoding.
///
/// Equivalent to `Lucene104PostingsWriter.writeImpacts`.
pub fn write_impacts(impacts: &[Impact], out: &mut dyn DataOutput) -> Result<()> {
    let mut previous = Impact::new(0, 0);
    for impact in impacts {
        debug_assert!(impact.freq > previous.freq);
        debug_assert!((impact.norm as u64) > (previous.norm as u64));
        let freq_delta = impact.freq - previous.freq - 1;
        let norm_delta = impact.norm - previous.norm - 1;
        if norm_delta == 0 {
            out.write_v_int(freq_delta << 1)?;
        } else {
            out.write_v_int((freq_delta << 1) | 1)?;
            out.write_z_long(norm_delta)?;
        }
        previous = *impact;
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Lucene104 postings writer
// -----------------------------------------------------------------------------

/// Low-level postings writer for the Lucene 10.4 format.
///
/// Implements [`PushPostingsWriterBase`] and therefore [`PostingsWriterBase`]
/// via the blanket implementation in [`crate::codecs::postings`].
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene104.Lucene104PostingsWriter`.
pub struct Lucene104PostingsWriter {
    version: i32,
    meta_out: Box<dyn IndexOutput>,
    doc_out: Box<dyn IndexOutput>,
    pos_out: Option<Box<dyn IndexOutput>>,
    pay_out: Option<Box<dyn IndexOutput>>,

    doc_delta_buffer: [i32; FOR_BLOCK_SIZE],
    freq_buffer: [i32; FOR_BLOCK_SIZE],
    pos_delta_buffer: [i32; FOR_BLOCK_SIZE],
    payload_length_buffer: [i32; FOR_BLOCK_SIZE],
    offset_start_delta_buffer: [i32; FOR_BLOCK_SIZE],
    offset_length_buffer: [i32; FOR_BLOCK_SIZE],

    payload_bytes: Vec<u8>,
    payload_byte_upto: i32,

    spare_bit_set: FixedBitSet,
    pfor_util: PForUtil,
    for_util: ForUtil,

    push_state: PushPostingsWriterState,
    scratch_output: ResettableOutput,
    level0_output: ResettableOutput,
    level1_output: ResettableOutput,

    last_state: BlockTermState,
    field_has_norms: bool,

    doc_start_fp: i64,
    pos_start_fp: i64,
    pay_start_fp: i64,
    level0_last_doc_id: i32,
    level0_last_pos_fp: i64,
    level0_last_pay_fp: i64,
    level1_last_doc_id: i32,
    level1_last_pos_fp: i64,
    level1_last_pay_fp: i64,

    doc_id: i32,
    last_doc_id: i32,
    last_position: i32,
    last_start_offset: i32,
    doc_count: i32,
    doc_buffer_upto: i32,
    pos_buffer_upto: i32,

    level0_freq_norm_accumulator: CompetitiveImpactAccumulator,
    level1_competitive_freq_norm_accumulator: CompetitiveImpactAccumulator,
    max_num_impacts_at_level0: i32,
    max_impact_num_bytes_at_level0: i32,
    max_num_impacts_at_level1: i32,
    max_impact_num_bytes_at_level1: i32,
}

impl std::fmt::Debug for Lucene104PostingsWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene104PostingsWriter")
            .field("version", &self.version)
            .field("doc_count", &self.doc_count)
            .field("doc_buffer_upto", &self.doc_buffer_upto)
            .field("pos_buffer_upto", &self.pos_buffer_upto)
            .finish_non_exhaustive()
    }
}

impl Lucene104PostingsWriter {
    /// Creates a postings writer for the current format version.
    pub fn new(state: &SegmentWriteState<'_>) -> Result<Self> {
        Self::with_version(state, VERSION_CURRENT)
    }

    /// Creates a postings writer with an explicit format version.
    pub fn with_version(state: &SegmentWriteState<'_>, version: i32) -> Result<Self> {
        if version < 0 || version > VERSION_CURRENT {
            return Err(LuceneError::IllegalArgument(format!(
                "Lucene104PostingsWriter version out of range: {version}"
            )));
        }

        let segment_info = state.segment_info;
        let id = segment_info.id();
        let suffix = &state.segment_suffix;
        let dir = state.directory;
        let context = state.context;

        let meta_name = segment_file_name(&segment_info.name, suffix, "psm");
        let doc_name = segment_file_name(&segment_info.name, suffix, "doc");

        let mut meta_out = dir.create_output(&meta_name, context)?;
        write_index_header(meta_out.as_mut(), META_CODEC, version, &id, suffix)?;

        let mut doc_out = dir.create_output(&doc_name, context)?;
        write_index_header(doc_out.as_mut(), DOC_CODEC, version, &id, suffix)?;

        let mut pos_out: Option<Box<dyn IndexOutput>> = None;
        let mut pay_out: Option<Box<dyn IndexOutput>> = None;
        let mut payload_bytes = Vec::new();

        let build_result: Result<()> = (|| {
            if state.field_infos.has_prox() {
                let pos_name = segment_file_name(&segment_info.name, suffix, "pos");
                let mut p = dir.create_output(&pos_name, context)?;
                write_index_header(p.as_mut(), POS_CODEC, version, &id, suffix)?;
                pos_out = Some(p);

                if state.field_infos.has_payloads() {
                    payload_bytes = Vec::with_capacity(128);
                }

                if state.field_infos.has_payloads() || state.field_infos.has_offsets() {
                    let pay_name = segment_file_name(&segment_info.name, suffix, "pay");
                    let mut py = dir.create_output(&pay_name, context)?;
                    write_index_header(py.as_mut(), PAY_CODEC, version, &id, suffix)?;
                    pay_out = Some(py);
                }
            }
            Ok(())
        })();

        if build_result.is_err() {
            let _ = meta_out.close();
            let _ = doc_out.close();
            if let Some(ref mut p) = pos_out {
                let _ = p.close();
            }
            if let Some(ref mut py) = pay_out {
                let _ = py.close();
            }
        }
        build_result?;

        Ok(Self {
            version,
            meta_out,
            doc_out,
            pos_out,
            pay_out,
            doc_delta_buffer: [0; FOR_BLOCK_SIZE],
            freq_buffer: [0; FOR_BLOCK_SIZE],
            pos_delta_buffer: [0; FOR_BLOCK_SIZE],
            payload_length_buffer: [0; FOR_BLOCK_SIZE],
            offset_start_delta_buffer: [0; FOR_BLOCK_SIZE],
            offset_length_buffer: [0; FOR_BLOCK_SIZE],
            payload_bytes,
            payload_byte_upto: 0,
            spare_bit_set: FixedBitSet::new(FOR_BLOCK_SIZE * 32),
            pfor_util: PForUtil::new(),
            for_util: ForUtil::new(),
            push_state: PushPostingsWriterState::new(),
            scratch_output: ResettableOutput::new(),
            level0_output: ResettableOutput::new(),
            level1_output: ResettableOutput::new(),
            last_state: BlockTermState::default(),
            field_has_norms: false,
            doc_start_fp: 0,
            pos_start_fp: 0,
            pay_start_fp: 0,
            level0_last_doc_id: -1,
            level0_last_pos_fp: 0,
            level0_last_pay_fp: 0,
            level1_last_doc_id: -1,
            level1_last_pos_fp: 0,
            level1_last_pay_fp: 0,
            doc_id: -1,
            last_doc_id: -1,
            last_position: 0,
            last_start_offset: 0,
            doc_count: 0,
            doc_buffer_upto: 0,
            pos_buffer_upto: 0,
            level0_freq_norm_accumulator: CompetitiveImpactAccumulator::new(),
            level1_competitive_freq_norm_accumulator: CompetitiveImpactAccumulator::new(),
            max_num_impacts_at_level0: 0,
            max_impact_num_bytes_at_level0: 0,
            max_num_impacts_at_level1: 0,
            max_impact_num_bytes_at_level1: 0,
        })
    }
}

impl PushPostingsWriterBase for Lucene104PostingsWriter {
    fn push_state(&self) -> &PushPostingsWriterState {
        &self.push_state
    }

    fn push_state_mut(&mut self) -> &mut PushPostingsWriterState {
        &mut self.push_state
    }

    fn init(
        &mut self,
        terms_out: &mut dyn IndexOutput,
        state: &SegmentWriteState<'_>,
    ) -> Result<()> {
        write_index_header(
            terms_out,
            TERMS_CODEC,
            self.version,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        terms_out.write_v_int(BLOCK_SIZE)?;
        Ok(())
    }

    fn set_field(&mut self, field_info: &FieldInfo) -> Result<()> {
        let state = self.push_state_mut();
        state.field_info = Some(field_info.clone());
        state.index_options = field_info.index_options;
        state.write_freqs = field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS);
        state.write_positions = field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        state.write_offsets = field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        state.write_payloads = field_info.has_payloads();
        // A field that stores payloads but not offsets must request PAYLOADS
        // only. Requesting ALL (which subsumes OFFSETS) makes a conforming
        // `Fields` implementation refuse the request, exactly as Lucene's
        // `FreqProxFields` does with "did not index offsets".
        state.enum_flags = if !state.write_freqs {
            crate::codecs::postings::POSTINGS_ENUM_NONE
        } else if !state.write_positions {
            crate::codecs::postings::POSTINGS_ENUM_FREQS
        } else if !state.write_offsets {
            if state.write_payloads {
                crate::codecs::postings::POSTINGS_ENUM_PAYLOADS
            } else {
                crate::codecs::postings::POSTINGS_ENUM_POSITIONS
            }
        } else if !state.write_payloads {
            crate::codecs::postings::POSTINGS_ENUM_OFFSETS
        } else {
            crate::codecs::postings::POSTINGS_ENUM_ALL
        };

        self.last_state = BlockTermState::default();
        self.field_has_norms = field_info.has_norms();
        Ok(())
    }

    fn new_term_state(&self) -> Result<BlockTermState> {
        Ok(BlockTermState::default())
    }

    fn start_term(&mut self, _norms: Option<&dyn NumericDocValues>) -> Result<()> {
        self.doc_start_fp = self.doc_out.file_pointer();
        if self.push_state.write_positions {
            let pos_out = self.pos_out.as_mut().expect("pos output must exist");
            self.pos_start_fp = pos_out.file_pointer();
            self.level0_last_pos_fp = self.pos_start_fp;
            self.level1_last_pos_fp = self.pos_start_fp;
            if self.push_state.write_payloads || self.push_state.write_offsets {
                let pay_out = self.pay_out.as_mut().expect("pay output must exist");
                self.pay_start_fp = pay_out.file_pointer();
                self.level0_last_pay_fp = self.pay_start_fp;
                self.level1_last_pay_fp = self.pay_start_fp;
            }
        }
        self.last_doc_id = -1;
        self.level0_last_doc_id = -1;
        self.level1_last_doc_id = -1;
        if self.push_state.write_freqs {
            self.level0_freq_norm_accumulator.clear();
        }
        Ok(())
    }

    fn start_doc(&mut self, doc_id: i32, term_doc_freq: i32) -> Result<()> {
        if self.doc_buffer_upto == BLOCK_SIZE {
            self.flush_doc_block(false)?;
            self.doc_buffer_upto = 0;
        }

        let doc_delta = doc_id - self.last_doc_id;
        if doc_id < 0 || doc_delta <= 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "docs out of order ({doc_id} <= {last_doc_id})",
                last_doc_id = self.last_doc_id
            )));
        }

        let idx = self.doc_buffer_upto as usize;
        self.doc_delta_buffer[idx] = doc_delta;
        if self.push_state.write_freqs {
            self.freq_buffer[idx] = term_doc_freq;
        }

        self.doc_id = doc_id;
        self.last_position = 0;
        self.last_start_offset = 0;

        if self.push_state.write_freqs {
            // Norms are treated as 1; see module-level note.
            let norm = 1i64;
            self.level0_freq_norm_accumulator.add(term_doc_freq, norm);
        }

        Ok(())
    }

    fn add_position(
        &mut self,
        position: i32,
        payload: Option<&[u8]>,
        start_offset: i32,
        end_offset: i32,
    ) -> Result<()> {
        if !(0..=MAX_POSITION).contains(&position) {
            return Err(LuceneError::CorruptIndex(format!(
                "position={position} out of range [0, {MAX_POSITION}]"
            )));
        }

        let idx = self.pos_buffer_upto as usize;
        self.pos_delta_buffer[idx] = position - self.last_position;

        if self.push_state.write_payloads {
            if let Some(bytes) = payload {
                if !bytes.is_empty() {
                    self.payload_length_buffer[idx] = bytes.len() as i32;
                    let upto = self.payload_byte_upto as usize;
                    let needed = upto + bytes.len();
                    if needed > self.payload_bytes.len() {
                        self.payload_bytes
                            .resize(needed.max(self.payload_bytes.len() * 2).max(128), 0);
                    }
                    self.payload_bytes[upto..upto + bytes.len()].copy_from_slice(bytes);
                    self.payload_byte_upto += bytes.len() as i32;
                } else {
                    self.payload_length_buffer[idx] = 0;
                }
            } else {
                self.payload_length_buffer[idx] = 0;
            }
        }

        if self.push_state.write_offsets {
            if start_offset < self.last_start_offset || end_offset < start_offset {
                return Err(LuceneError::CorruptIndex(
                    "offsets out of order".to_string(),
                ));
            }
            self.offset_start_delta_buffer[idx] = start_offset - self.last_start_offset;
            self.offset_length_buffer[idx] = end_offset - start_offset;
            self.last_start_offset = start_offset;
        }

        self.pos_buffer_upto += 1;
        self.last_position = position;

        if self.pos_buffer_upto == BLOCK_SIZE {
            self.flush_pos_block()?;
        }

        Ok(())
    }

    fn finish_doc(&mut self) -> Result<()> {
        self.doc_buffer_upto += 1;
        self.doc_count += 1;
        self.last_doc_id = self.doc_id;
        Ok(())
    }

    fn finish_term(&mut self, state: &mut BlockTermState) -> Result<()> {
        debug_assert!(state.doc_freq > 0);
        debug_assert_eq!(state.doc_freq, self.doc_count);

        let singleton_doc_id = if state.doc_freq == 1 {
            self.doc_delta_buffer[0] - 1
        } else {
            -1
        };

        if state.doc_freq != 1 {
            self.flush_doc_block(true)?;
        }

        let last_pos_block_offset = if self.push_state.write_positions {
            debug_assert!(state.total_term_freq != -1);
            if state.total_term_freq > BLOCK_SIZE as i64 {
                self.pos_out
                    .as_ref()
                    .expect("pos output must exist")
                    .file_pointer()
                    - self.pos_start_fp
            } else {
                -1
            }
        } else {
            -1
        };

        if self.push_state.write_positions && self.pos_buffer_upto > 0 {
            self.flush_tail_positions()?;
        }

        state.doc_start_fp = self.doc_start_fp;
        state.pos_start_fp = self.pos_start_fp;
        state.pay_start_fp = self.pay_start_fp;
        state.singleton_doc_id = singleton_doc_id;
        state.last_pos_block_offset = last_pos_block_offset;

        self.doc_buffer_upto = 0;
        self.pos_buffer_upto = 0;
        self.last_doc_id = -1;
        self.doc_count = 0;
        Ok(())
    }

    fn encode_term(
        &mut self,
        out: &mut dyn DataOutput,
        _field_info: &FieldInfo,
        state: &BlockTermState,
        absolute: bool,
    ) -> Result<()> {
        if absolute {
            self.last_state = BlockTermState::default();
            debug_assert_eq!(self.last_state.doc_start_fp, 0);
        }

        if self.last_state.singleton_doc_id != -1
            && state.singleton_doc_id != -1
            && state.doc_start_fp == self.last_state.doc_start_fp
        {
            let delta = state.singleton_doc_id as i64 - self.last_state.singleton_doc_id as i64;
            out.write_v_long((BitUtil::zig_zag_encode_long(delta) << 1) | 1)?;
        } else {
            out.write_v_long((state.doc_start_fp - self.last_state.doc_start_fp) << 1)?;
            if state.singleton_doc_id != -1 {
                out.write_v_int(state.singleton_doc_id)?;
            }
        }

        if self.push_state.write_positions {
            out.write_v_long(state.pos_start_fp - self.last_state.pos_start_fp)?;
            if self.push_state.write_payloads || self.push_state.write_offsets {
                out.write_v_long(state.pay_start_fp - self.last_state.pay_start_fp)?;
            }
            if state.last_pos_block_offset != -1 {
                out.write_v_long(state.last_pos_block_offset)?;
            }
        }

        self.last_state = *state;
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        let result: Result<()> = (|| {
            write_footer(self.doc_out.as_mut())?;
            if let Some(pos_out) = self.pos_out.as_mut() {
                write_footer(pos_out.as_mut())?;
            }
            if let Some(pay_out) = self.pay_out.as_mut() {
                write_footer(pay_out.as_mut())?;
            }

            self.meta_out.write_int(self.max_num_impacts_at_level0)?;
            self.meta_out
                .write_int(self.max_impact_num_bytes_at_level0)?;
            self.meta_out.write_int(self.max_num_impacts_at_level1)?;
            self.meta_out
                .write_int(self.max_impact_num_bytes_at_level1)?;
            self.meta_out.write_long(self.doc_out.file_pointer())?;
            if let Some(pos_out) = self.pos_out.as_ref() {
                self.meta_out.write_long(pos_out.file_pointer())?;
                if let Some(pay_out) = self.pay_out.as_ref() {
                    self.meta_out.write_long(pay_out.file_pointer())?;
                }
            }
            write_footer(self.meta_out.as_mut())?;
            Ok(())
        })();

        let close_result = self.meta_out.close().and_then(|_| self.doc_out.close());
        let close_result = if let Some(pos_out) = self.pos_out.as_mut() {
            close_result.and_then(|_| pos_out.close())
        } else {
            close_result
        };
        let close_result = if let Some(pay_out) = self.pay_out.as_mut() {
            close_result.and_then(|_| pay_out.close())
        } else {
            close_result
        };

        result.and(close_result)
    }
}

impl Lucene104PostingsWriter {
    fn flush_pos_block(&mut self) -> Result<()> {
        let pos_out = self.pos_out.as_mut().expect("pos output must exist");
        self.pfor_util
            .encode(&mut self.pos_delta_buffer, pos_out.as_mut())?;

        if self.push_state.write_payloads {
            let pay_out = self.pay_out.as_mut().expect("pay output must exist");
            self.pfor_util
                .encode(&mut self.payload_length_buffer, pay_out.as_mut())?;
            pay_out.write_v_int(self.payload_byte_upto)?;
            pay_out.write_bytes(&self.payload_bytes, 0, self.payload_byte_upto as usize)?;
            self.payload_byte_upto = 0;
        }

        if self.push_state.write_offsets {
            let pay_out = self.pay_out.as_mut().expect("pay output must exist");
            self.pfor_util
                .encode(&mut self.offset_start_delta_buffer, pay_out.as_mut())?;
            self.pfor_util
                .encode(&mut self.offset_length_buffer, pay_out.as_mut())?;
        }

        self.pos_buffer_upto = 0;
        Ok(())
    }

    fn flush_tail_positions(&mut self) -> Result<()> {
        let pos_out = self.pos_out.as_mut().expect("pos output must exist");
        let write_payloads = self.push_state.write_payloads;
        let write_offsets = self.push_state.write_offsets;
        let mut last_payload_length = -1i32;
        let mut last_offset_length = -1i32;
        let mut payload_bytes_read_upto = 0usize;
        let limit = self.pos_buffer_upto as usize;

        for i in 0..limit {
            let pos_delta = self.pos_delta_buffer[i];
            if write_payloads {
                let payload_length = self.payload_length_buffer[i];
                if payload_length != last_payload_length {
                    last_payload_length = payload_length;
                    pos_out.write_v_int((pos_delta << 1) | 1)?;
                    pos_out.write_v_int(payload_length)?;
                } else {
                    pos_out.write_v_int(pos_delta << 1)?;
                }
                if payload_length != 0 {
                    pos_out.write_bytes(
                        &self.payload_bytes,
                        payload_bytes_read_upto,
                        payload_length as usize,
                    )?;
                    payload_bytes_read_upto += payload_length as usize;
                }
            } else {
                pos_out.write_v_int(pos_delta)?;
            }

            if write_offsets {
                let delta = self.offset_start_delta_buffer[i];
                let length = self.offset_length_buffer[i];
                if length == last_offset_length {
                    pos_out.write_v_int(delta << 1)?;
                } else {
                    pos_out.write_v_int((delta << 1) | 1)?;
                    pos_out.write_v_int(length)?;
                    last_offset_length = length;
                }
            }
        }

        if write_payloads {
            debug_assert_eq!(payload_bytes_read_upto, self.payload_byte_upto as usize);
            self.payload_byte_upto = 0;
        }

        Ok(())
    }

    fn flush_doc_block(&mut self, finish_term: bool) -> Result<()> {
        debug_assert!(self.doc_buffer_upto != 0);
        let write_freqs = self.push_state.write_freqs;
        let write_positions = self.push_state.write_positions;
        let write_payloads = self.push_state.write_payloads;
        let write_offsets = self.push_state.write_offsets;
        let block_size = BLOCK_SIZE as usize;
        let block_size_i = BLOCK_SIZE;

        if self.doc_buffer_upto < block_size_i {
            debug_assert!(finish_term);
            write_v_int_block(
                &mut self.level0_output,
                &mut self.doc_delta_buffer,
                &self.freq_buffer,
                self.doc_buffer_upto as usize,
                write_freqs,
            )?;
        } else {
            if write_freqs {
                let impacts = self
                    .level0_freq_norm_accumulator
                    .get_competitive_freq_norm_pairs();
                self.max_num_impacts_at_level0 =
                    self.max_num_impacts_at_level0.max(impacts.len() as i32);
                write_impacts(&impacts, &mut self.scratch_output)?;
                debug_assert!(self.level0_output.is_empty());
                self.max_impact_num_bytes_at_level0 = self
                    .max_impact_num_bytes_at_level0
                    .max(self.scratch_output.len() as i32);
                self.level0_output.write_v_long(self.scratch_output.len())?;
                self.scratch_output.copy_to(&mut self.level0_output)?;
                self.scratch_output.clear();

                if write_positions {
                    let pos_out = self.pos_out.as_mut().expect("pos output must exist");
                    let delta = pos_out.file_pointer() - self.level0_last_pos_fp;
                    self.level0_output.write_v_long(delta)?;
                    self.level0_output.write_byte(self.pos_buffer_upto as u8)?;
                    self.level0_last_pos_fp = pos_out.file_pointer();
                    if write_offsets || write_payloads {
                        let pay_out = self.pay_out.as_mut().expect("pay output must exist");
                        let delta = pay_out.file_pointer() - self.level0_last_pay_fp;
                        self.level0_output.write_v_long(delta)?;
                        self.level0_output.write_v_int(self.payload_byte_upto)?;
                        self.level0_last_pay_fp = pay_out.file_pointer();
                    }
                }
            }

            let mut num_skip_bytes = self.level0_output.len();

            let mut or_val = 0i32;
            for i in 0..block_size {
                or_val |= self.doc_delta_buffer[i];
            }
            debug_assert!(or_val != 0);
            let bits_per_value = 32 - (or_val as u32).leading_zeros() as i32;
            let doc_range = self.last_doc_id - self.level0_last_doc_id;
            debug_assert_eq!(
                doc_range,
                self.doc_delta_buffer[..block_size].iter().sum::<i32>()
            );
            let num_bit_set_longs = FixedBitSet::bits2words(doc_range as usize);
            let num_bits_next_bits_per_value = (bits_per_value + 1).min(32) * block_size_i;

            if doc_range == block_size_i {
                self.level0_output.write_byte(0)?;
            } else if num_bits_next_bits_per_value <= doc_range {
                self.level0_output.write_byte(bits_per_value as u8)?;
                self.for_util.encode(
                    &mut self.doc_delta_buffer,
                    bits_per_value,
                    &mut self.level0_output,
                )?;
            } else {
                self.spare_bit_set.clear_all();
                let mut s = -1i32;
                for i in 0..block_size {
                    s += self.doc_delta_buffer[i];
                    self.spare_bit_set.set(s as usize);
                }
                debug_assert!(num_bit_set_longs <= (block_size_i / 2) as usize);
                self.level0_output
                    .write_byte((-(num_bit_set_longs as i8)) as u8)?;
                let bits = self.spare_bit_set.get_bits();
                for &bits in bits.iter().take(num_bit_set_longs) {
                    self.level0_output.write_long(bits as i64)?;
                }
            }

            if write_freqs {
                self.pfor_util
                    .encode(&mut self.freq_buffer, &mut self.level0_output)?;
            }

            write_v_int15(
                &mut self.scratch_output,
                self.doc_id - self.level0_last_doc_id,
            )?;
            write_v_long15(&mut self.scratch_output, self.level0_output.len())?;
            num_skip_bytes += self.scratch_output.len();
            self.level1_output.write_v_long(num_skip_bytes)?;
            self.scratch_output.copy_to(&mut self.level1_output)?;
            self.scratch_output.clear();
        }

        self.level0_output.copy_to(&mut self.level1_output)?;
        self.level0_output.clear();
        self.level0_last_doc_id = self.doc_id;

        if write_freqs {
            self.level1_competitive_freq_norm_accumulator
                .add_all(&self.level0_freq_norm_accumulator);
            self.level0_freq_norm_accumulator.clear();
        }

        if (self.doc_count & LEVEL1_MASK) == 0 {
            self.write_level1_skip_data()?;
            self.level1_last_doc_id = self.doc_id;
            self.level1_competitive_freq_norm_accumulator.clear();
        } else if finish_term {
            self.level1_output.copy_to(self.doc_out.as_mut())?;
            self.level1_output.clear();
            self.level1_competitive_freq_norm_accumulator.clear();
        }

        Ok(())
    }

    fn write_level1_skip_data(&mut self) -> Result<()> {
        let write_freqs = self.push_state.write_freqs;
        let write_positions = self.push_state.write_positions;
        let write_payloads = self.push_state.write_payloads;
        let write_offsets = self.push_state.write_offsets;

        self.doc_out
            .write_v_int(self.doc_id - self.level1_last_doc_id)?;

        let level1_end: i64;
        if write_freqs {
            let impacts = self
                .level1_competitive_freq_norm_accumulator
                .get_competitive_freq_norm_pairs();
            self.max_num_impacts_at_level1 =
                self.max_num_impacts_at_level1.max(impacts.len() as i32);
            write_impacts(&impacts, &mut self.scratch_output)?;
            let num_impact_bytes = self.scratch_output.len();
            self.max_impact_num_bytes_at_level1 = self
                .max_impact_num_bytes_at_level1
                .max(num_impact_bytes as i32);

            if write_positions {
                let pos_out = self.pos_out.as_mut().expect("pos output must exist");
                self.scratch_output
                    .write_v_long(pos_out.file_pointer() - self.level1_last_pos_fp)?;
                self.scratch_output.write_byte(self.pos_buffer_upto as u8)?;
                self.level1_last_pos_fp = pos_out.file_pointer();
                if write_offsets || write_payloads {
                    let pay_out = self.pay_out.as_mut().expect("pay output must exist");
                    self.scratch_output
                        .write_v_long(pay_out.file_pointer() - self.level1_last_pay_fp)?;
                    self.scratch_output.write_v_int(self.payload_byte_upto)?;
                    self.level1_last_pay_fp = pay_out.file_pointer();
                }
            }

            let level1_len = 2 * 2 + self.scratch_output.len() + self.level1_output.len();
            self.doc_out.write_v_long(level1_len)?;
            level1_end = self.doc_out.file_pointer() + level1_len;
            self.doc_out
                .write_short((self.scratch_output.len() + 2) as i16)?;
            self.doc_out.write_short(num_impact_bytes as i16)?;
            self.scratch_output.copy_to(self.doc_out.as_mut())?;
            self.scratch_output.clear();
        } else {
            self.doc_out.write_v_long(self.level1_output.len())?;
            level1_end = self.doc_out.file_pointer() + self.level1_output.len();
        }

        self.level1_output.copy_to(self.doc_out.as_mut())?;
        self.level1_output.clear();
        debug_assert_eq!(self.doc_out.file_pointer(), level1_end);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::codec_util::check_index_header;
    use crate::codecs::lucene104::{
        Lucene104PostingsFormat, DOC_CODEC, META_CODEC, PAY_CODEC, POS_CODEC,
    };
    use crate::codecs::postings::{NormsProducer, NumericDocValues, PostingsFormat};
    use crate::codecs::state::SegmentWriteState;
    use crate::index::{FieldInfo, FieldInfos, Fields, PostingsEnum, SeekStatus, Terms, TermsEnum};
    use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
    use crate::store::{Directory, RamDirectory};
    use crate::util::{AttributeSource, BytesRef};
    use base64::Engine as _;

    struct TestNorms;

    impl NumericDocValues for TestNorms {
        fn get(&self, _doc_id: i32) -> crate::error::Result<i64> {
            Ok(1)
        }
    }

    impl NormsProducer for TestNorms {
        fn get_norms(
            &self,
            _field_info: &FieldInfo,
        ) -> crate::error::Result<Box<dyn NumericDocValues>> {
            Ok(Box::new(TestNorms))
        }
    }

    fn test_segment_info(name: &str, max_doc: i32) -> crate::index::SegmentInfo {
        crate::codecs::tests::test_segment_info(name, max_doc)
    }

    fn test_write_state<'a>(
        dir: &'a dyn Directory,
        info: &'a crate::index::SegmentInfo,
        infos: &'a FieldInfos,
    ) -> SegmentWriteState<'a> {
        SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir,
            info,
            infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        )
    }

    fn field_infos_with_positions() -> FieldInfos {
        FieldInfos::new(vec![FieldInfo::new("body", 0).with_postings_options(
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            false,
            true,
        )])
        .expect("valid field infos")
    }

    // -------------------------------------------------------------------------
    // Minimal stub postings/fields for the end-to-end test.
    // -------------------------------------------------------------------------

    #[derive(Debug, Clone)]
    struct TestPostingsEnum {
        docs: Vec<i32>,
        freqs: Vec<i32>,
        positions: Vec<Vec<(i32, Option<&'static [u8]>, i32, i32)>>,
        doc_idx: i32,
        pos_idx: i32,
    }

    impl TestPostingsEnum {
        fn new(
            docs: Vec<i32>,
            freqs: Vec<i32>,
            positions: Vec<Vec<(i32, Option<&'static [u8]>, i32, i32)>>,
        ) -> Self {
            Self {
                docs,
                freqs,
                positions,
                doc_idx: -1,
                pos_idx: -1,
            }
        }
    }

    impl DocIdSetIterator for TestPostingsEnum {
        fn doc_id(&self) -> i32 {
            if self.doc_idx < 0 {
                -1
            } else if self.doc_idx as usize >= self.docs.len() {
                NO_MORE_DOCS
            } else {
                self.docs[self.doc_idx as usize]
            }
        }

        fn next_doc(&mut self) -> crate::error::Result<i32> {
            self.doc_idx += 1;
            self.pos_idx = -1;
            if self.doc_idx as usize >= self.docs.len() {
                Ok(NO_MORE_DOCS)
            } else {
                Ok(self.docs[self.doc_idx as usize])
            }
        }

        fn advance(&mut self, _target: i32) -> crate::error::Result<i32> {
            self.next_doc()
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    impl PostingsEnum for TestPostingsEnum {
        fn freq(&self) -> crate::error::Result<i32> {
            Ok(self.freqs[self.doc_idx as usize])
        }

        fn next_position(&mut self) -> crate::error::Result<i32> {
            self.pos_idx += 1;
            Ok(self.positions[self.doc_idx as usize][self.pos_idx as usize].0)
        }

        fn start_offset(&self) -> i32 {
            self.positions[self.doc_idx as usize][self.pos_idx as usize].2
        }

        fn end_offset(&self) -> i32 {
            self.positions[self.doc_idx as usize][self.pos_idx as usize].3
        }

        fn get_payload(&self) -> crate::error::Result<Option<&[u8]>> {
            Ok(self.positions[self.doc_idx as usize][self.pos_idx as usize].1)
        }
    }

    #[derive(Debug, Clone)]
    struct TestTermsEnum {
        terms: Vec<BytesRef>,
        postings: Vec<TestPostingsEnum>,
        idx: i32,
        atts: AttributeSource,
    }

    impl TestTermsEnum {
        fn new(terms: Vec<(BytesRef, TestPostingsEnum)>) -> Self {
            let (term_vec, posting_vec) = terms.into_iter().unzip();
            Self {
                terms: term_vec,
                postings: posting_vec,
                idx: -1,
                atts: AttributeSource::new(),
            }
        }
    }

    impl TermsEnum for TestTermsEnum {
        fn attributes(&mut self) -> &mut AttributeSource {
            &mut self.atts
        }

        fn term(&self) -> crate::error::Result<BytesRef> {
            Ok(self.terms[self.idx as usize].clone())
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> crate::error::Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(self.postings[self.idx as usize].clone()))
        }

        fn seek_exact(&mut self, _text: &BytesRef) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn seek_ceil(&mut self, _text: &BytesRef) -> crate::error::Result<SeekStatus> {
            Ok(SeekStatus::END)
        }

        fn seek_ord(&mut self, _ord: i64) -> crate::error::Result<()> {
            Ok(())
        }

        fn seek_term_state(
            &mut self,
            _text: &BytesRef,
            _state: &dyn crate::index::TermState,
        ) -> crate::error::Result<()> {
            Ok(())
        }

        fn ord(&self) -> crate::error::Result<i64> {
            Ok(self.idx as i64)
        }

        fn doc_freq(&self) -> crate::error::Result<i32> {
            Ok(self.postings[self.idx as usize].docs.len() as i32)
        }

        fn total_term_freq(&self) -> crate::error::Result<i64> {
            Ok(self.postings[self.idx as usize]
                .freqs
                .iter()
                .map(|f| *f as i64)
                .sum())
        }

        fn impacts(
            &mut self,
            _flags: i32,
        ) -> crate::error::Result<Box<dyn crate::index::ImpactsEnum>> {
            Err(crate::error::LuceneError::UnsupportedOperation(
                "impacts not supported".to_string(),
            ))
        }

        fn term_state(&mut self) -> crate::error::Result<Box<dyn crate::index::TermState>> {
            Ok(Box::new(BlockTermState::default()))
        }

        fn next(&mut self) -> crate::error::Result<Option<BytesRef>> {
            self.idx += 1;
            if self.idx as usize >= self.terms.len() {
                Ok(None)
            } else {
                Ok(Some(self.terms[self.idx as usize].clone()))
            }
        }
    }

    #[derive(Debug, Clone)]
    struct TestTerms {
        terms: Vec<(BytesRef, TestPostingsEnum)>,
    }

    impl Terms for TestTerms {
        fn iterator(&self) -> crate::error::Result<Box<dyn TermsEnum>> {
            Ok(Box::new(TestTermsEnum::new(self.terms.clone())))
        }

        fn size(&self) -> i64 {
            self.terms.len() as i64
        }

        fn sum_total_term_freq(&self) -> i64 {
            self.terms
                .iter()
                .map(|(_, p)| p.freqs.iter().map(|f| *f as i64).sum::<i64>())
                .sum()
        }

        fn sum_doc_freq(&self) -> i64 {
            self.terms.iter().map(|(_, p)| p.docs.len() as i64).sum()
        }

        fn doc_count(&self) -> i32 {
            self.terms.len() as i32
        }

        fn has_freqs(&self) -> bool {
            true
        }

        fn has_offsets(&self) -> bool {
            true
        }

        fn has_positions(&self) -> bool {
            true
        }

        fn has_payloads(&self) -> bool {
            true
        }
    }

    #[derive(Debug, Clone)]
    struct TestFields {
        terms: Vec<(BytesRef, TestPostingsEnum)>,
    }

    impl Fields for TestFields {
        fn size(&self) -> i32 {
            1
        }

        fn terms(&self, field: &str) -> crate::error::Result<Option<Box<dyn Terms>>> {
            if field == "body" {
                Ok(Some(Box::new(TestTerms {
                    terms: self.terms.clone(),
                })))
            } else {
                Ok(None)
            }
        }

        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::once("body".to_string()))
        }
    }

    fn assert_file_has_valid_header_footer(dir: &dyn Directory, name: &str, codec: &str) {
        let mut input = dir.open_checksum_input(name).expect("file should exist");
        check_index_header(input.as_mut(), codec, 0, VERSION_CURRENT, &[0u8; 16], "")
            .expect("header should match");
        crate::codecs::codec_util::retrieve_checksum(input.as_mut())
            .expect("footer checksum should be valid");
    }

    #[test]
    fn writes_postings_files_with_headers_and_footers() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("_0", 10);
        let field_infos = field_infos_with_positions();
        let write_state = test_write_state(dir_ref, &segment_info, &field_infos);

        let format = Lucene104PostingsFormat::new();
        let mut consumer = format
            .fields_consumer(&write_state)
            .expect("consumer builds");

        let term1 = BytesRef::new(b"term1".to_vec());
        let term2 = BytesRef::new(b"term2".to_vec());
        let postings = TestFields {
            terms: vec![
                (
                    term1,
                    TestPostingsEnum::new(
                        vec![0, 2],
                        vec![2, 1],
                        vec![
                            vec![(0, Some(b"p1".as_slice()), 0, 4), (3, None, 5, 8)],
                            vec![(1, None, 0, 3)],
                        ],
                    ),
                ),
                (
                    term2,
                    TestPostingsEnum::new(
                        vec![1],
                        vec![3],
                        vec![vec![
                            (0, Some(b"p2".as_slice()), 0, 2),
                            (2, None, 3, 5),
                            (5, None, 6, 9),
                        ]],
                    ),
                ),
            ],
        };
        consumer
            .write(&postings, &TestNorms)
            .expect("write succeeds");
        consumer.close().expect("close succeeds");

        let names = dir.list_all().expect("directory lists files");
        assert!(names.contains(&"_0.psm".to_string()));
        assert!(names.contains(&"_0.doc".to_string()));
        assert!(names.contains(&"_0.pos".to_string()));
        assert!(names.contains(&"_0.pay".to_string()));

        assert_file_has_valid_header_footer(dir_ref, "_0.psm", META_CODEC);
        assert_file_has_valid_header_footer(dir_ref, "_0.doc", DOC_CODEC);
        assert_file_has_valid_header_footer(dir_ref, "_0.pos", POS_CODEC);
        assert_file_has_valid_header_footer(dir_ref, "_0.pay", PAY_CODEC);
    }

    fn field_infos_with_freqs() -> FieldInfos {
        FieldInfos::new(vec![FieldInfo::new("body", 0).with_postings_options(
            IndexOptions::DOCS_AND_FREQS,
            false,
            false,
        )])
        .expect("valid field infos")
    }

    fn read_file_bytes(dir: &dyn Directory, name: &str) -> Vec<u8> {
        let mut input = dir
            .open_input(name, &*crate::store::DEFAULT_IO_CONTEXT)
            .expect("file should exist");
        let len = input.length() as usize;
        let mut bytes = vec![0u8; len];
        input
            .read_bytes(&mut bytes, 0, len)
            .expect("read should succeed");
        bytes
    }

    fn write_test_fields(
        dir: &dyn Directory,
        field_infos: &FieldInfos,
        terms: Vec<(BytesRef, TestPostingsEnum)>,
    ) {
        let max_doc = terms
            .iter()
            .map(|(_, p)| p.docs.last().map(|d| d + 1).unwrap_or(0).max(1))
            .max()
            .unwrap_or(1);
        let segment_info = test_segment_info("_0", max_doc);
        let write_state = test_write_state(dir, &segment_info, field_infos);

        let format = Lucene104PostingsFormat::new();
        let mut consumer = format
            .fields_consumer(&write_state)
            .expect("consumer builds");
        consumer
            .write(&TestFields { terms }, &TestNorms)
            .expect("write succeeds");
        consumer.close().expect("close succeeds");
    }

    fn assert_file_matches(dir: &dyn Directory, name: &str, expected_b64: Option<&str>) {
        let exists = dir
            .list_all()
            .expect("list_all should succeed")
            .contains(&name.to_string());
        if let Some(expected_b64) = expected_b64 {
            assert!(exists, "{name} should exist for this field config");
            let actual = read_file_bytes(dir, name);
            let expected = base64::engine::general_purpose::STANDARD
                .decode(expected_b64)
                .expect("expected base64 should decode");
            assert_eq!(
                actual, expected,
                "{name} bytes differ from Java Lucene reference"
            );
        } else {
            assert!(!exists, "{name} should not exist for this field config");
        }
    }

    // Java reference bytes were generated by
    // tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/PostingsWriterFixture.java
    // using Apache Lucene Core 10.5.0. Re-run that class for the matching case to
    // regenerate the values below.

    #[test]
    fn singleton_doc_term_matches_java_bytes() {
        // Reference: case=SINGLETON from PostingsWriterFixture.
        const DOC_B64: &str = "P9dsFxpMdWNlbmUxMDRQb3N0aW5nc1dyaXRlckRvYwAAAAAAAAAAAAAAAAAAAAAAAAAAAMAok+gAAAAAAAAAAMeH+LI=";
        const PSM_B64: &str = "P9dsFxtMdWNlbmUxMDRQb3N0aW5nc1dyaXRlck1ldGEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAARAAAAAAAAADAKJPoAAAAAAAAAACdve11";

        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let field_infos = field_infos_with_freqs();
        let postings = TestPostingsEnum::new(vec![0], vec![1], vec![vec![]]);
        write_test_fields(
            dir_ref,
            &field_infos,
            vec![(BytesRef::new(b"singleton".to_vec()), postings)],
        );

        assert_file_matches(dir_ref, "_0.doc", Some(DOC_B64));
        assert_file_matches(dir_ref, "_0.pos", None);
        assert_file_matches(dir_ref, "_0.pay", None);
        assert_file_matches(dir_ref, "_0.psm", Some(PSM_B64));
    }

    #[test]
    fn multi_doc_term_matches_java_bytes() {
        // Reference: case=MULTI_DOC from PostingsWriterFixture.
        const DOC_B64: &str = "P9dsFxpMdWNlbmUxMDRQb3N0aW5nc1dyaXRlckRvYwAAAAAAAAAAAAAAAAAAAAAAAAAAAAACBQYFAwLAKJPoAAAAAAAAAAAELOIl";
        const PSM_B64: &str = "P9dsFxtMdWNlbmUxMDRQb3N0aW5nc1dyaXRlck1ldGEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAASwAAAAAAAADAKJPoAAAAAAAAAAATV4pP";

        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let field_infos = field_infos_with_freqs();
        let postings = TestPostingsEnum::new(
            vec![0, 2, 5, 7],
            vec![3, 1, 2, 1],
            vec![vec![], vec![], vec![], vec![]],
        );
        write_test_fields(
            dir_ref,
            &field_infos,
            vec![(BytesRef::new(b"multidoc".to_vec()), postings)],
        );

        assert_file_matches(dir_ref, "_0.doc", Some(DOC_B64));
        assert_file_matches(dir_ref, "_0.pos", None);
        assert_file_matches(dir_ref, "_0.pay", None);
        assert_file_matches(dir_ref, "_0.psm", Some(PSM_B64));
    }

    #[test]
    fn positions_payloads_offsets_term_matches_java_bytes() {
        // Reference: case=POSITIONS from PostingsWriterFixture.
        const DOC_B64: &str = "P9dsFxpMdWNlbmUxMDRQb3N0aW5nc1dyaXRlckRvYwAAAAAAAAAAAAAAAAAAAAAAAAAAAAIFAgIDwCiT6AAAAAAAAAAAcGVSmg==";
        const POS_B64: &str = "P9dsFxpMdWNlbmUxMDRQb3N0aW5nc1dyaXRlclBvcwAAAAAAAAAAAAAAAAAAAAAAAAAAAAECcDABAgcACwMDAnAyAAEAAQQJAnAzCwIFAAbAKJPoAAAAAAAAAAD3rk8E";
        const PAY_B64: &str = "P9dsFxpMdWNlbmUxMDRQb3N0aW5nc1dyaXRlclBheQAAAAAAAAAAAAAAAAAAAAAAAAAAAMAok+gAAAAAAAAAAMpgXAw=";
        const PSM_B64: &str = "P9dsFxtMdWNlbmUxMDRQb3N0aW5nc1dyaXRlck1ldGEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAASQAAAAAAAABgAAAAAAAAAEQAAAAAAAAAwCiT6AAAAAAAAAAA872Nug==";

        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let field_infos = field_infos_with_positions();
        let postings = TestPostingsEnum::new(
            vec![0, 2, 3],
            vec![2, 1, 3],
            vec![
                vec![(0, Some(b"p0".as_slice()), 0, 2), (3, None, 5, 8)],
                vec![(1, Some(b"p2".as_slice()), 0, 3)],
                vec![
                    (0, None, 0, 4),
                    (4, Some(b"p3".as_slice()), 5, 7),
                    (6, None, 8, 10),
                ],
            ],
        );
        write_test_fields(
            dir_ref,
            &field_infos,
            vec![(BytesRef::new(b"positions".to_vec()), postings)],
        );

        assert_file_matches(dir_ref, "_0.doc", Some(DOC_B64));
        assert_file_matches(dir_ref, "_0.pos", Some(POS_B64));
        assert_file_matches(dir_ref, "_0.pay", Some(PAY_B64));
        assert_file_matches(dir_ref, "_0.psm", Some(PSM_B64));
    }

    #[test]
    fn full_block_256_term_matches_java_bytes() {
        // Reference: case=BLOCK_256 from PostingsWriterFixture.
        const DOC_B64: &str = "P9dsFxpMdWNlbmUxMDRQb3N0aW5nc1dyaXRlckRvYwAAAAAAAAAAAAAAAAAAAAAAAAAAAAYAAQUAAQAAAAHAKJPoAAAAAAAAAADESAyS";
        const PSM_B64: &str = "P9dsFxtMdWNlbmUxMDRQb3N0aW5nc1dyaXRlck1ldGEAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAAAAQAAAAAAAAAAAAAATgAAAAAAAADAKJPoAAAAAAAAAACZYa/u";

        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let field_infos = field_infos_with_freqs();
        let postings = TestPostingsEnum::new((0..256).collect(), vec![1; 256], vec![vec![]; 256]);
        write_test_fields(
            dir_ref,
            &field_infos,
            vec![(BytesRef::new(b"block256".to_vec()), postings)],
        );

        assert_file_matches(dir_ref, "_0.doc", Some(DOC_B64));
        assert_file_matches(dir_ref, "_0.pos", None);
        assert_file_matches(dir_ref, "_0.pay", None);
        assert_file_matches(dir_ref, "_0.psm", Some(PSM_B64));
    }

    #[test]
    fn level1_skip_8193_term_matches_java_bytes() {
        // Reference: case=LEVEL1_8193 from PostingsWriterFixture.
        const DOC_B64: &str = "P9dsFxpMdWNlbmUxMDRQb3N0aW5nc1dyaXRlckRvYwAAAAAAAAAAAAAAAAAAAAAAAAAAAIBAxQIDAAEAAAYAAQUAAQAAAAEGAAEFAAEAAAABBgABBQABAAAAAQYAAQUAAQAAAAEGAAEFAAEAAAABBgABBQABAAAAAQYAAQUAAQAAAAEGAAEFAAEAAAABBgABBQABAAAAAQYAAQUAAQAAAAEGAAEFAAEAAAABBgABBQABAAAAAQYAAQUAAQAAAAEGAAEFAAEAAAABBgABBQABAAAAAQYAAQUAAQAAAAEGAAEFAAEAAAABBgABBQABAAAAAQYAAQUAAQAAAAEGAAEFAAEAAAABBgABBQABAAAAAQYAAQUAAQAAAAEGAAEFAAEAAAABBgABBQABAAAAAQYAAQUAAQAAAAEGAAEFAAEAAAABBgABBQABAAAAAQYAAQUAAQAAAAEGAAEFAAEAAAABBgABBQABAAAAAQYAAQUAAQAAAAEGAAEFAAEAAAABA8Aok+gAAAAAAAAAAA5oVTQ=";
        const PSM_B64: &str = "P9dsFxtMdWNlbmUxMDRQb3N0aW5nc1dyaXRlck1ldGEAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAAAAQAAAAEAAAABAAAAjgEAAAAAAADAKJPoAAAAAAAAAAC2WzFp";

        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let field_infos = field_infos_with_freqs();
        let postings =
            TestPostingsEnum::new((0..8193).collect(), vec![1; 8193], vec![vec![]; 8193]);
        write_test_fields(
            dir_ref,
            &field_infos,
            vec![(BytesRef::new(b"level1".to_vec()), postings)],
        );

        assert_file_matches(dir_ref, "_0.doc", Some(DOC_B64));
        assert_file_matches(dir_ref, "_0.pos", None);
        assert_file_matches(dir_ref, "_0.pay", None);
        assert_file_matches(dir_ref, "_0.psm", Some(PSM_B64));
    }

    #[test]
    fn write_impacts_round_trip() {
        let mut out = crate::store::ByteArrayDataOutput::new();
        let impacts = vec![Impact::new(1, 1), Impact::new(3, 2), Impact::new(5, 3)];
        write_impacts(&impacts, &mut out).unwrap();
        // The exact bytes are tested against the format expectations indirectly
        // through the end-to-end write above; here we just assert no panic and
        // non-empty output.
        assert!(!out.into_inner().is_empty());
    }
}
