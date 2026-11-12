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

//! Port of `org.apache.lucene.codecs.uniformsplit.BlockReader`.

use std::fmt;
use std::sync::Arc;

use rucene::codecs::{BlockTermState, PostingsReaderBase};
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::java_object_to_string;
use rucene::index::{FieldInfos, ImpactsEnum, PostingsEnum, SeekStatus, TermState, TermsEnum};
use rucene::store::{ByteArrayDataInput, IndexInput};
use rucene::util::attribute::AttributeSource;
use rucene::util::{Accountable, BytesRef, RamUsageEstimator};

use super::block_decoder::BlockDecoder;
use super::block_header::{self, BlockHeader};
use super::block_line::{self, ReadLine};
use super::delta_base_term_state_serializer::DeltaBaseTermStateSerializer;
use super::field_metadata::FieldMetadata;
use super::index_dictionary::{Browser, BrowserSupplier};
use super::ram_usage_util;
use super::sharedterms::st_block_line;

use crate::blocktreeords::{array_len, java_assert};

/// The overrides of the `sharedterms` subclasses of `BlockReader` that
/// change the base class's own algorithms.
///
/// * `term_states_for_field` (`STBlockReader`, `STIntersectBlockReader`,
///   `STMergingBlockReader`): `readTermState()` reads the term state of this
///   reader's field among the fields of the line
///   (`STBlockLine.Serializer.readTermStateForField`), `None` when the term
///   does not occur in the field; the field infos resolve the line's field
///   numbers.
/// * `bounded` (`STBlockReader`, `STMergingBlockReader`):
///   `isBeyondLastTerm` also stops past the field's last block, and
///   `nextTerm` stops past the field's last term.
#[derive(Clone)]
pub(crate) struct SharedTermsHooks {
    pub(crate) field_infos: Arc<FieldInfos>,
    pub(crate) bounded: bool,
}

/// Seeks the block corresponding to a given term, read the block bytes, and
/// scans the block terms.
///
/// Reads fully the block in `blockReadBuffer`. Then scans the block terms in
/// memory. The details region is lazily decoded with `termStatesReadBuffer`
/// which shares the same byte array with `blockReadBuffer`. See
/// [`BlockWriter`](super::BlockWriter) and [`BlockLine`](super::BlockLine)
/// for the block format.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.BlockReader`
/// (`@lucene.experimental`). The two `ByteArrayDataInput`s over the block's
/// array are the array and two read positions; the block line Java reuses,
/// whose term is the line serializer's `currentTerm`, is a [`ReadLine`] (see
/// there). The `sharedterms` subclasses' overrides are `SharedTermsHooks`;
/// a subclass's overrides of the protected methods are
/// [`BlockReaderOverrides`].
pub struct BlockReader {
    /// The block file input the reader was created with (`blockInput` until
    /// `initializeBlockReadLazily` replaces it with a clone).
    pub(crate) original_block_input: Arc<rucene::store::SharedIndexInput>,
    /// `IndexInput` on the block file, the clone `initializeBlockReadLazily`
    /// makes.
    pub(crate) block_input: Option<Box<dyn IndexInput>>,

    pub(crate) postings_reader: Arc<dyn PostingsReaderBase>,
    pub(crate) field_metadata: Arc<FieldMetadata>,
    pub(crate) block_decoder: Option<Arc<dyn BlockDecoder>>,

    pub(crate) block_header_reader: block_header::Serializer,
    pub(crate) block_line_reader: block_line::Serializer,

    /// The array of `blockReadBuffer` and `termStatesReadBuffer` when the
    /// block is not decoded (`scratchBlockBytes`).
    pub(crate) scratch_block_bytes: Vec<u8>,
    /// The block's decoded bytes, when a block decoder is set.
    pub(crate) decoded_block: Option<BytesRef>,
    /// The window of the current block in its array: `offset`, `limit`.
    pub(crate) block_window: (usize, usize),
    /// `blockReadBuffer.getPosition()`.
    pub(crate) block_pos: usize,
    /// `termStatesReadBuffer.getPosition()`.
    pub(crate) term_states_pos: usize,

    pub(crate) term_state_serializer: DeltaBaseTermStateSerializer,

    /// `IndexDictionary.Browser` supplier for lazy loading.
    pub(crate) dictionary_browser_supplier: Arc<dyn BrowserSupplier>,
    /// Holds the `IndexDictionary.Browser` once loaded.
    pub(crate) dictionary_browser: Option<Box<dyn Browser>>,

    /// Current block start file pointer, absolute in the block file.
    pub(crate) block_start_fp: i64,
    /// Current block header.
    pub(crate) block_header: Option<BlockHeader>,
    /// Current block line.
    pub(crate) block_line: Option<ReadLine>,
    /// Whether the current block line details (`termState`) are set; they
    /// are `scratch_term_state`, the only state
    /// Java's `termState` ever references.
    pub(crate) term_state_set: bool,

    /// Offset of the start of the first line of the current block (just
    /// after the header), relative to the block start.
    pub(crate) block_first_line_start: usize,
    /// Current line index in the block.
    pub(crate) line_index_in_block: i32,

