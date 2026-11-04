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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextFieldsReader`.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use rucene::codecs::{FieldsProducer, SegmentReadState};
use rucene::error::{LuceneError, Result};
use rucene::index::{
    feature_requested, FieldInfo, FieldInfos, Fields, Impacts, ImpactsEnum, ImpactsSource,
    IndexOptions, PostingsEnum, SeekStatus, SlowImpactsEnum, Terms, TermsEnum,
    POSTINGS_ENUM_POSITIONS,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{BufferedChecksumIndexInput, IndexInput};
use rucene::util::attribute::AttributeSource;
use rucene::util::fst::{
    Builder, BytesRefFSTEnum, InputType, Pair, PairOutputs, PositiveIntOutputs, Util, FST,
};
use rucene::util::java_fmt::java_utf8_to_string;
use rucene::util::{BytesRef, FixedBitSet, IntsRefBuilder};

use super::fields_writer::{DOC, END, END_OFFSET, FIELD, FREQ, PAYLOAD, POS, START_OFFSET, TERM};
use super::postings_format::get_postings_file_name;
use super::skip_reader::SimpleTextSkipReader;
use super::skip_writer::{BLOCK_SIZE, SKIP_LIST};
use super::util::{self, parse_int_at, read_line};

/// `PairOutputs.Pair<PairOutputs.Pair<Long, Long>, PairOutputs.Pair<Long, Long>>`:
/// `((docsStart, skipPointer), (docFreq, totalTermFreq))`.
type TermOutput = Pair<Pair<i64, i64>, Pair<i64, i64>>;

/// The outputs of a field's terms FST.
type TermOutputs = PairOutputs<
    PairOutputs<PositiveIntOutputs, PositiveIntOutputs>,
    PairOutputs<PositiveIntOutputs, PositiveIntOutputs>,
>;

/// The reader's `in`: every enumeration clones it, and `close` closes it.
struct SharedInput {
    input: Mutex<Option<Box<dyn IndexInput>>>,
}

impl SharedInput {
    /// `in.clone()`.
    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        let guard = self.input.lock().map_err(|_| {
            LuceneError::illegal_state("postings input lock was poisoned".to_string())
        })?;
        match guard.as_ref() {
            Some(input) => input.clone_input(),
            None => Err(LuceneError::already_closed("this IndexInput is closed")),
        }
    }
}

/// Reads the plain-text postings of a segment.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextFieldsReader`.
///
/// `getMergeInstance()` returns the reader itself in Java; the port's
/// returns a handle that shares all of this reader's state.
#[derive(Clone)]
pub struct SimpleTextFieldsReader {
    /// `TreeMap<String, Long> fields`, in `String.compareTo` order.
    fields: Arc<Vec<(String, i64)>>,
    input: Arc<SharedInput>,
    field_infos: Arc<FieldInfos>,
    max_doc: i32,
    terms_cache: Arc<Mutex<HashMap<String, SimpleTextTerms>>>,
}

impl fmt::Debug for SimpleTextFieldsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for SimpleTextFieldsReader {
    /// `getClass().getSimpleName() + "(fields=" + fields.size() + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SimpleTextFieldsReader(fields={})", self.fields.len())
    }
}

