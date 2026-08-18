//! Lucene 10.4 postings reader.
//!
//! Port of `org.apache.lucene.codecs.lucene104.Lucene104PostingsReader`. This
//! reader decodes the `.psm` (metadata), `.doc`, `.pos` and `.pay` files for
//! the Lucene 10.4 postings format.
//!
//! Lucene Core equivalent: `org.apache.lucene.codecs.lucene104.Lucene104PostingsReader`.

// The postings enum holds references to its own IndexInput fields while also
// mutating its other fields. To avoid fighting the borrow checker we use raw
// pointers to the boxed inputs and convert them back to references in a small,
// documented unsafe helper. The boxes are never moved while the enum is alive.
#![allow(unsafe_code)]

use crate::codecs::codec_util::{
    check_footer, check_index_header, checksum_entire_file, retrieve_checksum_expected_length,
};
use crate::codecs::lucene104::{
    for_util::{ForUtil, BLOCK_SIZE as FOR_BLOCK_SIZE},
    p_for_util::PForUtil,
    posting_decoding_util::PostingDecodingUtil,
    postings_format::{
        DOC_CODEC, LEVEL1_NUM_DOCS, META_CODEC, PAY_CODEC, POS_CODEC, TERMS_CODEC, VERSION_CURRENT,
        VERSION_START,
    },
    postings_util::read_v_int_block,
};
use crate::codecs::postings::{ImpactsEnum, PostingsEnum, PostingsReaderBase};
use crate::codecs::state::SegmentReadState;
use crate::codecs::term_state::BlockTermState;
use crate::error::{LuceneError, Result};
use crate::index::{FieldInfo, IndexOptions};
use crate::index::{FreqAndNormBuffer, Impacts, ImpactsSource};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{DataInput, IndexInput};
use crate::util::{BitUtil, FixedBitSet};

use crate::index::index_file_names::segment_file_name;

/// Number of documents in a packed postings block.
const BS: usize = FOR_BLOCK_SIZE;
/// `BLOCK_SIZE` as `i32`.
const BS_I32: i32 = BS as i32;

// ---------------------------------------------------------------------------
// VInt15 / VLong15 helpers (mirrors Lucene104PostingsReader.readVInt15 /
// readVLong15).
// ---------------------------------------------------------------------------

fn read_v_int15(in_: &mut dyn DataInput) -> Result<i32> {
    let s = in_.read_short()? as i32;
    if s >= 0 {
        Ok(s)
    } else {
        Ok((s & 0x7FFF) | (in_.read_v_int()? << 15))
    }
}

fn read_v_long15(in_: &mut dyn DataInput) -> Result<i64> {
    let s = in_.read_short()? as i64;
    if s >= 0 {
        Ok(s)
    } else {
        Ok((s & 0x7FFF) | (in_.read_v_long()? << 15))
    }
}

fn prefix_sum(buffer: &mut [i32], count: usize, base: i32) {
    let mut sum = base;
    for value in buffer.iter_mut().take(count) {
        sum += *value;
        *value = sum;
    }
}

fn sum_over_range(arr: &[i32], start: usize, end: usize) -> i32 {
    arr[start..end].iter().sum()
}

/// Find the first index in `arr[start..end]` whose value is >= target.
fn find_next_geq(arr: &[i32], target: i32, start: usize, end: usize) -> usize {
    arr[start..end]
        .iter()
        .position(|&v| v >= target)
        .map(|p| start + p)
        .unwrap_or(end)
}

fn read_impacts(serialized: &[u8], buffer: &mut FreqAndNormBuffer) -> Result<()> {
    let mut input = crate::store::ByteArrayDataInput::new(serialized.to_vec());
    let mut freq = 0i32;
    let mut norm = 0i64;
    buffer.size = 0;
    while input.remaining() > 0 {
        let freq_delta = input.read_v_int()?;
        if (freq_delta & 0x01) != 0 {
            freq += 1 + (freq_delta >> 1);
            norm += 1 + input.read_z_long()?;
        } else {
            freq += 1 + (freq_delta >> 1);
            norm += 1;
        }
        buffer.add(freq, norm);
    }
    Ok(())
}

fn next_set_bit(bits: &FixedBitSet, from_index: i32) -> Option<i32> {
    let len = bits.length() as i32;
    if from_index >= len {
        return None;
    }
    let from_index = from_index.max(0) as usize;
    let words = bits.get_bits();
    let num_words = words.len();
    let mut word_index = from_index >> 6;
    if word_index >= num_words {
        return None;
    }
    let shift = from_index & 0x3f;
    let mut word = words[word_index] & (!0u64 << shift);
    while word == 0 {
        word_index += 1;
        if word_index >= num_words {
            return None;
        }
        word = words[word_index];
    }
    let bit_index = word.trailing_zeros() as usize;
    let candidate = (word_index << 6) + bit_index;
    if candidate >= bits.length() {
        return None;
    }
    Some(candidate as i32)
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// Low-level postings reader for the Lucene 10.4 format.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene104.Lucene104PostingsReader`.
pub struct Lucene104PostingsReader {
    version: i32,
    doc_in: Option<Box<dyn IndexInput>>,
    pos_in: Option<Box<dyn IndexInput>>,
    pay_in: Option<Box<dyn IndexInput>>,
    max_num_impacts_at_level0: i32,
    max_impact_num_bytes_at_level0: i32,
    max_num_impacts_at_level1: i32,
    max_impact_num_bytes_at_level1: i32,
}

impl std::fmt::Debug for Lucene104PostingsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene104PostingsReader")
            .field("version", &self.version)
            .field("has_positions", &self.pos_in.is_some())
            .field("has_payloads_or_offsets", &self.pay_in.is_some())
            .finish_non_exhaustive()
    }
}

impl Lucene104PostingsReader {
    /// Creates a postings reader for the given segment read state.
    pub fn new(state: &SegmentReadState<'_>) -> Result<Self> {
        let segment_info = state.segment_info;
        let suffix = &state.segment_suffix;
        let dir = state.directory;
        let id = segment_info.id();

        let meta_name = segment_file_name(&segment_info.name, suffix, "psm");
        let mut meta_in = dir.open_checksum_input(&meta_name)?;

        let mut expected_pos_file_length: i64 = -1;
        let mut expected_pay_file_length: i64 = -1;

        let version = check_index_header(
            meta_in.as_mut(),
            META_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            &id,
            suffix,
        )?;
        let max_num_impacts_at_level0 = meta_in.read_int()?;
        let max_impact_num_bytes_at_level0 = meta_in.read_int()?;
        let max_num_impacts_at_level1 = meta_in.read_int()?;
        let max_impact_num_bytes_at_level1 = meta_in.read_int()?;
        let expected_doc_file_length = meta_in.read_long()?;
        if state.field_infos.has_prox() {
            expected_pos_file_length = meta_in.read_long()?;
            if state.field_infos.has_payloads() || state.field_infos.has_offsets() {
                expected_pay_file_length = meta_in.read_long()?;
            }
        }
        check_footer(meta_in.as_mut())?;

        let doc_name = segment_file_name(&segment_info.name, suffix, "doc");
        let pos_name = segment_file_name(&segment_info.name, suffix, "pos");
        let pay_name = segment_file_name(&segment_info.name, suffix, "pay");

        let mut doc_in: Option<Box<dyn IndexInput>> = None;
        let mut pos_in: Option<Box<dyn IndexInput>> = None;
        let mut pay_in: Option<Box<dyn IndexInput>> = None;

        let result: Result<()> = (|| {
            let mut d = dir.open_input(&doc_name, state.context)?;
            check_index_header(d.as_mut(), DOC_CODEC, version, version, &id, suffix)?;
            retrieve_checksum_expected_length(d.as_mut(), expected_doc_file_length)?;
            doc_in = Some(d);

            if state.field_infos.has_prox() {
                let mut p = dir.open_input(&pos_name, state.context)?;
                check_index_header(p.as_mut(), POS_CODEC, version, version, &id, suffix)?;
                retrieve_checksum_expected_length(p.as_mut(), expected_pos_file_length)?;
                pos_in = Some(p);

                if state.field_infos.has_payloads() || state.field_infos.has_offsets() {
                    let mut y = dir.open_input(&pay_name, state.context)?;
                    check_index_header(y.as_mut(), PAY_CODEC, version, version, &id, suffix)?;
                    retrieve_checksum_expected_length(y.as_mut(), expected_pay_file_length)?;
                    pay_in = Some(y);
                }
            }
            Ok(())
        })();

        result.inspect_err(|_| {
            let _ = doc_in.as_mut().map(|i| i.close());
            let _ = pos_in.as_mut().map(|i| i.close());
            let _ = pay_in.as_mut().map(|i| i.close());
        })?;

        Ok(Self {
            version,
            doc_in,
            pos_in,
            pay_in,
            max_num_impacts_at_level0,
            max_impact_num_bytes_at_level0,
            max_num_impacts_at_level1,
            max_impact_num_bytes_at_level1,
        })
    }
}