    /// Whether the current `TermState` has been forced with a call to
    /// `seekExact(BytesRef, TermState)`.
    pub(crate) term_state_forced: bool,
    /// Set when `seekExact(BytesRef, TermState)` is called.
    ///
    /// This optimizes the use-case when the caller calls first
    /// `seekExact(BytesRef, TermState)` and then `postings`. In this case we
    /// don't access the terms block file (we don't seek) but directly the
    /// postings file because we already have the `TermState` with the file
    /// pointers to the postings file.
    pub(crate) forced_term: Option<Vec<u8>>,

    pub(crate) scratch_term_state: BlockTermState,

    pub(crate) shared_terms: Option<SharedTermsHooks>,
    /// The overrides of a subclass's protected methods, see
    /// [`BlockReaderOverrides`].
    pub(crate) overrides: Option<Arc<dyn BlockReaderOverrides>>,
    pub(crate) attributes: AttributeSource,
}

/// The protected methods of [`BlockReader`] that a Java subclass overrides,
/// as the `MockBlockReader` of `TestTermBytesComparator` and the
/// `MockSTBlockReader` of `TestSTBlockReader` do: `compareToMiddleAndJump`,
/// `readLineInBlock`, `initializeHeader`, `readHeader` and `readTermState`.
///
/// Each method receives the reader, whose protected state the accessors of
/// [`BlockReader`] expose; the default runs the base implementation (the
/// `base_*` method, Java's `super.` call), which is the reader without a
/// subclass. Every call the base class makes to one of these methods,
/// including the calls from inside another base method, dispatches through
/// the overrides, as Java's virtual dispatch does.
///
/// **Divergence (declared).** Java overrides the methods in a subclass of
/// `BlockReader`; Rust has no subclassing, so the reader holds the overrides
/// as an optional object ([`BlockReader::with_overrides`]) and dispatches the
/// five methods through it. The order of the calls is Java's.
pub trait BlockReaderOverrides: Send + Sync {
    /// `protected int compareToMiddleAndJump(BytesRef searchedTerm)`
    /// (`BlockReader.java:272-292`).
    ///
    /// # Errors
    ///
    /// The errors of the base implementation and of the override.
    fn compare_to_middle_and_jump(
        &self,
        reader: &mut BlockReader,
        searched_term: &[u8],
    ) -> Result<i32> {
        reader.base_compare_to_middle_and_jump(searched_term)
    }

    /// `protected BlockLine readLineInBlock()` (`BlockReader.java:298-307`).
    ///
    /// # Errors
    ///
    /// The errors of the base implementation and of the override.
    fn read_line_in_block(&self, reader: &mut BlockReader) -> Result<Option<ReadLine>> {
        reader.base_read_line_in_block()
    }

    /// `protected void initializeHeader(BytesRef searchedTerm, long
    /// targetBlockStartFP)` (`BlockReader.java:384-410`).
    ///
    /// # Errors
    ///
    /// The errors of the base implementation and of the override.
    fn initialize_header(
        &self,
        reader: &mut BlockReader,
        searched_term: Option<&[u8]>,
        target_block_start_fp: i64,
    ) -> Result<()> {
        reader.base_initialize_header(searched_term, target_block_start_fp)
    }

    /// `protected BlockHeader readHeader()` (`BlockReader.java:442-451`).
    ///
    /// # Errors
    ///
    /// The errors of the base implementation and of the override.
    fn read_header(&self, reader: &mut BlockReader) -> Result<Option<BlockHeader>> {
        reader.base_read_header()
    }

    /// `protected BlockTermState readTermState()`
    /// (`BlockReader.java:484-500`): reads the current line's term state
    /// into [`BlockReader::scratch_term_state_mut`] and returns whether it is
    /// set (Java's non-`null` `termState`).
    ///
    /// # Errors
    ///
    /// The errors of the base implementation and of the override.
    fn read_term_state(&self, reader: &mut BlockReader) -> Result<bool> {
        reader.base_read_term_state()
    }
}

impl BlockReader {
    /// Port of `BlockReader(IndexDictionary.BrowserSupplier, IndexInput,
    /// PostingsReaderBase, FieldMetadata, BlockDecoder)`.
    ///
    /// `dictionary_browser_supplier` loads the `IndexDictionary.Browser`
    /// lazily in `seekCeil`; `block_decoder` is optional, it can be used for
    /// decompression or decryption.
    ///
    /// # Errors
    ///
    /// The errors of the postings reader's `newTermState`.
    pub fn new(
        dictionary_browser_supplier: Arc<dyn BrowserSupplier>,
        block_input: Arc<rucene::store::SharedIndexInput>,
        postings_reader: Arc<dyn PostingsReaderBase>,
        field_metadata: Arc<FieldMetadata>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
    ) -> Result<Self> {
        let scratch_term_state = postings_reader.new_term_state()?;
        Ok(Self {
            original_block_input: block_input,
            block_input: None,
            postings_reader,
            field_metadata,
            block_decoder,
            block_header_reader: block_header::Serializer,
            block_line_reader: block_line::Serializer::new(),
            scratch_block_bytes: Vec::new(),
            decoded_block: None,
            block_window: (0, 0),
            block_pos: 0,
            term_states_pos: 0,
            term_state_serializer: DeltaBaseTermStateSerializer::new(),
            dictionary_browser_supplier,
            dictionary_browser: None,
            block_start_fp: -1,
            block_header: None,
            block_line: None,
            term_state_set: false,
            block_first_line_start: 0,
            line_index_in_block: 0,
            term_state_forced: false,
            forced_term: None,
            scratch_term_state,
            shared_terms: None,
            overrides: None,
            attributes: AttributeSource::new(),
        })
    }

