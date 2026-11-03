//! `SegmentTermsEnum` and `SegmentTermsEnumFrame` ported from
//! `org.apache.lucene.codecs.lucene103.blocktree`.
//!
//! The cursor that walks one field's terms: a stack of frames, each holding one
//! decoded block of the `.tim` file, navigated through the `.tip` trie.

use crate::codecs::lucene103::blocktree::{CompressionAlgorithm, Stats};
use crate::codecs::lucene103::field_reader::FieldReader;
use crate::codecs::lucene103::trie_reader::{Node, TrieReader};
use crate::codecs::term_state::BlockTermState;
use crate::error::{LuceneError, Result};
use crate::index::terms::{SeekStatus, TermState, Terms, TermsEnum};
use crate::index::ImpactsEnum;
use crate::index::{IndexOptions, PostingsEnum};
use crate::store::{ByteArrayDataInput, DataInput, IndexInput};
use crate::util::{AttributeSource, BytesRef, IOBooleanSupplier};
use std::sync::Arc;

/// One decoded block of the terms dictionary.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnumFrame`.
///
/// **Divergence from Lucene 10.5.0.** Java's frame holds a back-reference to its
/// `SegmentTermsEnum` and reads and writes the cursor's shared `term` buffer
/// directly. Rust cannot express that cycle, so the frame is plain data and the
/// cursor passes the shared state in as a parameter. The decoded fields and the
/// block layout are unchanged.
pub struct SegmentTermsEnumFrame {
    /// This frame's index in the stack.
    pub ord: usize,
    /// Whether this block holds terms.
    pub has_terms: bool,
    /// `has_terms` as the trie recorded it, before floor navigation changed it.
    pub has_terms_orig: bool,
    /// Whether this block is one of several floor blocks sharing a prefix.
    pub is_floor: bool,
    /// The trie node that pointed at this block, when there was one.
    pub node: Option<Node>,

    /// Where this block was loaded from.
    pub fp: i64,
    /// Where the block originally started, before floor navigation.
    pub fp_orig: i64,
    /// Where this block ends.
    pub fp_end: i64,

    // Java's frame keeps a `ByteArrayDataInput` next to each of these buffers
    // (`SegmentTermsEnumFrame.java:49`, `:52`, `:56`, `:97`) and resets it over
    // the buffer it sits beside — `suffixesReader.reset(suffixBytes, 0,
    // numSuffixBytes)` and its three siblings (`:201`, `:214`, `:231`, `:247`).
    // A struct that
    // holds both a buffer and a view of that same buffer is a self-reference
    // Rust forbids, so the frame keeps the view's two fields — position and
    // limit — beside the buffer instead, and [`ByteArrayDataInput`] is rebuilt
    // around them for each read. The state is exactly Java's, only spelled
    // inline; the frame already stores `floor_data_pos` this way.
    suffix_bytes: Vec<u8>,
    /// Read position inside `suffix_bytes`.
    suffixes_pos: usize,
    /// Where this block's suffix bytes end inside `suffix_bytes`. This is what
    /// Java's `suffixesReader.length()` reports.
    pub(crate) suffixes_limit: usize,
    suffix_length_bytes: Vec<u8>,
    /// Read position inside `suffix_length_bytes`.
    suffix_lengths_pos: usize,
    /// Where this block's suffix lengths end inside `suffix_length_bytes`.
    /// This is what Java's `suffixLengthsReader.length()` reports.
    pub(crate) suffix_lengths_limit: usize,
    stat_bytes: Vec<u8>,
    /// Read position inside `stat_bytes`.
    stats_pos: usize,
    /// Where this block's statistics end inside `stat_bytes`. This is what
    /// Java's `statsReader.length()` reports.
    pub(crate) stats_limit: usize,
    stats_singleton_run_length: i32,
    /// How many bytes of the block the suffixes and their lengths occupy.
    pub total_suffix_bytes: i64,
    /// Which algorithm compressed this block's suffix bytes.
    pub compression_alg: CompressionAlgorithm,

    /// Floor data, copied out of the `.tip` file.
    floor_data: Vec<u8>,
    /// Read position inside `floor_data`.
    floor_data_pos: usize,

    /// Length of the prefix every term in this block shares.
    pub prefix_length: usize,
    /// How many entries — terms or sub-blocks — the block holds.
    pub ent_count: i32,
    /// Which entry comes next, or `-1` when the block is not loaded.
    pub next_ent: i32,
    /// Whether this is the last block of its floor run.
    pub is_last_in_floor: bool,
    /// Whether every entry is a term.
    pub is_leaf_block: bool,
    /// Whether every suffix has the same length.
    pub all_equal: bool,
    /// Pointer to the last sub-block seen.
    pub last_sub_fp: i64,
    /// Label at which the next floor block starts, or `256` past the last.
    pub next_floor_label: i32,
    /// How many floor blocks still follow.
    pub num_follow_floor_blocks: i32,

    /// How many terms of this block have had their metadata decoded.
    pub meta_data_upto: i32,
    /// The term state the postings reader fills in.
    pub state: BlockTermState,
    bytes: Vec<u8>,
    /// Read position inside `bytes`.
    bytes_pos: usize,
    /// Where this block's term metadata ends inside `bytes`.
    bytes_limit: usize,

    /// Length of the current entry's suffix.
    pub suffix_length: usize,
    /// Where the current entry's suffix starts inside `suffix_bytes`.
    pub start_byte_pos: usize,
    /// Sub-block pointer of the current entry, or `0` for a term.
    pub sub_code: i64,
}

impl std::fmt::Debug for SegmentTermsEnumFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentTermsEnumFrame")
            .field("ord", &self.ord)
            .field("fp", &self.fp)
            .field("ent_count", &self.ent_count)
            .field("next_ent", &self.next_ent)
            .field("is_leaf_block", &self.is_leaf_block)
            .finish_non_exhaustive()
    }
}