impl PostingsReaderBase for Lucene104PostingsReader {
    fn init(&mut self, terms_in: &mut dyn IndexInput, state: &SegmentReadState) -> Result<()> {
        check_index_header(
            terms_in,
            TERMS_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        let index_block_size = terms_in.read_v_int()?;
        if index_block_size != BS_I32 {
            return Err(LuceneError::CorruptIndex(format!(
                "index-time BLOCK_SIZE ({}) != read-time BLOCK_SIZE ({BS_I32})",
                index_block_size
            )));
        }
        Ok(())
    }

    fn new_term_state(&self) -> Result<BlockTermState> {
        Ok(BlockTermState::default())
    }

    fn decode_term(
        &mut self,
        input: &mut dyn DataInput,
        field_info: &FieldInfo,
        state: &mut BlockTermState,
        absolute: bool,
    ) -> Result<()> {
        if absolute {
            state.doc_start_fp = 0;
            state.pos_start_fp = 0;
            state.pay_start_fp = 0;
        }

        let l = input.read_v_long()?;
        if (l & 0x01) == 0 {
            state.doc_start_fp += l >> 1;
            if state.doc_freq == 1 {
                state.singleton_doc_id = input.read_v_int()?;
            } else {
                state.singleton_doc_id = -1;
            }
        } else {
            debug_assert!(!absolute);
            debug_assert!(state.singleton_doc_id != -1);
            state.singleton_doc_id += BitUtil::zig_zag_decode_long(l >> 1) as i32;
        }

        if field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        {
            state.pos_start_fp += input.read_v_long()?;
            if field_info
                .index_options
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
                || field_info.has_payloads()
            {
                state.pay_start_fp += input.read_v_long()?;
            }
            if state.total_term_freq > BS_I32 as i64 {
                state.last_pos_block_offset = input.read_v_long()?;
            } else {
                state.last_pos_block_offset = -1;
            }
        }
        Ok(())
    }

    fn postings(
        &mut self,
        field_info: &FieldInfo,
        state: &BlockTermState,
        _reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        let mut enum_ = BlockPostingsEnum::new(
            field_info,
            flags,
            false,
            self.doc_in.as_deref(),
            self.pos_in.as_deref(),
            self.pay_in.as_deref(),
            self.max_num_impacts_at_level0,
            self.max_impact_num_bytes_at_level0,
            self.max_num_impacts_at_level1,
            self.max_impact_num_bytes_at_level1,
        )?;
        enum_.reset(state)?;
        Ok(Box::new(enum_))
    }

    fn impacts(
        &mut self,
        field_info: &FieldInfo,
        state: &BlockTermState,
        flags: i32,
    ) -> Result<Box<dyn ImpactsEnum>> {
        let mut enum_ = BlockPostingsEnum::new(
            field_info,
            flags,
            true,
            self.doc_in.as_deref(),
            self.pos_in.as_deref(),
            self.pay_in.as_deref(),
            self.max_num_impacts_at_level0,
            self.max_impact_num_bytes_at_level0,
            self.max_num_impacts_at_level1,
            self.max_impact_num_bytes_at_level1,
        )?;
        enum_.reset(state)?;
        Ok(Box::new(enum_))
    }

    fn check_integrity(&self) -> Result<()> {
        if let Some(ref doc_in) = self.doc_in {
            let mut clone = doc_in.clone_input()?;
            checksum_entire_file(clone.as_mut())?;
        }
        if let Some(ref pos_in) = self.pos_in {
            let mut clone = pos_in.clone_input()?;
            checksum_entire_file(clone.as_mut())?;
        }
        if let Some(ref pay_in) = self.pay_in {
            let mut clone = pay_in.clone_input()?;
            checksum_entire_file(clone.as_mut())?;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        if let Some(ref mut doc_in) = self.doc_in {
            doc_in.close()?;
        }
        if let Some(ref mut pos_in) = self.pos_in {
            pos_in.close()?;
        }
        if let Some(ref mut pay_in) = self.pay_in {
            pay_in.close()?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Block postings enum
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeltaEncoding {
    Packed,
    Unary,
}

/// Postings enumerator that decodes Lucene 10.4 packed doc/freq/position blocks.
///
/// Lucene Core equivalent:
/// `org.apache.lucene.codecs.lucene104.Lucene104PostingsReader.BlockPostingsEnum`.
pub struct BlockPostingsEnum {
    for_util: ForUtil,
    pfor_util: Option<PForUtil>,

    encoding: DeltaEncoding,
    doc: i32,

    doc_buffer: [i32; BS],
    doc_bit_set: FixedBitSet,
    doc_bit_set_base: i32,
    doc_cumulative_word_pop_counts: [i32; BS / 64],

    level0_last_doc_id: i32,
    level0_doc_end_fp: i64,

    level1_last_doc_id: i32,
    level1_doc_end_fp: i64,
    level1_doc_count_upto: i32,

    doc_freq: i32,
    total_term_freq: i64,
    singleton_doc_id: i32,
    doc_count_left: i32,
    prev_doc_id: i32,
    doc_buffer_size: i32,
    doc_buffer_upto: i32,

    doc_in: Option<Box<dyn IndexInput>>,

    freq_buffer: [i32; BS],
    pos_delta_buffer: Option<[i32; BS]>,
    payload_length_buffer: Option<[i32; BS]>,
    offset_start_delta_buffer: Option<[i32; BS]>,
    offset_length_buffer: Option<[i32; BS]>,

    payload_bytes: Vec<u8>,
    payload_byte_upto: i32,
    payload_length: i32,

    last_start_offset: i32,
    start_offset: i32,
    end_offset: i32,

    pos_buffer_upto: i32,

    pos_in: Option<Box<dyn IndexInput>>,
    pay_in: Option<Box<dyn IndexInput>>,

    index_has_freq: bool,
    index_has_pos: bool,
    index_has_offsets: bool,
    index_has_payloads: bool,
    index_has_offsets_or_payloads: bool,

    needs_freq: bool,
    needs_pos: bool,
    needs_offsets: bool,
    needs_payloads: bool,
    needs_offsets_or_payloads: bool,
    needs_impacts: bool,

    freq_fp: i64,
    position: i32,
    pos_doc_buffer_upto: i32,
    pos_pending_count: i32,
    last_pos_block_fp: i64,

    level0_pos_end_fp: i64,
    level0_block_pos_upto: i32,
    level0_pay_end_fp: i64,
    level0_block_pay_upto: i32,
    level0_serialized_impacts: Vec<u8>,

    level1_pos_end_fp: i64,
    level1_block_pos_upto: i32,
    level1_pay_end_fp: i64,
    level1_block_pay_upto: i32,
    level1_serialized_impacts: Vec<u8>,

    impacts: BlockImpacts,

    needs_refilling: bool,
}

impl std::fmt::Debug for BlockPostingsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockPostingsEnum")
            .field("doc", &self.doc)
            .field("doc_freq", &self.doc_freq)
            .finish_non_exhaustive()
    }
}

impl BlockPostingsEnum {
    #[allow(clippy::too_many_arguments)]
    fn new(
        field_info: &FieldInfo,
        flags: i32,
        needs_impacts: bool,
        parent_doc_in: Option<&dyn IndexInput>,
        parent_pos_in: Option<&dyn IndexInput>,
        parent_pay_in: Option<&dyn IndexInput>,
        max_num_impacts_at_level0: i32,
        max_impact_num_bytes_at_level0: i32,
        max_num_impacts_at_level1: i32,
        max_impact_num_bytes_at_level1: i32,
    ) -> Result<Self> {
        let index_options = field_info.index_options;
        let index_has_freq = index_options.subsumes(IndexOptions::DOCS_AND_FREQS);
        let index_has_pos = index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        let index_has_offsets =
            index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        let index_has_payloads = field_info.has_payloads();
        let index_has_offsets_or_payloads = index_has_offsets || index_has_payloads;

        let needs_freq = index_has_freq
            && crate::index::feature_requested(flags, crate::index::POSTINGS_ENUM_FREQS);
        let needs_pos = index_has_pos
            && crate::index::feature_requested(flags, crate::index::POSTINGS_ENUM_POSITIONS);
        let needs_offsets = index_has_offsets
            && crate::index::feature_requested(flags, crate::index::POSTINGS_ENUM_OFFSETS);
        let needs_payloads = index_has_payloads
            && crate::index::feature_requested(flags, crate::index::POSTINGS_ENUM_PAYLOADS);
        let needs_offsets_or_payloads = needs_offsets || needs_payloads;

        let freq_buffer = [1i32; BS];
        let pos_delta_buffer = if needs_pos { Some([0i32; BS]) } else { None };
        let payload_length_buffer = if index_has_payloads {
            Some([0i32; BS])
        } else {
            None
        };
        let (offset_start_delta_buffer, offset_length_buffer) = if needs_offsets {
            (Some([0i32; BS]), Some([0i32; BS]))
        } else {
            (None, None)
        };

        let payload_bytes = if index_has_payloads {
            vec![0u8; 128]
        } else {
            Vec::new()
        };

        let mut impact_buffer = FreqAndNormBuffer::new();
        let capacity = if needs_impacts && needs_freq {
            (max_num_impacts_at_level0.max(max_num_impacts_at_level1)).max(1) as usize
        } else {
            1
        };
        impact_buffer.grow_no_copy(capacity);

        let level0_serialized_impacts = if needs_impacts && needs_freq {
            vec![0u8; max_impact_num_bytes_at_level0 as usize]
        } else {
            Vec::new()
        };
        let level1_serialized_impacts = if needs_impacts && needs_freq {
            vec![0u8; max_impact_num_bytes_at_level1 as usize]
        } else {
            Vec::new()
        };

        let doc_in = parent_doc_in.map(|input| input.clone_input()).transpose()?;
        let pos_in = if needs_pos {
            parent_pos_in.map(|input| input.clone_input()).transpose()?
        } else {
            None
        };
        let pay_in = if needs_offsets_or_payloads {
            parent_pay_in.map(|input| input.clone_input()).transpose()?
        } else {
            None
        };

        let impacts = BlockImpacts {
            index_has_freq,
            level0_last_doc_id: -1,
            level1_last_doc_id: -1,
            level0_serialized_impacts: level0_serialized_impacts.clone(),
            level1_serialized_impacts: level1_serialized_impacts.clone(),
            impact_buffer: impact_buffer.clone(),
        };

        Ok(Self {
            for_util: ForUtil::new(),
            pfor_util: Some(PForUtil::new()),
            encoding: DeltaEncoding::Packed,
            doc: -1,
            doc_buffer: [0i32; BS],
            doc_bit_set: FixedBitSet::new(BS * 32),
            doc_bit_set_base: 0,
            doc_cumulative_word_pop_counts: [0i32; BS / 64],
            level0_last_doc_id: -1,
            level0_doc_end_fp: 0,
            level1_last_doc_id: -1,
            level1_doc_end_fp: 0,
            level1_doc_count_upto: 0,
            doc_freq: 0,
            total_term_freq: 0,
            singleton_doc_id: -1,
            doc_count_left: 0,
            prev_doc_id: -1,
            doc_buffer_size: BS_I32,
            doc_buffer_upto: BS_I32,
            doc_in,
            freq_buffer,
            pos_delta_buffer,
            payload_length_buffer,
            offset_start_delta_buffer,
            offset_length_buffer,
            payload_bytes,
            payload_byte_upto: 0,
            payload_length: 0,
            last_start_offset: 0,
            start_offset: -1,
            end_offset: -1,
            pos_buffer_upto: BS_I32,
            pos_in,
            pay_in,
            index_has_freq,
            index_has_pos,
            index_has_offsets,
            index_has_payloads,
            index_has_offsets_or_payloads,
            needs_freq,
            needs_pos,
            needs_offsets,
            needs_payloads,
            needs_offsets_or_payloads,
            needs_impacts,
            freq_fp: -1,
            position: 0,
            pos_doc_buffer_upto: BS_I32,
            pos_pending_count: 0,
            last_pos_block_fp: 0,
            level0_pos_end_fp: 0,
            level0_block_pos_upto: 0,
            level0_pay_end_fp: 0,
            level0_block_pay_upto: 0,
            level0_serialized_impacts,
            level1_pos_end_fp: 0,
            level1_block_pos_upto: 0,
            level1_pay_end_fp: 0,
            level1_block_pay_upto: 0,
            level1_serialized_impacts,
            impacts,
            needs_refilling: false,
        })
    }

    fn reset(&mut self, term_state: &BlockTermState) -> Result<()> {
        self.doc_freq = term_state.doc_freq;
        self.singleton_doc_id = term_state.singleton_doc_id;
        self.doc_count_left = self.doc_freq;
        self.prev_doc_id = -1;
        self.doc_buffer_size = BS_I32;
        self.doc_buffer_upto = BS_I32;
        self.pos_doc_buffer_upto = BS_I32;
        self.pos_pending_count = 0;
        self.payload_byte_upto = 0;
        self.position = 0;
        self.last_start_offset = 0;
        self.start_offset = -1;
        self.end_offset = -1;
        self.payload_length = 0;
        self.needs_refilling = false;
        self.level0_last_doc_id = -1;
        self.level0_doc_end_fp = 0;
        self.level1_doc_count_upto = 0;
        self.doc = -1;
        self.freq_fp = -1;
        self.pos_buffer_upto = BS_I32;
        self.level0_serialized_impacts.fill(0);
        self.level1_serialized_impacts.fill(0);

        self.total_term_freq = if self.index_has_freq {
            term_state.total_term_freq
        } else {
            term_state.doc_freq as i64
        };

        if self.doc_freq > 1 && self.doc_in.is_none() {
            // doc_in is lazily initialized by the parent reader in postings().
            return Err(LuceneError::IllegalState(
                "BlockPostingsEnum.doc_in must be initialized by the reader".to_string(),
            ));
        }

        let pos_term_start_fp = term_state.pos_start_fp;
        let pay_term_start_fp = term_state.pay_start_fp;
        if let Some(ref mut pos_in) = self.pos_in {
            pos_in.seek(pos_term_start_fp)?;
            if let Some(ref mut pay_in) = self.pay_in {
                pay_in.seek(pay_term_start_fp)?;
            }
        }
        self.level1_pos_end_fp = pos_term_start_fp;
        self.level1_pay_end_fp = pay_term_start_fp;
        self.level0_pos_end_fp = pos_term_start_fp;
        self.level0_pay_end_fp = pay_term_start_fp;

        if self.total_term_freq < BS_I32 as i64 {
            self.last_pos_block_fp = pos_term_start_fp;
        } else if self.total_term_freq == BS_I32 as i64 {
            self.last_pos_block_fp = -1;
        } else {
            self.last_pos_block_fp = pos_term_start_fp + term_state.last_pos_block_offset;
        }

        self.level1_block_pos_upto = 0;
        self.level1_block_pay_upto = 0;
        self.level0_block_pos_upto = 0;
        self.level0_block_pay_upto = 0;

        if self.doc_freq < LEVEL1_NUM_DOCS {
            self.level1_last_doc_id = NO_MORE_DOCS;
            if self.doc_freq > 1 {
                if let Some(ref mut doc_in) = self.doc_in {
                    doc_in.seek(term_state.doc_start_fp)?;
                }
            }
        } else {
            self.level1_last_doc_id = -1;
            self.level1_doc_end_fp = term_state.doc_start_fp;
        }

        Ok(())
    }

    /// Returns a raw pointer to the doc input stream. This is used to work around
    /// Rust's borrow checker when the enum needs to hold references to its own
    /// inputs while also mutating its other fields. The pointer is valid for as
    /// long as the enum exists because the boxed input is never moved.
    fn doc_in_ptr(&mut self) -> Result<*mut dyn IndexInput> {
        self.doc_in
            .as_mut()
            .map(|b| b.as_mut() as *mut dyn IndexInput)
            .ok_or_else(|| LuceneError::IllegalState("doc_in is not open".to_string()))
    }

    fn pos_in_ptr(&mut self) -> Result<*mut dyn IndexInput> {
        self.pos_in
            .as_mut()
            .map(|b| b.as_mut() as *mut dyn IndexInput)
            .ok_or_else(|| LuceneError::IllegalState("pos_in is not open".to_string()))
    }

    fn pay_in_ptr(&mut self) -> Result<*mut dyn IndexInput> {
        self.pay_in
            .as_mut()
            .map(|b| b.as_mut() as *mut dyn IndexInput)
            .ok_or_else(|| LuceneError::IllegalState("pay_in is not open".to_string()))
    }

    /// # Safety
    /// The pointer must have been obtained from one of this enum's input fields
    /// and the enum must not have been dropped or had its input replaced while
    /// the returned reference is in use.
    unsafe fn input_ref<'a>(ptr: *mut dyn IndexInput) -> &'a mut dyn IndexInput {
        &mut *ptr
    }

    fn refill_full_block(&mut self) -> Result<()> {
        let doc_in = unsafe { Self::input_ref(self.doc_in_ptr()?) };
        let bits_per_value = doc_in.read_byte()? as i32;
        if bits_per_value > 0 {
            let mut pdu = PostingDecodingUtil::new(doc_in);
            self.for_util
                .decode(bits_per_value, &mut pdu, &mut self.doc_buffer)?;
            prefix_sum(&mut self.doc_buffer, BS, self.prev_doc_id);
            self.encoding = DeltaEncoding::Packed;
        } else {
            self.doc_bit_set_base = self.prev_doc_id + 1;
            self.doc_bit_set.clear_all();
            if bits_per_value == 0 {
                // A full 256-doc block: no bitset longs are written.
                for i in 0..BS {
                    self.doc_bit_set.set(i);
                }
                if self.needs_freq {
                    for i in 0..BS / 64 {
                        self.doc_cumulative_word_pop_counts[i] = ((i + 1) * 64) as i32;
                    }
                }
            } else {
                let num_longs = (-bits_per_value) as usize;
                let mut longs = vec![0i64; num_longs];
                doc_in.read_longs(&mut longs, 0, num_longs)?;
                for (i, &l) in longs.iter().enumerate() {
                    let word = l as u64;
                    for b in 0..64 {
                        if (word >> b) & 1 != 0 {
                            self.doc_bit_set.set(i * 64 + b);
                        }
                    }
                }
                if self.needs_freq {
                    for (i, &l) in longs.iter().take(num_longs - 1).enumerate() {
                        self.doc_cumulative_word_pop_counts[i] = (l as u64).count_ones() as i32;
                    }
                    prefix_sum(&mut self.doc_cumulative_word_pop_counts, num_longs - 1, 0);
                    self.doc_cumulative_word_pop_counts[num_longs - 1] = BS_I32;
                }
            }
            self.encoding = DeltaEncoding::Unary;
        }

        if self.index_has_freq {
            if self.needs_freq {
                self.freq_fp = doc_in.file_pointer();
            }
            PForUtil::skip(doc_in)?;
        }

        self.doc_count_left -= BS_I32;
        self.prev_doc_id = self.doc_buffer[BS - 1];
        self.doc_buffer_upto = 0;
        self.pos_doc_buffer_upto = 0;
        Ok(())
    }

    fn refill_remainder(&mut self) -> Result<()> {
        if self.doc_freq == 1 {
            self.doc_buffer[0] = self.singleton_doc_id;
            self.freq_buffer[0] = self.total_term_freq as i32;
            self.doc_buffer[1] = NO_MORE_DOCS;
            self.doc_count_left = 0;
            self.doc_buffer_size = 1;
            self.freq_fp = -1;
        } else {
            let doc_in = unsafe { Self::input_ref(self.doc_in_ptr()?) };
            let count = self.doc_count_left as usize;
            read_v_int_block(
                doc_in,
                &mut self.doc_buffer,
                &mut self.freq_buffer,
                count,
                self.index_has_freq,
                self.needs_freq,
            )?;
            prefix_sum(&mut self.doc_buffer, count, self.prev_doc_id);
            self.doc_buffer[count] = NO_MORE_DOCS;
            self.freq_fp = -1;
            self.doc_buffer_size = count as i32;
            self.doc_count_left = 0;
        }
        self.prev_doc_id = self.doc_buffer[BS - 1];
        self.doc_buffer_upto = 0;
        self.pos_doc_buffer_upto = 0;
        self.encoding = DeltaEncoding::Packed;
        debug_assert!(self.doc_buffer[self.doc_buffer_size as usize] == NO_MORE_DOCS);
        Ok(())
    }

    fn refill_docs(&mut self) -> Result<()> {
        if self.doc_count_left >= BS_I32 {
            self.refill_full_block()
        } else {
            self.refill_remainder()
        }
    }

    fn skip_level1_to(&mut self, target: i32) -> Result<()> {
        loop {
            self.prev_doc_id = self.level1_last_doc_id;
            self.level0_last_doc_id = self.level1_last_doc_id;
            if let Some(ref mut doc_in) = self.doc_in {
                doc_in.seek(self.level1_doc_end_fp)?;
            }
            self.level0_pos_end_fp = self.level1_pos_end_fp;
            self.level0_block_pos_upto = self.level1_block_pos_upto;
            self.level0_pay_end_fp = self.level1_pay_end_fp;
            self.level0_block_pay_upto = self.level1_block_pay_upto;
            self.doc_count_left = self.doc_freq - self.level1_doc_count_upto;
            self.level1_doc_count_upto += LEVEL1_NUM_DOCS;

            if self.doc_count_left < LEVEL1_NUM_DOCS {
                self.level1_last_doc_id = NO_MORE_DOCS;
                break;
            }

            let doc_in = unsafe { Self::input_ref(self.doc_in_ptr()?) };
            self.level1_last_doc_id += doc_in.read_v_int()?;
            let delta = doc_in.read_v_long()?;
            self.level1_doc_end_fp = delta + doc_in.file_pointer();

            if self.index_has_freq {
                let skip1_end_fp = doc_in.read_short()? as i64 + doc_in.file_pointer();
                let num_impact_bytes = doc_in.read_short()? as usize;
                if self.needs_impacts && self.level1_last_doc_id >= target {
                    doc_in.read_bytes(&mut self.level1_serialized_impacts, 0, num_impact_bytes)?;
                } else {
                    doc_in.skip_bytes(num_impact_bytes as i64)?;
                }
                if self.index_has_pos {
                    self.level1_pos_end_fp += doc_in.read_v_long()?;
                    self.level1_block_pos_upto = doc_in.read_byte()? as i32;
                    if self.index_has_offsets_or_payloads {
                        self.level1_pay_end_fp += doc_in.read_v_long()?;
                        self.level1_block_pay_upto = doc_in.read_v_int()?;
                    }
                }
                debug_assert_eq!(doc_in.file_pointer(), skip1_end_fp);
            }

            if self.level1_last_doc_id >= target {
                break;
            }
        }
        Ok(())
    }

    fn do_move_to_next_level0_block(&mut self) -> Result<()> {
        debug_assert_eq!(self.doc, self.level0_last_doc_id);
        if self.pos_in.is_some() {
            if self.level0_pos_end_fp
                >= unsafe { Self::input_ref(self.pos_in_ptr()?) }.file_pointer()
            {
                unsafe { Self::input_ref(self.pos_in_ptr()?) }.seek(self.level0_pos_end_fp)?;
                self.pos_pending_count = self.level0_block_pos_upto;
                if self.pay_in.is_some() {
                    unsafe { Self::input_ref(self.pay_in_ptr()?) }.seek(self.level0_pay_end_fp)?;
                    self.payload_byte_upto = self.level0_block_pay_upto;
                }
                self.pos_buffer_upto = BS_I32;
            } else {
                debug_assert_eq!(self.freq_fp, -1);
                self.pos_pending_count +=
                    sum_over_range(&self.freq_buffer, self.pos_doc_buffer_upto as usize, BS);
            }
        }

        if self.doc_count_left >= BS_I32 {
            let doc_in = unsafe { Self::input_ref(self.doc_in_ptr()?) };
            doc_in.read_v_long()?; // level0NumBytes
            let doc_delta = read_v_int15(doc_in)?;
            self.level0_last_doc_id += doc_delta;
            let block_length = read_v_long15(doc_in)?;
            self.level0_doc_end_fp = doc_in.file_pointer() + block_length;

            if self.index_has_freq {
                let num_impact_bytes = doc_in.read_v_int()? as usize;
                if self.needs_impacts {
                    doc_in.read_bytes(&mut self.level0_serialized_impacts, 0, num_impact_bytes)?;
                } else {
                    doc_in.skip_bytes(num_impact_bytes as i64)?;
                }

                if self.index_has_pos {
                    self.level0_pos_end_fp += doc_in.read_v_long()?;
                    self.level0_block_pos_upto = doc_in.read_byte()? as i32;
                    if self.index_has_offsets_or_payloads {
                        self.level0_pay_end_fp += doc_in.read_v_long()?;
                        self.level0_block_pay_upto = doc_in.read_v_int()?;
                    }
                }
            }
            self.refill_full_block()
        } else {
            self.level0_last_doc_id = NO_MORE_DOCS;
            self.refill_remainder()
        }
    }

    fn move_to_next_level0_block(&mut self) -> Result<()> {
        if self.doc == self.level1_last_doc_id {
            self.skip_level1_to(self.doc + 1)?;
        }
        self.prev_doc_id = self.level0_last_doc_id;
        self.do_move_to_next_level0_block()
    }

    fn read_level0_pos_data(&mut self) -> Result<()> {
        let pos_in = unsafe { Self::input_ref(self.pos_in_ptr()?) };
        self.level0_pos_end_fp += pos_in.read_v_long()?;
        self.level0_block_pos_upto = pos_in.read_byte()? as i32;
        if self.index_has_offsets_or_payloads {
            self.level0_pay_end_fp +=
                unsafe { Self::input_ref(self.pay_in_ptr()?) }.read_v_long()?;
            self.level0_block_pay_upto =
                unsafe { Self::input_ref(self.pay_in_ptr()?) }.read_v_int()?;
        }
        Ok(())
    }

    fn seek_pos_data(
        &mut self,
        pos_fp: i64,
        pos_upto: i32,
        pay_fp: i64,
        pay_upto: i32,
    ) -> Result<()> {
        if pos_fp >= unsafe { Self::input_ref(self.pos_in_ptr()?) }.file_pointer() {
            unsafe { Self::input_ref(self.pos_in_ptr()?) }.seek(pos_fp)?;
            self.pos_pending_count = pos_upto;
            if self.pay_in.is_some() {
                unsafe { Self::input_ref(self.pay_in_ptr()?) }.seek(pay_fp)?;
                self.payload_byte_upto = pay_upto;
            }
            self.pos_buffer_upto = BS_I32;
        } else {
            self.pos_pending_count +=
                sum_over_range(&self.freq_buffer, self.pos_doc_buffer_upto as usize, BS);
        }
        Ok(())
    }

    fn skip_level0_to(&mut self, target: i32) -> Result<()> {
        let mut pos_fp: i64;
        let mut pos_upto: i32;
        let mut pay_fp: i64;
        let mut pay_upto: i32;

        loop {
            self.prev_doc_id = self.level0_last_doc_id;
            pos_fp = self.level0_pos_end_fp;
            pos_upto = self.level0_block_pos_upto;
            pay_fp = self.level0_pay_end_fp;
            pay_upto = self.level0_block_pay_upto;

            if self.doc_count_left >= BS_I32 {
                let doc_in = unsafe { Self::input_ref(self.doc_in_ptr()?) };
                let num_skip_bytes = doc_in.read_v_long()?;
                let skip0_end = doc_in.file_pointer() + num_skip_bytes;
                let doc_delta = read_v_int15(doc_in)?;
                self.level0_last_doc_id += doc_delta;
                let found = target <= self.level0_last_doc_id;
                let block_length = read_v_long15(doc_in)?;
                self.level0_doc_end_fp = doc_in.file_pointer() + block_length;

                if self.index_has_freq {
                    if !found && !self.needs_pos {
                        doc_in.seek(skip0_end)?;
                    } else {
                        let num_impact_bytes = doc_in.read_v_int()? as usize;
                        if self.needs_impacts && found {
                            doc_in.read_bytes(
                                &mut self.level0_serialized_impacts,
                                0,
                                num_impact_bytes,
                            )?;
                        } else {
                            doc_in.skip_bytes(num_impact_bytes as i64)?;
                        }

                        if self.needs_pos {
                            self.read_level0_pos_data()?;
                        } else {
                            doc_in.seek(skip0_end)?;
                        }
                    }
                }

                if found {
                    break;
                }

                doc_in.seek(self.level0_doc_end_fp)?;
                self.doc_count_left -= BS_I32;
            } else {
                self.level0_last_doc_id = NO_MORE_DOCS;
                break;
            }
        }

        if self.pos_in.is_some() {
            self.seek_pos_data(pos_fp, pos_upto, pay_fp, pay_upto)?;
        }
        Ok(())
    }

    fn do_advance_shallow(&mut self, target: i32) -> Result<()> {
        if target > self.level1_last_doc_id {
            self.skip_level1_to(target)?;
        } else if self.needs_refilling {
            if let Some(ref mut doc_in) = self.doc_in {
                doc_in.seek(self.level0_doc_end_fp)?;
            }
            self.doc_count_left -= BS_I32;
        }
        self.skip_level0_to(target)
    }

    fn skip_positions(&mut self, freq: i32) -> Result<()> {
        let mut to_skip = self.pos_pending_count - freq;
        let left_in_block = BS_I32 - self.pos_buffer_upto;
        if to_skip < left_in_block {
            let end = (self.pos_buffer_upto + to_skip) as usize;
            if self.needs_payloads {
                self.payload_byte_upto += sum_over_range(
                    self.payload_length_buffer.as_ref().unwrap(),
                    self.pos_buffer_upto as usize,
                    end,
                );
            }
            self.pos_buffer_upto = end as i32;
        } else {
            to_skip -= left_in_block;
            let pos_in = unsafe { Self::input_ref(self.pos_in_ptr()?) };
            let mut pay_in = if self.index_has_offsets_or_payloads {
                Some(unsafe { Self::input_ref(self.pay_in_ptr()?) })
            } else {
                None
            };
            while to_skip >= BS_I32 {
                debug_assert_ne!(pos_in.file_pointer(), self.last_pos_block_fp);
                PForUtil::skip(pos_in)?;

                if let Some(pay_in) = pay_in.as_deref_mut() {
                    if self.index_has_payloads {
                        PForUtil::skip(pay_in)?;
                        let num_bytes = pay_in.read_v_int()?;
                        pay_in.seek(pay_in.file_pointer() + num_bytes as i64)?;
                    }
                    if self.index_has_offsets {
                        PForUtil::skip(pay_in)?;
                        PForUtil::skip(pay_in)?;
                    }
                }
                to_skip -= BS_I32;
            }
            self.refill_positions()?;
            if self.needs_payloads {
                self.payload_byte_upto = sum_over_range(
                    self.payload_length_buffer.as_ref().unwrap(),
                    0,
                    to_skip as usize,
                );
            }
            self.pos_buffer_upto = to_skip;
        }
        Ok(())
    }

    fn refill_last_position_block(&mut self) -> Result<()> {
        let count = (self.total_term_freq % BS_I32 as i64) as usize;
        let mut payload_length = 0i32;
        let mut offset_length = 0i32;
        self.payload_byte_upto = 0;
        let pos_in = unsafe { Self::input_ref(self.pos_in_ptr()?) };
        for i in 0..count {
            let code = pos_in.read_v_int()?;
            if self.index_has_payloads {
                if (code & 1) != 0 {
                    payload_length = pos_in.read_v_int()?;
                }
                if let Some(ref mut payload_length_buffer) = self.payload_length_buffer {
                    payload_length_buffer[i] = payload_length;
                    self.pos_delta_buffer.as_mut().unwrap()[i] = code >> 1;
                    if payload_length != 0 {
                        let needed = self.payload_byte_upto as usize + payload_length as usize;
                        if needed > self.payload_bytes.len() {
                            self.payload_bytes
                                .resize(needed.max(self.payload_bytes.len() * 2).max(128), 0);
                        }
                        pos_in.read_bytes(
                            &mut self.payload_bytes,
                            self.payload_byte_upto as usize,
                            payload_length as usize,
                        )?;
                        self.payload_byte_upto += payload_length;
                    }
                } else {
                    pos_in.skip_bytes(payload_length as i64)?;
                }
            } else {
                self.pos_delta_buffer.as_mut().unwrap()[i] = code;
            }

            if self.index_has_offsets {
                let delta_code = pos_in.read_v_int()?;
                if (delta_code & 1) != 0 {
                    offset_length = pos_in.read_v_int()?;
                }
                if let Some(ref mut offset_start_delta_buffer) = self.offset_start_delta_buffer {
                    offset_start_delta_buffer[i] = delta_code >> 1;
                    self.offset_length_buffer.as_mut().unwrap()[i] = offset_length;
                }
            }
        }
        self.payload_byte_upto = 0;
        Ok(())
    }

    fn refill_offsets_or_payloads(&mut self) -> Result<()> {
        if self.index_has_payloads {
            if self.needs_payloads {
                {
                    let pay_in = unsafe { Self::input_ref(self.pay_in_ptr()?) };
                    let mut pdu = PostingDecodingUtil::new(pay_in);
                    self.pfor_util
                        .as_mut()
                        .unwrap()
                        .decode(&mut pdu, self.payload_length_buffer.as_mut().unwrap())?;
                }
                let pay_in = unsafe { Self::input_ref(self.pay_in_ptr()?) };
                let num_bytes = pay_in.read_v_int()? as usize;
                if num_bytes > self.payload_bytes.len() {
                    self.payload_bytes.resize(num_bytes, 0);
                }
                pay_in.read_bytes(&mut self.payload_bytes, 0, num_bytes)?;
            } else {
                let pay_in = unsafe { Self::input_ref(self.pay_in_ptr()?) };
                PForUtil::skip(pay_in)?;
                let num_bytes = pay_in.read_v_int()?;
                pay_in.seek(pay_in.file_pointer() + num_bytes as i64)?;
            }
            self.payload_byte_upto = 0;
        }

        if self.index_has_offsets {
            if self.needs_offsets {
                let pay_in = unsafe { Self::input_ref(self.pay_in_ptr()?) };
                let mut pdu = PostingDecodingUtil::new(pay_in);
                self.pfor_util
                    .as_mut()
                    .unwrap()
                    .decode(&mut pdu, self.offset_start_delta_buffer.as_mut().unwrap())?;
                self.pfor_util
                    .as_mut()
                    .unwrap()
                    .decode(&mut pdu, self.offset_length_buffer.as_mut().unwrap())?;
            } else {
                let pay_in = unsafe { Self::input_ref(self.pay_in_ptr()?) };
                PForUtil::skip(pay_in)?;
                PForUtil::skip(pay_in)?;
            }
        }
        Ok(())
    }

    fn refill_positions(&mut self) -> Result<()> {
        let fp = unsafe { Self::input_ref(self.pos_in_ptr()?) }.file_pointer();
        if fp == self.last_pos_block_fp {
            self.refill_last_position_block()
        } else {
            let pos_in = unsafe { Self::input_ref(self.pos_in_ptr()?) };
            let mut pdu = PostingDecodingUtil::new(pos_in);
            self.pfor_util
                .as_mut()
                .unwrap()
                .decode(&mut pdu, self.pos_delta_buffer.as_mut().unwrap())?;

            if self.index_has_offsets_or_payloads {
                self.refill_offsets_or_payloads()?;
            }
            Ok(())
        }
    }

    fn accumulate_pending_positions(&mut self) -> Result<()> {
        let freq = self.freq()?;
        self.pos_pending_count += sum_over_range(
            &self.freq_buffer,
            self.pos_doc_buffer_upto as usize,
            self.doc_buffer_upto as usize,
        );
        self.pos_doc_buffer_upto = self.doc_buffer_upto;

        debug_assert!(self.pos_pending_count > 0);

        if self.pos_pending_count > freq {
            self.skip_positions(freq)?;
            self.pos_pending_count = freq;
        }
        Ok(())
    }

    fn accumulate_payload_and_offsets(&mut self) {
        if self.needs_payloads {
            self.payload_length =
                self.payload_length_buffer.as_ref().unwrap()[self.pos_buffer_upto as usize];
            self.payload_byte_upto += self.payload_length;
        }

        if self.needs_offsets {
            self.start_offset = self.last_start_offset
                + self.offset_start_delta_buffer.as_ref().unwrap()[self.pos_buffer_upto as usize];
            self.end_offset = self.start_offset
                + self.offset_length_buffer.as_ref().unwrap()[self.pos_buffer_upto as usize];
            self.last_start_offset = self.start_offset;
        }
    }

    fn update_impacts_state(&mut self) {
        self.impacts.index_has_freq = self.index_has_freq;
        self.impacts.level0_last_doc_id = self.level0_last_doc_id;
        self.impacts.level1_last_doc_id = self.level1_last_doc_id;
        self.impacts
            .level0_serialized_impacts
            .clone_from(&self.level0_serialized_impacts);
        self.impacts
            .level1_serialized_impacts
            .clone_from(&self.level1_serialized_impacts);
    }
}

impl DocIdSetIterator for BlockPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.doc == self.level0_last_doc_id || self.needs_refilling {
            if self.needs_refilling {
                self.refill_docs()?;
                self.needs_refilling = false;
            } else {
                self.move_to_next_level0_block()?;
            }
        }

        match self.encoding {
            DeltaEncoding::Packed => {
                self.doc = self.doc_buffer[self.doc_buffer_upto as usize];
            }
            DeltaEncoding::Unary => {
                let next = next_set_bit(&self.doc_bit_set, self.doc - self.doc_bit_set_base + 1)
                    .expect("UNARY block must have next set bit");
                self.doc = self.doc_bit_set_base + next;
            }
        }

        self.doc_buffer_upto += 1;
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target > self.level0_last_doc_id || self.needs_refilling {
            if target > self.level0_last_doc_id {
                self.do_advance_shallow(target)?;
            }
            self.refill_docs()?;
            self.needs_refilling = false;
        }

        match self.encoding {
            DeltaEncoding::Packed => {
                let next = find_next_geq(
                    &self.doc_buffer,
                    target,
                    self.doc_buffer_upto as usize,
                    self.doc_buffer_size as usize,
                );
                self.doc = self.doc_buffer[next];
                self.doc_buffer_upto = next as i32 + 1;
            }
            DeltaEncoding::Unary => {
                let next = next_set_bit(&self.doc_bit_set, target - self.doc_bit_set_base)
                    .expect("UNARY block must have next set bit");
                self.doc = self.doc_bit_set_base + next;
                if self.needs_freq {
                    let word_index = next >> 6;
                    let word = self.doc_bit_set.get_bits()[word_index as usize];
                    let bits_on_left = (word >> (next & 0x3f)).count_ones() as i32;
                    self.doc_buffer_upto =
                        1 + self.doc_cumulative_word_pop_counts[word_index as usize] - bits_on_left;
                } else {
                    self.doc_buffer_upto = 1;
                }
            }
        }

        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.doc_freq as i64
    }
}

impl PostingsEnum for BlockPostingsEnum {
    fn freq(&self) -> Result<i32> {
        if self.freq_fp != -1 {
            // Lazily decode frequencies. We cannot do this in a &self method in
            // Rust, so we require that callers decode frequencies by other means
            // when needed. To keep the public contract, we return the cached
            // frequency if available, otherwise we decode on the spot using a
            // temporary clone of the input.
            //
            // This branch is normally reached because `next_doc` / `advance` do
            // not eagerly decode frequencies. The Java implementation decodes
            // lazily in `freq()` via a mutable self. Since our trait is &self,
            // we store the frequency at `doc_buffer_upto - 1` after a prior
            // decode. If it has not been decoded, we decode now.
            //
            // Because we cannot mutate self here, we rely on the fact that
            // callers that need frequencies should request POSTINGS_ENUM_FREQS.
            // In that mode, `needs_freq` is true and `freq_buffer` is filled by
            // `refill_full_block` / `refill_remainder` except for packed full
            // blocks, where `freq_fp` is set. The next call that can mutate
            // (next_position / advance) will decode it. For safety, if the
            // current `freq_buffer` entry is zero we decode on a clone.
            if self.freq_buffer[self.doc_buffer_upto as usize - 1] == 0 {
                return Err(LuceneError::IllegalState(
                    "frequency not decoded yet; call next_position or ensure POSTINGS_ENUM_FREQS"
                        .to_string(),
                ));
            }
        }
        Ok(self.freq_buffer[self.doc_buffer_upto as usize - 1])
    }