    /// The reader with its protected methods overridden by `overrides` (a
    /// Java subclass's overrides, see [`BlockReaderOverrides`]).
    #[must_use]
    pub fn with_overrides(mut self, overrides: Arc<dyn BlockReaderOverrides>) -> Self {
        self.overrides = Some(overrides);
        self
    }

    /// The protected field `lineIndexInBlock`: the current line index in the
    /// block.
    pub fn line_index_in_block(&self) -> i32 {
        self.line_index_in_block
    }

    /// Assigns the protected field `lineIndexInBlock`.
    pub fn set_line_index_in_block(&mut self, line_index_in_block: i32) {
        self.line_index_in_block = line_index_in_block;
    }

    /// The protected field `blockHeader`, Java's `null` being `None`.
    pub fn block_header(&self) -> Option<BlockHeader> {
        self.block_header
    }

    /// Assigns the protected field `blockHeader`.
    pub fn set_block_header(&mut self, block_header: Option<BlockHeader>) {
        self.block_header = block_header;
    }

    /// The protected field `blockStartFP`.
    pub fn block_start_fp(&self) -> i64 {
        self.block_start_fp
    }

    /// Assigns the protected field `blockStartFP`.
    pub fn set_block_start_fp(&mut self, block_start_fp: i64) {
        self.block_start_fp = block_start_fp;
    }

    /// The protected field `fieldMetadata`.
    pub fn field_metadata(&self) -> &FieldMetadata {
        &self.field_metadata
    }

    /// The state the protected field `termState` references once
    /// `readTermState()` has set it (`scratchTermState`).
    pub fn scratch_term_state_mut(&mut self) -> &mut BlockTermState {
        &mut self.scratch_term_state
    }

    /// Assigns the protected field `blockLine` (`blockLine = line`), Java's
    /// `null` being `None`; returns the line as the reader holds it.
    ///
    /// The port keeps the current line's term in the line serializer (see
    /// [`ReadLine`]), so the term of `line` becomes the serializer's current
    /// term.
    pub fn set_block_line(&mut self, line: Option<&block_line::BlockLine>) -> Option<ReadLine> {
        self.block_line = line.map(|line| {
            let term_bytes = line.get_term_bytes();
            let term = term_bytes.get_term();
            self.block_line_reader.current_term = BytesRef::new(term.slice().to_vec());
            ReadLine {
                mdp_length: term_bytes.get_mdp_length(),
                term_state_relative_offset: line.get_term_state_relative_offset(),
            }
        });
        self.block_line
    }

    /// Port of `BlockReader.seekCeil(BytesRef)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing block; I/O errors.
    pub fn seek_ceil_impl(&mut self, searched_term: &BytesRef) -> Result<SeekStatus> {
        if self.is_current_term(searched_term) {
            return Ok(SeekStatus::FOUND);
        }
        self.clear_term_state();

        let block_start_fp = self
            .get_or_create_dictionary_browser()?
            .seek_block(searched_term)?;
        let block_start_fp = block_start_fp.max(self.field_metadata.get_first_block_start_fp());
        if self.is_beyond_last_term(searched_term.slice(), block_start_fp) {
            return Ok(SeekStatus::END);
        }
        let seek_status = self.seek_in_block_at(searched_term.slice(), block_start_fp)?;
        if seek_status != SeekStatus::END {
            return Ok(seek_status);
        }
        // Go to next block.
        Ok(if self.next_term()?.is_none() {
            SeekStatus::END
        } else {
            SeekStatus::NOT_FOUND
        })
    }

    /// Port of `BlockReader.seekExact(BytesRef)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing block; I/O errors.
    pub fn seek_exact_impl(&mut self, searched_term: &BytesRef) -> Result<bool> {
        if self.is_current_term(searched_term) {
            return Ok(true);
        }
        self.clear_term_state();

        let block_start_fp = self
            .get_or_create_dictionary_browser()?
            .seek_block(searched_term)?;
        if block_start_fp < self.field_metadata.get_first_block_start_fp()
            || self.is_beyond_last_term(searched_term.slice(), block_start_fp)
        {
            return Ok(false);
        }
        Ok(self.seek_in_block_at(searched_term.slice(), block_start_fp)? == SeekStatus::FOUND)
    }

    /// Port of `BlockReader.isCurrentTerm(BytesRef)`.
    pub fn is_current_term(&self, searched_term: &BytesRef) -> bool {
        // Optimization and also required to not search with the same BytesRef
        // instance as the BytesRef used to read the block line (BlockLine.Serializer).
        // Indeed getCurrentTerm() is allowed to return the same BytesRef instance.
        self.current_term() == Some(searched_term.slice())
    }