impl SegmentTermsEnumFrame {
    /// Runs `f` over this block's suffix bytes and saves the read position
    /// back.
    ///
    /// Stands in for Java's `suffixesReader` field, which cannot be a field
    /// here; see the note beside `suffix_bytes`.
    fn with_suffixes<T>(
        &mut self,
        f: impl FnOnce(&mut ByteArrayDataInput<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut reader = ByteArrayDataInput::with_range(&self.suffix_bytes, 0, self.suffixes_limit);
        reader.set_position(self.suffixes_pos);
        let out = f(&mut reader);
        let position = reader.position();
        self.suffixes_pos = position;
        out
    }

    /// Runs `f` over this block's suffix lengths and saves the read position
    /// back.
    ///
    /// Stands in for Java's `suffixLengthsReader` field.
    fn with_suffix_lengths<T>(
        &mut self,
        f: impl FnOnce(&mut ByteArrayDataInput<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut reader =
            ByteArrayDataInput::with_range(&self.suffix_length_bytes, 0, self.suffix_lengths_limit);
        reader.set_position(self.suffix_lengths_pos);
        let out = f(&mut reader);
        let position = reader.position();
        self.suffix_lengths_pos = position;
        out
    }

    /// Runs `f` over this block's term statistics and saves the read position
    /// back.
    ///
    /// Stands in for Java's `statsReader` field.
    fn with_stats<T>(
        &mut self,
        f: impl FnOnce(&mut ByteArrayDataInput<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut reader = ByteArrayDataInput::with_range(&self.stat_bytes, 0, self.stats_limit);
        reader.set_position(self.stats_pos);
        let out = f(&mut reader);
        let position = reader.position();
        self.stats_pos = position;
        out
    }

    /// Creates an empty frame at stack position `ord`.
    pub fn new(ord: usize, state: BlockTermState) -> Self {
        Self {
            ord,
            has_terms: false,
            has_terms_orig: false,
            is_floor: false,
            node: None,
            fp: 0,
            fp_orig: 0,
            fp_end: 0,
            suffix_bytes: Vec::new(),
            suffixes_pos: 0,
            suffixes_limit: 0,
            suffix_length_bytes: Vec::new(),
            suffix_lengths_pos: 0,
            suffix_lengths_limit: 0,
            stat_bytes: Vec::new(),
            stats_pos: 0,
            stats_limit: 0,
            stats_singleton_run_length: 0,
            total_suffix_bytes: 0,
            compression_alg: CompressionAlgorithm::NoCompression,
            floor_data: Vec::new(),
            floor_data_pos: 0,
            prefix_length: 0,
            ent_count: 0,
            next_ent: -1,
            is_last_in_floor: false,
            is_leaf_block: false,
            all_equal: false,
            last_sub_fp: -1,
            next_floor_label: 0,
            num_follow_floor_blocks: 0,
            meta_data_upto: 0,
            state,
            bytes: Vec::new(),
            bytes_pos: 0,
            bytes_limit: 0,
            suffix_length: 0,
            start_byte_pos: 0,
            sub_code: 0,
        }
    }

    /// Records this block's floor data, read out of the term index.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.setFloorData`.
    pub fn set_floor_data(&mut self, floor_data: Vec<u8>) -> Result<()> {
        let mut reader = ByteArrayDataInput::new(&floor_data);
        self.num_follow_floor_blocks = reader.read_v_int()?;
        self.next_floor_label = i32::from(reader.read_byte()?);
        self.floor_data_pos = reader.position();
        self.floor_data = floor_data;
        Ok(())
    }

    /// Returns how many terms of this block have been passed.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.getTermBlockOrd`.
    pub fn get_term_block_ord(&self) -> i32 {
        if self.is_leaf_block {
            self.next_ent
        } else {
            self.state.term_block_ord
        }
    }

    /// Reads this frame's block out of the terms file.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.loadBlock`, and the definition of
    /// the on-disk block layout.
    pub fn load_block(&mut self, terms_in: &mut dyn IndexInput) -> Result<()> {
        if self.next_ent != -1 {
            return Ok(());
        }
        terms_in.seek(self.fp)?;

        let code = terms_in.read_v_int()?;
        self.ent_count = code >> 1;
        if self.ent_count <= 0 {
            return Err(LuceneError::corrupt_index(
                format!("invalid entry count: {}", self.ent_count),
                "terms dictionary",
            ));
        }
        self.is_last_in_floor = code & 1 != 0;

        let start_suffix_fp = terms_in.file_pointer();
        let code_l = terms_in.read_v_long()?;
        self.is_leaf_block = code_l & 0x04 != 0;
        let num_suffix_bytes = ((code_l as u64) >> 3) as usize;
        if self.suffix_bytes.len() < num_suffix_bytes {
            self.suffix_bytes.resize(num_suffix_bytes, 0);
        }
        let compression = CompressionAlgorithm::by_code((code_l & 0x03) as i32)?;
        self.compression_alg = compression;
        compression.read(terms_in, &mut self.suffix_bytes, num_suffix_bytes)?;
        self.suffixes_pos = 0;
        self.suffixes_limit = num_suffix_bytes;

        let mut num_suffix_length_bytes = terms_in.read_v_int()?;
        self.all_equal = num_suffix_length_bytes & 0x01 != 0;
        num_suffix_length_bytes >>= 1;
        let n = num_suffix_length_bytes.max(0) as usize;
        if self.suffix_length_bytes.len() < n {
            self.suffix_length_bytes.resize(n, 0);
        }
        if self.all_equal {
            let b = terms_in.read_byte()?;
            self.suffix_length_bytes[..n].fill(b);
        } else {
            terms_in.read_bytes(&mut self.suffix_length_bytes, 0, n)?;
        }
        self.suffix_lengths_pos = 0;
        self.suffix_lengths_limit = n;
        self.total_suffix_bytes = terms_in.file_pointer() - start_suffix_fp;

        let num_stat_bytes = terms_in.read_v_int()?.max(0) as usize;
        if self.stat_bytes.len() < num_stat_bytes {
            self.stat_bytes.resize(num_stat_bytes, 0);
        }
        terms_in.read_bytes(&mut self.stat_bytes, 0, num_stat_bytes)?;
        self.stats_pos = 0;
        self.stats_limit = num_stat_bytes;
        self.stats_singleton_run_length = 0;

        self.meta_data_upto = 0;
        self.state.term_block_ord = 0;
        self.next_ent = 0;
        self.last_sub_fp = -1;

        let num_meta_bytes = terms_in.read_v_int()?.max(0) as usize;
        if self.bytes.len() < num_meta_bytes {
            self.bytes.resize(num_meta_bytes, 0);
        }
        terms_in.read_bytes(&mut self.bytes, 0, num_meta_bytes)?;
        self.bytes_pos = 0;
        self.bytes_limit = num_meta_bytes;

        self.fp_end = terms_in.file_pointer();
        Ok(())
    }

    /// Moves to the next floor block of this run.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.loadNextFloorBlock`.
    pub fn load_next_floor_block(&mut self, terms_in: &mut dyn IndexInput) -> Result<()> {
        self.fp = self.fp_end;
        self.next_ent = -1;
        self.load_block(terms_in)
    }

    /// Rewinds this frame to the start of its block.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.rewind`.
    pub fn rewind(&mut self) -> Result<()> {
        self.fp = self.fp_orig;
        self.next_ent = -1;
        self.has_terms = self.has_terms_orig;
        if self.is_floor {
            let floor_data = std::mem::take(&mut self.floor_data);
            self.set_floor_data(floor_data)?;
        }
        Ok(())
    }

    /// Advances within a floor run until the block that could hold `target`.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.scanToFloorFrame`.
    pub fn scan_to_floor_frame(&mut self, target: &[u8]) -> Result<()> {
        if !self.is_floor || target.len() <= self.prefix_length {
            return Ok(());
        }
        let target_label = i32::from(target[self.prefix_length]);
        if target_label < self.next_floor_label {
            return Ok(());
        }

        // The reader borrows `floor_data`, so the fields it writes are taken
        // apart from it here; Java simply aliases the frame's own array.
        let Self {
            floor_data,
            floor_data_pos,
            fp_orig,
            has_terms,
            is_last_in_floor,
            num_follow_floor_blocks,
            next_floor_label,
            ..
        } = self;
        let mut new_fp;
        let mut reader = ByteArrayDataInput::new(floor_data);
        reader.set_position(*floor_data_pos);
        loop {
            let code = reader.read_v_long()?;
            new_fp = *fp_orig + ((code as u64) >> 1) as i64;
            *has_terms = code & 1 != 0;
            *is_last_in_floor = *num_follow_floor_blocks == 1;
            *num_follow_floor_blocks -= 1;
            if *is_last_in_floor {
                *next_floor_label = 256;
                break;
            }
            *next_floor_label = i32::from(reader.read_byte()?);
            if target_label < *next_floor_label {
                break;
            }
        }
        let position = reader.position();
        self.floor_data_pos = position;

        if new_fp != self.fp {
            self.next_ent = -1;
            self.fp = new_fp;
        }
        Ok(())
    }

    /// Advances within the block until the sub-block at `sub_fp`.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.scanToSubBlock`.
    pub fn scan_to_sub_block(&mut self, sub_fp: i64, term: &mut Vec<u8>) -> Result<()> {
        if self.last_sub_fp == sub_fp {
            return Ok(());
        }
        while self.next_ent < self.ent_count {
            self.next_non_leaf_entry(term)?;
            if self.last_sub_fp == sub_fp {
                return Ok(());
            }
        }
        Err(LuceneError::corrupt_index(
            format!("sub-block at fp {sub_fp} was not found in its parent block"),
            "terms dictionary",
        ))
    }

    /// Reads the next entry of a leaf block into `term`.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.nextLeaf`.
    pub fn next_leaf(&mut self, term: &mut Vec<u8>) -> Result<()> {
        self.next_ent += 1;
        self.suffix_length = self.with_suffix_lengths(|r| r.read_v_int())?.max(0) as usize;
        self.start_byte_pos = self.suffixes_pos;
        term.truncate(self.prefix_length);
        term.resize(self.prefix_length + self.suffix_length, 0);
        let (prefix_length, suffix_length) = (self.prefix_length, self.suffix_length);
        self.with_suffixes(|r| r.read_bytes(&mut term[..], prefix_length, suffix_length))?;
        Ok(())
    }

    /// Reads the next entry of a non-leaf block into `term`.
    ///
    /// Returns `true` when the entry is a sub-block rather than a term.
    /// Equivalent to the body of `SegmentTermsEnumFrame.nextNonLeaf`, without
    /// its floor-block loop, which the cursor drives.
    pub fn next_non_leaf_entry(&mut self, term: &mut Vec<u8>) -> Result<bool> {
        self.next_ent += 1;
        let code = self.with_suffix_lengths(|r| r.read_v_int())?;
        self.suffix_length = ((code as u32) >> 1) as usize;
        self.start_byte_pos = self.suffixes_pos;
        term.truncate(self.prefix_length);
        term.resize(self.prefix_length + self.suffix_length, 0);
        let (prefix_length, suffix_length) = (self.prefix_length, self.suffix_length);
        self.with_suffixes(|r| r.read_bytes(&mut term[..], prefix_length, suffix_length))?;

        if code & 1 == 0 {
            // A term.
            self.sub_code = 0;
            self.state.term_block_ord += 1;
            Ok(false)
        } else {
            // A sub-block: its pointer is stored as a delta from this block.
            self.sub_code = self.with_suffix_lengths(|r| r.read_v_long())?;
            self.last_sub_fp = self.fp - self.sub_code;
            Ok(true)
        }
    }

    /// Returns whether `target` really does start with this block's prefix.
    ///
    /// Java uses this only inside an assertion; this port uses it the same way,
    /// from `debug_assert!`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnumFrame#prefixMatches`.
    pub fn prefix_matches(&self, target: &[u8], term: &[u8]) -> bool {
        for byte_pos in 0..self.prefix_length {
            if target.get(byte_pos) != term.get(byte_pos) {
                return false;
            }
        }
        true
    }

    /// Writes the current entry — this block's prefix followed by the suffix the
    /// scan stopped on — into the cursor's term buffer.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnumFrame#fillTerm`.
    pub fn fill_term(&self, term: &mut Vec<u8>) {
        let term_length = self.prefix_length + self.suffix_length;
        term.resize(term_length, 0);
        term[self.prefix_length..term_length].copy_from_slice(
            &self.suffix_bytes[self.start_byte_pos..self.start_byte_pos + self.suffix_length],
        );
    }

    /// Scans this block for `target`, picking the search this block's shape
    /// allows: a binary search when every suffix is the same length, a linear
    /// scan otherwise, and the non-leaf scan when the block also holds
    /// sub-blocks.
    ///
    /// The second half of the result is `true` when the caller must descend
    /// into the sub-block the scan stopped on; see
    /// [`Self::scan_to_term_non_leaf`].
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnumFrame#scanToTerm`.
    ///
    /// # Errors
    ///
    /// Returns whatever error decoding an entry of the block raised.
    pub fn scan_to_term(
        &mut self,
        target: &[u8],
        exact_only: bool,
        term: &mut Vec<u8>,
        term_exists: &mut bool,
    ) -> Result<(SeekStatus, bool)> {
        if self.is_leaf_block {
            if self.all_equal {
                self.binary_search_term_leaf(target, exact_only, term, term_exists)
            } else {
                self.scan_to_term_leaf(target, exact_only, term, term_exists)
            }
        } else {
            self.scan_to_term_non_leaf(target, exact_only, term, term_exists)
        }
    }

    /// Returns the part of `target` that lies past this block's prefix.
    fn target_suffix<'a>(&self, target: &'a [u8]) -> &'a [u8] {
        &target[self.prefix_length.min(target.len())..]
    }

    /// Scans a leaf block entry by entry until an entry reaches `target`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnumFrame#scanToTermLeaf`.
    ///
    /// # Errors
    ///
    /// Returns whatever error decoding an entry of the block raised.
    pub fn scan_to_term_leaf(
        &mut self,
        target: &[u8],
        exact_only: bool,
        term: &mut Vec<u8>,
        term_exists: &mut bool,
    ) -> Result<(SeekStatus, bool)> {
        debug_assert!(self.next_ent != -1);

        *term_exists = true;
        self.sub_code = 0;

        if self.next_ent == self.ent_count {
            if exact_only {
                self.fill_term(term);
            }
            return Ok((SeekStatus::END, false));
        }

        debug_assert!(self.prefix_matches(target, term));

        let target_suffix = self.target_suffix(target);
        loop {
            self.next_ent += 1;
            self.suffix_length = self.with_suffix_lengths(|r| r.read_v_int())?.max(0) as usize;
            self.start_byte_pos = self.suffixes_pos;
            self.suffixes_pos = self.start_byte_pos + self.suffix_length;

            let suffix =
                &self.suffix_bytes[self.start_byte_pos..self.start_byte_pos + self.suffix_length];
            match suffix.cmp(target_suffix) {
                // Still before the target; keep scanning.
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Greater => {
                    self.fill_term(term);
                    return Ok((SeekStatus::NOT_FOUND, false));
                }
                std::cmp::Ordering::Equal => {
                    // This cannot be a sub-block: the index would have taken us
                    // straight into it.
                    self.fill_term(term);
                    return Ok((SeekStatus::FOUND, false));
                }
            }
            if self.next_ent >= self.ent_count {
                break;
            }
        }

        // The index can legitimately point at a block that does not hold the
        // target: the target is after this block's last term but before the
        // first term of the next block.
        if exact_only {
            self.fill_term(term);
        }
        Ok((SeekStatus::END, false))
    }

    /// Binary-searches a leaf block whose suffixes all have the same length.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnumFrame#binarySearchTermLeaf`.
    ///
    /// # Errors
    ///
    /// Returns whatever error decoding an entry of the block raised.
    pub fn binary_search_term_leaf(
        &mut self,
        target: &[u8],
        exact_only: bool,
        term: &mut Vec<u8>,
        term_exists: &mut bool,
    ) -> Result<(SeekStatus, bool)> {
        debug_assert!(self.next_ent != -1);

        *term_exists = true;
        self.sub_code = 0;

        if self.next_ent == self.ent_count {
            if exact_only {
                self.fill_term(term);
            }
            return Ok((SeekStatus::END, false));
        }

        debug_assert!(self.prefix_matches(target, term));

        self.suffix_length = self.with_suffix_lengths(|r| r.read_v_int())?.max(0) as usize;
        let target_suffix = self.target_suffix(target);
        let mut start = self.next_ent;
        let mut end = self.ent_count - 1;
        let mut cmp = std::cmp::Ordering::Equal;
        while start <= end {
            let mid = ((start as u32 + end as u32) >> 1) as i32;
            self.next_ent = mid + 1;
            self.start_byte_pos = mid as usize * self.suffix_length;

            let suffix =
                &self.suffix_bytes[self.start_byte_pos..self.start_byte_pos + self.suffix_length];
            cmp = suffix.cmp(target_suffix);
            match cmp {
                std::cmp::Ordering::Less => start = mid + 1,
                std::cmp::Ordering::Greater => end = mid - 1,
                std::cmp::Ordering::Equal => {
                    self.suffixes_pos = self.start_byte_pos + self.suffix_length;
                    self.fill_term(term);
                    return Ok((SeekStatus::FOUND, false));
                }
            }
        }

        // As in `scan_to_term_leaf`, the index may point at a block that does
        // not hold the target at all.
        let seek_status = if end < self.ent_count - 1 {
            // The search ended on a smaller term while a greater one exists, so
            // step onto that greater term.
            if cmp == std::cmp::Ordering::Less {
                self.start_byte_pos += self.suffix_length;
                self.next_ent += 1;
            }
            self.suffixes_pos = self.start_byte_pos + self.suffix_length;
            self.fill_term(term);
            SeekStatus::NOT_FOUND
        } else {
            self.suffixes_pos = self.start_byte_pos + self.suffix_length;
            if exact_only {
                self.fill_term(term);
            }
            SeekStatus::END
        };
        Ok((seek_status, false))
    }

    /// Scans a block that also holds sub-blocks.
    ///
    /// The `bool` half of the result is `true` when the scan stopped past the
    /// target on a *sub-block* and the caller did not ask for an exact match:
    /// the cursor must then descend into that sub-block, and keep descending,
    /// until it stands on the first term after the target.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's frame holds a back-reference to
    /// its `SegmentTermsEnum` and performs that descent itself
    /// (`SegmentTermsEnumFrame.java:832-843`). Rust cannot express that cycle —
    /// this file's frames are plain data — so the frame reports that the descent
    /// is due and the cursor performs it. The blocks visited and the term the
    /// cursor ends on are unchanged.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnumFrame#scanToTermNonLeaf`.
    ///
    /// # Errors
    ///
    /// Returns whatever error decoding an entry of the block raised.
    pub fn scan_to_term_non_leaf(
        &mut self,
        target: &[u8],
        exact_only: bool,
        term: &mut Vec<u8>,
        term_exists: &mut bool,
    ) -> Result<(SeekStatus, bool)> {
        debug_assert!(self.next_ent != -1);

        if self.next_ent == self.ent_count {
            if exact_only {
                self.fill_term(term);
                *term_exists = self.sub_code == 0;
            }
            return Ok((SeekStatus::END, false));
        }

        debug_assert!(self.prefix_matches(target, term));

        let target_suffix = self.target_suffix(target);
        while self.next_ent < self.ent_count {
            self.next_ent += 1;

            let code = self.with_suffix_lengths(|r| r.read_v_int())?;
            self.suffix_length = ((code as u32) >> 1) as usize;
            self.start_byte_pos = self.suffixes_pos;
            self.suffixes_pos = self.start_byte_pos + self.suffix_length;
            *term_exists = code & 1 == 0;
            if *term_exists {
                self.state.term_block_ord += 1;
                self.sub_code = 0;
            } else {
                self.sub_code = self.with_suffix_lengths(|r| r.read_v_long())?;
                self.last_sub_fp = self.fp - self.sub_code;
            }

            let suffix =
                &self.suffix_bytes[self.start_byte_pos..self.start_byte_pos + self.suffix_length];
            match suffix.cmp(target_suffix) {
                // Still before the target; keep scanning.
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Greater => {
                    self.fill_term(term);
                    let descend = !exact_only && !*term_exists;
                    return Ok((SeekStatus::NOT_FOUND, descend));
                }
                std::cmp::Ordering::Equal => {
                    // This cannot be a sub-block: the index would have taken us
                    // straight into it.
                    debug_assert!(*term_exists);
                    self.fill_term(term);
                    return Ok((SeekStatus::FOUND, false));
                }
            }
        }

        // As in `scan_to_term_leaf`, the index may point at a block that does
        // not hold the target at all.
        if exact_only {
            self.fill_term(term);
        }
        Ok((SeekStatus::END, false))
    }

    /// Decodes the statistics and postings metadata of every term passed so far.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.decodeMetaData`.
    pub fn decode_meta_data(
        &mut self,
        field_info: &crate::index::FieldInfo,
        postings_reader: &dyn crate::codecs::postings::PostingsReaderBase,
    ) -> Result<()> {
        let limit = self.get_term_block_ord();
        let mut absolute = self.meta_data_upto == 0;
        while self.meta_data_upto < limit {
            if self.stats_singleton_run_length > 0 {
                self.state.doc_freq = 1;
                self.state.total_term_freq = 1;
                self.stats_singleton_run_length -= 1;
            } else {
                let token = self.with_stats(|r| r.read_v_int())?;
                if token & 1 == 1 {
                    // A run of terms that each appear in exactly one document.
                    self.state.doc_freq = 1;
                    self.state.total_term_freq = 1;
                    self.stats_singleton_run_length = ((token as u32) >> 1) as i32;
                } else {
                    self.state.doc_freq = ((token as u32) >> 1) as i32;
                    self.state.total_term_freq = if field_info.index_options == IndexOptions::DOCS {
                        i64::from(self.state.doc_freq)
                    } else {
                        i64::from(self.state.doc_freq) + self.with_stats(|r| r.read_v_long())?
                    };
                }
            }
            // `bytes` and `state` are borrowed at once, which Java gets for
            // free by aliasing; the position is written back whether or not the
            // decode succeeded, exactly as Java's shared reader leaves it.
            let Self {
                bytes,
                bytes_pos,
                bytes_limit,
                state,
                ..
            } = self;
            let mut reader = ByteArrayDataInput::with_range(bytes, 0, *bytes_limit);
            reader.set_position(*bytes_pos);
            let decoded = postings_reader.decode_term(&mut reader, field_info, state, absolute);
            *bytes_pos = reader.position();
            decoded?;
            self.meta_data_upto += 1;
            absolute = false;
        }
        self.state.term_block_ord = self.meta_data_upto;
        Ok(())
    }
}

/// What the index walk of [`SegmentTermsEnum::prepare_seek_exact_state`] found,
/// before any block of the terms file was read.
enum PrepareSeek {
    /// The target cannot be a term of this field.
    NoMatch,
    /// The cursor already stands on the target.
    AlreadyPositioned,
    /// The block that would hold the target has been located; scanning it
    /// settles whether the term is really there.
    ScanBlock,
}

/// Walks the terms of one field.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum`.
///
/// **Divergence from Lucene 10.5.0.** Java's cursor keeps its own `IndexInput`
/// over the `.tim` file and its frames read through a back-reference to it. This
/// port holds that input in the cursor and passes it to each frame, which is
/// the same single reader with the ownership made explicit. As in Java, the
/// input is cloned lazily, by [`init_index_input`](Self::init_index_input), the
/// first time a block is loaded or prefetched. `seek_ord` is unsupported here
/// as it is in Java, and `intersect` lives in its own cursor.
pub struct SegmentTermsEnum {
    field: FieldReader,
    /// This cursor's clone of the `.tim` input, or `None` until a block is
    /// first loaded or prefetched.
    ///
    /// Equivalent to `SegmentTermsEnum.in`, which is `null` until
    /// `initIndexInput()` runs.
    terms_in: Option<Box<dyn IndexInput>>,
    trie: TrieReader,
    /// The frame stack. Frame `n` holds a block whose prefix is `n` bytes deep.
    stack: Vec<SegmentTermsEnumFrame>,
    /// Index of the frame the cursor is positioned in.
    current: usize,
    /// The trie node reached at each stack depth.
    nodes: Vec<Node>,
    /// The current term.
    term: Vec<u8>,
    /// Whether the current position names a term rather than a sub-block.
    term_exists: bool,
    /// Whether the cursor has run past the last term.
    eof: bool,
    /// Whether the first frame has been pushed.
    started: bool,
    /// How long a prefix of the current term the last seek left valid in the
    /// index; a later seek can re-use the frames covering it.
    valid_index_prefix: usize,
    /// Ord of the frame the cursor stood in when the current operation began;
    /// frames at or below it are still positioned before the new target and
    /// need no rewind. `-1` means there was no prior seek.
    target_before_current_length: i64,
    attributes: AttributeSource,
}

impl std::fmt::Debug for SegmentTermsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentTermsEnum")
            .field("field", &self.field.field_info().name)
            .field("term_exists", &self.term_exists)
            .field("eof", &self.eof)
            .finish_non_exhaustive()
    }
}