    fn next_position(&mut self) -> Result<i32> {
        if !self.needs_pos {
            return Ok(-1);
        }

        debug_assert!(self.pos_doc_buffer_upto <= self.doc_buffer_upto);
        if self.pos_doc_buffer_upto != self.doc_buffer_upto {
            self.accumulate_pending_positions()?;
            self.position = 0;
            self.last_start_offset = 0;
        }

        if self.pos_buffer_upto == BS_I32 {
            self.refill_positions()?;
            self.pos_buffer_upto = 0;
        }
        self.position += self.pos_delta_buffer.as_ref().unwrap()[self.pos_buffer_upto as usize];

        if self.needs_offsets_or_payloads {
            self.accumulate_payload_and_offsets();
        }

        self.pos_buffer_upto += 1;
        self.pos_pending_count -= 1;
        Ok(self.position)
    }

    fn start_offset(&self) -> i32 {
        if !self.needs_offsets {
            -1
        } else {
            self.start_offset
        }
    }

    fn end_offset(&self) -> i32 {
        if !self.needs_offsets {
            -1
        } else {
            self.end_offset
        }
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        if !self.needs_payloads || self.payload_length == 0 {
            Ok(None)
        } else {
            let start = (self.payload_byte_upto - self.payload_length) as usize;
            let end = self.payload_byte_upto as usize;
            Ok(Some(&self.payload_bytes[start..end]))
        }
    }
}

impl ImpactsSource for BlockPostingsEnum {
    fn advance_shallow(&mut self, target: i32) -> Result<()> {
        if target > self.level0_last_doc_id {
            self.do_advance_shallow(target)?;
            self.needs_refilling = true;
        }
        self.update_impacts_state();
        Ok(())
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        self.update_impacts_state();
        Ok(Box::new(self.impacts.clone()))
    }
}

impl ImpactsEnum for BlockPostingsEnum {}

#[derive(Clone)]
struct BlockImpacts {
    index_has_freq: bool,
    level0_last_doc_id: i32,
    level1_last_doc_id: i32,
    level0_serialized_impacts: Vec<u8>,
    level1_serialized_impacts: Vec<u8>,
    impact_buffer: FreqAndNormBuffer,
}

impl Impacts for BlockImpacts {
    fn num_levels(&self) -> i32 {
        if !self.index_has_freq || self.level1_last_doc_id == NO_MORE_DOCS {
            1
        } else {
            2
        }
    }