    /// Indicates whether the searched term is beyond the last term of the
    /// field; `block_start_fp` is the current block start file pointer.
    ///
    /// Port of `BlockReader.isBeyondLastTerm(BytesRef, long)`, with the
    /// override of `STBlockReader` when `SharedTermsHooks::bounded`.
    pub fn is_beyond_last_term(&self, searched_term: &[u8], block_start_fp: i64) -> bool {
        let base = block_start_fp == self.field_metadata.get_last_block_start_fp()
            && self
                .field_metadata
                .get_last_term()
                .is_some_and(|last| searched_term > last.slice());
        match &self.shared_terms {
            Some(hooks) if hooks.bounded => {
                block_start_fp > self.field_metadata.get_last_block_start_fp() || base
            }
            _ => base,
        }
    }

    /// Seeks to the provided term in the block starting at the provided file
    /// pointer. Does not exceed the block.
    ///
    /// Port of `BlockReader.seekInBlock(BytesRef, long)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing block; I/O errors.
    pub fn seek_in_block_at(
        &mut self,
        searched_term: &[u8],
        block_start_fp: i64,
    ) -> Result<SeekStatus> {
        self.initialize_header(Some(searched_term), block_start_fp)?;
        if self.block_header.is_none() {
            return Err(
                self.new_corrupt_index_exception("Illegal absence of block", Some(block_start_fp))
            );
        }
        self.seek_in_block(searched_term)
    }

    /// Seeks to the provided term in this block.
    ///
    /// Does not exceed this block; `SeekStatus.END` is returned if it follows
    /// the block.
    ///
    /// Compares the line terms with the `searched_term`, taking advantage of
    /// the incremental encoding properties.
    ///
    /// Scans linearly the terms. Updates the current block line with the
    /// current term.
    ///
    /// Port of `BlockReader.seekInBlock(BytesRef)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing middle line; I/O errors.
    pub fn seek_in_block(&mut self, searched_term: &[u8]) -> Result<SeekStatus> {
        if self.compare_to_middle_and_jump(searched_term)? == 0 {
            return Ok(SeekStatus::FOUND);
        }
        let mut comparison_offset = 0usize;
        loop {
            let Some(line) = self.read_line_in_block()? else {
                // No more terms for the block.
                return Ok(SeekStatus::END);
            };
            let line_term = self.block_line_reader.current_term();

            // Equivalent to comparing with BytesRef.compareTo(),
            // but faster since we start comparing from min(comparisonOffset, suffixOffset).
            let suffix_offset = line.suffix_offset() as usize;
            let start = comparison_offset.min(suffix_offset);
            let end = searched_term.len().min(line_term.len());
            let mut comparison = searched_term.len() as i64 - line_term.len() as i64;
            for i in start..end {
                // Compare unsigned bytes.
                let byte_diff = i64::from(searched_term[i]) - i64::from(line_term[i]);
                if byte_diff != 0 {
                    comparison = byte_diff;
                    break;
                }
                comparison_offset = i + 1;
            }
            if comparison == 0 {
                return Ok(SeekStatus::FOUND);
            } else if comparison < 0 {
                return Ok(SeekStatus::NOT_FOUND);
            }
        }
    }

    /// Compares the searched term to the middle term of the block. If the
    /// searched term is lexicographically equal or after the middle term
    /// then jumps to the second half of the block directly.
    ///
    /// Returns the comparison between the searched term and the middle term.
    ///
    /// Port of `BlockReader.compareToMiddleAndJump(BytesRef)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing middle line; I/O errors.
    pub fn compare_to_middle_and_jump(&mut self, searched_term: &[u8]) -> Result<i32> {
        match self.overrides.clone() {
            Some(overrides) => overrides.compare_to_middle_and_jump(self, searched_term),
            None => self.base_compare_to_middle_and_jump(searched_term),
        }
    }

    /// The base implementation of `compare_to_middle_and_jump`
    /// (`super.compareToMiddleAndJump`).
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing middle line; I/O errors.
    pub fn base_compare_to_middle_and_jump(&mut self, searched_term: &[u8]) -> Result<i32> {
        if self.line_index_in_block != 0 {
            // Don't try to compare and jump if we are not positioned at the first line.
            // This can happen if we seek in the same current block and we continue
            // scanning from the current line (see initializeHeader()).
            return Ok(-1);
        }
        let header = self.header()?;
        self.skip_block_bytes(header.get_middle_line_offset())?;
        self.line_index_in_block = header.get_middle_line_index();
        self.read_line_in_block()?;
        if self.block_line.is_none() {
            return Err(self.new_corrupt_index_exception(
                "Illegal absence of line at the middle of the block",
                None,
            ));
        }
        let compare = match searched_term.cmp(self.block_line_reader.current_term()) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        if compare < 0 {
            self.block_pos = self.block_first_line_start;
            self.line_index_in_block = 0;
        }
        Ok(compare)
    }

    /// `blockReadBuffer.skipBytes(n)`: Java's `ByteArrayDataInput.skipBytes`
    /// only moves the position.
    fn skip_block_bytes(&mut self, n: i32) -> Result<()> {
        let n = array_len(n)?;
        self.block_pos += n;
        Ok(())
    }

    /// The current block header, which the callers have checked is set.
    pub(crate) fn header(&self) -> Result<BlockHeader> {
        self.block_header.ok_or_else(|| {
            super::npe("Cannot invoke \"org.apache.lucene.codecs.uniformsplit.BlockHeader.getLinesCount()\" because \"this.blockHeader\" is null")
        })
    }

