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
    check_footer, check_footer_with_prior, check_index_header, checksum_entire_file,
    retrieve_checksum_expected_length,
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
use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::{FieldInfo, IndexOptions};
use crate::index::{FreqAndNormBuffer, Impacts, ImpactsSource};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{close_after, DataInput, IndexInput, SharedIndexInput};
use crate::util::{BitUtil, FixedBitSet, IOUtils};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use crate::index::index_file_names::segment_file_name;

/// Number of documents in a packed postings block.
const BS: usize = FOR_BLOCK_SIZE;
/// `BLOCK_SIZE` as `i32`.
const BS_I32: i32 = BS as i32;

/// Source of the number each [`Lucene104PostingsReader`] takes to stand for
/// the identity of its `docIn`; see [`Lucene104PostingsReader::doc_in_identity`].
static NEXT_DOC_IN_IDENTITY: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// VInt15 / VLong15 helpers (mirrors Lucene104PostingsReader.readVInt15 /
// readVLong15).
// ---------------------------------------------------------------------------

/// `Lucene104PostingsReader.readVInt15`; package-private in Java, `pub` for
/// the core tests.
pub fn read_v_int15(in_: &mut dyn DataInput) -> Result<i32> {
    let s = in_.read_short()? as i32;
    if s >= 0 {
        Ok(s)
    } else {
        Ok((s & 0x7FFF) | (in_.read_v_int()? << 15))
    }
}

