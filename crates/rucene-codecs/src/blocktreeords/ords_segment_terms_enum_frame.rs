/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.codecs.blocktreeords.OrdsSegmentTermsEnumFrame`.
//!
//! Java's frame reaches its enum (`ste`) for the enum's input, current term
//! and `termExists` flag, and the field reader. The port hands those to each
//! method as a [`SteCtx`]; `scanToTermNonLeaf`'s descent into sub-frames,
//! which pushes frames on the enum, is reported back to the enum
//! ([`NonLeafScan::descend`]) and performed there, at the same point.

use std::cmp::Ordering;

use rucene::codecs::BlockTermState;
use rucene::error::Result;
use rucene::index::{IndexOptions, SeekStatus};
use rucene::store::IndexInput;

use super::ords_field_reader::OrdsFieldReader;
use super::{java_assert, OwnedArrayInput, TermBuilder};

/// What a frame reaches through Java's `ste` field.
pub(crate) struct SteCtx<'a> {
    /// `ste.term`.
    pub(crate) term: &'a mut TermBuilder,
    /// `ste.termExists`.
    pub(crate) term_exists: &'a mut bool,
    /// `ste.in`, cloned lazily by `ste.initIndexInput()`.
    pub(crate) input: &'a mut Option<Box<dyn IndexInput>>,
    /// `ste.fr`.
    pub(crate) fr: &'a OrdsFieldReader,
}

impl SteCtx<'_> {
    /// Port of `OrdsSegmentTermsEnum.initIndexInput()`.
    pub(crate) fn init_index_input(&mut self) -> Result<&mut dyn IndexInput> {
        if self.input.is_none() {
            *self.input = Some(self.fr.parent.input.clone_input()?);
        }
        Ok(self
            .input
            .as_deref_mut()
            .expect("INVARIANT: initialized above"))
    }
}

/// The descent `scanToTermNonLeaf` performs through `ste.pushFrame` when it
/// stops on a sub-block and the caller wants the next term.
pub(crate) struct NonLeafScan {
    pub(crate) status: SeekStatus,
    /// `Some((prefixLength + suffixLength, prevTermOrd))` when Java pushes
    /// `ste.pushFrame(null, ste.currentFrame.lastSubFP, prefixLength +
    /// suffixLength, prevTermOrd)` and keeps descending.
    pub(crate) descend: Option<(usize, i64)>,
}

/// Port of the package-private `OrdsSegmentTermsEnumFrame`.
pub(crate) struct OrdsSegmentTermsEnumFrame {
    // Our index in stack[]:
    pub(crate) ord: i32,

    pub(crate) has_terms: bool,
    pub(crate) has_terms_orig: bool,
    pub(crate) is_floor: bool,

    /// Java's `arc != null`: the frame keeps its index arc only for that
    /// test (two assertions and the unused `printSeekState`).
    pub(crate) has_arc: bool,

    // File pointer where this block was loaded from
    pub(crate) fp: i64,
    pub(crate) fp_orig: i64,
    pub(crate) fp_end: i64,

    /// `suffixBytes` + `suffixesReader`.
    pub(crate) suffixes: OwnedArrayInput,

    /// `statBytes` + `statsReader`.
    pub(crate) stats: OwnedArrayInput,

    /// `floorData` + `floorDataReader`.
    pub(crate) floor_data: OwnedArrayInput,

    // Length of prefix shared by all terms in this block
    pub(crate) prefix_length: usize,

    // Number of entries (term or sub-block) in this block
    pub(crate) ent_count: i32,

    // Which term we will next read, or -1 if the block
    // isn't loaded yet
    pub(crate) next_ent: i32,

    // Starting termOrd for this frame, used to reset termOrd in rewind()
    pub(crate) term_ord_orig: i64,

    // 1 + ordinal of the current term
    pub(crate) term_ord: i64,

    // True if this block is either not a floor block,
    // or, it's the last sub-block of a floor block
    pub(crate) is_last_in_floor: bool,

    // True if all entries are terms
    pub(crate) is_leaf_block: bool,

    pub(crate) last_sub_fp: i64,

    // Starting byte of next floor block:
    pub(crate) next_floor_label: i32,

    // Starting termOrd of next floor block:
    pub(crate) next_floor_term_ord: i64,

    pub(crate) num_follow_floor_blocks: i32,