impl SimpleTextFieldsReader {
    /// Port of `SimpleTextFieldsReader(SegmentReadState)`.
    ///
    /// # Errors
    ///
    /// Propagates the directory's error, and the footer check's.
    pub fn new(state: &SegmentReadState<'_>) -> Result<Self> {
        let max_doc = state.segment_info.max_doc()?;
        let field_infos = state.field_infos.clone();
        let input = state.directory.open_input(
            &get_postings_file_name(&state.segment_info.name, &state.segment_suffix),
            state.context,
        )?;
        let fields = Self::read_fields(input.clone_input()?)?;
        Ok(Self {
            fields: Arc::new(fields),
            input: Arc::new(SharedInput {
                input: Mutex::new(Some(input)),
            }),
            field_infos: Arc::new(field_infos),
            max_doc,
            terms_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Port of the private `SimpleTextFieldsReader.readFields`.
    fn read_fields(input: Box<dyn IndexInput>) -> Result<Vec<(String, i64)>> {
        let mut input = BufferedChecksumIndexInput::new(input);
        let mut scratch = Vec::new();
        let mut fields: Vec<(String, i64)> = Vec::new();

        loop {
            read_line(&mut input, &mut scratch)?;
            if scratch.as_slice() == END {
                util::check_footer(&mut input)?;
                return Ok(fields);
            } else if scratch.starts_with(FIELD) {
                let field_name = java_utf8_to_string(&scratch[FIELD.len()..]);
                let fp = input.file_pointer();
                // `TreeMap.put`.
                match fields.binary_search_by(|(name, _)| {
                    name.encode_utf16().cmp(field_name.encode_utf16())
                }) {
                    Ok(i) => fields[i].1 = fp,
                    Err(i) => fields.insert(i, (field_name, fp)),
                }
            }
        }
    }
}

impl Fields for SimpleTextFieldsReader {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.fields.iter().map(|(name, _)| name.clone()))
    }

    /// Port of the `synchronized` `SimpleTextFieldsReader.terms`.
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let mut cache = self
            .terms_cache
            .lock()
            .map_err(|_| LuceneError::illegal_state("terms cache lock was poisoned".to_string()))?;
        if let Some(terms) = cache.get(field) {
            return Ok(Some(Box::new(terms.clone())));
        }
        let Some(fp) = self
            .fields
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, fp)| *fp)
        else {
            return Ok(None);
        };
        let terms = SimpleTextTerms::new(
            field,
            fp,
            self.max_doc,
            &self.field_infos,
            Arc::clone(&self.input),
        )?;
        cache.insert(field.to_string(), terms.clone());
        Ok(Some(Box::new(terms)))
    }

    fn size(&self) -> i32 {
        -1
    }
}

impl FieldsProducer for SimpleTextFieldsReader {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        let input = self
            .input
            .input
            .lock()
            .map_err(|_| {
                LuceneError::illegal_state("postings input lock was poisoned".to_string())
            })?
            .take();
        match input {
            Some(mut input) => input.close(),
            None => Ok(()),
        }
    }
}

/// The per-term outputs a terms enumeration exposes.
#[derive(Clone, Copy, Default)]
struct TermMeta {
    doc_freq: i32,
    total_term_freq: i64,
    docs_start: i64,
    skip_pointer: i64,
}