impl SegmentTermsEnum {
    /// Opens a cursor over `field`.
    pub fn new(field: FieldReader) -> Result<Self> {
        let trie = field.new_reader()?;

        Ok(Self {
            field,
            terms_in: None,
            trie,
            stack: Vec::new(),
            current: 0,
            nodes: Vec::new(),
            term: Vec::new(),
            term_exists: false,
            eof: false,
            started: false,
            valid_index_prefix: 0,
            target_before_current_length: -1,
            attributes: AttributeSource::new(),
        })
    }

    /// Clones the terms dictionary input the first time this cursor needs it,
    /// so that a consumer that only seeks by `TermState` never pays for the
    /// clone.
    ///
    /// Equivalent to `SegmentTermsEnum#initIndexInput`
    /// (`SegmentTermsEnum.java:81-85`), which clones `fr.parent.termsIn` when
    /// `in` is still `null`. Java's frames call it from `loadBlock` and
    /// `prefetchBlock`; here the cursor calls it before handing the input to
    /// a frame, at the same two points.
    ///
    /// # Errors
    ///
    /// Propagates the error cloning the input raises.
    fn init_index_input(&mut self) -> Result<()> {
        if self.terms_in.is_none() {
            self.terms_in = Some(self.field.shared().terms_in.clone_input()?);
        }
        Ok(())
    }