    // Next term to decode metaData; we decode metaData
    // lazily so that scanning to find the matching term is
    // fast and only if you find a match and app wants the
    // stats or docs/positions enums, will we decode the
    // metaData
    pub(crate) meta_data_upto: i32,

    pub(crate) state: BlockTermState,

    /// `bytes` + `bytesReader` (the metadata).
    pub(crate) bytes: OwnedArrayInput,

    start_byte_pos: usize,
    suffix_length: usize,
    sub_code: i64,
}

impl OrdsSegmentTermsEnumFrame {
    /// Port of `OrdsSegmentTermsEnumFrame(OrdsSegmentTermsEnum, int)`. The
    /// other fields keep Java's defaults (`0`, `false`).
    pub(crate) fn new(fr: &OrdsFieldReader, ord: i32) -> Result<Self> {
        let mut state = fr.parent.postings_reader.new_term_state()?;
        state.total_term_freq = -1;
        Ok(Self {
            ord,
            has_terms: false,
            has_terms_orig: false,
            is_floor: false,
            has_arc: false,
            fp: 0,
            fp_orig: 0,
            fp_end: 0,
            suffixes: OwnedArrayInput::default(),
            stats: OwnedArrayInput::default(),
            floor_data: OwnedArrayInput::default(),
            prefix_length: 0,
            ent_count: 0,
            next_ent: 0,
            term_ord_orig: 0,
            term_ord: 0,
            is_last_in_floor: false,
            is_leaf_block: false,
            last_sub_fp: 0,
            next_floor_label: 0,
            next_floor_term_ord: 0,
            num_follow_floor_blocks: 0,
            meta_data_upto: 0,
            state,
            bytes: OwnedArrayInput::default(),
            start_byte_pos: 0,
            suffix_length: 0,
            sub_code: 0,
        })
    }

    /// Port of `setFloorData(ByteArrayDataInput, BytesRef)`: `source` is
    /// the frame data, `pos` the reader's position in it.
    pub(crate) fn set_floor_data(&mut self, source: &[u8], pos: usize) -> Result<()> {
        let num_bytes = source.len() - pos;
        java_assert!(num_bytes > 0);
        self.floor_data.fill_from(&source[pos..]);
        self.num_follow_floor_blocks = self.floor_data.read_v_int()?;
        self.next_floor_label = i32::from(self.floor_data.read_byte()?);
        self.next_floor_term_ord = self.term_ord_orig + self.floor_data.read_v_long()?;
        Ok(())
    }

    /// Port of `getTermBlockOrd()`.
    pub(crate) fn get_term_block_ord(&self) -> i32 {
        if self.is_leaf_block {
            self.next_ent
        } else {
            self.state.term_block_ord
        }
    }

    /// Port of `loadNextFloorBlock()`.
    pub(crate) fn load_next_floor_block(&mut self, ctx: &mut SteCtx<'_>) -> Result<()> {
        java_assert!(!self.has_arc || self.is_floor, "isFloor={}", self.is_floor);
        // NOTE: we don't need to touch termOrd here, because we fully scanned this current frame
        self.fp = self.fp_end;
        self.next_ent = -1;
        self.load_block(ctx)
    }

    /// Does initial decode of next block of terms; this doesn't actually
    /// decode the docFreq, totalTermFreq, postings details (frq/prx offset,
    /// etc.) metadata; it just loads them as byte[] blobs which are then
    /// decoded on-demand if the metadata is ever requested for any term in
    /// this block. This enables terms-only intensive consumes (eg certain
    /// MTQs, respelling) to not pay the price of decoding metadata they
    /// won't use.
    ///
    /// Port of `loadBlock()`.
    pub(crate) fn load_block(&mut self, ctx: &mut SteCtx<'_>) -> Result<()> {
        // Clone the IndexInput lazily, so that consumers
        // that just pull a TermsEnum to
        // seekExact(TermState) don't pay this cost:
        let input = ctx.init_index_input()?;

        if self.next_ent != -1 {
            // Already loaded
            return Ok(());
        }

        input.seek(self.fp)?;
        let code = input.read_v_int()?;
        self.ent_count = ((code as u32) >> 1) as i32;
        java_assert!(self.ent_count > 0);
        self.is_last_in_floor = (code & 1) != 0;
        java_assert!(!self.has_arc || self.is_last_in_floor || self.is_floor);

        // TODO: if suffixes were stored in random-access
        // array structure, then we could do binary search
        // instead of linear scan to find target term; eg
        // we could have simple array of offsets

        // term suffixes:
        let code = input.read_v_int()?;
        self.is_leaf_block = (code & 1) != 0;
        let num_bytes = ((code as u32) >> 1) as i32;
        self.suffixes.fill(input, num_bytes)?;

        // stats
        let num_bytes = input.read_v_int()?;
        self.stats.fill(input, num_bytes)?;
        self.meta_data_upto = 0;

        self.state.term_block_ord = 0;
        self.next_ent = 0;
        self.last_sub_fp = -1;

        // TODO: we could skip this if !hasTerms; but
        // that's rare so won't help much
        // metadata
        let num_bytes = input.read_v_int()?;
        self.bytes.fill(input, num_bytes)?;

        // Sub-blocks of a single floor block are always
        // written one after another -- tail recurse:
        self.fp_end = input.file_pointer()?;
        Ok(())
    }