/// `Lucene104PostingsReader.readVLong15`; package-private in Java, `pub` for
/// the core tests.
pub fn read_v_long15(in_: &mut dyn DataInput) -> Result<i64> {
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

/// Decodes the impacts `input` holds into `reuse`, from its first entry.
///
/// Equivalent to `Lucene104PostingsReader.readImpacts(ByteArrayDataInput,
/// FreqAndNormBuffer)` (`Lucene104PostingsReader.java:1449-1472`),
/// package-private in Java and `pub` for the core tests: each entry is stored
/// at its index of `reuse`'s arrays, which the caller grew beforehand (an
/// index past them panics, as Java's store throws
/// `ArrayIndexOutOfBoundsException`), and `reuse.size` is set at the end.
/// Java returns `reuse`; the caller keeps its own borrow of it here.
///
/// # Errors
///
/// What reading `input` raises; Java wraps the `IOException` of `readZLong`
/// in a `RuntimeException` ("cannot happen on a BADI").
pub fn read_impacts(
    input: &mut crate::store::ByteArrayDataInput<'_>,
    reuse: &mut FreqAndNormBuffer,
) -> Result<()> {
    let mut freq = 0i32;
    let mut norm = 0i64;
    let mut size = 0usize;
    // Java's own bound, `while (in.getPosition() < in.length())`
    // (`Lucene104PostingsReader.java:1453`), which stops even if a read has
    // carried the position past the limit.
    while input.position() < input.length() {
        let freq_delta = input.read_v_int()?;
        if (freq_delta & 0x01) != 0 {
            freq += 1 + ((freq_delta as u32) >> 1) as i32;
            norm += 1 + input.read_z_long()?;
        } else {
            freq += 1 + ((freq_delta as u32) >> 1) as i32;
            norm += 1;
        }
        reuse.freqs[size] = freq;
        reuse.norms[size] = norm;
        size += 1;
    }
    reuse.size = size;
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
    let mut word = words[word_index].load(std::sync::atomic::Ordering::Relaxed) & (!0u64 << shift);
    while word == 0 {
        word_index += 1;
        if word_index >= num_words {
            return None;
        }
        word = words[word_index].load(std::sync::atomic::Ordering::Relaxed);
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
    /// `docIn`, `posIn` and `payIn`: shared by every enum this reader hands
    /// out (as clones), and closed by [`close`](PostingsReaderBase::close)
    /// whoever else still holds the reader, as Java's are.
    doc_in: Option<SharedIndexInput>,
    /// Stands for the identity of `doc_in`, which
    /// `BlockPostingsEnum.canReuse` compares by reference
    /// (`docIn == Lucene104PostingsReader.this.docIn`,
    /// `Lucene104PostingsReader.java:493-500`). A Rust value has no address it
    /// keeps across moves, and every reader opens its own `docIn`, so each
    /// reader takes a number no other reader of the process takes, and the two
    /// are compared on that instead; the comparison answers exactly what the
    /// reference comparison answers.
    doc_in_identity: u64,
    pos_in: Option<SharedIndexInput>,
    pay_in: Option<SharedIndexInput>,
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

        // The `try` block, which ends with `CodecUtil.checkFooter(metaIn,
        // null)`; its `catch (Throwable t)` hands any failure — that footer
        // check's too — to `CodecUtil.checkFooter(metaIn, t)`, all inside the
        // try-with-resources (`Lucene104PostingsReader.java:84-118`).
        let read = (|| -> Result<_> {
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
            Ok((
                version,
                max_num_impacts_at_level0,
                max_impact_num_bytes_at_level0,
                max_num_impacts_at_level1,
                max_impact_num_bytes_at_level1,
                expected_doc_file_length,
                expected_pos_file_length,
                expected_pay_file_length,
            ))
        })()
        .map_err(|t| check_footer_with_prior(meta_in.as_mut(), t));
        let (
            version,
            max_num_impacts_at_level0,
            max_impact_num_bytes_at_level0,
            max_num_impacts_at_level1,
            max_impact_num_bytes_at_level1,
            expected_doc_file_length,
            expected_pos_file_length,
            expected_pay_file_length,
        ) = close_after(meta_in.as_mut(), read)?;

        let doc_name = segment_file_name(&segment_info.name, suffix, "doc");
        let pos_name = segment_file_name(&segment_info.name, suffix, "pos");
        let pay_name = segment_file_name(&segment_info.name, suffix, "pay");

        let mut doc_in: Option<Box<dyn IndexInput>> = None;
        let mut pos_in: Option<Box<dyn IndexInput>> = None;
        let mut pay_in: Option<Box<dyn IndexInput>> = None;

        // Each input is held as soon as it is opened, as Java assigns `docIn`,
        // `posIn` and `payIn` before checking their headers.
        let result: Result<()> = (|| {
            let d = doc_in.insert(
                dir.open_input(
                    &doc_name,
                    /* withHints, `Lucene104PostingsReader.java:137` */
                    state
                        .context
                        .with_hints(&[
                            std::sync::Arc::new(crate::store::FileTypeHint::Data),
                            std::sync::Arc::new(crate::store::FileDataHint::Postings),
                        ])
                        .as_ref(),
                )?,
            );
            check_index_header(d.as_mut(), DOC_CODEC, version, version, &id, suffix)?;
            retrieve_checksum_expected_length(d.as_mut(), expected_doc_file_length)?;

            if state.field_infos.has_prox() {
                let p = pos_in.insert(
                    dir.open_input(
                        &pos_name,
                        /* withHints, `Lucene104PostingsReader.java:146` */
                        state
                            .context
                            .with_hints(&[std::sync::Arc::new(crate::store::FileTypeHint::Data)])
                            .as_ref(),
                    )?,
                );
                check_index_header(p.as_mut(), POS_CODEC, version, version, &id, suffix)?;
                retrieve_checksum_expected_length(p.as_mut(), expected_pos_file_length)?;

                if state.field_infos.has_payloads() || state.field_infos.has_offsets() {
                    let y = pay_in.insert(
                        dir.open_input(
                            &pay_name,
                            /* withHints, `Lucene104PostingsReader.java:157` */
                            state
                                .context
                                .with_hints(&[std::sync::Arc::new(
                                    crate::store::FileTypeHint::Data,
                                )])
                                .as_ref(),
                        )?,
                    );
                    check_index_header(y.as_mut(), PAY_CODEC, version, version, &id, suffix)?;
                    retrieve_checksum_expected_length(y.as_mut(), expected_pay_file_length)?;
                }
            }
            Ok(())
        })();

        if let Err(error) = result {
            // `IOUtils.closeWhileHandlingException(docIn, posIn, payIn)`
            // (`Lucene104PostingsReader.java:166-172`).
            IOUtils::close_while_handling_exception(
                [doc_in, pos_in, pay_in].map(|i| i.map(|mut i| move || i.close())),
            )?;
            return Err(error);
        }

        Ok(Self {
            version,
            doc_in: doc_in.map(SharedIndexInput::new),
            doc_in_identity: NEXT_DOC_IN_IDENTITY.fetch_add(1, AtomicOrdering::Relaxed),
            pos_in: pos_in.map(SharedIndexInput::new),
            pay_in: pay_in.map(SharedIndexInput::new),
            max_num_impacts_at_level0,
            max_impact_num_bytes_at_level0,
            max_num_impacts_at_level1,
            max_impact_num_bytes_at_level1,
        })
    }

    /// Equivalent to `new BlockPostingsEnum(fieldInfo, flags, needsImpacts)`,
    /// the inner class's constructor, which clones the inputs of this reader.
    fn new_block_postings_enum(
        &self,
        field_info: &FieldInfo,
        flags: i32,
        needs_impacts: bool,
    ) -> Result<BlockPostingsEnum> {
        let doc_in = self.doc_in.as_ref().map(SharedIndexInput::read);
        let pos_in = self.pos_in.as_ref().map(SharedIndexInput::read);
        let pay_in = self.pay_in.as_ref().map(SharedIndexInput::read);
        BlockPostingsEnum::new(
            field_info,
            flags,
            needs_impacts,
            doc_in.as_deref().map(|i| &**i),
            self.doc_in_identity,
            pos_in.as_deref().map(|i| &**i),
            pay_in.as_deref().map(|i| &**i),
            self.max_num_impacts_at_level0,
            self.max_impact_num_bytes_at_level0,
            self.max_num_impacts_at_level1,
            self.max_impact_num_bytes_at_level1,
        )
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
            return Err(LuceneError::from(LuceneErrorKind::CorruptIndex(format!(
                "index-time BLOCK_SIZE ({}) != read-time BLOCK_SIZE ({BS_I32})",
                index_block_size
            ))));
        }
        Ok(())
    }

    fn new_term_state(&self) -> Result<BlockTermState> {
        Ok(BlockTermState::default())
    }

    fn decode_term(
        &self,
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
            // `l >>> 1` (`Lucene104PostingsReader.java:227`).
            state.doc_start_fp += ((l as u64) >> 1) as i64;
            if state.doc_freq == 1 {
                state.singleton_doc_id = input.read_v_int()?;
            } else {
                state.singleton_doc_id = -1;
            }
        } else {
            debug_assert!(!absolute);
            debug_assert!(state.singleton_doc_id != -1);
            // `zigZagDecode(l >>> 1)` (`Lucene104PostingsReader.java:236`).
            // `int += long` narrows the `long` sum, so it wraps.
            state.singleton_doc_id = (i64::from(state.singleton_doc_id)
                + BitUtil::zig_zag_decode_long(((l as u64) >> 1) as i64))
                as i32;
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

    /// Equivalent to `Lucene104PostingsReader.postings(FieldInfo, BlockTermState, PostingsEnum, int)`
    /// (`Lucene104PostingsReader.java:256-264`): `reuse` is reset in place and
    /// handed back exactly when it is a [`BlockPostingsEnum`]
    /// ([`PostingsEnum::as_any_mut`] standing for `instanceof`) that
    /// `BlockPostingsEnum::can_reuse` accepts for this reader's `docIn`, the
    /// field and `flags`; otherwise a new enum is built.
    fn postings(
        &self,
        field_info: &FieldInfo,
        state: &BlockTermState,
        mut reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        // `reuse instanceof BlockPostingsEnum everythingEnum
        //     && everythingEnum.canReuse(docIn, fieldInfo, flags, false)`
        let can_reuse = reuse
            .as_mut()
            .and_then(|reuse| reuse.as_any_mut())
            .and_then(|any| any.downcast_mut::<BlockPostingsEnum>())
            .is_some_and(|everything_enum| {
                everything_enum.can_reuse(self.doc_in_identity, field_info, flags, false)
            });
        let mut postings: Box<dyn PostingsEnum> = match reuse {
            Some(everything_enum) if can_reuse => everything_enum,
            _ => Box::new(self.new_block_postings_enum(field_info, flags, false)?),
        };
        postings
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<BlockPostingsEnum>())
            .ok_or_else(|| {
                LuceneError::illegal_state(
                    "INVARIANT: the enum was just checked or built as a BlockPostingsEnum"
                        .to_string(),
                )
            })?
            .reset(state)?;
        Ok(postings)
    }

    fn impacts(
        &self,
        field_info: &FieldInfo,
        state: &BlockTermState,
        flags: i32,
    ) -> Result<Box<dyn ImpactsEnum>> {
        let mut enum_ = self.new_block_postings_enum(field_info, flags, true)?;
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

    /// Equivalent to `Lucene104PostingsReader.close()`
    /// (`Lucene104PostingsReader.java:209-212`): `IOUtils.close(docIn, posIn,
    /// payIn)`.
    fn close(&self) -> Result<()> {
        IOUtils::close(
            [&self.doc_in, &self.pos_in, &self.pay_in]
                .map(|i| i.as_ref().map(|i| move || i.close())),
        )
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

    /// The frequencies of the current block. A full block's are decoded on
    /// the first [`freq`](PostingsEnum::freq), which takes `&self`, hence the
    /// cell; everything else reaches them through `get_mut`.
    freq_buffer: RefCell<[i32; BS]>,
    pos_delta_buffer: Option<[i32; BS]>,
    payload_length_buffer: Option<[i32; BS]>,
    offset_start_delta_buffer: Option<[i32; BS]>,
    offset_length_buffer: Option<[i32; BS]>,

    payload_bytes: Vec<u8>,
    payload_byte_upto: i32,
    payload_length: i32,
    /// Where the current payload starts in `payload_bytes`.
    ///
    /// Equivalent to `payload.offset` (`Lucene104PostingsReader.java:1278-1285`):
    /// Java keeps the payload as a `BytesRef` view that
    /// `accumulatePayloadAndOffsets` re-points at each position and nothing
    /// else touches, so a reused enum still answers `getPayload()` with the
    /// previous term's payload until its first position is read, although
    /// `reset` rewinds `payloadByteUpto` to zero.
    payload_offset: i32,

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
    /// Java's `final IndexOptions options`, which
    /// [`can_reuse`](Self::can_reuse) compares.
    options: IndexOptions,
    /// Java's `final int flags`, which [`can_reuse`](Self::can_reuse)
    /// compares.
    flags: i32,
    /// The identity of the `docIn` of the reader that built this enum: Java
    /// reaches it as `Lucene104PostingsReader.this.docIn`, through the inner
    /// class's enclosing instance. See
    /// [`Lucene104PostingsReader::doc_in_identity`].
    parent_doc_in_identity: u64,

    needs_freq: bool,
    needs_pos: bool,
    needs_offsets: bool,
    needs_payloads: bool,
    needs_offsets_or_payloads: bool,
    needs_impacts: bool,
    /// `needsDocsAndFreqsOnly = needsPos == false && needsImpacts == false`
    /// (`Lucene104PostingsReader.java:428`).
    needs_docs_and_freqs_only: bool,

    /// Where the current full block's frequencies start, or `-1` once they
    /// are decoded. Equivalent to `BlockPostingsEnum.freqFP`.
    freq_fp: Cell<i64>,
    /// The input the frequencies are decoded from on the first `freq()`; see
    /// [`freq`](PostingsEnum::freq).
    freq_in: RefCell<Option<Box<dyn IndexInput>>>,
    position: i32,
    pos_doc_buffer_upto: i32,
    pos_pending_count: i32,
    last_pos_block_fp: i64,

    level0_pos_end_fp: i64,
    level0_block_pos_upto: i32,
    level0_pay_end_fp: i64,
    level0_block_pay_upto: i32,
    level0_serialized_impacts: Vec<u8>,
    /// `level0SerializedImpacts.length`: how many leading bytes of
    /// [`level0_serialized_impacts`](Self::level0_serialized_impacts) the
    /// last read filled (`Lucene104PostingsReader.java:751, 850`).
    level0_serialized_impacts_length: usize,

    level1_pos_end_fp: i64,
    level1_block_pos_upto: i32,
    level1_pay_end_fp: i64,
    level1_block_pay_upto: i32,
    level1_serialized_impacts: Vec<u8>,
    /// `level1SerializedImpacts.length` (`Lucene104PostingsReader.java:702`).
    level1_serialized_impacts_length: usize,

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
        parent_doc_in_identity: u64,
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
            level0_serialized_impacts_length: 0,
            level1_serialized_impacts: level1_serialized_impacts.clone(),
            level1_serialized_impacts_length: 0,
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
            freq_buffer: RefCell::new(freq_buffer),
            pos_delta_buffer,
            payload_length_buffer,
            offset_start_delta_buffer,
            offset_length_buffer,
            payload_bytes,
            payload_byte_upto: 0,
            payload_length: 0,
            payload_offset: 0,
            last_start_offset: 0,
            // Java leaves both at the `int` default of zero when the enum
            // reads offsets, and sets them to -1 only when it does not
            // (`Lucene104PostingsReader.java:472-480`), where `startOffset()`
            // and `endOffset()` answer -1 regardless.
            start_offset: if needs_offsets { 0 } else { -1 },
            end_offset: if needs_offsets { 0 } else { -1 },
            pos_buffer_upto: BS_I32,
            pos_in,
            pay_in,
            index_has_freq,
            index_has_pos,
            index_has_offsets,
            index_has_payloads,
            index_has_offsets_or_payloads,
            options: index_options,
            flags,
            parent_doc_in_identity,
            needs_freq,
            needs_pos,
            needs_offsets,
            needs_payloads,
            needs_offsets_or_payloads,
            needs_impacts,
            needs_docs_and_freqs_only: !needs_pos && !needs_impacts,
            freq_fp: Cell::new(-1),
            freq_in: RefCell::new(None),
            position: 0,
            pos_doc_buffer_upto: BS_I32,
            pos_pending_count: 0,
            last_pos_block_fp: 0,
            level0_pos_end_fp: 0,
            level0_block_pos_upto: 0,
            level0_pay_end_fp: 0,
            level0_block_pay_upto: 0,
            level0_serialized_impacts,
            // `new BytesRef(capacity)` has length 0.
            level0_serialized_impacts_length: 0,
            level1_pos_end_fp: 0,
            level1_block_pos_upto: 0,
            level1_pay_end_fp: 0,
            level1_block_pay_upto: 0,
            level1_serialized_impacts,
            level1_serialized_impacts_length: 0,
            impacts,
            needs_refilling: false,
        })
    }

    /// Equivalent to `BlockPostingsEnum.canReuse(IndexInput, FieldInfo, int, boolean)`
    /// (`Lucene104PostingsReader.java:493-500`): this enum serves the next term
    /// exactly when the reader asking built it — its `docIn` is the one this
    /// enum cloned — for the same index options, payloads, flags and need for
    /// impacts.
    fn can_reuse(
        &self,
        doc_in_identity: u64,
        field_info: &FieldInfo,
        flags: i32,
        needs_impacts: bool,
    ) -> bool {
        doc_in_identity == self.parent_doc_in_identity
            && self.options == field_info.index_options
            && self.index_has_payloads == field_info.has_payloads()
            && self.flags == flags
            && self.needs_impacts == needs_impacts
    }

    /// Equivalent to `BlockPostingsEnum.reset(IntBlockTermState, int)`
    /// (`Lucene104PostingsReader.java:502-575`), which resets exactly the
    /// fields below and leaves every other one — the position, offsets and
    /// payload length of the last position read, `needsRefilling`,
    /// `level0DocEndFP`, the serialized impacts — as the previous term left
    /// it: a reused enum answers `startOffset()` before `nextDoc()` with the
    /// previous term's offset (observed with the 10.5.0 oracle).
    fn reset(&mut self, term_state: &BlockTermState) -> Result<()> {
        self.doc_freq = term_state.doc_freq;
        self.singleton_doc_id = term_state.singleton_doc_id;

        self.total_term_freq = if self.index_has_freq {
            term_state.total_term_freq
        } else {
            term_state.doc_freq as i64
        };

        if self.doc_freq > 1 {
            if self.doc_in.is_none() {
                // doc_in is lazily initialized by the parent reader in postings().
                return Err(LuceneError::illegal_state(
                    "BlockPostingsEnum.doc_in must be initialized by the reader".to_string(),
                ));
            }
            let doc_in = self
                .doc_in
                .as_ref()
                .expect("INVARIANT: doc_in was just checked to be Some");
            Self::prefetch_postings(doc_in.as_ref(), term_state)?;
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
        self.pos_pending_count = 0;
        self.payload_byte_upto = 0;
        // Java tests the term state's own `totalTermFreq` here, not the field
        // it just derived (`Lucene104PostingsReader.java:539-545`).
        if term_state.total_term_freq < BS_I32 as i64 {
            self.last_pos_block_fp = pos_term_start_fp;
        } else if term_state.total_term_freq == BS_I32 as i64 {
            self.last_pos_block_fp = -1;
        } else {
            self.last_pos_block_fp = pos_term_start_fp + term_state.last_pos_block_offset;
        }

        self.level1_block_pos_upto = 0;
        self.level1_block_pay_upto = 0;
        self.level0_block_pos_upto = 0;
        self.level0_block_pay_upto = 0;
        self.pos_buffer_upto = BS_I32;

        self.doc = -1;
        self.prev_doc_id = -1;
        self.doc_count_left = self.doc_freq;
        self.freq_fp.set(-1);
        self.level0_last_doc_id = -1;
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
        self.level1_doc_count_upto = 0;
        self.doc_buffer_size = BS_I32;
        self.doc_buffer_upto = BS_I32;
        self.pos_doc_buffer_upto = BS_I32;

        Ok(())
    }

    /// Prefetches the start of this term's postings in the `.doc` file.
    ///
    /// Nothing is prefetched when the input already sits at the right offset:
    /// that suggests the caller is streaming the whole inverted index (for
    /// example while merging), and the read-ahead logic does a better job.
    /// Note that this heuristic does not work for terms that have skip data,
    /// since skip data is stored after the last term, but handling every term
    /// with fewer than 256 docs is already a good start. Positions and offsets
    /// are deliberately not prefetched: they are less likely to be needed.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104PostingsReader#prefetchPostings`.
    fn prefetch_postings(doc_in: &dyn IndexInput, state: &BlockTermState) -> Result<()> {
        // Singletons are inlined in the terms dict, nothing to prefetch.
        debug_assert!(state.doc_freq > 1);
        if doc_in.file_pointer() != state.doc_start_fp {
            doc_in.prefetch(state.doc_start_fp, 1)?;
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
            .ok_or_else(|| LuceneError::illegal_state("doc_in is not open".to_string()))
    }

    fn pos_in_ptr(&mut self) -> Result<*mut dyn IndexInput> {
        self.pos_in
            .as_mut()
            .map(|b| b.as_mut() as *mut dyn IndexInput)
            .ok_or_else(|| LuceneError::illegal_state("pos_in is not open".to_string()))
    }

    fn pay_in_ptr(&mut self) -> Result<*mut dyn IndexInput> {
        self.pay_in
            .as_mut()
            .map(|b| b.as_mut() as *mut dyn IndexInput)
            .ok_or_else(|| LuceneError::illegal_state("pay_in is not open".to_string()))
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
        // Java's `readByte()` is signed (`Lucene104PostingsReader.java:594`): a
        // negative value is the number of longs of a bit-set block.
        let bits_per_value = i32::from(doc_in.read_byte()? as i8);
        if bits_per_value > 0 {
            let mut pdu = PostingDecodingUtil::new(doc_in);
            self.for_util
                .decode(bits_per_value, &mut pdu, &mut self.doc_buffer)?;
            prefix_sum(&mut self.doc_buffer, BS, self.prev_doc_id);
            self.encoding = DeltaEncoding::Packed;
        } else {
            // block is encoded as a bit set
            debug_assert!(self.level0_last_doc_id != NO_MORE_DOCS);
            self.doc_bit_set_base = self.prev_doc_id + 1;
            self.doc_bit_set.clear_all();
            if bits_per_value == 0 {
                // A full 256-doc block: no bitset longs are written.
                for i in 0..BS {
                    self.doc_bit_set.set(i);
                }
                if self.needs_freq {
                    // The cumulative pop counts of the words of the bit set,
                    // written over `docBuffer` as Java's
                    // `docCumulativeWordPopCounts = docBuffer` does
                    // (`Lucene104PostingsReader.java:312, 614-622`).
                    for i in 0..BS / 64 {
                        self.doc_buffer[i] = ((i + 1) * 64) as i32;
                    }
                }
            } else {
                let num_longs = (-bits_per_value) as usize;
                let mut longs = vec![0i64; num_longs];
                doc_in.read_longs(Some(&mut longs), 0, num_longs)?;
                for (i, &l) in longs.iter().enumerate() {
                    let word = l as u64;
                    for b in 0..64 {
                        if (word >> b) & 1 != 0 {
                            self.doc_bit_set.set(i * 64 + b);
                        }
                    }
                }
                if self.needs_freq {
                    // The cumulative pop counts, over `docBuffer` as in Java.
                    for (i, &l) in longs.iter().take(num_longs - 1).enumerate() {
                        self.doc_buffer[i] = (l as u64).count_ones() as i32;
                    }
                    prefix_sum(&mut self.doc_buffer, num_longs - 1, 0);
                    self.doc_buffer[num_longs - 1] = BS_I32;
                    debug_assert!(
                        self.doc_buffer[num_longs - 2]
                            + (longs[num_longs - 1] as u64).count_ones() as i32
                            == BS_I32
                    );
                }
            }
            self.encoding = DeltaEncoding::Unary;
        }

        if self.index_has_freq {
            if self.needs_freq {
                // Decoded lazily, on the first `freq()` of the block
                // (`Lucene104PostingsReader.java:628-633`).
                self.freq_fp.set(doc_in.file_pointer());
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
        debug_assert!(self.doc_count_left >= 0 && self.doc_count_left < BS_I32);
        if self.doc_freq == 1 {
            self.doc_buffer[0] = self.singleton_doc_id;
            self.freq_buffer.get_mut()[0] = self.total_term_freq as i32;
            self.doc_buffer[1] = NO_MORE_DOCS;
            self.doc_count_left = 0;
            self.doc_buffer_size = 1;
            debug_assert_eq!(self.freq_fp.get(), -1);
        } else {
            let doc_in = unsafe { Self::input_ref(self.doc_in_ptr()?) };
            let count = self.doc_count_left as usize;
            read_v_int_block(
                doc_in,
                &mut self.doc_buffer,
                self.freq_buffer.get_mut(),
                count,
                self.index_has_freq,
                self.needs_freq,
            )?;
            prefix_sum(&mut self.doc_buffer, count, self.prev_doc_id);
            self.doc_buffer[count] = NO_MORE_DOCS;
            self.freq_fp.set(-1);
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
        debug_assert!(self.doc_count_left >= 0);
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
                    doc_in.read_bytes(
                        Some(&mut self.level1_serialized_impacts),
                        0,
                        num_impact_bytes,
                    )?;
                    self.level1_serialized_impacts_length = num_impact_bytes;
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
                    debug_assert!(
                        self.level0_pay_end_fp
                            >= unsafe { Self::input_ref(self.pay_in_ptr()?) }.file_pointer()
                    );
                    unsafe { Self::input_ref(self.pay_in_ptr()?) }.seek(self.level0_pay_end_fp)?;
                    self.payload_byte_upto = self.level0_block_pay_upto;
                }
                self.pos_buffer_upto = BS_I32;
            } else {
                debug_assert_eq!(self.freq_fp.get(), -1);
                self.pos_pending_count += sum_over_range(
                    self.freq_buffer.get_mut(),
                    self.pos_doc_buffer_upto as usize,
                    BS,
                );
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
                    doc_in.read_bytes(
                        Some(&mut self.level0_serialized_impacts),
                        0,
                        num_impact_bytes,
                    )?;
                    self.level0_serialized_impacts_length = num_impact_bytes;
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

        if self.needs_docs_and_freqs_only && self.doc_count_left >= BS_I32 {
            // Optimize the common path for exhaustive evaluation
            // (`Lucene104PostingsReader.java:780-786`): skip the rest of the
            // level-0 skip data in one seek.
            let doc_delta = {
                let doc_in = self
                    .doc_in
                    .as_mut()
                    .ok_or_else(|| LuceneError::illegal_state("doc_in is not open".to_string()))?;
                let level0_num_bytes = doc_in.read_v_long()?;
                let level0_end = doc_in.file_pointer() + level0_num_bytes;
                let doc_delta = read_v_int15(doc_in.as_mut())?;
                doc_in.seek(level0_end)?;
                doc_delta
            };
            self.level0_last_doc_id += doc_delta;
            self.refill_full_block()
        } else {
            self.do_move_to_next_level0_block()
        }
    }

    /// Reads the position and payload file pointers of the next level-0 block.
    ///
    /// Equivalent to `readLevel0PosData()` (`Lucene104PostingsReader.java:792-799`).
    /// The skip data is interleaved with the doc blocks, so every field is
    /// read from the **doc** file; the positions and payloads files are only
    /// seeked to afterwards, by [`seek_pos_data`](Self::seek_pos_data).
    fn read_level0_pos_data(&mut self) -> Result<()> {
        let index_has_offsets_or_payloads = self.index_has_offsets_or_payloads;
        let doc_in = self
            .doc_in
            .as_mut()
            .ok_or_else(|| LuceneError::illegal_state("doc_in is not open".to_string()))?;
        let pos_delta = doc_in.read_v_long()?;
        let block_pos_upto = i32::from(doc_in.read_byte()?);
        let pay = if index_has_offsets_or_payloads {
            Some((doc_in.read_v_long()?, doc_in.read_v_int()?))
        } else {
            None
        };
        self.level0_pos_end_fp += pos_delta;
        self.level0_block_pos_upto = block_pos_upto;
        if let Some((pay_delta, block_pay_upto)) = pay {
            self.level0_pay_end_fp += pay_delta;
            self.level0_block_pay_upto = block_pay_upto;
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
                // needs payloads or offsets
                debug_assert!(
                    self.level0_pay_end_fp
                        >= unsafe { Self::input_ref(self.pay_in_ptr()?) }.file_pointer()
                );
                unsafe { Self::input_ref(self.pay_in_ptr()?) }.seek(pay_fp)?;
                self.payload_byte_upto = pay_upto;
            }
            self.pos_buffer_upto = BS_I32;
        } else {
            self.pos_pending_count += sum_over_range(
                self.freq_buffer.get_mut(),
                self.pos_doc_buffer_upto as usize,
                BS,
            );
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
                                Some(&mut self.level0_serialized_impacts),
                                0,
                                num_impact_bytes,
                            )?;
                            self.level0_serialized_impacts_length = num_impact_bytes;
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
            // `if (payIn != null)` (`Lucene104PostingsReader.java:1153`): the
            // payloads file is only open when offsets or payloads are
            // requested, which a field that indexes them need not be.
            let mut pay_in = if self.pay_in.is_some() {
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
                    // `code >>> 1` (`Lucene104PostingsReader.java:1191`).
                    self.pos_delta_buffer.as_mut().unwrap()[i] = ((code as u32) >> 1) as i32;
                    if payload_length != 0 {
                        let needed = self.payload_byte_upto as usize + payload_length as usize;
                        if needed > self.payload_bytes.len() {
                            self.payload_bytes
                                .resize(needed.max(self.payload_bytes.len() * 2).max(128), 0);
                        }
                        pos_in.read_bytes(
                            Some(&mut self.payload_bytes),
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
                    // `deltaCode >>> 1` (`Lucene104PostingsReader.java:1212`): a
                    // delta past 2^30 was written as a negative `deltaCode`.
                    offset_start_delta_buffer[i] = ((delta_code as u32) >> 1) as i32;
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
                pay_in.read_bytes(Some(&mut self.payload_bytes), 0, num_bytes)?;
            } else if self.pay_in.is_some() {
                // Needs offsets: skip the payload lengths and bytes
                // (`Lucene104PostingsReader.java:1230-1235`).
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
            } else if self.pay_in.is_some() {
                // Needs payloads: skip the offset starts and lengths
                // (`Lucene104PostingsReader.java:1244-1248`).
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
        // Trigger the lazy decoding of the frequencies.
        let freq = self.freq()?;
        self.pos_pending_count += sum_over_range(
            self.freq_buffer.get_mut(),
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
            self.payload_offset = self.payload_byte_upto;
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
        self.impacts.level0_serialized_impacts_length = self.level0_serialized_impacts_length;
        self.impacts
            .level1_serialized_impacts
            .clone_from(&self.level1_serialized_impacts);
        self.impacts.level1_serialized_impacts_length = self.level1_serialized_impacts_length;
    }
}

impl BlockPostingsEnum {
    /// Equivalent to `BlockPostingsEnum.computeBufferEndBoundary(int)`
    /// (`Lucene104PostingsReader.java:1082-1090`): the index of the first
    /// buffered doc at or above `up_to`.
    fn compute_buffer_end_boundary(&self, up_to: i32) -> usize {
        let size = self.doc_buffer_size as usize;
        if size != 0 && self.doc_buffer[size - 1] < up_to {
            // All docs in the buffer are under upTo
            size
        } else {
            // Find the index of the first doc that is greater than or equal to upTo
            find_next_geq(&self.doc_buffer, up_to, self.doc_buffer_upto as usize, size)
        }
    }
}

impl DocIdSetIterator for BlockPostingsEnum {
    /// The class identity `ScorerUtil.likelyImpactsEnum` compares with
    /// `Lucene104PostingsFormat.getImpactsEnumImpl()` (`ScorerUtil.java:70`).
    fn disi_as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

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
                // Java only asserts that the bit set has a next document
                // (`Lucene104PostingsReader.java:916`); without assertions it
                // adds `NO_MORE_DOCS` to the base, as here.
                let next = next_set_bit(&self.doc_bit_set, self.doc - self.doc_bit_set_base + 1);
                debug_assert!(next.is_some());
                let next = next.unwrap_or(NO_MORE_DOCS);
                self.doc = self.doc_bit_set_base.wrapping_add(next);
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
                // Java only asserts this (`Lucene104PostingsReader.java:946`).
                let next = next_set_bit(&self.doc_bit_set, target - self.doc_bit_set_base);
                debug_assert!(next.is_some());
                let next = next.unwrap_or(NO_MORE_DOCS);
                self.doc = self.doc_bit_set_base.wrapping_add(next);
                if self.needs_freq {
                    let word_index = next >> 6;
                    let word = self.doc_bit_set.get_bits()[word_index as usize]
                        .load(std::sync::atomic::Ordering::Relaxed);
                    let bits_on_left = (word >> (next & 0x3f)).count_ones() as i32;
                    // Take the cumulative pop count for the given word, held in
                    // `docBuffer`, and subtract the bits on the left of the
                    // current doc.
                    self.doc_buffer_upto = 1 + self.doc_buffer[word_index as usize] - bits_on_left;
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

    /// Equivalent to `BlockPostingsEnum.intoBitSet(int, FixedBitSet, int)`
    /// (`Lucene104PostingsReader.java:970-1033`): the current doc, then whole
    /// blocks at a time, the docs of a packed block through the buffer, those
    /// of a unary block by `FixedBitSet.orRange`; the enum is left on the
    /// first doc at or above `up_to`.
    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        if self.doc >= up_to {
            return Ok(());
        }

        // Handle the current doc separately, it may be on the previous docBuffer.
        bit_set.set((self.doc - offset) as usize);

        loop {
            if self.doc == self.level0_last_doc_id {
                // refill
                self.move_to_next_level0_block()?;
            }

            match self.encoding {
                DeltaEncoding::Packed => {
                    let start = self.doc_buffer_upto as usize;
                    let end = self.compute_buffer_end_boundary(up_to);
                    if end != 0 {
                        self.buffer_into_bit_set(start, end, bit_set, offset);
                        self.doc = self.doc_buffer[end - 1];
                    }
                    self.doc_buffer_upto = end as i32;
                    if end != BS {
                        // Either the block is a tail block, or the block did not fully match,
                        // we're done.
                        self.next_doc()?;
                        debug_assert!(self.doc >= up_to);
                        return Ok(());
                    }
                }
                DeltaEncoding::Unary => {
                    let source_from = if self.doc_buffer_upto == 0 {
                        // start from beginning
                        0
                    } else {
                        // start after the current doc
                        self.doc - self.doc_bit_set_base + 1
                    };

                    let dest_from = self.doc_bit_set_base - offset + source_from;

                    debug_assert!(self.level0_last_doc_id != NO_MORE_DOCS);
                    let source_to = up_to.min(self.level0_last_doc_id + 1) - self.doc_bit_set_base;

                    if source_to > source_from {
                        FixedBitSet::or_range(
                            &self.doc_bit_set,
                            source_from as usize,
                            bit_set,
                            dest_from as usize,
                            (source_to - source_from) as usize,
                        );
                    }
                    if self.doc_bit_set_base + source_to <= self.level0_last_doc_id {
                        // We stopped before the end of the current bit set, which means that
                        // we're done. Set the current doc before returning.
                        self.advance(self.doc_bit_set_base + source_to)?;
                        return Ok(());
                    }
                    self.doc = self.level0_last_doc_id;
                    self.doc_buffer_upto = BS as i32;
                }
            }
        }
    }

    /// Equivalent to `BlockPostingsEnum.docIDRunEnd()`
    /// (`Lucene104PostingsReader.java:1100-1128`): in a dense unary block,
    /// the end of the level-1 block when that is dense too, else of the
    /// level-0 block; otherwise `DocIdSetIterator.docIDRunEnd()`.
    fn doc_id_run_end(&mut self) -> Result<i32> {
        if self.encoding == DeltaEncoding::Unary {
            // Note: this assumes that BLOCK_SIZE == 256, this bit of the code would need to be
            // changed if the block size was changed.
            debug_assert_eq!(BS as i64, 4 * 64);
            let bits = self.doc_bit_set.get_bits();
            let level0_is_dense =
                (0..4).all(|i| bits[i].load(std::sync::atomic::Ordering::Relaxed) == u64::MAX);
            if level0_is_dense {
                let level0_doc_count_upto = self.doc_freq - self.doc_count_left;
                let level1_is_dense = self.level1_last_doc_id - self.level0_last_doc_id
                    == self.level1_doc_count_upto - level0_doc_count_upto;
                if level1_is_dense {
                    return Ok(self.level1_last_doc_id + 1);
                }
                return Ok(self.level0_last_doc_id + 1);
            }
        }

        debug_assert!(self.doc != NO_MORE_DOCS);
        Ok(self.doc + 1)
    }
}

impl BlockPostingsEnum {
    /// Equivalent to `BlockPostingsEnum.bufferIntoBitSet(int, int,
    /// FixedBitSet, int)` (`Lucene104PostingsReader.java:1092-1098`).
    fn buffer_into_bit_set(
        &self,
        start: usize,
        end: usize,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) {
        // bitSet#set and `doc - offset` get auto-vectorized
        for &doc in &self.doc_buffer[start..end] {
            bit_set.set((doc - offset) as usize);
        }
    }
}

impl std::fmt::Display for BlockPostingsEnum {
    /// `BlockPostingsEnum` declares no `toString()` in Lucene 10.5.0, so
    /// `java.lang.Object`'s applies: the binary class name, `@`, and the
    /// identity hash. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.codecs.lucene104.Lucene104PostingsReader$BlockPostingsEnum",
            self,
        ))
    }
}

impl PostingsEnum for BlockPostingsEnum {
    /// Equivalent to `BlockPostingsEnum.freq()` (`Lucene104PostingsReader.java:584-591`):
    /// a full block's frequencies are decoded on the first call, from where
    /// `refill_full_block` recorded them.
    ///
    /// Java seeks `docIn` there and decodes, which leaves `docIn` where the
    /// block's skip had already left it, at the end of the frequencies. This
    /// method takes `&self`, so it reads through its own clone of `docIn`
    /// instead; `docIn` keeps the position Java's ends at, and the decoded
    /// values are the same.
    ///
    /// A document position that a corrupt bit-set block put outside the block
    /// is an error, where Java would throw `ArrayIndexOutOfBoundsException`.
    fn freq(&self) -> Result<i32> {
        let freq_fp = self.freq_fp.get();
        if freq_fp != -1 {
            let mut freq_in = self.freq_in.borrow_mut();
            if freq_in.is_none() {
                let doc_in = self.doc_in.as_ref().ok_or_else(|| {
                    LuceneError::illegal_state("BlockPostingsEnum has no doc input".to_string())
                })?;
                *freq_in = Some(doc_in.clone_input()?);
            }
            let input = freq_in
                .as_mut()
                .expect("INVARIANT: the input was set one statement above");
            input.seek(freq_fp)?;
            let mut pdu = PostingDecodingUtil::new(input.as_mut());
            PForUtil::new().decode(&mut pdu, &mut self.freq_buffer.borrow_mut())?;
            self.freq_fp.set(-1);
        }
        let upto = self.doc_buffer_upto;
        if !(1..=BS_I32).contains(&upto) {
            return Err(LuceneError::from(LuceneErrorKind::CorruptIndex(format!(
                "postings block position {} is outside the {BS}-document block",
                upto - 1
            ))));
        }
        Ok(self.freq_buffer.borrow()[upto as usize - 1])
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
            // `return payload;`, the view `accumulatePayloadAndOffsets` last
            // set (`Lucene104PostingsReader.java:1340-1346`).
            let start = self.payload_offset as usize;
            let end = start + self.payload_length as usize;
            Ok(Some(&self.payload_bytes[start..end]))
        }
    }

    /// Equivalent to `BlockPostingsEnum.nextPostings(int,
    /// DocAndFloatFeatureBuffer)` (`Lucene104PostingsReader.java:1036-1080`):
    /// without frequencies, the default batches of 16; otherwise the docs and
    /// frequencies of the current block that are below `up_to`, after which
    /// the enum advances to `up_to`.
    fn next_postings(
        &mut self,
        up_to: i32,
        buffer: &mut crate::index::postings_enum::DocAndFloatFeatureBuffer,
    ) -> Result<()> {
        debug_assert!(!self.needs_refilling);

        if !self.needs_freq {
            return crate::index::postings_enum::default_next_postings(self, up_to, buffer);
        }

        buffer.size = 0;
        if self.doc >= up_to {
            return Ok(());
        }

        // Only return docs from the current block
        // +1 to make FixedBitSet#intoArray perform a bit faster, it uses a slower path if it doesn't
        // have a free slot after the last element
        buffer.grow_no_copy(BS + 1);
        let up_to = i64::from(up_to).min(i64::from(self.level0_last_doc_id) + 1) as i32;

        // Frequencies are decoded lazily, calling freq() makes sure that the freq block is decoded
        self.freq()?;

        let start = (self.doc_buffer_upto - 1) as usize;
        match self.encoding {
            DeltaEncoding::Packed => {
                let end = self.compute_buffer_end_boundary(up_to);
                buffer.size = end - start;
                buffer.docs[..buffer.size].copy_from_slice(&self.doc_buffer[start..end]);
            }
            DeltaEncoding::Unary => {
                buffer.size = self.doc_bit_set.into_array(
                    (self.doc - self.doc_bit_set_base) as usize,
                    (up_to - self.doc_bit_set_base) as usize,
                    self.doc_bit_set_base,
                    &mut buffer.docs,
                );
                // buffer's capacity is BLOCK_SIZE+1
                debug_assert!(buffer.size < buffer.docs.len());
            }
        }

        debug_assert!(buffer.size > 0);
        {
            let freqs = self.freq_buffer.borrow();
            for i in 0..buffer.size {
                buffer.features[i] = freqs[start + i] as f32;
            }
        }

        self.advance(up_to)?;
        Ok(())
    }

    /// This enum answers with itself: it is the `true` arm of Java's
    /// `reuse instanceof BlockPostingsEnum` (`Lucene104PostingsReader.java:259`).
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
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
        debug_assert!(self.needs_impacts);
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
    /// `level0SerializedImpacts.length`.
    level0_serialized_impacts_length: usize,
    level1_serialized_impacts: Vec<u8>,
    /// `level1SerializedImpacts.length`.
    level1_serialized_impacts_length: usize,
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
            // `scratch.reset(serialized.bytes, 0, serialized.length)`
            // (`Lucene104PostingsReader.java:1398`): only the bytes the last
            // read filled, never the stale tail of the buffer.
            read_impacts(
                &mut crate::store::ByteArrayDataInput::new(
                    &self.level0_serialized_impacts[..self.level0_serialized_impacts_length],
                ),
                &mut buffer,
            )
            .expect("impacts from memory");
            return buffer;
        }
        if level == 1 {
            read_impacts(
                &mut crate::store::ByteArrayDataInput::new(
                    &self.level1_serialized_impacts[..self.level1_serialized_impacts_length],
                ),
                &mut buffer,
            )
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
    use crate::codecs::postings::PushPostingsWriterBase;
    use crate::codecs::state::SegmentWriteState;
    use crate::index::{FieldInfo, FieldInfos, IndexOptions};
    use crate::search::DocIdSetIterator;
    use crate::store::{
        ByteBuffersDataOutput, ByteBuffersDirectory, ByteBuffersIndexOutput, Directory,
        MockIndexInput,
    };
    use std::sync::Arc;

    fn test_segment_info(name: &str, max_doc: i32) -> crate::index::SegmentInfo {
        crate::codecs::tests::test_segment_info(name, max_doc)
    }

    fn test_write_state<'a>(
        dir: &'a Arc<dyn Directory>,
        info: &'a crate::index::SegmentInfo,
        infos: &'a FieldInfos,
    ) -> SegmentWriteState<'a> {
        SegmentWriteState::new(
            &crate::util::NoOutputInfoStream,
            Arc::clone(dir),
            info,
            infos,
            Box::leak(Box::new(crate::codecs::stub::BufferedUpdates::default())),
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
        dir: &Arc<dyn Directory>,
        field_infos: &FieldInfos,
        field_info: &FieldInfo,
        postings: &TestPostingsEnum,
    ) -> Result<(Lucene104PostingsReader, BlockTermState)> {
        write_term_and_open_reader_with(dir, field_infos, field_info, postings, |_, _| {})
    }

    /// [`write_term_and_open_reader`] that runs `before_open` on the written
    /// files, with the term's state, before the reader opens them.
    fn write_term_and_open_reader_with(
        dir: &Arc<dyn Directory>,
        field_infos: &FieldInfos,
        field_info: &FieldInfo,
        postings: &TestPostingsEnum,
        before_open: impl FnOnce(&Arc<dyn Directory>, &BlockTermState),
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
        // A norm of one for every document, as the flush hands the writer.
        let norms = crate::codecs::norms::NormsProducer::get_norms(
            &crate::index::indexing_chain::EmptyNormsProducer::new(max_doc),
            field_info,
        )?;
        writer.start_term(norms)?;
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
        before_open(dir, &state);

        let read_state = test_read_state(&**dir, &segment_info, field_infos);
        let mut reader = Lucene104PostingsReader::new(&read_state)?;
        let terms_bytes = terms_out.to_array_copy()?;
        let mut terms_in = MockIndexInput::new(terms_bytes, "terms");
        reader.init(&mut terms_in, &read_state)?;
        Ok((reader, state))
    }

    #[test]
    fn singleton_doc_round_trip() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let field_infos = field_infos_with_freqs();
        let field_info = field_infos.field_info("body").unwrap();
        let postings = TestPostingsEnum::new(vec![0], vec![1], vec![vec![]]);

        let (reader, state) = write_term_and_open_reader(&dir, &field_infos, field_info, &postings)
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let field_infos = field_infos_with_freqs();
        let field_info = field_infos.field_info("body").unwrap();
        let postings = TestPostingsEnum::new(
            vec![0, 2, 5, 7],
            vec![3, 1, 2, 1],
            vec![vec![], vec![], vec![], vec![]],
        );

        let (reader, state) = write_term_and_open_reader(&dir, &field_infos, field_info, &postings)
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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

        let (reader, state) = write_term_and_open_reader(&dir, &field_infos, field_info, &postings)
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let field_infos = field_infos_with_freqs();
        let field_info = field_infos.field_info("body").unwrap();
        let postings = TestPostingsEnum::new((0..256).collect(), vec![1; 256], vec![vec![]; 256]);

        let (reader, state) = write_term_and_open_reader(&dir, &field_infos, field_info, &postings)
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

    /// rmp #241: a full block whose width byte is past `ForUtil`'s mask table
    /// fails as Java does — `ForUtil.decode` throws
    /// `ArrayIndexOutOfBoundsException` from `MASKS32[bitsPerValue]`
    /// (`ForUtil.java:168-170`), reached from `refillFullBlock`
    /// (`Lucene104PostingsReader.java:594-597`) — instead of panicking.
    #[test]
    fn a_corrupt_full_block_width_is_an_error_not_a_panic() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let field_infos = field_infos_with_freqs();
        let field_info = field_infos.field_info("body").unwrap();
        // Deltas of 1000 pack in 10 bits, so the block is PACKED, not a bit set.
        let postings = TestPostingsEnum::new(
            (0..512).map(|i| i * 1000).collect(),
            vec![1; 512],
            vec![vec![]; 512],
        );

        let corrupt = |dir: &Arc<dyn Directory>, state: &BlockTermState| {
            let name = dir
                .list_all()
                .unwrap()
                .into_iter()
                .find(|name| name.ends_with(".doc"))
                .expect("the postings writer wrote a .doc file");
            let length = dir.file_length(&name).unwrap() as usize;
            let mut bytes = vec![0u8; length];
            dir.open_input(&name, &*crate::store::DEFAULT_IO_CONTEXT)
                .unwrap()
                .read_bytes(Some(&mut bytes), 0, length)
                .unwrap();
            // Walk the first block's level-0 skip data the way
            // `doMoveToNextLevel0Block` reads it
            // (`Lucene104PostingsReader.java:740-753`); the width byte follows.
            let mut input = MockIndexInput::new(bytes.clone(), "doc");
            input.seek(state.doc_start_fp).unwrap();
            input.read_v_long().unwrap(); // level0NumBytes
            read_v_int15(&mut input).unwrap(); // docDelta
            read_v_long15(&mut input).unwrap(); // blockLength
            let num_impact_bytes = input.read_v_int().unwrap();
            input.skip_bytes(i64::from(num_impact_bytes)).unwrap();
            let width_at = input.file_pointer() as usize;
            assert_eq!(bytes[width_at], 10, "the width byte of the first block");
            bytes[width_at] = 40;
            dir.delete_file(&name).unwrap();
            let mut out = dir
                .create_output(&name, &*crate::store::DEFAULT_IO_CONTEXT)
                .unwrap();
            out.write_bytes(&bytes, 0, bytes.len()).unwrap();
            out.close().unwrap();
        };
        let (reader, state) =
            write_term_and_open_reader_with(&dir, &field_infos, field_info, &postings, corrupt)
                .expect("reader builds");

        let mut it = reader
            .postings(field_info, &state, None, crate::index::POSTINGS_ENUM_FREQS)
            .expect("postings exist");
        match it.next_doc() {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) => {
                assert_eq!(message, "Index 40 out of bounds for length 32")
            }
            other => panic!("expected Java's index-out-of-bounds failure, got {other:?}"),
        }
        reader.close().expect("close succeeds");
    }

    #[test]
    fn level1_skip_8193_round_trip() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let field_infos = field_infos_with_freqs();
        let field_info = field_infos.field_info("body").unwrap();
        let postings =
            TestPostingsEnum::new((0..8193).collect(), vec![1; 8193], vec![vec![]; 8193]);

        let (reader, state) = write_term_and_open_reader(&dir, &field_infos, field_info, &postings)
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