    /// Whether [`init_index_input`](Self::init_index_input) has cloned the
    /// terms input yet; a test probe for Java's `in != null`.
    #[cfg(test)]
    pub(crate) fn index_input_initialized(&self) -> bool {
        self.terms_in.is_some()
    }

    /// The input [`init_index_input`](Self::init_index_input) cloned.
    ///
    /// Takes the field rather than `self` so that a frame of the stack can be
    /// borrowed beside it.
    fn index_input(terms_in: &mut Option<Box<dyn IndexInput>>) -> &mut dyn IndexInput {
        terms_in
            .as_deref_mut()
            .expect("INVARIANT: init_index_input runs before the terms input is read")
    }

    /// Returns the frame at stack position `ord`, creating it when needed.
    ///
    /// Equivalent to `SegmentTermsEnum.getFrame`.
    fn ensure_frame(&mut self, ord: usize) -> Result<()> {
        while self.stack.len() <= ord {
            let state = self.postings_reader_new_state()?;
            self.stack
                .push(SegmentTermsEnumFrame::new(self.stack.len(), state));
        }
        Ok(())
    }

    fn postings_reader_new_state(&self) -> Result<BlockTermState> {
        // Every frame needs its own term state; the postings reader knows the
        // shape the format uses.
        Ok(BlockTermState::default())
    }