impl TermMeta {
    fn from_output(output: &TermOutput) -> Self {
        Self {
            docs_start: output.output1.output1,
            skip_pointer: output.output1.output2,
            doc_freq: output.output2.output1 as i32,
            total_term_freq: output.output2.output2,
        }
    }
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextFieldsReader.SimpleTextTermsEnum`.
struct SimpleTextTermsEnum {
    index_options: IndexOptions,
    meta: TermMeta,
    fst_enum: BytesRefFSTEnum<'static, TermOutputs>,
    input: Arc<SharedInput>,
    attributes: Option<AttributeSource>,
}

impl fmt::Display for SimpleTextTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SimpleTextTermsEnum")
    }
}

impl SimpleTextTermsEnum {
    fn has_positions(&self) -> bool {
        self.index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    /// A new `SimpleTextPostingsEnum`, reset to the current term.
    fn new_postings_enum(&self) -> Result<SimpleTextPostingsEnum> {
        let mut e = SimpleTextPostingsEnum::new(Arc::clone(&self.input))?;
        let meta = self.meta;
        e.reset(
            meta.docs_start,
            self.index_options,
            meta.doc_freq,
            meta.skip_pointer,
        )?;
        Ok(e)
    }

    /// A new `SimpleTextDocsEnum`, reset to the current term.
    fn new_docs_enum(&self) -> Result<SimpleTextDocsEnum> {
        let mut e = SimpleTextDocsEnum::new(Arc::clone(&self.input))?;
        let meta = self.meta;
        e.reset(
            meta.docs_start,
            self.index_options == IndexOptions::DOCS,
            meta.doc_freq,
            meta.skip_pointer,
        )?;
        Ok(e)
    }
}

/// `reuse != null && reuse instanceof T && ((T) reuse).canReuse(in)`, then
/// the reused enum, or `None`.
fn reusable<T: PostingsEnum + 'static>(
    reuse: Option<Box<dyn PostingsEnum>>,
    can_reuse: impl Fn(&T) -> bool,
) -> Option<Box<dyn PostingsEnum>> {
    let mut reuse = reuse?;
    let ok = reuse
        .as_any_mut()
        .and_then(|any| any.downcast_mut::<T>())
        .is_some_and(|e| can_reuse(e));
    ok.then_some(reuse)
}

/// The concrete enum inside a box [`reusable`] accepted.
fn concrete<T: 'static>(e: &mut Box<dyn PostingsEnum>) -> Result<&mut T> {
    e.as_any_mut()
        .and_then(|any| any.downcast_mut::<T>())
        .ok_or_else(|| {
            LuceneError::illegal_state("INVARIANT: the reused enum was checked".to_string())
        })
}

impl TermsEnum for SimpleTextTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.attributes.get_or_insert_with(AttributeSource::new)
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        let meta = self
            .fst_enum
            .seek_exact(text)?
            .map(|result| TermMeta::from_output(result.output));
        match meta {
            Some(meta) => {
                self.meta = meta;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        let found = self.fst_enum.seek_ceil(text)?.map(|result| {
            (
                TermMeta::from_output(result.output),
                result.input.slice() == text.slice(),
            )
        });
        match found {
            None => Ok(SeekStatus::END),
            Some((meta, equal)) => {
                self.meta = meta;
                Ok(if equal {
                    SeekStatus::FOUND
                } else {
                    SeekStatus::NOT_FOUND
                })
            }
        }
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        let result = self
            .fst_enum
            .next()?
            .map(|result| (TermMeta::from_output(result.output), result.input.clone()));
        match result {
            Some((meta, term)) => {
                self.meta = meta;
                Ok(Some(BytesRef::new(term.slice().to_vec())))
            }
            None => Ok(None),
        }
    }

    fn term(&self) -> Result<BytesRef> {
        self.fst_enum
            .current()
            .map(|current| BytesRef::new(current.input.slice().to_vec()))
            .ok_or_else(|| {
                LuceneError::illegal_state(
                    "Cannot read field \"input\" because the return value of \
                     \"org.apache.lucene.util.fst.BytesRefFSTEnum.current()\" is null"
                        .to_string(),
                )
            })
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Ok(self.meta.doc_freq)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Ok(if self.index_options == IndexOptions::DOCS {
            i64::from(self.meta.doc_freq)
        } else {
            self.meta.total_term_freq
        })
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        let meta = self.meta;
        if self.has_positions() && feature_requested(flags, POSTINGS_ENUM_POSITIONS) {
            let input = Arc::clone(&self.input);
            return match reusable::<SimpleTextPostingsEnum>(reuse, |e| e.can_reuse(&input)) {
                Some(mut e) => {
                    concrete::<SimpleTextPostingsEnum>(&mut e)?.reset(
                        meta.docs_start,
                        self.index_options,
                        meta.doc_freq,
                        meta.skip_pointer,
                    )?;
                    Ok(e)
                }
                None => Ok(Box::new(self.new_postings_enum()?)),
            };
        }

        let input = Arc::clone(&self.input);
        match reusable::<SimpleTextDocsEnum>(reuse, |e| e.can_reuse(&input)) {
            Some(mut e) => {
                concrete::<SimpleTextDocsEnum>(&mut e)?.reset(
                    meta.docs_start,
                    self.index_options == IndexOptions::DOCS,
                    meta.doc_freq,
                    meta.skip_pointer,
                )?;
                Ok(e)
            }
            None => Ok(Box::new(self.new_docs_enum()?)),
        }
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        if self.meta.doc_freq <= BLOCK_SIZE {
            // no skip data
            return Ok(Box::new(SlowImpactsEnum::new(self.postings(None, flags)?)));
        }
        // `(ImpactsEnum) postings(null, flags)`.
        if self.has_positions() && feature_requested(flags, POSTINGS_ENUM_POSITIONS) {
            Ok(Box::new(self.new_postings_enum()?))
        } else {
            Ok(Box::new(self.new_docs_enum()?))
        }
    }
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextFieldsReader.SimpleTextDocsEnum`.
struct SimpleTextDocsEnum {
    in_start: Arc<SharedInput>,
    input: Box<dyn IndexInput>,
    omit_tf: bool,
    doc_id: i32,
    tf: i32,
    scratch: Vec<u8>,
    cost: i32,

    // for skip list data
    skip_reader: SimpleTextSkipReader,
    next_skip_doc: i32,
    seek_to: i64,
}

impl SimpleTextDocsEnum {
    fn new(in_start: Arc<SharedInput>) -> Result<Self> {
        let input = in_start.clone_input()?;
        let skip_reader = SimpleTextSkipReader::new(in_start.clone_input()?);
        Ok(Self {
            in_start,
            input,
            omit_tf: false,
            doc_id: -1,
            tf: 0,
            scratch: Vec::new(),
            cost: 0,
            skip_reader,
            next_skip_doc: 0,
            seek_to: -1,
        })
    }