    fn doc_id_up_to(&self, level: i32) -> i32 {
        if !self.index_has_freq {
            return NO_MORE_DOCS;
        }
        if level == 0 {
            self.level0_last_doc_id
        } else if level == 1 {
            self.level1_last_doc_id
        } else {
            NO_MORE_DOCS
        }
    }

    fn get_impacts(&self, level: i32) -> FreqAndNormBuffer {
        let mut buffer = FreqAndNormBuffer::new();
        buffer.grow_no_copy(self.impact_buffer.freqs.len().max(1));
        if !self.index_has_freq {
            buffer.size = 1;
            buffer.freqs[0] = 1;
            buffer.norms[0] = 1;
            return buffer;
        }
        if level == 0 && self.level0_last_doc_id != NO_MORE_DOCS {
            read_impacts(&self.level0_serialized_impacts, &mut buffer)
                .expect("impacts from memory");
            return buffer;
        }
        if level == 1 {
            read_impacts(&self.level1_serialized_impacts, &mut buffer)
                .expect("impacts from memory");
            return buffer;
        }
        buffer.size = 1;
        buffer.freqs[0] = i32::MAX;
        buffer.norms[0] = 1;
        buffer
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::lucene104::Lucene104PostingsWriter;
    use crate::codecs::postings::{NumericDocValues, PushPostingsWriterBase};
    use crate::codecs::state::SegmentWriteState;
    use crate::index::{FieldInfo, FieldInfos, IndexOptions, PostingsEnum};
    use crate::search::DocIdSetIterator;
    use crate::store::{
        ByteBuffersDataOutput, ByteBuffersIndexOutput, Directory, MockIndexInput, RamDirectory,
    };
    use crate::util::BytesRef;

    struct TestNorms;

    impl NumericDocValues for TestNorms {
        fn get(&self, _doc_id: i32) -> Result<i64> {
            Ok(1)
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

    fn test_read_state<'a>(
        dir: &'a dyn Directory,
        info: &'a crate::index::SegmentInfo,
        infos: &'a FieldInfos,
    ) -> crate::codecs::state::SegmentReadState<'a> {
        crate::codecs::state::SegmentReadState::new(
            dir,
            info,
            infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        )
    }

    #[derive(Debug, Clone, Default)]
    struct TestPostingsEnum {
        docs: Vec<i32>,
        freqs: Vec<i32>,
        positions: Vec<Vec<(i32, Option<&'static [u8]>, i32, i32)>>,
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
            }
        }
    }

