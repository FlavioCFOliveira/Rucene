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

//! Port of `org.apache.lucene.codecs.blockterms.FixedGapTermsIndexWriter`.

use std::cell::RefCell;
use std::rc::Rc;

use rucene::codecs::codec_util::{write_footer, write_index_header};
use rucene::codecs::{SegmentWriteState, TermStats};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::FieldInfo;
use rucene::store::{ByteBuffersDataOutput, IndexOutput};
use rucene::util::packed::{MonotonicBlockPackedWriter, PackedInts};
use rucene::util::string_helper::StringHelper;
use rucene::util::{BytesRef, IOUtils};

use super::terms_index_writer_base::{FieldWriter, TermsIndexWriterBase};

/// Extension of terms index file.
pub(crate) const TERMS_INDEX_EXTENSION: &str = "tii";
pub(crate) const CODEC_NAME: &str = "FixedGapTermsIndex";
pub(crate) const VERSION_START: i32 = 4;
pub(crate) const VERSION_CURRENT: i32 = VERSION_START;
pub(crate) const BLOCKSIZE: usize = 4096;

/// Port of `FixedGapTermsIndexWriter.DEFAULT_TERM_INDEX_INTERVAL`.
pub const DEFAULT_TERM_INDEX_INTERVAL: i32 = 32;

/// What the writer and its field writers share: Java's `out` and `fields`,
/// which the inner `SimpleFieldWriter`s reach through their outer instance.
struct Shared {
    out: Option<Box<dyn IndexOutput>>,
    term_index_interval: i32,
    fields: Vec<Rc<RefCell<SimpleFieldWriterState>>>,
}

impl Shared {
    fn out(&mut self) -> Result<&mut Box<dyn IndexOutput>> {
        self.out
            .as_mut()
            .ok_or_else(|| LuceneError::already_closed("this IndexOutput is closed"))
    }
}

/// Selects every Nth term as and index term, and hold term bytes (mostly)
/// fully expanded in memory. This terms index supports seeking by ord. See
/// [`VariableGapTermsIndexWriter`](super::VariableGapTermsIndexWriter) for a
/// more memory efficient terms index that does not support seeking by ord.
///
/// Port of `org.apache.lucene.codecs.blockterms.FixedGapTermsIndexWriter`.
///
/// Java's protected hook `indexedTermPrefixLength` (for codecs that do not
/// sort in unicode code point order) has no override in Lucene 10.5.0; it is
/// the private `indexed_term_prefix_length`.
pub struct FixedGapTermsIndexWriter {
    shared: Rc<RefCell<Shared>>,
}

impl FixedGapTermsIndexWriter {
    /// Port of `FixedGapTermsIndexWriter(SegmentWriteState)`, with
    /// [`DEFAULT_TERM_INDEX_INTERVAL`].
    pub fn new(state: &SegmentWriteState<'_>) -> Result<Self> {
        Self::with_interval(state, DEFAULT_TERM_INDEX_INTERVAL)
    }

    /// Port of `FixedGapTermsIndexWriter(SegmentWriteState, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `term_index_interval <= 0`; the I/O
    /// errors of creating the file and writing its header.
    pub fn with_interval(state: &SegmentWriteState<'_>, term_index_interval: i32) -> Result<Self> {
        if term_index_interval <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid termIndexInterval: {term_index_interval}"
            )));
        }
        let index_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_INDEX_EXTENSION,
        );
        let mut out = state
            .directory
            .create_output(&index_file_name, state.context)?;
        let written = (|| -> Result<()> {
            write_index_header(
                out.as_mut(),
                CODEC_NAME,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            out.write_v_int(term_index_interval)?;
            out.write_v_int(PackedInts::VERSION_CURRENT)?;
            out.write_v_int(BLOCKSIZE as i32)
        })();
        if let Err(e) = written {
            IOUtils::close_while_handling_exception([IOUtils::closeable(|| out.close())])?;
            return Err(e);
        }
        Ok(Self {
            shared: Rc::new(RefCell::new(Shared {
                out: Some(out),
                term_index_interval,
                fields: Vec::new(),
            })),
        })
    }
}

/// Port of `FixedGapTermsIndexWriter.indexedTermPrefixLength`:
/// `StringHelper.sortKeyLength(priorTerm, indexedTerm)`.
fn indexed_term_prefix_length(prior_term: &BytesRef, indexed_term: &BytesRef) -> Result<i32> {
    StringHelper::sort_key_length(prior_term, indexed_term)
}

/// The fields of `SimpleFieldWriter` that `close()` reads back.
struct SimpleFieldWriterState {
    field_number: i32,
    num_index_terms: i32,
    index_start: i64,
    terms_start: i64,
    packed_index_start: i64,
    packed_offsets_start: i64,
}

/// Port of the private inner class `FixedGapTermsIndexWriter.SimpleFieldWriter`.
///
/// Java feeds `termOffsets` and `termAddresses`, two
/// `MonotonicBlockPackedWriter`s over the buffers `offsetsBuffer` and
/// `addressBuffer`, value by value, and finishes them in `finish`. A Rust
/// writer borrows its output for its whole life, so the values are kept here
/// and fed to the writers in `finish`; a block-packed writer's bytes depend
/// only on the sequence of values, so the buffers are byte for byte Java's.
struct SimpleFieldWriter {
    shared: Rc<RefCell<Shared>>,
    state: Rc<RefCell<SimpleFieldWriterState>>,
    num_terms: i64,
    term_offsets: Vec<i64>,
    current_offset: i64,
    term_addresses: Vec<i64>,
    last_term: BytesRef,
}