    /// The array and window of `blockReadBuffer` and `termStatesReadBuffer`.
    fn block_array(&self) -> (&[u8], usize, usize) {
        let (offset, limit) = self.block_window;
        match &self.decoded_block {
            Some(decoded) => (&decoded.bytes, offset, limit),
            None => (&self.scratch_block_bytes, offset, limit),
        }
    }

    /// Reads the current block line. Sets the block line and increments the
    /// line index; returns `None` if there no more line in the block.
    ///
    /// Port of `BlockReader.readLineInBlock()`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a negative term state offset; I/O errors.
    pub fn read_line_in_block(&mut self) -> Result<Option<ReadLine>> {
        match self.overrides.clone() {
            Some(overrides) => overrides.read_line_in_block(self),
            None => self.base_read_line_in_block(),
        }
    }

    /// The base implementation of `read_line_in_block`
    /// (`super.readLineInBlock`).
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a negative term state offset; I/O errors.
    pub fn base_read_line_in_block(&mut self) -> Result<Option<ReadLine>> {
        let header = self.header()?;
        if self.line_index_in_block >= header.get_lines_count() {
            self.block_line = None;
            return Ok(None);
        }
        let is_incremental_encoding_seed = self.line_index_in_block == 0
            || self.line_index_in_block == header.get_middle_line_index();
        self.line_index_in_block += 1;
        let (offset, limit) = self.block_window;
        let bytes: &[u8] = match &self.decoded_block {
            Some(decoded) => &decoded.bytes,
            None => &self.scratch_block_bytes,
        };
        let mut input = ByteArrayDataInput::with_range(bytes, offset, limit - offset);
        input.set_position(self.block_pos);
        let line = self
            .block_line_reader
            .read_line(&mut input, is_incremental_encoding_seed);
        self.block_pos = input.position();
        let line = line?;
        self.block_line = Some(line);
        Ok(Some(line))
    }

    /// Positions this [`BlockReader`] without re-seeking the term dictionary.
    ///
    /// The block containing the term is not read by this method. It will be
    /// read lazily only if needed, for example if `next()` is called.
    /// Calling `postings` after this method does require the block to be
    /// read.
    ///
    /// Port of `BlockReader.seekExact(BytesRef, TermState)`.
    pub fn seek_exact_state(&mut self, term: &BytesRef, state: &dyn TermState) {
        self.term_state_forced = true;
        self.term_state_set = true;
        TermState::copy_from(&mut self.scratch_term_state, state);
        self.forced_term = Some(term.slice().to_vec());
    }

    /// Port of `BlockReader.next()`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing block; I/O errors.
    pub fn next_impl(&mut self) -> Result<Option<BytesRef>> {
        if self.term_state_forced {
            let forced = self.forced_term.clone().unwrap_or_default();
            let fp = self.scratch_term_state.block_file_pointer;
            self.initialize_header(Some(&forced), fp)?;
            if self.block_header.is_none() {
                return Err(self.new_corrupt_index_exception(
                    "Illegal absence of block for TermState",
                    Some(fp),
                ));
            }
            for _ in self.line_index_in_block..self.scratch_term_state.term_block_ord {
                self.read_line_in_block()?;
            }
            java_assert!(self.block_line_reader.current_term() == forced.as_slice());
        }
        self.clear_term_state();
        self.next_term()
    }

    /// Moves to the next term line and reads it, it may be in the next block.
    /// The term details are not read yet. They will be read only when needed
    /// with `readTermStateIfNotRead()`.
    ///
    /// Returns the read term bytes, or `None` if there is no more term for
    /// the field.
    ///
    /// Port of `BlockReader.nextTerm()`, with the override of
    /// `STBlockReader` when `SharedTermsHooks::bounded`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing first block; I/O errors.
    pub fn next_term(&mut self) -> Result<Option<BytesRef>> {
        let next_term = self.base_next_term()?;
        if let (Some(term), Some(hooks)) = (&next_term, &self.shared_terms) {
            if hooks.bounded {
                // `super.isBeyondLastTerm`, the base class's.
                let beyond = self.block_start_fp == self.field_metadata.get_last_block_start_fp()
                    && self
                        .field_metadata
                        .get_last_term()
                        .is_some_and(|last| term.slice() > last.slice());
                if beyond {
                    return Ok(None);
                }
            }
        }
        Ok(next_term)
    }

    fn base_next_term(&mut self) -> Result<Option<BytesRef>> {
        if self.block_header.is_none() {
            // Read the first block for the field.
            let first = self.field_metadata.get_first_block_start_fp();
            self.initialize_header(None, first)?;
            if self.block_header.is_none() {
                return Err(
                    self.new_corrupt_index_exception("Illegal absence of first block", Some(first))
                );
            }
        }
        if self.read_line_in_block()?.is_none() {
            // No more line in the current block.
            // Read the next block starting at the current file pointer in the block file.
            let fp = self.input()?.file_pointer()?;
            self.initialize_header(None, fp)?;
            if self.block_header.is_none() {
                // No more block for the field.
                return Ok(None);
            }
            self.read_line_in_block()?;
        }
        Ok(self.term_bytes())
    }