    /// Port of `rewind()`.
    pub(crate) fn rewind(&mut self) -> Result<()> {
        // Force reload:
        self.fp = self.fp_orig;
        self.term_ord = self.term_ord_orig;
        self.next_ent = -1;
        self.has_terms = self.has_terms_orig;
        if self.is_floor {
            self.floor_data.rewind();
            self.num_follow_floor_blocks = self.floor_data.read_v_int()?;
            java_assert!(self.num_follow_floor_blocks > 0);
            self.next_floor_label = i32::from(self.floor_data.read_byte()?);
            self.next_floor_term_ord = self.term_ord_orig + self.floor_data.read_v_long()?;
        }
        Ok(())
    }

    /// Port of `next()`.
    pub(crate) fn next(&mut self, ctx: &mut SteCtx<'_>) -> Result<bool> {
        if self.is_leaf_block {
            self.next_leaf(ctx)
        } else {
            self.next_non_leaf(ctx)
        }
    }

    /// Decodes next entry; returns true if it's a sub-block.
    ///
    /// Port of `nextLeaf()`.
    pub(crate) fn next_leaf(&mut self, ctx: &mut SteCtx<'_>) -> Result<bool> {
        java_assert!(
            self.next_ent != -1 && self.next_ent < self.ent_count,
            "nextEnt={} entCount={} fp={} termOrd={}",
            self.next_ent,
            self.ent_count,
            self.fp,
            self.term_ord
        );
        self.next_ent += 1;
        self.term_ord += 1;
        self.suffix_length = super::array_len(self.suffixes.read_v_int()?)?;
        self.start_byte_pos = self.suffixes.position();
        ctx.term.set_length(self.prefix_length + self.suffix_length);
        self.suffixes
            .read_into(&mut ctx.term.bytes, self.prefix_length, self.suffix_length)?;
        // A normal term
        *ctx.term_exists = true;
        Ok(false)
    }

    /// Port of `nextNonLeaf()`.
    pub(crate) fn next_non_leaf(&mut self, ctx: &mut SteCtx<'_>) -> Result<bool> {
        java_assert!(
            self.next_ent != -1 && self.next_ent < self.ent_count,
            "nextEnt={} entCount={} fp={}",
            self.next_ent,
            self.ent_count,
            self.fp
        );
        self.next_ent += 1;
        let code = self.suffixes.read_v_int()?;
        self.suffix_length = ((code as u32) >> 1) as usize;
        self.start_byte_pos = self.suffixes.position();
        ctx.term.set_length(self.prefix_length + self.suffix_length);
        self.suffixes
            .read_into(&mut ctx.term.bytes, self.prefix_length, self.suffix_length)?;
        if (code & 1) == 0 {
            // A normal term
            *ctx.term_exists = true;
            self.sub_code = 0;
            self.state.term_block_ord += 1;
            self.term_ord += 1;
            Ok(false)
        } else {
            // A sub-block; make sub-FP absolute:
            *ctx.term_exists = false;
            self.sub_code = self.suffixes.read_v_long()?;
            self.term_ord += self.suffixes.read_v_long()?;
            self.last_sub_fp = self.fp - self.sub_code;
            Ok(true)
        }
    }