    /// Pushes a frame for the block at `fp`, whose terms share a `length`-byte
    /// prefix.
    ///
    /// Equivalent to `SegmentTermsEnum.pushFrame(Node, long, int)`.
    fn push_frame(&mut self, node: Option<Node>, fp: i64, length: usize) -> Result<usize> {
        let ord = if self.started { self.current + 1 } else { 0 };
        self.ensure_frame(ord)?;
        let frame = &mut self.stack[ord];
        frame.node = node;

        if frame.fp_orig == fp && frame.next_ent != -1 {
            // The frame already holds this block; rewind it rather than reload.
            if frame.ord as i64 > self.target_before_current_length {
                frame.rewind()?;
            }
        } else {
            frame.next_ent = -1;
            frame.prefix_length = length;
            frame.state.term_block_ord = 0;
            frame.fp = fp;
            frame.fp_orig = fp;
            frame.last_sub_fp = -1;
        }
        self.current = ord;
        self.started = true;
        Ok(ord)
    }

    /// Pushes a frame for the block a trie node points at, reading its floor
    /// data when it has any.
    ///
    /// Equivalent to `SegmentTermsEnum.pushFrame(Node, int)`.
    fn push_frame_from_node(&mut self, node: Node, length: usize) -> Result<usize> {
        let floor_data = if node.is_floor() {
            // The floor data runs to the end of the field's slice of the index,
            // which the trie reader addresses from zero.
            let end = self.field.index_end() - self.field.index_start();
            let len = (end - node.floor_data_fp).max(0) as usize;
            Some(self.trie.floor_data(&node, len.min(1 << 16))?)
        } else {
            None
        };

        let ord = self.push_frame(Some(node), node.output_fp, length)?;
        let frame = &mut self.stack[ord];
        frame.has_terms = node.has_terms;
        frame.has_terms_orig = node.has_terms;
        frame.is_floor = node.is_floor();
        if let Some(floor_data) = floor_data {
            frame.set_floor_data(floor_data)?;
        }
        Ok(ord)
    }

    /// Loads the current frame's block.
    fn load_current_block(&mut self) -> Result<()> {
        self.init_index_input()?;
        let terms_in = Self::index_input(&mut self.terms_in);
        self.stack[self.current].load_block(terms_in)
    }

    /// Positions the cursor at the first term of the field.
    fn start(&mut self) -> Result<()> {
        let root = self.trie.root;
        self.nodes.clear();
        self.nodes.push(root);
        self.push_frame_from_node(root, 0)?;
        self.load_current_block()
    }

    /// Returns the trie node slot for depth `ord`, growing the array first.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum#getNode`.
    fn get_node(&mut self, ord: usize) -> &mut Node {
        if ord >= self.nodes.len() {
            self.nodes.resize(ord + 1, Node::default());
        }
        &mut self.nodes[ord]
    }

    /// Clears the end-of-input flag.
    ///
    /// Java calls this from an assertion, so that the flag is only maintained
    /// when assertions are on; this port keeps the flag itself and returns the
    /// same `true` so the call reads the same way.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum#clearEOF`.
    fn clear_eof(&mut self) -> bool {
        self.eof = false;
        true
    }

    /// Raises the end-of-input flag.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum#setEOF`.
    fn set_eof(&mut self) -> bool {
        self.eof = true;
        true
    }

    /// Hints that the current frame's block is about to be read.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnumFrame#prefetchBlock`.
    fn prefetch_block(&mut self) -> Result<()> {
        if self.stack[self.current].next_ent != -1 {
            // Already loaded.
            return Ok(());
        }
        // Clone the input lazily, so that consumers that only seek by
        // `TermState` do not pay for it.
        self.init_index_input()?;
        let fp = self.stack[self.current].fp;
        Self::index_input(&mut self.terms_in).prefetch(fp, 1)
    }

    /// Reads the next entry of the current frame, moving on to the next floor
    /// block when this one is exhausted.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.next()`, whose non-leaf half loops
    /// over the floor blocks; the loop lives here because the frame cannot
    /// reach the terms file on its own in this port.
    fn frame_next(&mut self) -> Result<bool> {
        loop {
            if self.stack[self.current].is_leaf_block {
                let mut term = std::mem::take(&mut self.term);
                let result = self.stack[self.current].next_leaf(&mut term);
                self.term = term;
                result?;
                self.term_exists = true;
                return Ok(false);
            }
            if self.stack[self.current].next_ent == self.stack[self.current].ent_count {
                self.init_index_input()?;
                let terms_in = Self::index_input(&mut self.terms_in);
                self.stack[self.current].load_next_floor_block(terms_in)?;
                continue;
            }
            let mut term = std::mem::take(&mut self.term);
            let result = self.stack[self.current].next_non_leaf_entry(&mut term);
            self.term = term;
            let is_sub_block = result?;
            self.term_exists = !is_sub_block;
            return Ok(is_sub_block);
        }
    }

    /// Scans the current frame's block for `target`, descending into sub-blocks
    /// when the scan asks for it.
    ///
    /// Equivalent to `SegmentTermsEnumFrame.scanToTerm` together with the
    /// descent its non-leaf half performs through its cursor.
    fn scan_to_term_current(&mut self, target: &[u8], exact_only: bool) -> Result<SeekStatus> {
        let mut term = std::mem::take(&mut self.term);
        let mut term_exists = self.term_exists;
        let result =
            self.stack[self.current].scan_to_term(target, exact_only, &mut term, &mut term_exists);
        self.term = term;
        self.term_exists = term_exists;
        let (status, descend) = result?;

        if descend {
            // We stopped past the target on a sub-block, and the caller wants
            // the next term rather than an exact match, so recurse into the
            // sub-frame(s).
            let (sub_fp, length) = {
                let frame = &self.stack[self.current];
                (frame.last_sub_fp, frame.prefix_length + frame.suffix_length)
            };
            self.push_frame(None, sub_fp, length)?;
            self.load_current_block()?;
            while self.frame_next()? {
                let sub_fp = self.stack[self.current].last_sub_fp;
                let length = self.term.len();
                self.push_frame(None, sub_fp, length)?;
                self.load_current_block()?;
            }
        }
        Ok(status)
    }