impl FieldWriter for SimpleFieldWriter {
    fn check_index_term(&mut self, text: &BytesRef, _stats: &TermStats) -> Result<bool> {
        let interval = i64::from(self.shared.borrow().term_index_interval);
        // First term is first indexed term:
        let n = self.num_terms;
        self.num_terms += 1;
        if 0 == n % interval {
            Ok(true)
        } else {
            if 0 == self.num_terms % interval {
                // save last term just before next index term so we
                // can compute wasted suffix
                self.last_term = BytesRef::deep_copy_of(text);
            }
            Ok(false)
        }
    }

    fn add(&mut self, text: &BytesRef, _stats: &TermStats, terms_file_pointer: i64) -> Result<()> {
        let mut state = self.state.borrow_mut();
        let indexed_term_length = if state.num_index_terms == 0 {
            // no previous term: no bytes to write
            0
        } else {
            indexed_term_prefix_length(&self.last_term, text)?
        };

        // write only the min prefix that shows the diff
        // against prior term
        self.shared.borrow_mut().out()?.write_bytes(
            &text.bytes,
            text.offset,
            indexed_term_length as usize,
        )?;

        // save delta terms pointer
        self.term_addresses
            .push(terms_file_pointer - state.terms_start);

        // save term length (in bytes)
        debug_assert!(indexed_term_length <= i32::from(i16::MAX));
        self.current_offset += i64::from(indexed_term_length);
        self.term_offsets.push(self.current_offset);

        self.last_term = BytesRef::deep_copy_of(text);
        state.num_index_terms += 1;
        Ok(())
    }

    fn finish(&mut self, _terms_file_pointer: i64) -> Result<()> {
        let mut shared = self.shared.borrow_mut();
        let out = shared.out()?;
        let mut state = self.state.borrow_mut();

        // write primary terms dict offsets
        state.packed_index_start = out.file_pointer();

        // relative to our indexStart
        let mut address_buffer = ByteBuffersDataOutput::new_resettable_instance();
        {
            let mut term_addresses =
                MonotonicBlockPackedWriter::new(&mut address_buffer, BLOCKSIZE)?;
            for &v in &self.term_addresses {
                term_addresses.add(v)?;
            }
            term_addresses.finish()?;
        }
        address_buffer.copy_to(out.as_mut())?;

        state.packed_offsets_start = out.file_pointer();

        // write offsets into the byte[] terms
        let mut offsets_buffer = ByteBuffersDataOutput::new_resettable_instance();
        {
            let mut term_offsets = MonotonicBlockPackedWriter::new(&mut offsets_buffer, BLOCKSIZE)?;
            for &v in &self.term_offsets {
                term_offsets.add(v)?;
            }
            term_offsets.finish()?;
        }
        offsets_buffer.copy_to(out.as_mut())?;
        self.term_offsets = Vec::new();
        self.term_addresses = Vec::new();
        Ok(())
    }
}

impl TermsIndexWriterBase for FixedGapTermsIndexWriter {
    fn add_field(
        &mut self,
        field_info: &FieldInfo,
        terms_file_pointer: i64,
    ) -> Result<Box<dyn FieldWriter>> {
        let index_start = self.shared.borrow_mut().out()?.file_pointer();
        let state = Rc::new(RefCell::new(SimpleFieldWriterState {
            field_number: field_info.number,
            num_index_terms: 0,
            index_start,
            terms_start: terms_file_pointer,
            packed_index_start: 0,
            packed_offsets_start: 0,
        }));
        self.shared.borrow_mut().fields.push(Rc::clone(&state));
        Ok(Box::new(SimpleFieldWriter {
            shared: Rc::clone(&self.shared),
            state,
            num_terms: 0,
            // `termOffsets.add(0L)` in Java's constructor.
            term_offsets: vec![0],
            current_offset: 0,
            term_addresses: Vec::new(),
            last_term: BytesRef::default(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        let mut shared = self.shared.borrow_mut();
        let Some(mut out) = shared.out.take() else {
            return Ok(());
        };
        let written = (|| -> Result<()> {
            let dir_start = out.file_pointer();
            let fields: Vec<_> = shared.fields.iter().map(|f| f.borrow()).collect();
            let non_null_field_count = fields.iter().filter(|f| f.num_index_terms > 0).count();
            out.write_v_int(non_null_field_count as i32)?;
            for field in fields.iter().filter(|f| f.num_index_terms > 0) {
                out.write_v_int(field.field_number)?;
                out.write_v_int(field.num_index_terms)?;
                out.write_v_long(field.terms_start)?;
                out.write_v_long(field.index_start)?;
                out.write_v_long(field.packed_index_start)?;
                out.write_v_long(field.packed_offsets_start)?;
            }
            // writeTrailer(dirStart)
            out.write_long(dir_start)?;
            write_footer(out.as_mut())
        })();
        match written {
            Ok(()) => out.close(),
            Err(e) => {
                IOUtils::close_while_handling_exception([IOUtils::closeable(|| out.close())])?;
                Err(e)
            }
        }
    }
}