    /// The cloned block input, which `initializeBlockReadLazily` set.
    pub(crate) fn input(&mut self) -> Result<&mut Box<dyn IndexInput>> {
        if self.block_input.is_none() {
            self.block_input = Some(self.original_block_input.clone_input()?);
        }
        self.block_input
            .as_mut()
            .ok_or_else(|| LuceneError::illegal_state("the block input is set above"))
    }

    /// Reads and sets the block header. Sets `None` if there is no block for
    /// the field anymore.
    ///
    /// `searched_term` is the searched term, or `None` if none;
    /// `target_block_start_fp` the file pointer of the block to read.
    ///
    /// Port of `BlockReader.initializeHeader(BytesRef, long)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing current block; I/O errors.
    pub fn initialize_header(
        &mut self,
        searched_term: Option<&[u8]>,
        target_block_start_fp: i64,
    ) -> Result<()> {
        match self.overrides.clone() {
            Some(overrides) => {
                overrides.initialize_header(self, searched_term, target_block_start_fp)
            }
            None => self.base_initialize_header(searched_term, target_block_start_fp),
        }
    }

    /// The base implementation of `initialize_header`
    /// (`super.initializeHeader`).
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing current block; I/O errors.
    pub fn base_initialize_header(
        &mut self,
        searched_term: Option<&[u8]>,
        target_block_start_fp: i64,
    ) -> Result<()> {
        self.initialize_block_read_lazily()?;
        if self.block_start_fp == target_block_start_fp {
            // Optimization: If the block to read is already the current block, then
            // reuse it directly without reading nor decoding the block bytes.
            if self.block_header.is_none() {
                return Err(self.new_corrupt_index_exception(
                    "Illegal absence of block",
                    Some(self.block_start_fp),
                ));
            }
            let reset = match (searched_term, &self.block_line) {
                (None, _) | (_, None) => true,
                (Some(searched), Some(_)) => searched <= self.block_line_reader.current_term(),
            };
            if reset {
                // If the searched term precedes lexicographically the current term,
                // then reset the position to the first term line of the block.
                // If the searched term equals the current term, we also need to reset
                // to scan again the current line.
                self.block_pos = self.block_first_line_start;
                self.line_index_in_block = 0;
            }
        } else {
            self.input()?.seek(target_block_start_fp)?;
            self.block_start_fp = target_block_start_fp;
            self.read_header()?;
            self.block_first_line_start = self.block_pos;
            self.line_index_in_block = 0;
        }
        Ok(())
    }

    /// Port of `BlockReader.initializeBlockReadLazily()`.
    ///
    /// # Errors
    ///
    /// The I/O errors of cloning the block input.
    pub fn initialize_block_read_lazily(&mut self) -> Result<()> {
        if self.block_start_fp == -1 {
            self.block_input = Some(self.original_block_input.clone_input()?);
            self.block_header_reader = block_header::Serializer;
            self.block_line_reader = block_line::Serializer::new();
            self.block_window = (0, 0);
            self.block_pos = 0;
            self.term_states_pos = 0;
            self.term_state_serializer = DeltaBaseTermStateSerializer::new();
            self.scratch_block_bytes = Vec::new();
            self.decoded_block = None;
        }
        Ok(())
    }

    /// Reads the block header. Sets the block header; `None` if there is no
    /// block for the field anymore.
    ///
    /// Port of `BlockReader.readHeader()`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a bad header; I/O errors.
    pub fn read_header(&mut self) -> Result<Option<BlockHeader>> {
        match self.overrides.clone() {
            Some(overrides) => overrides.read_header(self),
            None => self.base_read_header(),
        }
    }

    /// The base implementation of `read_header` (`super.readHeader`).
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a bad header; I/O errors.
    pub fn base_read_header(&mut self) -> Result<Option<BlockHeader>> {
        if self.input()?.file_pointer()? > self.field_metadata.get_last_block_start_fp() {
            self.block_header = None;
            return Ok(None);
        }
        let num_block_bytes = self.input()?.read_v_int()?;
        self.decode_block_bytes_if_needed(num_block_bytes)?;
        let (offset, limit) = self.block_window;
        self.block_pos = offset;
        self.term_states_pos = offset;
        let (bytes, _, _) = self.block_array();
        let mut input = ByteArrayDataInput::with_range(bytes, offset, limit - offset);
        let header = self.block_header_reader.read(&mut input, self.block_header);
        let pos = input.position();
        self.block_pos = pos;
        let header = header?;
        self.block_header = Some(header);
        Ok(Some(header))
    }