    /// Port of `scanToFloorFrame(BytesRef)`.
    // TODO: make this array'd so we can do bin search?
    // likely not worth it?  need to measure how many
    // floor blocks we "typically" get
    pub(crate) fn scan_to_floor_frame(&mut self, target: &[u8]) -> Result<()> {
        if !self.is_floor || target.len() <= self.prefix_length {
            return Ok(());
        }

        let target_label = i32::from(target[self.prefix_length]);

        if target_label < self.next_floor_label {
            return Ok(());
        }

        java_assert!(self.num_follow_floor_blocks != 0);

        let mut new_fp;
        let mut last_floor_term_ord;
        loop {
            let code = self.floor_data.read_v_long()?;
            new_fp = self.fp_orig + ((code as u64) >> 1) as i64;
            self.has_terms = (code & 1) != 0;

            self.is_last_in_floor = self.num_follow_floor_blocks == 1;
            self.num_follow_floor_blocks -= 1;

            last_floor_term_ord = self.next_floor_term_ord;

            if self.is_last_in_floor {
                self.next_floor_label = 256;
                self.next_floor_term_ord = i64::MAX;
                break;
            } else {
                self.next_floor_label = i32::from(self.floor_data.read_byte()?);
                self.next_floor_term_ord += self.floor_data.read_v_long()?;
                if target_label < self.next_floor_label {
                    break;
                }
            }
        }

        if new_fp != self.fp {
            // Force re-load of the block:
            self.next_ent = -1;
            self.term_ord = last_floor_term_ord;
            self.fp = new_fp;
        }
        Ok(())
    }

    /// Port of `scanToFloorFrame(long)`.
    // TODO: make this array'd so we can do bin search?
    // likely not worth it?  need to measure how many
    // floor blocks we "typically" get
    pub(crate) fn scan_to_floor_frame_ord(&mut self, target_ord: i64) -> Result<()> {
        if !self.is_floor || target_ord < self.next_floor_term_ord {
            return Ok(());
        }

        java_assert!(self.num_follow_floor_blocks != 0);

        let mut new_fp;
        let mut last_floor_term_ord;
        loop {
            let code = self.floor_data.read_v_long()?;
            new_fp = self.fp_orig + ((code as u64) >> 1) as i64;
            self.has_terms = (code & 1) != 0;

            self.is_last_in_floor = self.num_follow_floor_blocks == 1;
            self.num_follow_floor_blocks -= 1;

            last_floor_term_ord = self.next_floor_term_ord;

            if self.is_last_in_floor {
                self.next_floor_label = 256;
                self.next_floor_term_ord = i64::MAX;
                break;
            } else {
                self.next_floor_label = i32::from(self.floor_data.read_byte()?);
                self.next_floor_term_ord += self.floor_data.read_v_long()?;
                if target_ord < self.next_floor_term_ord {
                    break;
                }
            }
        }

        if new_fp != self.fp {
            // Force re-load of the block:
            self.next_ent = -1;
            self.term_ord = last_floor_term_ord;
            self.fp = new_fp;
        }
        Ok(())
    }

    /// Port of `decodeMetaData()`.
    pub(crate) fn decode_meta_data(&mut self, fr: &OrdsFieldReader) -> Result<()> {
        java_assert!(self.next_ent >= 0);

        // lazily catch up on metadata decode:
        let limit = self.get_term_block_ord();
        let mut absolute = self.meta_data_upto == 0;
        java_assert!(
            limit > 0,
            "limit={limit} isLeafBlock={} nextEnt={}",
            self.is_leaf_block,
            self.next_ent
        );

        let field_info = &fr.data.field_info;
        // TODO: better API would be "jump straight to term=N"???
        while self.meta_data_upto < limit {
            // TODO: we could make "tiers" of metadata, ie,
            // decode docFreq/totalTF but don't decode postings
            // metadata; this way caller could get
            // docFreq/totalTF w/o paying decode cost for
            // postings

            // TODO: if docFreq were bulk decoded we could
            // just skipN here:

            // stats
            self.state.doc_freq = self.stats.read_v_int()?;
            if field_info.get_index_options() == IndexOptions::DOCS {
                self.state.total_term_freq = i64::from(self.state.doc_freq); // all tf values are 1
            } else {
                self.state.total_term_freq =
                    i64::from(self.state.doc_freq) + self.stats.read_v_long()?;
            }

            // metadata
            let state = &mut self.state;
            self.bytes.with(|i| {
                fr.parent
                    .postings_reader
                    .decode_term(i, field_info, state, absolute)
            })?;

            self.meta_data_upto += 1;
            absolute = false;
        }
        self.state.term_block_ord = self.meta_data_upto;
        Ok(())
    }