impl std::fmt::Display for Lucene104PostingsReader {
    /// Port of `Lucene104PostingsReader.toString()`, which reports whether the
    /// `.pos` and `.pay` files were opened as Java's `boolean` literals `true`
    /// and `false`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Lucene104PostingsReader(positions={},payloads={})",
            self.pos_in.is_some(),
            self.pay_in.is_some()
        )
    }
}

/// The `IOUtils` close family at the postings reader's call sites (#308).
#[cfg(test)]
mod close_family_tests {
    use super::Lucene104PostingsReader;
    use crate::codecs::lucene104::Lucene104PostingsFormat;
    use crate::codecs::postings::{PostingsFormat, PostingsReaderBase};
    use crate::codecs::tests::{
        read_state, rewrite_file, test_segment_info, tracking_directory, write_state,
    };
    use crate::index::{FieldInfo, FieldInfos, IndexOptions};

    fn positions() -> FieldInfos {
        FieldInfos::new(vec![FieldInfo::new("body", 0).with_postings_options(
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            false,
            true,
        )])
        .unwrap()
    }

    fn write_empty(
        dir: &std::sync::Arc<dyn crate::store::Directory>,
        info: &crate::index::SegmentInfo,
        fis: &FieldInfos,
    ) {
        let mut consumer = Lucene104PostingsFormat::new()
            .fields_consumer(&write_state(dir, info, fis))
            .unwrap();
        consumer
            .write(&crate::index::terms::EmptyFields::new(), None)
            .unwrap();
        consumer.close().unwrap();
    }