    /// Writes `byte` at `index` of the term buffer, growing it when needed.
    ///
    /// Equivalent to `BytesRefBuilder.setByteAt` over the cursor's term, which
    /// Java can call past the term's current length because the builder keeps
    /// buffer and length apart.
    fn set_term_byte(&mut self, index: usize, byte: u8) {
        if self.term.len() <= index {
            self.term.resize(index + 1, 0);
        }
        self.term[index] = byte;
    }

    /// Walks the term index towards `target` without reading any block.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `prepareSeekExact` both walks
    /// the index and builds the `IOBooleanSupplier` it returns
    /// (`SegmentTermsEnum.java:229-473`). Rust cannot return a value borrowing
    /// `self` from a point the function then keeps running past, so the walk is
    /// this method and the supplier is built by its caller from the outcome the
    /// walk reports. The index walk and the state it leaves behind are
    /// unchanged.
    fn prepare_seek_exact_state(&mut self, target: &[u8]) -> Result<PrepareSeek> {
        if self.field.size()? > 0
            && (target < self.field.get_min().slice() || target > self.field.get_max().slice())
        {
            return Ok(PrepareSeek::NoMatch);
        }

        debug_assert!(self.clear_eof());
        self.clear_eof();

        let mut node;
        let mut target_upto;

        self.target_before_current_length = if self.started {
            self.current as i64
        } else {
            -1
        };

        if self.started {
            // We are already seek'd: find the common prefix of the new target
            // and the current term, and re-use the seek state that covers it.
            node = self.nodes[0];
            debug_assert!(node.has_output());
            target_upto = 0;

            let mut last_frame_ord = 0usize;
            debug_assert!(self.valid_index_prefix <= self.term.len());
            let target_limit = target.len().min(self.valid_index_prefix);

            let mut cmp = std::cmp::Ordering::Equal;
            while target_upto < target_limit {
                cmp = self.term[target_upto].cmp(&target[target_upto]);
                if cmp != std::cmp::Ordering::Equal {
                    break;
                }
                node = self.nodes[1 + target_upto];
                debug_assert_eq!(node.label, i32::from(target[target_upto]));
                if node.has_output() {
                    last_frame_ord += 1;
                }
                target_upto += 1;
            }

            if cmp == std::cmp::Ordering::Equal {
                // Compare the rest of the term, without saving any state: this
                // only decides whether the target is before, at or after it.
                cmp = self.term[target_upto..].cmp(&target[target_upto..]);
            }

            match cmp {
                std::cmp::Ordering::Less => {
                    // Common case: the target is after the current term, ie the
                    // caller is seeking terms in order.
                    self.current = last_frame_ord;
                }
                std::cmp::Ordering::Greater => {
                    // The target is before the current term: keep the frame but
                    // rewind it, so the scan starts from the block's first entry.
                    self.target_before_current_length = last_frame_ord as i64;
                    self.current = last_frame_ord;
                    self.stack[self.current].rewind()?;
                }
                std::cmp::Ordering::Equal => {
                    debug_assert_eq!(self.term.len(), target.len());
                    if self.term_exists {
                        return Ok(PrepareSeek::AlreadyPositioned);
                    }
                }
            }
        } else {
            self.target_before_current_length = -1;
            let root = self.trie.root;
            *self.get_node(0) = root;
            node = root;
            // The empty prefix must have a block in the index.
            debug_assert!(node.has_output());
            target_upto = 0;
            self.push_frame_from_node(node, 0)?;
        }

        // Done sharing the prefix with wherever we were: walk the index on.
        while target_upto < target.len() {
            let target_label = i32::from(target[target_upto]);
            let next_node = self.trie.lookup_child(target_label, &node)?;

            let Some(next_node) = next_node else {
                // The index is exhausted.
                self.valid_index_prefix = self.stack[self.current].prefix_length;
                self.stack[self.current].scan_to_floor_frame(target)?;

                if !self.stack[self.current].has_terms {
                    self.term_exists = false;
                    self.set_term_byte(target_upto, target_label as u8);
                    self.term.truncate(1 + target_upto);
                    return Ok(PrepareSeek::NoMatch);
                }
                return Ok(PrepareSeek::ScanBlock);
            };

            // Follow this node.
            node = next_node;
            self.set_term_byte(target_upto, target_label as u8);
            *self.get_node(1 + target_upto) = node;
            target_upto += 1;

            if node.has_output() {
                self.push_frame_from_node(node, target_upto)?;
            }
        }

        self.valid_index_prefix = self.stack[self.current].prefix_length;
        self.stack[self.current].scan_to_floor_frame(target)?;

        // The target term is entirely contained in the index.
        if !self.stack[self.current].has_terms {
            self.term_exists = false;
            self.term.truncate(target_upto);
            return Ok(PrepareSeek::NoMatch);
        }

        Ok(PrepareSeek::ScanBlock)
    }

    /// Returns the deferred half of a two-phase seek: reading the block the
    /// index walk landed on and scanning it for `target`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum#getIOBooleanSupplier`.
    ///
    /// # Errors
    ///
    /// Returns whatever error prefetching the block raised; the scan's own
    /// errors surface when the supplier is called.
    fn get_io_boolean_supplier(
        &mut self,
        target: BytesRef,
        prefetch: bool,
    ) -> Result<Box<dyn IOBooleanSupplier + '_>> {
        if prefetch {
            self.prefetch_block()?;
        }