    fn field_infos_with_freqs() -> FieldInfos {
        FieldInfos::new(vec![FieldInfo::new("body", 0).with_postings_options(
            IndexOptions::DOCS_AND_FREQS,
            false,
            false,
        )])
        .expect("valid field infos")
    }

    fn field_infos_with_positions() -> FieldInfos {
        FieldInfos::new(vec![FieldInfo::new("body", 0).with_postings_options(
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            false,
            true,
        )])
        .expect("valid field infos")
    }

    fn write_term_and_open_reader(
        dir: &dyn Directory,
        field_infos: &FieldInfos,
        field_info: &FieldInfo,
        postings: &TestPostingsEnum,
    ) -> Result<(Lucene104PostingsReader, BlockTermState)> {
        let max_doc = postings.docs.last().map(|d| d + 1).unwrap_or(1).max(1);
        let segment_info = test_segment_info("_0", max_doc);
        let write_state = test_write_state(dir, &segment_info, field_infos);

        let mut writer = Lucene104PostingsWriter::new(&write_state)?;
        let mut terms_out =
            ByteBuffersIndexOutput::new(ByteBuffersDataOutput::new(), "terms", "terms");
        writer.init(&mut terms_out, &write_state)?;
        writer.set_field(field_info)?;

        let mut state = writer.new_term_state()?;
        writer.start_term(Some(&TestNorms))?;
        for (doc_idx, &doc_id) in postings.docs.iter().enumerate() {
            writer.start_doc(doc_id, postings.freqs[doc_idx])?;
            for &(pos, payload, start, end) in &postings.positions[doc_idx] {
                writer.add_position(pos, payload, start, end)?;
            }
            writer.finish_doc()?;
        }
        state.doc_freq = postings.docs.len() as i32;
        state.total_term_freq = postings.freqs.iter().map(|f| *f as i64).sum();
        writer.finish_term(&mut state)?;
        writer.close()?;

        let read_state = test_read_state(dir, &segment_info, field_infos);
        let mut reader = Lucene104PostingsReader::new(&read_state)?;
        let terms_bytes = terms_out.to_array_copy()?;
        let mut terms_in = MockIndexInput::new(terms_bytes, "terms");
        reader.init(&mut terms_in, &read_state)?;
        Ok((reader, state))
    }