    /// Used only by assert.
    ///
    /// Port of `prefixMatches(BytesRef)`.
    fn prefix_matches(&self, target: &[u8], term: &TermBuilder) -> bool {
        (0..self.prefix_length).all(|i| target.get(i).copied() == term.bytes.get(i).copied())
    }

    /// Scans to sub-block that has this target fp; only called by next();
    /// NOTE: does not set startBytePos/suffix as a side effect.
    ///
    /// Port of `scanToSubBlock(long)`.
    pub(crate) fn scan_to_sub_block(&mut self, sub_fp: i64) -> Result<()> {
        java_assert!(!self.is_leaf_block);
        if self.last_sub_fp == sub_fp {
            return Ok(());
        }
        java_assert!(sub_fp < self.fp, "fp={} subFP={sub_fp}", self.fp);
        let target_sub_code = self.fp - sub_fp;
        loop {
            java_assert!(self.next_ent < self.ent_count);
            self.next_ent += 1;
            let code = self.suffixes.read_v_int()?;
            self.suffixes.skip_bytes(if self.is_leaf_block {
                code
            } else {
                ((code as u32) >> 1) as i32
            })?;
            if (code & 1) != 0 {
                let sub_code = self.suffixes.read_v_long()?;
                self.term_ord += self.suffixes.read_v_long()?;

                if target_sub_code == sub_code {
                    self.last_sub_fp = sub_fp;
                    return Ok(());
                }
            } else {
                self.state.term_block_ord += 1;
                self.term_ord += 1;
            }
        }
    }

    /// The suffix of the current entry, `suffixBytes[startBytePos ..
    /// startBytePos + suffixLength]`.
    fn suffix(&self) -> &[u8] {
        &self.suffixes.bytes[self.start_byte_pos..self.start_byte_pos + self.suffix_length]
    }