        Ok(Box::new(move || -> Result<bool> {
            self.load_current_block()?;
            let result = self.scan_to_term_current(target.slice(), true)?;
            Ok(result == SeekStatus::FOUND)
        }))
    }

    /// Two-phase [`TermsEnum::seek_exact`]: this call walks the term index and
    /// prefetches the block the term would live in, and calling
    /// [`IOBooleanSupplier::get`] on the returned supplier reads that block and
    /// answers whether the term is there. `None` means the index alone settled
    /// it: the term is not in this field.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum#prepareSeekExact`.
    ///
    /// # Errors
    ///
    /// Returns whatever error walking the term index raised.
    pub fn prepare_seek_exact(
        &mut self,
        target: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + '_>>> {
        self.prepare_seek_exact_with_prefetch(target, true)
    }

    /// [`Self::prepare_seek_exact`], with the prefetch made optional.
    ///
    /// Equivalent to the private
    /// `SegmentTermsEnum.prepareSeekExact(BytesRef, boolean)`.
    fn prepare_seek_exact_with_prefetch(
        &mut self,
        target: &BytesRef,
        prefetch: bool,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + '_>>> {
        match self.prepare_seek_exact_state(target.slice())? {
            PrepareSeek::NoMatch => Ok(None),
            PrepareSeek::AlreadyPositioned => Ok(Some(Box::new(|| -> Result<bool> { Ok(true) }))),
            PrepareSeek::ScanBlock => {
                let target = target.clone();
                Ok(Some(self.get_io_boolean_supplier(target, prefetch)?))
            }
        }
    }

    /// Walks every term of the field, gathering the block statistics
    /// `CheckIndex` reports, and leaves the cursor back at the root block.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum#computeBlockStats`.
    ///
    /// # Errors
    ///
    /// Returns whatever error reading a block of the terms file raised.
    pub fn compute_block_stats(&mut self) -> Result<Stats> {
        let mut stats = Stats::new(
            self.field.shared().segment.clone(),
            self.field.field_info().name.clone(),
        );

        self.started = false;
        let root = self.trie.root;
        *self.get_node(0) = root;

        // The empty prefix must have a block in the index.
        self.push_frame_from_node(root, 0)?;
        self.stack[self.current].fp_orig = self.stack[self.current].fp;
        self.load_current_block()?;
        self.valid_index_prefix = 0;

        let is_floor = !self.stack[self.current].is_last_in_floor;
        stats.start_block(&self.stack[self.current], is_floor);

        'all_terms: loop {
            // Pop finished blocks.
            while self.stack[self.current].next_ent == self.stack[self.current].ent_count {
                stats.end_block(&self.stack[self.current]);
                if !self.stack[self.current].is_last_in_floor {
                    // Advance to the next floor block.
                    self.init_index_input()?;
                    let terms_in = Self::index_input(&mut self.terms_in);
                    self.stack[self.current].load_next_floor_block(terms_in)?;
                    stats.start_block(&self.stack[self.current], true);
                    break;
                }
                if self.stack[self.current].ord == 0 {
                    break 'all_terms;
                }
                self.current -= 1;
            }

            loop {
                if self.frame_next()? {
                    // Push to a new block.
                    let sub_fp = self.stack[self.current].last_sub_fp;
                    let length = self.term.len();
                    self.push_frame(None, sub_fp, length)?;
                    self.stack[self.current].fp_orig = self.stack[self.current].fp;
                    // This is a "next" frame: even when it is floor'd we must
                    // pretend it is not, so that we do not scan to a floor block.
                    self.load_current_block()?;
                    let is_floor = !self.stack[self.current].is_last_in_floor;
                    stats.start_block(&self.stack[self.current], is_floor);
                } else {
                    stats.term(&BytesRef::new(self.term.clone()));
                    break;
                }
            }
        }

        stats.finish();

        // Put the root frame back.
        self.started = false;
        let root = self.trie.root;
        *self.get_node(0) = root;
        self.push_frame_from_node(root, 0)?;
        self.stack[self.current].rewind()?;
        self.load_current_block()?;
        self.valid_index_prefix = 0;
        self.term.clear();

        Ok(stats)
    }

    /// Prints the frame stack the last seek left behind, checking as it goes
    /// that every frame still agrees with the term index.
    ///
    /// This is a debugging aid, kept because Lucene keeps it; it is what one
    /// reaches for when a seek lands on the wrong block.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum#printSeekState`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when a frame no longer matches the
    /// index, and whatever error writing to `out` or reading the index raised.
    #[allow(dead_code)]
    pub fn print_seek_state(&mut self, out: &mut dyn std::io::Write) -> Result<()> {
        if !self.started {
            writeln!(out, "  no prior seek")?;
            return Ok(());
        }

        writeln!(out, "  prior seek state:")?;
        let mut ord = 0usize;
        let mut is_seek_frame = true;
        loop {
            self.ensure_frame(ord)?;
            let f = &self.stack[ord];
            let prefix: Vec<u8> = self.term[..f.prefix_length.min(self.term.len())].to_vec();
            if f.next_ent == -1 {
                writeln!(
                    out,
                    "    frame {} ord={} fp={}{} prefixLen={} prefix={:?} hasTerms={} isFloor={} isLastInFloor={} mdUpto={} tbOrd={}",
                    if is_seek_frame { "(seek)" } else { "(next)" },
                    ord,
                    f.fp,
                    if f.is_floor {
                        format!(" (fpOrig={})", f.fp_orig)
                    } else {
                        String::new()
                    },
                    f.prefix_length,
                    prefix,
                    f.has_terms,
                    f.is_floor,
                    f.is_last_in_floor,
                    f.meta_data_upto,
                    f.get_term_block_ord()
                )?;
            } else {
                writeln!(
                    out,
                    "    frame {} ord={} fp={}{} prefixLen={} prefix={:?} nextEnt={} (of {}) hasTerms={} isFloor={} lastSubFP={} isLastInFloor={} mdUpto={} tbOrd={}",
                    if is_seek_frame {
                        "(seek, loaded)"
                    } else {
                        "(next, loaded)"
                    },
                    ord,
                    f.fp,
                    if f.is_floor {
                        format!(" (fpOrig={})", f.fp_orig)
                    } else {
                        String::new()
                    },
                    f.prefix_length,
                    prefix,
                    f.next_ent,
                    f.ent_count,
                    f.has_terms,
                    f.is_floor,
                    f.last_sub_fp,
                    f.is_last_in_floor,
                    f.meta_data_upto,
                    f.get_term_block_ord()
                )?;
            }

            let frame_node = f.node;
            let frame_fp = f.fp;
            let frame_has_terms = f.has_terms;
            let frame_is_floor = f.is_floor;
            let frame_prefix_length = f.prefix_length;
            let is_current = ord == self.current;

            if frame_prefix_length > 0 && is_seek_frame {
                let node = frame_node.ok_or_else(|| {
                    LuceneError::illegal_state(format!(
                        "seek state is broken: seek frame {ord} has no index node"
                    ))
                })?;
                if node.label != i32::from(self.term[frame_prefix_length - 1]) {
                    writeln!(
                        out,
                        "      broken seek state: node.label={} vs term byte={}",
                        node.label,
                        self.term[frame_prefix_length - 1]
                    )?;
                    return Err(LuceneError::illegal_state(
                        "seek state is broken".to_string(),
                    ));
                }
            }

            let mut node = self.trie.root;
            for &label in &prefix {
                match self.trie.lookup_child(i32::from(label), &node)? {
                    Some(child) => node = child,
                    None => {
                        return Err(LuceneError::illegal_state(
                            "seek state is broken, prefix not exist in index".to_string(),
                        ))
                    }
                }
            }
            if !node.has_output() {
                writeln!(out, "      broken seek state: prefix is not final in index")?;
                return Err(LuceneError::illegal_state(
                    "seek state is broken".to_string(),
                ));
            } else if is_seek_frame
                && !frame_is_floor
                && (frame_fp != node.output_fp
                    || frame_has_terms != node.has_terms
                    || frame_is_floor != node.is_floor())
            {
                writeln!(
                    out,
                    "      broken seek state: output fp={}, hasTerms={}, isFloor={} doesn't match frame fp={}, hasTerms={}, isFloor={}",
                    node.output_fp,
                    node.has_terms,
                    node.is_floor(),
                    frame_fp,
                    frame_has_terms,
                    frame_is_floor
                )?;
                return Err(LuceneError::illegal_state(
                    "seek state is broken".to_string(),
                ));
            }

            if is_current {
                break;
            }
            if frame_prefix_length == self.valid_index_prefix {
                is_seek_frame = false;
            }
            ord += 1;
        }
        Ok(())
    }

    /// Turns the status a block scan ended on into the status `seek_ceil`
    /// reports: a scan that ran off the end of its block has to step into the
    /// next one before it can say whether any term follows the target.
    ///
    /// Equivalent to the tail shared by both `SegmentTermsEnum.seekCeil` exits.
    fn finish_seek_ceil(&mut self, result: SeekStatus, target: &[u8]) -> Result<SeekStatus> {
        if result != SeekStatus::END {
            return Ok(result);
        }
        self.term.clear();
        self.term.extend_from_slice(target);
        self.term_exists = false;
        if TermsEnum::next(self)?.is_some() {
            Ok(SeekStatus::NOT_FOUND)
        } else {
            Ok(SeekStatus::END)
        }
    }

    /// Returns the field this cursor walks.
    pub fn field_reader(&self) -> &FieldReader {
        &self.field
    }

    /// Catches the current frame up on the metadata decode it defers.
    ///
    /// Equivalent to `SegmentTermsEnum.decodeMetaData`, which every accessor of
    /// the current term's statistics and postings calls before reading `state`.
    /// The decode is lazy in Lucene and lazy here: a scan that never asks for a
    /// term's statistics never pays for them.
    fn decode_meta_data(&mut self) -> Result<()> {
        let field_info = self.field.field_info().clone();
        let reader = Arc::clone(&self.field.shared().postings_reader);
        self.stack[self.current].decode_meta_data(&field_info, reader.as_ref())
    }
}