    fn can_reuse(&self, input: &Arc<SharedInput>) -> bool {
        Arc::ptr_eq(input, &self.in_start)
    }

    fn reset(&mut self, fp: i64, omit_tf: bool, doc_freq: i32, skip_pointer: i64) -> Result<()> {
        self.input.seek(fp)?;
        self.omit_tf = omit_tf;
        self.doc_id = -1;
        self.tf = 1;
        self.cost = doc_freq;
        self.skip_reader.reset(skip_pointer, doc_freq)?;
        self.next_skip_doc = 0;
        self.seek_to = -1;
        Ok(())
    }

    // Java's chain of `startsWith` branches, the skipped line kinds included,
    // is kept as it is.
    #[allow(clippy::if_same_then_else)]
    fn read_doc(&mut self) -> Result<i32> {
        if self.doc_id == NO_MORE_DOCS {
            return Ok(self.doc_id);
        }
        let mut first = true;
        let mut term_freq = 0;
        loop {
            let line_start = self.input.file_pointer();
            read_line(self.input.as_mut(), &mut self.scratch)?;
            if self.scratch.starts_with(DOC) {
                if !first {
                    self.input.seek(line_start)?;
                    if !self.omit_tf {
                        self.tf = term_freq;
                    }
                    return Ok(self.doc_id);
                }
                self.doc_id = parse_int_at(&self.scratch, DOC.len())?;
                term_freq = 0;
                first = false;
            } else if self.scratch.starts_with(FREQ) {
                term_freq = parse_int_at(&self.scratch, FREQ.len())?;
            } else if self.scratch.starts_with(POS) {
                // skip termFreq++;
            } else if self.scratch.starts_with(START_OFFSET) {
                // skip
            } else if self.scratch.starts_with(END_OFFSET) {
                // skip
            } else if self.scratch.starts_with(PAYLOAD) {
                // skip
            } else {
                debug_assert!(
                    self.scratch.starts_with(SKIP_LIST)
                        || self.scratch.starts_with(TERM)
                        || self.scratch.starts_with(FIELD)
                        || self.scratch.starts_with(END),
                    "scratch={}",
                    java_utf8_to_string(&self.scratch)
                );
                if !first {
                    self.input.seek(line_start)?;
                    if !self.omit_tf {
                        self.tf = term_freq;
                    }
                    return Ok(self.doc_id);
                }
                self.doc_id = NO_MORE_DOCS;
                return Ok(self.doc_id);
            }
        }
    }

    fn advance_target(&mut self, target: i32) -> Result<i32> {
        if self.seek_to > 0 {
            self.input.seek(self.seek_to)?;
            self.seek_to = -1;
        }
        debug_assert!(self.doc_id < target);
        loop {
            let doc = self.read_doc()?;
            if doc >= target {
                return Ok(doc);
            }
        }
    }
}

impl fmt::Display for SimpleTextDocsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SimpleTextDocsEnum")
    }
}

impl DocIdSetIterator for SimpleTextDocsEnum {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc_id + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.advance_shallow(target)?;
        self.advance_target(target)
    }

    fn cost(&self) -> i64 {
        i64::from(self.cost)
    }
}

impl PostingsEnum for SimpleTextDocsEnum {
    fn freq(&self) -> Result<i32> {
        Ok(self.tf)
    }

    fn next_position(&mut self) -> Result<i32> {
        Ok(-1)
    }

    fn start_offset(&self) -> i32 {
        -1
    }