    #[test]
    fn singleton_doc_round_trip() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let field_infos = field_infos_with_freqs();
        let field_info = field_infos.field_info("body").unwrap();
        let postings = TestPostingsEnum::new(vec![0], vec![1], vec![vec![]]);

        let (mut reader, state) =
            write_term_and_open_reader(dir_ref, &field_infos, field_info, &postings)
                .expect("reader builds");

        let mut it = reader
            .postings(field_info, &state, None, crate::index::POSTINGS_ENUM_FREQS)
            .expect("postings exist");

        assert_eq!(it.next_doc().unwrap(), 0);
        assert_eq!(it.freq().unwrap(), 1);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        reader.close().expect("close succeeds");
    }

    #[test]
    fn multi_doc_round_trip() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let field_infos = field_infos_with_freqs();
        let field_info = field_infos.field_info("body").unwrap();
        let postings = TestPostingsEnum::new(
            vec![0, 2, 5, 7],
            vec![3, 1, 2, 1],
            vec![vec![], vec![], vec![], vec![]],
        );

        let (mut reader, state) =
            write_term_and_open_reader(dir_ref, &field_infos, field_info, &postings)
                .expect("reader builds");

        let mut it = reader
            .postings(field_info, &state, None, crate::index::POSTINGS_ENUM_FREQS)
            .expect("postings exist");