impl std::fmt::Display for SegmentTermsEnum {
    /// `org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum` declares no `toString()` in Lucene 10.5.0, so
    /// `java.lang.Object`'s applies: the binary class name, `@`, and the
    /// identity hash. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum",
            self,
        ))
    }
}

impl TermsEnum for SegmentTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        if !self.started {
            self.start()?;
        }
        self.target_before_current_length = self.current as i64;
        if self.eof {
            return Ok(None);
        }

        loop {
            // Exhausted the current block: either move to the next floor block,
            // or pop back to the parent.
            while self.stack[self.current].next_ent == self.stack[self.current].ent_count {
                if !self.stack[self.current].is_last_in_floor {
                    self.init_index_input()?;
                    let terms_in = Self::index_input(&mut self.terms_in);
                    self.stack[self.current].load_next_floor_block(terms_in)?;
                    break;
                }
                if self.stack[self.current].ord == 0 {
                    self.set_eof();
                    self.term.clear();
                    self.valid_index_prefix = 0;
                    self.stack[self.current].rewind()?;
                    self.term_exists = false;
                    return Ok(None);
                }
                let last_fp = self.stack[self.current].fp_orig;
                self.current -= 1;
                if self.stack[self.current].next_ent == -1
                    || self.stack[self.current].last_sub_fp != last_fp
                {
                    let target = self.term.clone();
                    self.stack[self.current].scan_to_floor_frame(&target)?;
                    self.init_index_input()?;
                    let terms_in = Self::index_input(&mut self.terms_in);
                    self.stack[self.current].load_block(terms_in)?;
                    let mut term = std::mem::take(&mut self.term);
                    let result = self.stack[self.current].scan_to_sub_block(last_fp, &mut term);
                    self.term = term;
                    result?;
                }

                // The seek state is invalid beyond this depth now.
                self.valid_index_prefix = self
                    .valid_index_prefix
                    .min(self.stack[self.current].prefix_length);
            }

            let is_leaf = self.stack[self.current].is_leaf_block;
            if is_leaf {
                let mut term = std::mem::take(&mut self.term);
                let result = self.stack[self.current].next_leaf(&mut term);
                self.term = term;
                result?;
                self.term_exists = true;
                return Ok(Some(BytesRef::new(self.term.clone())));
            }

            let mut term = std::mem::take(&mut self.term);
            let is_sub_block = self.stack[self.current].next_non_leaf_entry(&mut term);
            self.term = term;
            if is_sub_block? {
                // Descend into the sub-block and keep going.
                let sub_fp = self.stack[self.current].last_sub_fp;
                let length = self.term.len();
                self.push_frame(None, sub_fp, length)?;
                self.load_current_block()?;
            } else {
                self.term_exists = true;
                return Ok(Some(BytesRef::new(self.term.clone())));
            }
        }
    }

    /// Two-phase [`TermsEnum::seek_exact`] that walks the term index first.
    ///
    /// Equivalent to `SegmentTermsEnum.prepareSeekExact(BytesRef)`, which
    /// overrides `TermsEnum.prepareSeekExact`, so that `TermStates` and the
    /// other callers that hold a plain `TermsEnum` reach the block-tree walk
    /// and its prefetch instead of the default supplier.
    fn prepare_seek_exact<'a>(
        &'a mut self,
        text: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + 'a>>> {
        SegmentTermsEnum::prepare_seek_exact(self, text)
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        match self.prepare_seek_exact_with_prefetch(text, false)? {
            None => Ok(false),
            Some(mut supplier) => supplier.get(),
        }
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        let target = text.slice();

        debug_assert!(self.clear_eof());
        self.clear_eof();

        let mut node;
        let mut target_upto;

        self.target_before_current_length = if self.started {
            self.current as i64
        } else {
            -1
        };

        if self.started {
            // We are already seek'd: find the common prefix of the new target
            // and the current term, and re-use the seek state that covers it.
            node = self.nodes[0];
            debug_assert!(node.has_output());
            target_upto = 0;

            let mut last_frame_ord = 0usize;
            debug_assert!(self.valid_index_prefix <= self.term.len());
            let target_limit = target.len().min(self.valid_index_prefix);

            let mut cmp = std::cmp::Ordering::Equal;
            while target_upto < target_limit {
                cmp = self.term[target_upto].cmp(&target[target_upto]);
                if cmp != std::cmp::Ordering::Equal {
                    break;
                }
                node = self.nodes[1 + target_upto];
                debug_assert_eq!(node.label, i32::from(target[target_upto]));
                if node.has_output() {
                    last_frame_ord += 1;
                }
                target_upto += 1;
            }

            if cmp == std::cmp::Ordering::Equal {
                // Compare the rest of the term, without saving any state.
                cmp = self.term[target_upto..].cmp(&target[target_upto..]);
            }

            match cmp {
                std::cmp::Ordering::Less => {
                    // Common case: the target is after the current term.
                    self.current = last_frame_ord;
                }
                std::cmp::Ordering::Greater => {
                    // The target is before the current term: keep the frame but
                    // rewind it, so the scan starts from the block's first entry.
                    self.target_before_current_length = 0;
                    self.current = last_frame_ord;
                    self.stack[self.current].rewind()?;
                }
                std::cmp::Ordering::Equal => {
                    debug_assert_eq!(self.term.len(), target.len());
                    if self.term_exists {
                        return Ok(SeekStatus::FOUND);
                    }
                }
            }
        } else {
            self.target_before_current_length = -1;
            let root = self.trie.root;
            *self.get_node(0) = root;
            node = root;
            // The empty prefix must have a block in the index.
            debug_assert!(node.has_output());
            target_upto = 0;
            self.push_frame_from_node(node, 0)?;
        }

        // Done sharing the prefix with wherever we were: walk the index on.
        while target_upto < target.len() {
            let target_label = i32::from(target[target_upto]);
            let next_node = self.trie.lookup_child(target_label, &node)?;

            let Some(next_node) = next_node else {
                // The index is exhausted; the block we stand on is the one that
                // would hold the target, so scan it.
                self.valid_index_prefix = self.stack[self.current].prefix_length;
                self.stack[self.current].scan_to_floor_frame(target)?;
                self.load_current_block()?;
                let result = self.scan_to_term_current(target, false)?;
                return self.finish_seek_ceil(result, target);
            };

            // Follow this node.
            self.set_term_byte(target_upto, target_label as u8);
            node = next_node;
            *self.get_node(1 + target_upto) = node;
            target_upto += 1;

            if node.has_output() {
                self.push_frame_from_node(node, target_upto)?;
            }
        }

        self.valid_index_prefix = self.stack[self.current].prefix_length;
        self.stack[self.current].scan_to_floor_frame(target)?;
        self.load_current_block()?;
        let result = self.scan_to_term_current(target, false)?;
        self.finish_seek_ceil(result, target)
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "the block-tree terms dictionary does not support seeking by ordinal".to_string(),
        ))
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(BytesRef::new(self.term.clone()))
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "the block-tree terms dictionary does not track term ordinals".to_string(),
        ))
    }

    fn doc_freq(&mut self) -> Result<i32> {
        // Java asserts `!eof` and calls `currentFrame.decodeMetaData()` before
        // reading `state`; without it the frame still holds the zero it was
        // built with, which is what made every term look absent.
        debug_assert!(!self.eof);
        self.decode_meta_data()?;
        Ok(self.stack[self.current].state.doc_freq)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        debug_assert!(!self.eof);
        self.decode_meta_data()?;
        Ok(self.stack[self.current].state.total_term_freq)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.decode_meta_data()?;
        let field_info = self.field.field_info().clone();
        let state = self.stack[self.current].state;
        let reader = Arc::clone(&self.field.shared().postings_reader);
        reader.impacts(&field_info, &state, flags)
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.decode_meta_data()?;
        let field_info = self.field.field_info().clone();
        let state = self.stack[self.current].state;
        let reader = Arc::clone(&self.field.shared().postings_reader);
        reader.postings(&field_info, &state, reuse, flags)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.decode_meta_data()?;
        Ok(Box::new(self.stack[self.current].state.clone()))
    }
}