    fn end_offset(&self) -> i32 {
        -1
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(None)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl ImpactsSource for SimpleTextDocsEnum {
    fn advance_shallow(&mut self, target: i32) -> Result<()> {
        if target > self.next_skip_doc {
            self.skip_reader.skip_to(target)?;
            if self.skip_reader.get_next_skip_doc() != NO_MORE_DOCS {
                self.seek_to = self.skip_reader.get_next_skip_doc_fp();
            }
            self.next_skip_doc = self.skip_reader.get_next_skip_doc();
        }
        debug_assert!(self.next_skip_doc >= target);
        Ok(())
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        self.advance_shallow(self.doc_id)?;
        Ok(Box::new(self.skip_reader.get_impacts()))
    }
}

impl ImpactsEnum for SimpleTextDocsEnum {}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextFieldsReader.SimpleTextPostingsEnum`.
struct SimpleTextPostingsEnum {
    in_start: Arc<SharedInput>,
    input: Box<dyn IndexInput>,
    doc_id: i32,
    tf: i32,
    scratch: Vec<u8>,
    pos: i32,
    payload: Option<Vec<u8>>,
    next_doc_start: i64,
    read_offsets: bool,
    read_positions: bool,
    start_offset: i32,
    end_offset: i32,
    cost: i32,

    // for skip list data
    skip_reader: SimpleTextSkipReader,
    next_skip_doc: i32,
    seek_to: i64,
}

impl SimpleTextPostingsEnum {
    fn new(in_start: Arc<SharedInput>) -> Result<Self> {
        let input = in_start.clone_input()?;
        let skip_reader = SimpleTextSkipReader::new(in_start.clone_input()?);
        Ok(Self {
            in_start,
            input,
            doc_id: -1,
            tf: 0,
            scratch: Vec::new(),
            pos: 0,
            payload: None,
            next_doc_start: 0,
            read_offsets: false,
            read_positions: false,
            start_offset: 0,
            end_offset: 0,
            cost: 0,
            skip_reader,
            next_skip_doc: 0,
            seek_to: -1,
        })
    }

    fn can_reuse(&self, input: &Arc<SharedInput>) -> bool {
        Arc::ptr_eq(input, &self.in_start)
    }

    fn reset(
        &mut self,
        fp: i64,
        index_options: IndexOptions,
        doc_freq: i32,
        skip_pointer: i64,
    ) -> Result<()> {
        self.next_doc_start = fp;
        self.doc_id = -1;
        self.read_positions = index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        self.read_offsets =
            index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        if !self.read_offsets {
            self.start_offset = -1;
            self.end_offset = -1;
        }
        self.cost = doc_freq;
        self.skip_reader.reset(skip_pointer, doc_freq)?;
        self.next_skip_doc = 0;
        self.seek_to = -1;
        Ok(())
    }

    // Java's chain of `startsWith` branches, the skipped line kinds included,
    // is kept as it is.
    #[allow(clippy::if_same_then_else)]
    fn read_doc(&mut self) -> Result<i32> {
        let mut first = true;
        self.input.seek(self.next_doc_start)?;
        let mut pos_start = 0i64;
        loop {
            let line_start = self.input.file_pointer();
            read_line(self.input.as_mut(), &mut self.scratch)?;
            if self.scratch.starts_with(DOC) {
                if !first {
                    self.next_doc_start = line_start;
                    self.input.seek(pos_start)?;
                    return Ok(self.doc_id);
                }
                self.doc_id = parse_int_at(&self.scratch, DOC.len())?;
                self.tf = 0;
                first = false;
            } else if self.scratch.starts_with(FREQ) {
                self.tf = parse_int_at(&self.scratch, FREQ.len())?;
                pos_start = self.input.file_pointer();
            } else if self.scratch.starts_with(POS) {
                // skip
            } else if self.scratch.starts_with(START_OFFSET) {
                // skip
            } else if self.scratch.starts_with(END_OFFSET) {
                // skip
            } else if self.scratch.starts_with(PAYLOAD) {
                // skip
            } else {
                debug_assert!(
                    self.scratch.starts_with(SKIP_LIST)
                        || self.scratch.starts_with(TERM)
                        || self.scratch.starts_with(FIELD)
                        || self.scratch.starts_with(END)
                );
                if !first {
                    self.next_doc_start = line_start;
                    self.input.seek(pos_start)?;
                    return Ok(self.doc_id);
                }
                self.doc_id = NO_MORE_DOCS;
                return Ok(self.doc_id);
            }
        }
    }

    fn advance_target(&mut self, target: i32) -> Result<i32> {
        if self.seek_to > 0 {
            self.next_doc_start = self.seek_to;
            self.seek_to = -1;
        }
        debug_assert!(self.doc_id < target);
        loop {
            let doc = self.read_doc()?;
            if doc >= target {
                return Ok(doc);
            }
        }
    }
}

impl fmt::Display for SimpleTextPostingsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SimpleTextPostingsEnum")
    }
}

impl DocIdSetIterator for SimpleTextPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc_id + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.advance_shallow(target)?;
        self.advance_target(target)
    }

    fn cost(&self) -> i64 {
        i64::from(self.cost)
    }
}

impl PostingsEnum for SimpleTextPostingsEnum {
    fn freq(&self) -> Result<i32> {
        Ok(self.tf)
    }

    fn next_position(&mut self) -> Result<i32> {
        if self.read_positions {
            read_line(self.input.as_mut(), &mut self.scratch)?;
            debug_assert!(
                self.scratch.starts_with(POS),
                "got line={}",
                java_utf8_to_string(&self.scratch)
            );
            self.pos = parse_int_at(&self.scratch, POS.len())?;
        } else {
            self.pos = -1;
        }

        if self.read_offsets {
            read_line(self.input.as_mut(), &mut self.scratch)?;
            debug_assert!(
                self.scratch.starts_with(START_OFFSET),
                "got line={}",
                java_utf8_to_string(&self.scratch)
            );
            self.start_offset = parse_int_at(&self.scratch, START_OFFSET.len())?;
            read_line(self.input.as_mut(), &mut self.scratch)?;
            debug_assert!(
                self.scratch.starts_with(END_OFFSET),
                "got line={}",
                java_utf8_to_string(&self.scratch)
            );
            self.end_offset = parse_int_at(&self.scratch, END_OFFSET.len())?;
        }

        let fp = self.input.file_pointer();
        read_line(self.input.as_mut(), &mut self.scratch)?;
        if self.scratch.starts_with(PAYLOAD) {
            self.payload = Some(self.scratch[PAYLOAD.len()..].to_vec());
        } else {
            self.payload = None;
            self.input.seek(fp)?;
        }
        Ok(self.pos)
    }

    fn start_offset(&self) -> i32 {
        self.start_offset
    }

    fn end_offset(&self) -> i32 {
        self.end_offset
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(self.payload.as_deref())
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl ImpactsSource for SimpleTextPostingsEnum {
    fn advance_shallow(&mut self, target: i32) -> Result<()> {
        if target > self.next_skip_doc {
            self.skip_reader.skip_to(target)?;
            if self.skip_reader.get_next_skip_doc() != NO_MORE_DOCS {
                self.seek_to = self.skip_reader.get_next_skip_doc_fp();
            }
        }
        self.next_skip_doc = self.skip_reader.get_next_skip_doc();
        debug_assert!(self.next_skip_doc >= target);
        Ok(())
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        self.advance_shallow(self.doc_id)?;
        Ok(Box::new(self.skip_reader.get_impacts()))
    }
}

impl ImpactsEnum for SimpleTextPostingsEnum {}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextFieldsReader.SimpleTextTerms`.
#[derive(Clone)]
struct SimpleTextTerms {
    inner: Arc<TermsInner>,
}

struct TermsInner {
    field_info: FieldInfo,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    doc_count: i32,
    fst: Option<Arc<FST<TermOutputs>>>,
    term_count: i32,
    input: Arc<SharedInput>,
}

impl SimpleTextTerms {
    fn new(
        field: &str,
        terms_start: i64,
        max_doc: i32,
        field_infos: &FieldInfos,
        input: Arc<SharedInput>,
    ) -> Result<Self> {
        let field_info = field_infos.field_info(field).cloned().ok_or_else(|| {
            LuceneError::illegal_state(format!("no field info for field {field}"))
        })?;
        let inner = Self::load_terms(field_info, terms_start, max_doc, input)?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Port of the private `SimpleTextTerms.loadTerms`.
    fn load_terms(
        field_info: FieldInfo,
        terms_start: i64,
        max_doc: i32,
        shared: Arc<SharedInput>,
    ) -> Result<TermsInner> {
        let pos_int_outputs = PositiveIntOutputs::get_singleton();
        let outputs_outer = PairOutputs::new(pos_int_outputs, pos_int_outputs);
        let outputs_inner = PairOutputs::new(pos_int_outputs, pos_int_outputs);
        let outputs: TermOutputs = PairOutputs::new(outputs_outer.clone(), outputs_inner.clone());
        let mut fst_compiler = Builder::new(InputType::Byte1, outputs.clone()).build()?;
        let mut input = shared.clone_input()?;
        input.seek(terms_start)?;
        let mut last_term: Vec<u8> = Vec::new();
        let mut last_docs_start = -1i64;
        let mut doc_freq = 0i32;
        let mut total_term_freq = 0i64;
        let mut skip_pointer = 0i64;
        let mut visited_docs = FixedBitSet::new(max_doc.max(0) as usize);
        let mut scratch_ints_ref = IntsRefBuilder::new();
        let mut scratch = Vec::new();
        let mut sum_total_term_freq = 0i64;
        let mut sum_doc_freq = 0i64;
        let mut term_count = 0i32;
        loop {
            read_line(input.as_mut(), &mut scratch)?;
            if scratch.as_slice() == END || scratch.starts_with(FIELD) {
                if last_docs_start != -1 {
                    fst_compiler.add(
                        &Util::to_ints_ref(
                            &BytesRef::new(last_term.clone()),
                            &mut scratch_ints_ref,
                        ),
                        outputs.new_pair(
                            outputs_outer.new_pair(last_docs_start, skip_pointer),
                            outputs_inner.new_pair(i64::from(doc_freq), total_term_freq),
                        ),
                    )?;
                    sum_total_term_freq += total_term_freq;
                }
                break;
            } else if scratch.starts_with(DOC) {
                doc_freq += 1;
                sum_doc_freq += 1;
                total_term_freq += 1;
                let doc_id = parse_int_at(&scratch, DOC.len())?;
                if doc_id < 0 || doc_id as usize >= visited_docs.length() {
                    return Err(LuceneError::illegal_state(format!(
                        "Index {doc_id} out of bounds for length {max_doc}"
                    )));
                }
                visited_docs.set(doc_id as usize);
            } else if scratch.starts_with(FREQ) {
                total_term_freq += i64::from(parse_int_at(&scratch, FREQ.len())? - 1);
            } else if scratch.starts_with(SKIP_LIST) {
                skip_pointer = input.file_pointer();
            } else if scratch.starts_with(TERM) {
                if last_docs_start != -1 {
                    fst_compiler.add(
                        &Util::to_ints_ref(
                            &BytesRef::new(last_term.clone()),
                            &mut scratch_ints_ref,
                        ),
                        outputs.new_pair(
                            outputs_outer.new_pair(last_docs_start, skip_pointer),
                            outputs_inner.new_pair(i64::from(doc_freq), total_term_freq),
                        ),
                    )?;
                }
                last_docs_start = input.file_pointer();
                last_term.clear();
                last_term.extend_from_slice(&scratch[TERM.len()..]);
                doc_freq = 0;
                sum_total_term_freq += total_term_freq;
                total_term_freq = 0;
                term_count += 1;
                skip_pointer = 0;
            }
        }
        let doc_count = visited_docs.cardinality() as i32;
        let metadata = fst_compiler.compile()?;
        let fst = FST::from_fst_reader(metadata, fst_compiler.into_fst_reader()?).map(Arc::new);
        Ok(TermsInner {
            field_info,
            sum_total_term_freq,
            sum_doc_freq,
            doc_count,
            fst,
            term_count,
            input: shared,
        })
    }
}

impl fmt::Display for SimpleTextTerms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SimpleTextTerms(terms={},postings={},positions={},docs={})",
            self.inner.term_count,
            self.inner.sum_doc_freq,
            self.inner.sum_total_term_freq,
            self.inner.doc_count
        )
    }
}

impl Terms for SimpleTextTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        match &self.inner.fst {
            Some(fst) => Ok(Box::new(SimpleTextTermsEnum {
                index_options: self.inner.field_info.get_index_options(),
                meta: TermMeta::default(),
                fst_enum: BytesRefFSTEnum::new_shared(Arc::clone(fst))?,
                input: Arc::clone(&self.inner.input),
                attributes: None,
            })),
            None => Ok(Box::new(rucene::index::EmptyTermsEnum::new())),
        }
    }

    fn size(&self) -> Result<i64> {
        Ok(i64::from(self.inner.term_count))
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Ok(self.inner.sum_total_term_freq)
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok(self.inner.sum_doc_freq)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.inner.doc_count)
    }

    fn has_freqs(&self) -> bool {
        self.inner
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS)
    }

    fn has_offsets(&self) -> bool {
        self.inner
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
    }

    fn has_positions(&self) -> bool {
        self.inner
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    fn has_payloads(&self) -> bool {
        self.inner.field_info.has_payloads()
    }
}