    /// `docIn` is held before its header is checked, so a bad header closes
    /// it: `IOUtils.closeWhileHandlingException(docIn, posIn, payIn)`
    /// (`Lucene104PostingsReader.java:166-172`).
    #[test]
    fn a_bad_doc_header_closes_the_doc_input() {
        let (dir, tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        let fis = FieldInfos::default();
        write_empty(&dir, &info, &fis);
        let before = log.closed().len();
        rewrite_file(&tracking, "_0.doc", |bytes| bytes[6] ^= 0xff);
        Lucene104PostingsReader::new(&read_state(&*dir, &info, &fis)).unwrap_err();
        // The metadata input is closed first, by its try-with-resources.
        assert_eq!(log.closed()[before..], ["_0.psm", "_0.doc"]);
    }

    /// `close()` is `IOUtils.close(docIn, posIn, payIn)`
    /// (`Lucene104PostingsReader.java:209-212`): a failing `.doc` does not
    /// keep `.pos` and `.pay` open, and its failure is the one raised.
    #[test]
    fn close_closes_every_input_past_a_failure() {
        let (dir, _tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        let fis = positions();
        write_empty(&dir, &info, &fis);
        let reader = Lucene104PostingsReader::new(&read_state(&*dir, &info, &fis)).unwrap();
        let before = log.closed().len();
        log.fail_close("_0.doc");
        let error = reader.close().unwrap_err();
        assert!(format!("{error}").contains("_0.doc"), "{error}");
        assert_eq!(log.closed()[before..], ["_0.doc", "_0.pos", "_0.pay"]);
    }
}
