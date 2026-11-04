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

//! Port of `org.apache.lucene.codecs.blockterms.VariableGapTermsIndexWriter`.

use std::cell::RefCell;
use std::rc::Rc;

use rucene::codecs::codec_util::{write_footer, write_index_header};
use rucene::codecs::{SegmentWriteState, TermStats};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::FieldInfo;
use rucene::store::IndexOutput;
use rucene::util::fst::{Builder, FSTCompiler, InputType, PositiveIntOutputs, Util, FST};
use rucene::util::{BytesRef, IOUtils, IntsRef, IntsRefBuilder};

use super::terms_index_writer_base::{FieldWriter, TermsIndexWriterBase};

/// Extension of terms index file.
pub(crate) const TERMS_INDEX_EXTENSION: &str = "tiv";
/// Extension of terms meta file.
pub(crate) const TERMS_META_EXTENSION: &str = "tmv";
pub(crate) const META_CODEC_NAME: &str = "VariableGapTermsMeta";
pub(crate) const CODEC_NAME: &str = "VariableGapTermsIndex";
pub(crate) const VERSION_START: i32 = 4;
pub(crate) const VERSION_CURRENT: i32 = VERSION_START;

/// Hook for selecting which terms should be placed in the terms index.
///
/// [`new_field`](Self::new_field) is called at the start of each new field,
/// and [`is_index_term`](Self::is_index_term) for each term in that field.
///
/// Port of `VariableGapTermsIndexWriter.IndexTermSelector`.
pub trait IndexTermSelector {
    /// Called sequentially on every term being written, returning true if
    /// this term should be indexed.
    fn is_index_term(&mut self, term: &BytesRef, stats: &TermStats) -> bool;

    /// Called when a new field is started.
    fn new_field(&mut self, field_info: &FieldInfo);
}

/// Same policy as [`FixedGapTermsIndexWriter`](super::FixedGapTermsIndexWriter).
///
/// Port of `VariableGapTermsIndexWriter.EveryNTermSelector`.
#[derive(Debug, Clone)]
pub struct EveryNTermSelector {
    count: i32,
    interval: i32,
}

impl EveryNTermSelector {
    /// Port of `EveryNTermSelector(int)`.
    pub fn new(interval: i32) -> Self {
        // First term is first indexed term:
        Self {
            count: interval,
            interval,
        }
    }
}

impl IndexTermSelector for EveryNTermSelector {
    fn is_index_term(&mut self, _term: &BytesRef, _stats: &TermStats) -> bool {
        if self.count >= self.interval {
            self.count = 1;
            true
        } else {
            self.count += 1;
            false
        }
    }

    fn new_field(&mut self, _field_info: &FieldInfo) {
        self.count = self.interval;
    }
}

/// Sets an index term when `docFreq >= docFreqThresh`, or every interval
/// terms. This should reduce seek time to high docFreq terms.
///
/// Port of `VariableGapTermsIndexWriter.EveryNOrDocFreqTermSelector`.
#[derive(Debug, Clone)]
pub struct EveryNOrDocFreqTermSelector {
    count: i32,
    doc_freq_thresh: i32,
    interval: i32,
}

impl EveryNOrDocFreqTermSelector {
    /// Port of `EveryNOrDocFreqTermSelector(int, int)`.
    pub fn new(doc_freq_thresh: i32, interval: i32) -> Self {
        // First term is first indexed term:
        Self {
            count: interval,
            doc_freq_thresh,
            interval,
        }
    }
}

impl IndexTermSelector for EveryNOrDocFreqTermSelector {
    fn is_index_term(&mut self, _term: &BytesRef, stats: &TermStats) -> bool {
        if stats.doc_freq >= self.doc_freq_thresh || self.count >= self.interval {
            self.count = 1;
            true
        } else {
            self.count += 1;
            false
        }
    }

    fn new_field(&mut self, _field_info: &FieldInfo) {
        self.count = self.interval;
    }
}

/// What the writer and its field writers share: Java's `metaOut`, `out` and
/// `policy`, which the inner `FSTFieldWriter`s reach through their outer
/// instance.
struct Shared {
    meta_out: Option<Box<dyn IndexOutput>>,
    out: Option<Box<dyn IndexOutput>>,
    policy: Box<dyn IndexTermSelector>,
}

fn closed() -> LuceneError {
    LuceneError::already_closed("this IndexOutput is closed")
}

/// Selects index terms according to provided pluggable
/// [`IndexTermSelector`], and stores them in a prefix trie that's loaded
/// entirely in RAM stored as an FST. This terms index only supports unsigned
/// byte term sort order (unicode codepoint order when the bytes are UTF8).
///
/// Port of `org.apache.lucene.codecs.blockterms.VariableGapTermsIndexWriter`.
///
/// Java's protected hook `indexedTermPrefixLength` has no override in Lucene
/// 10.5.0; it is the private `indexed_term_prefix_length`.
pub struct VariableGapTermsIndexWriter {
    shared: Rc<RefCell<Shared>>,
}