    /// Port of `BlockReader.decodeBlockBytesIfNeeded(int)`: reads the block
    /// bytes and, with a block decoder, decodes them; sets the block's array
    /// and window.
    ///
    /// # Errors
    ///
    /// The I/O errors of the input and the errors of the decoder.
    pub fn decode_block_bytes_if_needed(&mut self, num_block_bytes: i32) -> Result<()> {
        let n = array_len(num_block_bytes)?;
        if self.scratch_block_bytes.len() < n {
            self.scratch_block_bytes.resize(n, 0);
        }
        let mut scratch = std::mem::take(&mut self.scratch_block_bytes);
        let read = self.input()?.read_bytes(Some(&mut scratch), 0, n);
        self.scratch_block_bytes = scratch;
        read?;
        match &self.block_decoder {
            None => {
                self.decoded_block = None;
                self.block_window = (0, n);
            }
            Some(decoder) => {
                let mut input = ByteArrayDataInput::with_range(&self.scratch_block_bytes, 0, n);
                let decoded = decoder.decode(&mut input, i64::from(num_block_bytes))?;
                self.block_window = (decoded.offset, decoded.offset + decoded.length);
                self.decoded_block = Some(decoded);
            }
        }
        Ok(())
    }

    /// Reads the [`BlockTermState`] if it is not already set; returns whether
    /// it is set.
    ///
    /// Port of `BlockReader.readTermStateIfNotRead()`.
    ///
    /// # Errors
    ///
    /// The I/O errors of decoding the term state.
    pub fn read_term_state_if_not_read(&mut self) -> Result<bool> {
        if !self.term_state_set {
            self.term_state_set = self.read_term_state()?;
            if self.term_state_set {
                self.scratch_term_state.term_block_ord = self.line_index_in_block;
                self.scratch_term_state.block_file_pointer = self.block_start_fp;
            }
        }
        Ok(self.term_state_set)
    }

    /// Reads the [`BlockTermState`] on the current line into the scratch
    /// term state; returns `false` if there is none (the term does not occur
    /// in the field of a shared-terms reader).
    ///
    /// Port of `BlockReader.readTermState()`, with the override of the
    /// `sharedterms` readers when `SharedTermsHooks` are set.
    ///
    /// # Errors
    ///
    /// The I/O errors of decoding the term state.
    pub fn read_term_state(&mut self) -> Result<bool> {
        match self.overrides.clone() {
            Some(overrides) => overrides.read_term_state(self),
            None => self.base_read_term_state(),
        }
    }

    /// The base implementation of `read_term_state` (`super.readTermState`,
    /// with the `sharedterms` readers' override when `SharedTermsHooks` are
    /// set).
    ///
    /// # Errors
    ///
    /// The I/O errors of decoding the term state.
    pub fn base_read_term_state(&mut self) -> Result<bool> {
        // We reuse scratchTermState safely as the read TermState is cloned in the termState()
        // method.
        let header = self.header()?;
        let line = self.block_line.ok_or_else(|| {
            super::npe("Cannot invoke \"org.apache.lucene.codecs.uniformsplit.BlockLine.getTermStateRelativeOffset()\" because \"this.blockLine\" is null")
        })?;
        let pos = self.block_first_line_start
            + array_len(header.get_term_states_base_offset())?
            + array_len(line.term_state_relative_offset)?;
        self.term_states_pos = pos;
        let (offset, limit) = self.block_window;
        let bytes: &[u8] = match &self.decoded_block {
            Some(decoded) => &decoded.bytes,
            None => &self.scratch_block_bytes,
        };
        let mut input = ByteArrayDataInput::with_range(bytes, offset, limit - offset);
        input.set_position(pos);
        let read = match &self.shared_terms {
            None => {
                let field_info = self.field_metadata.field_info_or_npe()?;
                self.term_state_serializer
                    .read_term_state(
                        header.get_base_docs_fp(),
                        header.get_base_positions_fp(),
                        header.get_base_payloads_fp(),
                        &mut input,
                        field_info,
                        Some(&mut self.scratch_term_state),
                    )
                    .map(|_| true)
            }
            Some(hooks) => {
                let field_number = self.field_metadata.field_info_or_npe()?.number;
                st_block_line::read_term_state_for_field(
                    field_number,
                    &mut input,
                    &self.term_state_serializer,
                    &header,
                    &hooks.field_infos,
                    Some(&mut self.scratch_term_state),
                )
                .map(|state| state.is_some())
            }
        };
        self.term_states_pos = input.position();
        read
    }

    /// Port of `BlockReader.term()`, Java's `null` being `None`.
    pub fn term_bytes(&self) -> Option<BytesRef> {
        self.current_term().map(|t| BytesRef::new(t.to_vec()))
    }

    /// The bytes of `term_bytes`, without a copy.
    pub fn current_term(&self) -> Option<&[u8]> {
        if self.term_state_forced {
            return Some(self.forced_term.as_deref().unwrap_or_default());
        }
        self.block_line
            .as_ref()
            .map(|_| self.block_line_reader.current_term())
    }

    /// The current term state, Java's `termState` after
    /// `readTermStateIfNotRead()`, or its `NullPointerException`.
    pub(crate) fn read_term_state_or_npe(&mut self, field: &str) -> Result<&BlockTermState> {
        if self.read_term_state_if_not_read()? {
            Ok(&self.scratch_term_state)
        } else {
            Err(super::npe(&format!(
                "Cannot read field \"{field}\" because \"this.termState\" is null"
            )))
        }
    }

    /// Port of `BlockReader.docFreq()`.
    ///
    /// # Errors
    ///
    /// The I/O errors of decoding the term state.
    pub fn doc_freq_impl(&mut self) -> Result<i32> {
        Ok(self.read_term_state_or_npe("docFreq")?.doc_freq)
    }