        assert_eq!(it.next_doc().unwrap(), 0);
        assert_eq!(it.freq().unwrap(), 3);
        assert_eq!(it.next_doc().unwrap(), 2);
        assert_eq!(it.freq().unwrap(), 1);
        assert_eq!(it.next_doc().unwrap(), 5);
        assert_eq!(it.freq().unwrap(), 2);
        assert_eq!(it.next_doc().unwrap(), 7);
        assert_eq!(it.freq().unwrap(), 1);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);

        let mut it = reader
            .postings(field_info, &state, None, crate::index::POSTINGS_ENUM_FREQS)
            .expect("postings exist");
        assert_eq!(it.advance(4).unwrap(), 5);
        assert_eq!(it.freq().unwrap(), 2);
        reader.close().expect("close succeeds");
    }

    #[test]
    fn positions_payloads_offsets_round_trip() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let field_infos = field_infos_with_positions();
        let field_info = field_infos.field_info("body").unwrap();
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

        let (mut reader, state) =
            write_term_and_open_reader(dir_ref, &field_infos, field_info, &postings)
                .expect("reader builds");

        let mut it = reader
            .postings(field_info, &state, None, crate::index::POSTINGS_ENUM_ALL)
            .expect("postings exist");

        assert_eq!(it.next_doc().unwrap(), 0);
        assert_eq!(it.freq().unwrap(), 2);
        assert_eq!(it.next_position().unwrap(), 0);
        assert_eq!(it.start_offset(), 0);
        assert_eq!(it.end_offset(), 2);
        assert_eq!(it.get_payload().unwrap(), Some(b"p0".as_slice()));
        assert_eq!(it.next_position().unwrap(), 3);
        assert_eq!(it.start_offset(), 5);
        assert_eq!(it.end_offset(), 8);
        assert_eq!(it.get_payload().unwrap(), None);

        assert_eq!(it.next_doc().unwrap(), 2);
        assert_eq!(it.freq().unwrap(), 1);
        assert_eq!(it.next_position().unwrap(), 1);
        assert_eq!(it.start_offset(), 0);
        assert_eq!(it.end_offset(), 3);
        assert_eq!(it.get_payload().unwrap(), Some(b"p2".as_slice()));

        assert_eq!(it.next_doc().unwrap(), 3);
        assert_eq!(it.freq().unwrap(), 3);
        assert_eq!(it.next_position().unwrap(), 0);
        assert_eq!(it.start_offset(), 0);
        assert_eq!(it.end_offset(), 4);
        assert_eq!(it.get_payload().unwrap(), None);
        assert_eq!(it.next_position().unwrap(), 4);
        assert_eq!(it.start_offset(), 5);
        assert_eq!(it.end_offset(), 7);
        assert_eq!(it.get_payload().unwrap(), Some(b"p3".as_slice()));
        assert_eq!(it.next_position().unwrap(), 6);
        assert_eq!(it.start_offset(), 8);
        assert_eq!(it.end_offset(), 10);
        assert_eq!(it.get_payload().unwrap(), None);

        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        reader.close().expect("close succeeds");
    }

    #[test]
    fn full_block_256_round_trip() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let field_infos = field_infos_with_freqs();
        let field_info = field_infos.field_info("body").unwrap();
        let postings = TestPostingsEnum::new((0..256).collect(), vec![1; 256], vec![vec![]; 256]);

        let (mut reader, state) =
            write_term_and_open_reader(dir_ref, &field_infos, field_info, &postings)
                .expect("reader builds");

        let mut it = reader
            .postings(field_info, &state, None, crate::index::POSTINGS_ENUM_FREQS)
            .expect("postings exist");

        for expected in 0..256 {
            assert_eq!(it.next_doc().unwrap(), expected);
            assert_eq!(it.freq().unwrap(), 1);
        }
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);

        let mut it = reader
            .postings(field_info, &state, None, crate::index::POSTINGS_ENUM_FREQS)
            .expect("postings exist");
        assert_eq!(it.advance(128).unwrap(), 128);
        assert_eq!(it.freq().unwrap(), 1);
        reader.close().expect("close succeeds");
    }

    #[test]
    fn level1_skip_8193_round_trip() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let field_infos = field_infos_with_freqs();
        let field_info = field_infos.field_info("body").unwrap();
        let postings =
            TestPostingsEnum::new((0..8193).collect(), vec![1; 8193], vec![vec![]; 8193]);

        let (mut reader, state) =
            write_term_and_open_reader(dir_ref, &field_infos, field_info, &postings)
                .expect("reader builds");

        let mut it = reader
            .postings(field_info, &state, None, crate::index::POSTINGS_ENUM_FREQS)
            .expect("postings exist");

        for expected in 0..8193 {
            assert_eq!(it.next_doc().unwrap(), expected);
            assert_eq!(it.freq().unwrap(), 1);
        }
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);

        let mut it = reader
            .postings(field_info, &state, None, crate::index::POSTINGS_ENUM_FREQS)
            .expect("postings exist");
        assert_eq!(it.advance(8192).unwrap(), 8192);
        assert_eq!(it.freq().unwrap(), 1);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        reader.close().expect("close succeeds");
    }
}