    /// `target.bytes[target.offset + prefixLength .. target.offset +
    /// target.length]`.
    fn target_suffix<'t>(&self, target: &'t [u8]) -> &'t [u8] {
        target.get(self.prefix_length..).unwrap_or(&[])
    }

    /// Target's prefix matches this block's prefix; we scan the entries
    /// check if the suffix matches.
    ///
    /// Port of `scanToTermLeaf(BytesRef, boolean)`; sets
    /// startBytePos/suffix as a side effect.
    pub(crate) fn scan_to_term_leaf(
        &mut self,
        ctx: &mut SteCtx<'_>,
        target: &[u8],
        exact_only: bool,
    ) -> Result<SeekStatus> {
        java_assert!(self.next_ent != -1);

        *ctx.term_exists = true;
        self.sub_code = 0;

        if self.next_ent == self.ent_count {
            if exact_only {
                self.fill_term(ctx.term);
            }
            return Ok(SeekStatus::END);
        }

        java_assert!(self.prefix_matches(target, ctx.term));

        // Loop over each entry (term or sub-block) in this block:
        loop {
            self.next_ent += 1;
            self.term_ord += 1;

            self.suffix_length = super::array_len(self.suffixes.read_v_int()?)?;

            self.start_byte_pos = self.suffixes.position();
            self.suffixes.skip_bytes(self.suffix_length as i32)?;

            // Compare suffix and target.
            let cmp = self.suffix().cmp(self.target_suffix(target));

            match cmp {
                Ordering::Less => {
                    // Current entry is still before the target;
                    // keep scanning
                }
                Ordering::Greater => {
                    // Done!  Current entry is after target --
                    // return NOT_FOUND:
                    self.fill_term(ctx.term);

                    return Ok(SeekStatus::NOT_FOUND);
                }
                Ordering::Equal => {
                    // Exact match!

                    // This cannot be a sub-block because we
                    // would have followed the index to this
                    // sub-block from the start:

                    self.fill_term(ctx.term);
                    return Ok(SeekStatus::FOUND);
                }
            }
            if self.next_ent >= self.ent_count {
                break;
            }
        }

        // It is possible (and OK) that terms index pointed us
        // at this block, but, we scanned the entire block and
        // did not find the term to position to.  This happens
        // when the target is after the last term in the block
        // (but, before the next term in the index).  EG
        // target could be foozzz, and terms index pointed us
        // to the foo* block, but the last term in this block
        // was fooz (and, eg, first term in the next block will
        // bee fop).
        if exact_only {
            self.fill_term(ctx.term);
        }

        // TODO: not consistent that in the
        // not-exact case we don't next() into the next
        // frame here
        Ok(SeekStatus::END)
    }

    /// Target's prefix matches this block's prefix; we scan the entries
    /// check if the suffix matches.
    ///
    /// Port of `scanToTermNonLeaf(BytesRef, boolean)`, but for the descent
    /// into sub-frames, which the result asks the enum to perform.
    pub(crate) fn scan_to_term_non_leaf(
        &mut self,
        ctx: &mut SteCtx<'_>,
        target: &[u8],
        exact_only: bool,
    ) -> Result<NonLeafScan> {
        java_assert!(self.next_ent != -1);

        if self.next_ent == self.ent_count {
            if exact_only {
                self.fill_term(ctx.term);
                *ctx.term_exists = self.sub_code == 0;
            }
            return Ok(NonLeafScan {
                status: SeekStatus::END,
                descend: None,
            });
        }

        java_assert!(self.prefix_matches(target, ctx.term));

        // Loop over each entry (term or sub-block) in this block:
        while self.next_ent < self.ent_count {
            self.next_ent += 1;

            let code = self.suffixes.read_v_int()?;
            self.suffix_length = ((code as u32) >> 1) as usize;

            *ctx.term_exists = (code & 1) == 0;
            self.start_byte_pos = self.suffixes.position();
            self.suffixes.skip_bytes(self.suffix_length as i32)?;
            // Must save ord before we skip over a sub-block in case we push, below:
            let prev_term_ord = self.term_ord;
            if *ctx.term_exists {
                self.state.term_block_ord += 1;
                self.term_ord += 1;
                self.sub_code = 0;
            } else {
                self.sub_code = self.suffixes.read_v_long()?;
                self.term_ord += self.suffixes.read_v_long()?;
                self.last_sub_fp = self.fp - self.sub_code;
            }

            // Compare suffix and target.
            let cmp = self.suffix().cmp(self.target_suffix(target));

            match cmp {
                Ordering::Less => {
                    // Current entry is still before the target;
                    // keep scanning
                }
                Ordering::Greater => {
                    // Done!  Current entry is after target --
                    // return NOT_FOUND:
                    self.fill_term(ctx.term);

                    let descend = if !exact_only && !*ctx.term_exists {
                        // We are on a sub-block, and caller wants
                        // us to position to the next term after
                        // the target, so we must recurse into the
                        // sub-frame(s):
                        Some((self.prefix_length + self.suffix_length, prev_term_ord))
                    } else {
                        None
                    };

                    return Ok(NonLeafScan {
                        status: SeekStatus::NOT_FOUND,
                        descend,
                    });
                }
                Ordering::Equal => {
                    // Exact match!

                    // This cannot be a sub-block because we
                    // would have followed the index to this
                    // sub-block from the start:

                    java_assert!(*ctx.term_exists);
                    self.fill_term(ctx.term);
                    return Ok(NonLeafScan {
                        status: SeekStatus::FOUND,
                        descend: None,
                    });
                }
            }
        }

        // It is possible (and OK) that terms index pointed us
        // at this block, but, we scanned the entire block and
        // did not find the term to position to.  This happens
        // when the target is after the last term in the block
        // (but, before the next term in the index).  EG
        // target could be foozzz, and terms index pointed us
        // to the foo* block, but the last term in this block
        // was fooz (and, eg, first term in the next block will
        // bee fop).
        if exact_only {
            self.fill_term(ctx.term);
        }

        // TODO: not consistent that in the
        // not-exact case we don't next() into the next
        // frame here
        Ok(NonLeafScan {
            status: SeekStatus::END,
            descend: None,
        })
    }

    /// Port of `fillTerm()`.
    fn fill_term(&self, term: &mut TermBuilder) {
        let term_length = self.prefix_length + self.suffix_length;
        term.set_length(term_length);
        term.bytes[self.prefix_length..term_length].copy_from_slice(self.suffix());
    }
}