    /// Port of `BlockReader.totalTermFreq()`.
    ///
    /// # Errors
    ///
    /// The I/O errors of decoding the term state.
    pub fn total_term_freq_impl(&mut self) -> Result<i64> {
        Ok(self
            .read_term_state_or_npe("totalTermFreq")?
            .total_term_freq)
    }

    /// Port of `BlockReader.termState()`.
    ///
    /// # Errors
    ///
    /// The I/O errors of decoding the term state.
    pub fn term_state_impl(&mut self) -> Result<Box<dyn TermState>> {
        let state = *self.read_term_state_or_npe("termState")?;
        Ok(Box::new(state))
    }

    /// Port of `BlockReader.postings(PostingsEnum, int)`.
    ///
    /// # Errors
    ///
    /// The errors of decoding the term state and of the postings reader.
    pub fn postings_impl(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        let state = *self.read_term_state_or_npe("termState")?;
        let field_info = self.field_metadata.field_info_or_npe()?;
        self.postings_reader
            .postings(field_info, &state, reuse, flags)
    }

    /// Port of `BlockReader.impacts(int)`.
    ///
    /// # Errors
    ///
    /// The errors of decoding the term state and of the postings reader.
    pub fn impacts_impl(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        let state = *self.read_term_state_or_npe("termState")?;
        let field_info = self.field_metadata.field_info_or_npe()?;
        self.postings_reader.impacts(field_info, &state, flags)
    }

    /// Port of `BlockReader.getOrCreateDictionaryBrowser()`.
    ///
    /// # Errors
    ///
    /// The errors of loading the dictionary.
    pub fn get_or_create_dictionary_browser(&mut self) -> Result<&mut Box<dyn Browser>> {
        if self.dictionary_browser.is_none() {
            self.dictionary_browser = Some(self.dictionary_browser_supplier.get()?);
        }
        self.dictionary_browser
            .as_mut()
            .ok_or_else(|| LuceneError::illegal_state("the dictionary browser is set above"))
    }

    /// Called by the primary `TermsEnum` methods to clear the previous
    /// `TermState`.
    ///
    /// Port of `BlockReader.clearTermState()`.
    pub fn clear_term_state(&mut self) {
        self.term_state_set = false;
        self.term_state_forced = false;
    }

    /// Port of `BlockReader.newCorruptIndexException(String, Long)`.
    pub fn new_corrupt_index_exception(&self, msg: &str, fp: Option<i64>) -> LuceneError {
        let field = match self.field_metadata.field_name() {
            Ok(field) => field,
            Err(npe) => return npe,
        };
        let resource = match &self.block_input {
            Some(input) => input.resource_description().to_string(),
            None => self
                .original_block_input
                .read()
                .resource_description()
                .to_string(),
        };
        LuceneError::corrupt_index(
            format!(
                "{msg}{} for field \"{field}\"",
                fp.map(|fp| format!(" at FP {fp}")).unwrap_or_default()
            ),
            resource,
        )
    }
}

impl Accountable for BlockReader {
    /// Port of `BlockReader.ramBytesUsed()`.
    fn ram_bytes_used(&self) -> i64 {
        let initialized = self.block_input.is_some();
        RamUsageEstimator::shallow_size_of_instance::<BlockReader>()
            + RamUsageEstimator::shallow_size_of_instance::<Box<dyn IndexInput>>()
            + RamUsageEstimator::shallow_size_of_instance::<ByteArrayDataInput<'_>>() * 2
            + if initialized {
                self.block_line_reader.ram_bytes_used()
                    + ram_usage_util::ram_bytes_used_by_byte_array_of_length(
                        self.block_window.1 as i32,
                    )
                    + self.term_state_serializer.ram_bytes_used()
            } else {
                0
            }
            + self
                .forced_term
                .as_deref()
                .map_or(0, ram_usage_util::ram_bytes_used_bytes_ref_builder)
            + self
                .block_header
                .as_ref()
                .map_or(0, Accountable::ram_bytes_used)
            + self.block_line.map_or(0, |_| {
                RamUsageEstimator::shallow_size_of_instance::<super::BlockLine>()
                    + RamUsageEstimator::shallow_size_of_instance::<super::TermBytes>()
            })
            + if self.term_state_set {
                ram_usage_util::ram_bytes_used_term_state(&self.scratch_term_state)
            } else {
                0
            }
    }
}

impl fmt::Display for BlockReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.uniformsplit.BlockReader",
            self,
        ))
    }
}

fn unsupported() -> LuceneError {
    LuceneError::unsupported_operation(String::new())
}

impl TermsEnum for BlockReader {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        self.seek_exact_impl(text)
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.seek_ceil_impl(text)
    }

    /// Not supported.
    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(unsupported())
    }

    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.seek_exact_state(text, state);
        Ok(())
    }

    fn term(&self) -> Result<Option<BytesRef>> {
        // `null` when unpositioned, as Java's `term()` returns.
        Ok(self.term_bytes())
    }

    fn ord(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn doc_freq(&mut self) -> Result<i32> {
        self.doc_freq_impl()
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.total_term_freq_impl()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        Ok(Some(self.postings_impl(reuse, flags)?))
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.impacts_impl(flags)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.term_state_impl()
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.next_impl()
    }
}