impl VariableGapTermsIndexWriter {
    /// Port of `VariableGapTermsIndexWriter(SegmentWriteState,
    /// IndexTermSelector)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of creating the two files and writing their headers;
    /// on failure the writer is closed (`IOUtils.closeWhileHandlingException(this)`).
    pub fn new(state: &SegmentWriteState<'_>, policy: Box<dyn IndexTermSelector>) -> Result<Self> {
        let meta_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_META_EXTENSION,
        );
        let index_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_INDEX_EXTENSION,
        );
        let mut writer = Self {
            shared: Rc::new(RefCell::new(Shared {
                meta_out: None,
                out: None,
                policy,
            })),
        };
        let opened = (|| -> Result<()> {
            let mut shared = writer.shared.borrow_mut();
            shared.meta_out = Some(
                state
                    .directory
                    .create_output(&meta_file_name, state.context)?,
            );
            shared.out = Some(
                state
                    .directory
                    .create_output(&index_file_name, state.context)?,
            );
            let segment_id = state.segment_info.id();
            write_index_header(
                shared.meta_out.as_mut().ok_or_else(closed)?.as_mut(),
                META_CODEC_NAME,
                VERSION_CURRENT,
                &segment_id,
                &state.segment_suffix,
            )?;
            write_index_header(
                shared.out.as_mut().ok_or_else(closed)?.as_mut(),
                CODEC_NAME,
                VERSION_CURRENT,
                &segment_id,
                &state.segment_suffix,
            )
        })();
        if let Err(e) = opened {
            // `IOUtils.closeWhileHandlingException(this)`
            IOUtils::close_while_handling_exception([IOUtils::closeable(|| writer.close())])?;
            return Err(e);
        }
        Ok(writer)
    }
}

/// Port of `VariableGapTermsIndexWriter.indexedTermPrefixLength`.
fn indexed_term_prefix_length(prior_term: &[u8], indexed_term: &[u8]) -> usize {
    // As long as codec sorts terms in unicode codepoint
    // order, we can safely strip off the non-distinguishing
    // suffix to save RAM in the loaded terms index.
    let limit = prior_term.len().min(indexed_term.len());
    for byte_idx in 0..limit {
        if prior_term[byte_idx] != indexed_term[byte_idx] {
            return byte_idx + 1;
        }
    }
    (1 + prior_term.len()).min(indexed_term.len())
}

/// Port of the private inner class `VariableGapTermsIndexWriter.FSTFieldWriter`.
struct FSTFieldWriter {
    shared: Rc<RefCell<Shared>>,
    fst_compiler: FSTCompiler<'static, PositiveIntOutputs>,
    start_terms_file_pointer: i64,
    field_number: i32,
    last_term: Vec<u8>,
    first: bool,
    scratch_ints_ref: IntsRefBuilder,
}

impl FieldWriter for FSTFieldWriter {
    fn check_index_term(&mut self, text: &BytesRef, stats: &TermStats) -> Result<bool> {
        if self.shared.borrow_mut().policy.is_index_term(text, stats) || self.first {
            self.first = false;
            Ok(true)
        } else {
            self.last_term = text.slice().to_vec();
            Ok(false)
        }
    }

    fn add(&mut self, text: &BytesRef, _stats: &TermStats, terms_file_pointer: i64) -> Result<()> {
        if text.length == 0 {
            // We already added empty string in ctor
            debug_assert_eq!(terms_file_pointer, self.start_terms_file_pointer);
            return Ok(());
        }
        let prefix_length = indexed_term_prefix_length(&self.last_term, text.slice());
        let prefix = BytesRef::new(text.slice()[..prefix_length].to_vec());
        let input = Util::to_ints_ref(&prefix, &mut self.scratch_ints_ref);
        self.fst_compiler.add(&input, terms_file_pointer)?;
        self.last_term = text.slice().to_vec();
        Ok(())
    }

    fn finish(&mut self, _terms_file_pointer: i64) -> Result<()> {
        let metadata = self.fst_compiler.compile()?;
        let compiler = std::mem::replace(
            &mut self.fst_compiler,
            Builder::new(InputType::Byte1, PositiveIntOutputs::get_singleton()).build()?,
        );
        let fst: Option<FST<PositiveIntOutputs>> =
            FST::from_fst_reader(metadata, compiler.into_fst_reader()?);
        if let Some(fst) = fst {
            let mut shared = self.shared.borrow_mut();
            let Shared { meta_out, out, .. } = &mut *shared;
            let meta_out = meta_out.as_mut().ok_or_else(closed)?;
            let out = out.as_mut().ok_or_else(closed)?;
            meta_out.write_int(self.field_number)?;
            meta_out.write_v_long(out.file_pointer())?;
            fst.save(meta_out.as_mut(), out.as_mut())?;
        }
        Ok(())
    }
}

impl TermsIndexWriterBase for VariableGapTermsIndexWriter {
    fn add_field(
        &mut self,
        field_info: &FieldInfo,
        terms_file_pointer: i64,
    ) -> Result<Box<dyn FieldWriter>> {
        self.shared.borrow_mut().policy.new_field(field_info);
        let mut fst_compiler =
            Builder::new(InputType::Byte1, PositiveIntOutputs::get_singleton()).build()?;
        // Always put empty string in
        fst_compiler.add(&IntsRef::default(), terms_file_pointer)?;
        Ok(Box::new(FSTFieldWriter {
            shared: Rc::clone(&self.shared),
            fst_compiler,
            start_terms_file_pointer: terms_file_pointer,
            field_number: field_info.number,
            last_term: Vec::new(),
            first: true,
            scratch_ints_ref: IntsRefBuilder::new(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        let mut shared = self.shared.borrow_mut();
        let written = (|| -> Result<()> {
            if let Some(meta_out) = shared.meta_out.as_mut() {
                meta_out.write_int(-1)?;
                write_footer(meta_out.as_mut())?;
            }
            if let Some(out) = shared.out.as_mut() {
                write_footer(out.as_mut())?;
            }
            Ok(())
        })();
        // finally: IOUtils.close(out, metaOut); out = null; metaOut = null;
        let mut out = shared.out.take();
        let mut meta_out = shared.meta_out.take();
        let closed = IOUtils::close([
            out.as_mut()
                .and_then(|o| IOUtils::closeable(move || o.close())),
            meta_out
                .as_mut()
                .and_then(|o| IOUtils::closeable(move || o.close())),
        ]);
        // A `finally` that throws replaces the exception of its `try`.
        closed?;
        written
    }
}
