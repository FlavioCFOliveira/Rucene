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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsReader`.

use std::fmt;
use std::sync::{Arc, Mutex};

use rucene::codecs::TermVectorsReader;
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{
    feature_requested, Fields, ImpactsEnum, PostingsEnum, SeekStatus, SegmentInfo, SlowImpactsEnum,
    TermVectors, Terms, TermsEnum, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_POSITIONS,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{BufferedChecksumIndexInput, Directory, IOContext, IndexInput};
use rucene::util::attribute::AttributeSource;
use rucene::util::java_fmt::java_utf8_to_string;
use rucene::util::BytesRef;

use super::term_vectors_writer::{
    DOC, END, ENDOFFSET, FIELD, FIELDNAME, FIELDOFFSETS, FIELDPAYLOADS, FIELDPOSITIONS,
    FIELDTERMCOUNT, NUMFIELDS, PAYLOAD, POSITION, STARTOFFSET, TERMFREQ, TERMTEXT,
    VECTORS_EXTENSION,
};
use super::util::{self, parse_boolean, parse_int_at, read_line};

/// Reads plain-text term vectors.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsReader`.
///
/// **FOR RECREATIONAL USE ONLY**
///
/// Java's `get` moves the reader's one `IndexInput` and reuses its scratch
/// buffers; both sit behind a [`Mutex`] here, because the port's
/// `TermVectors::get` takes `&self`.
pub struct SimpleTextTermVectorsReader {
    /// docid -> offset in .vec file
    offsets: Option<Arc<Vec<i64>>>,
    state: Mutex<Option<ReaderState>>,
}

struct ReaderState {
    input: Box<dyn IndexInput>,
    scratch: Vec<u8>,
}

impl fmt::Debug for SimpleTextTermVectorsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `getClass().getSimpleName()`.
        f.write_str("SimpleTextTermVectorsReader")
    }
}

fn poisoned<T>(_: T) -> LuceneError {
    LuceneError::illegal_state("term vectors reader lock was poisoned".to_string())
}

fn closed() -> LuceneError {
    LuceneError::already_closed("this TermVectorsReader is closed")
}

/// Java's `ArrayIndexOutOfBoundsException` on `offsets[index]`.
fn array_index_out_of_bounds(index: i64, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// The `NullPointerException` of `current.<method>` on an enum that is not
/// positioned on a term (`this.current` is `null`).
fn current_is_null(method: &str) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.NullPointerException",
        format!("Cannot invoke \"java.util.Map$Entry.{method}\" because \"this.current\" is null"),
    )
}

impl SimpleTextTermVectorsReader {
    /// Opens `si`'s `.tvc` file and reads its index.
    ///
    /// # Errors
    ///
    /// Propagates the directory's error, and [`util::check_footer`]'s.
    pub fn new(
        directory: &dyn Directory,
        si: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Self> {
        let input =
            directory.open_input(&segment_file_name(&si.name, "", VECTORS_EXTENSION), context)?;
        let mut reader = Self {
            offsets: None,
            state: Mutex::new(Some(ReaderState {
                input,
                scratch: Vec::new(),
            })),
        };
        reader.read_index(si.max_doc()?)?;
        Ok(reader)
    }

    /// We don't actually write a .tvx-like index, instead we read the vectors
    /// file in entirety up-front and save the offsets so we can seek to the
    /// data later.
    fn read_index(&mut self, max_doc: i32) -> Result<()> {
        let state = self.state.get_mut().map_err(poisoned)?;
        let state = state.as_mut().ok_or_else(closed)?;
        // Java wraps `in` itself; a clone at the same (initial) position reads
        // the same bytes, and `get` seeks `in` before every read anyway.
        let mut input = BufferedChecksumIndexInput::new(state.input.clone_input()?);
        let mut offsets = vec![0i64; max_doc.max(0) as usize];
        let mut upto = 0usize;
        let scratch = &mut state.scratch;
        while scratch.as_slice() != END {
            read_line(&mut input, scratch)?;
            if scratch.starts_with(DOC) {
                *offsets.get_mut(upto).ok_or_else(|| {
                    array_index_out_of_bounds(upto as i64, max_doc.max(0) as usize)
                })? = input.file_pointer();
                upto += 1;
            }
        }
        util::check_footer(&mut input)?;
        debug_assert_eq!(upto, offsets.len());
        self.offsets = Some(Arc::new(offsets));
        Ok(())
    }
}

/// `readString(offset, scratch)`: the line from `offset`, through
/// `CharsRefBuilder.copyUTF8Bytes`.
fn read_string(offset: usize, scratch: &[u8]) -> String {
    util::read_string(scratch, offset)
}

impl TermVectors for SimpleTextTermVectorsReader {
    /// Port of `SimpleTextTermVectorsReader.get`.
    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        let offsets = self.offsets.as_ref().ok_or_else(closed)?;
        let offset = *offsets
            .get(usize::try_from(doc).unwrap_or(usize::MAX))
            .ok_or_else(|| array_index_out_of_bounds(i64::from(doc), offsets.len()))?;
        let mut guard = self.state.lock().map_err(poisoned)?;
        let state = guard.as_mut().ok_or_else(closed)?;
        let input = state.input.as_mut();
        let scratch = &mut state.scratch;
        // `SortedMap<String, SimpleTVTerms> fields = new TreeMap<>()`.
        let mut fields: Vec<(String, SimpleTVTerms)> = Vec::new();
        input.seek(offset)?;
        read_line(input, scratch)?;
        debug_assert!(scratch.starts_with(NUMFIELDS));
        let num_fields = parse_int_at(scratch, NUMFIELDS.len())?;
        if num_fields == 0 {
            return Ok(None); // no vectors for this doc
        }
        for _ in 0..num_fields {
            read_line(input, scratch)?;
            debug_assert!(scratch.starts_with(FIELD));
            // skip fieldNumber:
            parse_int_at(scratch, FIELD.len())?;

            read_line(input, scratch)?;
            debug_assert!(scratch.starts_with(FIELDNAME));
            let field_name = read_string(FIELDNAME.len(), scratch);

            read_line(input, scratch)?;
            debug_assert!(scratch.starts_with(FIELDPOSITIONS));
            let positions = parse_boolean(&read_string(FIELDPOSITIONS.len(), scratch));

            read_line(input, scratch)?;
            debug_assert!(scratch.starts_with(FIELDOFFSETS));
            let offsets = parse_boolean(&read_string(FIELDOFFSETS.len(), scratch));

            read_line(input, scratch)?;
            debug_assert!(scratch.starts_with(FIELDPAYLOADS));
            let payloads = parse_boolean(&read_string(FIELDPAYLOADS.len(), scratch));

            read_line(input, scratch)?;
            debug_assert!(scratch.starts_with(FIELDTERMCOUNT));
            let term_count = parse_int_at(scratch, FIELDTERMCOUNT.len())?;

            // `new TreeMap<>()` of the field's terms.
            let mut terms: Vec<(BytesRef, SimpleTVPostings)> = Vec::new();

            for _ in 0..term_count {
                read_line(input, scratch)?;
                debug_assert!(scratch.starts_with(TERMTEXT));
                let term = BytesRef::new(scratch[TERMTEXT.len()..].to_vec());

                let mut postings = SimpleTVPostings::default();

                read_line(input, scratch)?;
                debug_assert!(scratch.starts_with(TERMFREQ));
                postings.freq = parse_int_at(scratch, TERMFREQ.len())?;

                if positions || offsets {
                    let freq = postings.freq.max(0) as usize;
                    if positions {
                        postings.positions = Some(vec![0; freq]);
                        if payloads {
                            postings.payloads = Some(vec![None; freq]);
                        }
                    }

                    if offsets {
                        postings.start_offsets = Some(vec![0; freq]);
                        postings.end_offsets = Some(vec![0; freq]);
                    }

                    for k in 0..freq {
                        if positions {
                            read_line(input, scratch)?;
                            debug_assert!(scratch.starts_with(POSITION));
                            let position = parse_int_at(scratch, POSITION.len())?;
                            if let Some(p) = postings.positions.as_mut() {
                                p[k] = position;
                            }
                            if payloads {
                                read_line(input, scratch)?;
                                debug_assert!(scratch.starts_with(PAYLOAD));
                                let payload = if scratch.len() - PAYLOAD.len() == 0 {
                                    None
                                } else {
                                    Some(BytesRef::new(scratch[PAYLOAD.len()..].to_vec()))
                                };
                                if let Some(p) = postings.payloads.as_mut() {
                                    p[k] = payload;
                                }
                            }
                        }

                        if offsets {
                            read_line(input, scratch)?;
                            debug_assert!(scratch.starts_with(STARTOFFSET));
                            let start = parse_int_at(scratch, STARTOFFSET.len())?;

                            read_line(input, scratch)?;
                            debug_assert!(scratch.starts_with(ENDOFFSET));
                            let end = parse_int_at(scratch, ENDOFFSET.len())?;
                            if let Some(s) = postings.start_offsets.as_mut() {
                                s[k] = start;
                            }
                            if let Some(e) = postings.end_offsets.as_mut() {
                                e[k] = end;
                            }
                        }
                    }
                }
                tree_put(&mut terms, term, postings, |a, b| a.slice().cmp(b.slice()));
            }
            tree_put(
                &mut fields,
                field_name,
                SimpleTVTerms {
                    terms: Arc::new(terms),
                    has_offsets: offsets,
                    has_positions: positions,
                    has_payloads: payloads,
                },
                |a, b| a.encode_utf16().cmp(b.encode_utf16()),
            );
        }
        Ok(Some(Box::new(SimpleTVFields {
            fields: Arc::new(fields),
        })))
    }
}

/// `TreeMap.put`: inserts in key order, replacing the value of an equal key.
fn tree_put<K, V>(
    map: &mut Vec<(K, V)>,
    key: K,
    value: V,
    cmp: impl Fn(&K, &K) -> std::cmp::Ordering,
) {
    match map.binary_search_by(|(k, _)| cmp(k, &key)) {
        Ok(i) => map[i].1 = value,
        Err(i) => map.insert(i, (key, value)),
    }
}

impl TermVectorsReader for SimpleTextTermVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    /// Port of `SimpleTextTermVectorsReader.clone`: a closed reader throws
    /// `AlreadyClosedException("this TermVectorsReader is closed")`
    /// (`SimpleTextTermVectorsReader.java:212-218`).
    fn clone_reader(&self) -> Result<Box<dyn TermVectorsReader>> {
        let guard = self.state.lock().map_err(poisoned)?;
        let state = guard.as_ref().ok_or_else(closed)?;
        let state = ReaderState {
            input: state.input.clone_input()?,
            scratch: Vec::new(),
        };
        Ok(Box::new(Self {
            offsets: self.offsets.clone(),
            state: Mutex::new(Some(state)),
        }))
    }

    fn close(&mut self) -> Result<()> {
        let state = self.state.get_mut().map_err(poisoned)?.take();
        self.offsets = None;
        match state {
            Some(mut state) => state.input.close(),
            None => Ok(()),
        }
    }
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsReader.SimpleTVFields`.
struct SimpleTVFields {
    fields: Arc<Vec<(String, SimpleTVTerms)>>,
}

impl Fields for SimpleTVFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.fields.iter().map(|(name, _)| name.clone()))
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(self
            .fields
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, terms)| Box::new(terms.clone()) as Box<dyn Terms>))
    }

    fn size(&self) -> i32 {
        self.fields.len() as i32
    }
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsReader.SimpleTVTerms`.
#[derive(Clone)]
struct SimpleTVTerms {
    terms: Arc<Vec<(BytesRef, SimpleTVPostings)>>,
    has_offsets: bool,
    has_positions: bool,
    has_payloads: bool,
}

impl Terms for SimpleTVTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        // TODO: reuse
        Ok(Box::new(SimpleTVTermsEnum::new(Arc::clone(&self.terms))))
    }

    fn size(&self) -> Result<i64> {
        Ok(self.terms.len() as i64)
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        // TODO: make it constant-time
        let mut ttf = 0i64;
        let mut iterator = self.iterator()?;
        while iterator.next()?.is_some() {
            ttf += iterator.total_term_freq()?;
        }
        Ok(ttf)
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok(self.terms.len() as i64)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(1)
    }

    fn has_freqs(&self) -> bool {
        true
    }

    fn has_offsets(&self) -> bool {
        self.has_offsets
    }

    fn has_positions(&self) -> bool {
        self.has_positions
    }

    fn has_payloads(&self) -> bool {
        self.has_payloads
    }
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsReader.SimpleTVPostings`.
#[derive(Clone, Default)]
struct SimpleTVPostings {
    freq: i32,
    positions: Option<Vec<i32>>,
    start_offsets: Option<Vec<i32>>,
    end_offsets: Option<Vec<i32>>,
    payloads: Option<Vec<Option<BytesRef>>>,
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsReader.SimpleTVTermsEnum`.
struct SimpleTVTermsEnum {
    terms: Arc<Vec<(BytesRef, SimpleTVPostings)>>,
    /// The iterator's next entry.
    next: usize,
    /// The index of `current`, Java's last entry returned by `next()`.
    current: Option<usize>,
    attributes: Option<AttributeSource>,
}

impl SimpleTVTermsEnum {
    fn new(terms: Arc<Vec<(BytesRef, SimpleTVPostings)>>) -> Self {
        Self {
            terms,
            next: 0,
            current: None,
            attributes: None,
        }
    }

    /// `current`, the entry the enum is positioned on; `method` names the
    /// `Map.Entry` accessor the caller dereferences it with, for the
    /// `NullPointerException` an unpositioned enum throws.
    fn current(&self, method: &str) -> Result<&(BytesRef, SimpleTVPostings)> {
        self.current
            .map(|i| &self.terms[i])
            .ok_or_else(|| current_is_null(method))
    }
}

impl fmt::Display for SimpleTVTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SimpleTVTermsEnum")
    }
}

impl TermsEnum for SimpleTVTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.attributes.get_or_insert_with(AttributeSource::new)
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        // `iterator = terms.tailMap(text).entrySet().iterator()`.
        self.next = self
            .terms
            .partition_point(|(k, _)| k.slice() < text.slice());
        if self.next >= self.terms.len() {
            Ok(SeekStatus::END)
        } else {
            let term = self.next()?.expect("INVARIANT: the tail map is not empty");
            Ok(if term.slice() == text.slice() {
                SeekStatus::FOUND
            } else {
                SeekStatus::NOT_FOUND
            })
        }
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.next >= self.terms.len() {
            Ok(None)
        } else {
            self.current = Some(self.next);
            self.next += 1;
            Ok(Some(self.terms[self.next - 1].0.clone()))
        }
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(self.current("getKey()")?.0.clone())
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Ok(1)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Ok(i64::from(self.current("getValue()")?.1.freq))
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        if feature_requested(flags, POSTINGS_ENUM_POSITIONS) {
            let postings = &self.current("getValue()")?.1;
            if postings.positions.is_some() || postings.start_offsets.is_some() {
                // TODO: reuse
                let mut e = SimpleTVPostingsEnum::default();
                e.reset(
                    postings.positions.clone(),
                    postings.start_offsets.clone(),
                    postings.end_offsets.clone(),
                    postings.payloads.clone(),
                );
                return Ok(Box::new(e));
            }
        }

        // TODO: reuse
        let mut e = SimpleTVDocsEnum::default();
        e.reset(if !feature_requested(flags, POSTINGS_ENUM_FREQS) {
            1
        } else {
            self.current("getValue()")?.1.freq
        });
        Ok(Box::new(e))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Ok(Box::new(SlowImpactsEnum::new(
            self.postings(None, POSTINGS_ENUM_FREQS)?,
        )))
    }
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsReader.SimpleTVDocsEnum`.
#[derive(Default)]
struct SimpleTVDocsEnum {
    did_next: bool,
    doc: i32,
    freq: i32,
}

impl SimpleTVDocsEnum {
    fn reset(&mut self, freq: i32) {
        self.freq = freq;
        self.doc = -1;
        self.did_next = false;
    }
}

impl fmt::Display for SimpleTVDocsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SimpleTVDocsEnum")
    }
}

impl DocIdSetIterator for SimpleTVDocsEnum {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if !self.did_next {
            self.did_next = true;
            self.doc = 0;
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        1
    }
}

impl PostingsEnum for SimpleTVDocsEnum {
    fn freq(&self) -> Result<i32> {
        debug_assert_ne!(self.freq, -1);
        Ok(self.freq)
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
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsReader.SimpleTVPostingsEnum`.
#[derive(Default)]
struct SimpleTVPostingsEnum {
    did_next: bool,
    doc: i32,
    next_pos: usize,
    positions: Option<Vec<i32>>,
    payloads: Option<Vec<Option<BytesRef>>>,
    start_offsets: Option<Vec<i32>>,
    end_offsets: Option<Vec<i32>>,
}

impl SimpleTVPostingsEnum {
    fn reset(
        &mut self,
        positions: Option<Vec<i32>>,
        start_offsets: Option<Vec<i32>>,
        end_offsets: Option<Vec<i32>>,
        payloads: Option<Vec<Option<BytesRef>>>,
    ) {
        self.positions = positions;
        self.start_offsets = start_offsets;
        self.end_offsets = end_offsets;
        self.payloads = payloads;
        self.doc = -1;
        self.did_next = false;
        self.next_pos = 0;
    }
}

impl fmt::Display for SimpleTVPostingsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SimpleTVPostingsEnum")
    }
}

impl DocIdSetIterator for SimpleTVPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if !self.did_next {
            self.did_next = true;
            self.doc = 0;
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        1
    }
}

impl PostingsEnum for SimpleTVPostingsEnum {
    fn freq(&self) -> Result<i32> {
        if let Some(positions) = &self.positions {
            return Ok(positions.len() as i32);
        }
        // `assert startOffsets != null; return startOffsets.length;`
        // (`SimpleTextTermVectorsReader.java:503-504`); `postings` never builds
        // this enum with both arrays `null`.
        debug_assert!(self.start_offsets.is_some());
        match &self.start_offsets {
            Some(start_offsets) => Ok(start_offsets.len() as i32),
            None => Err(LuceneError::unchecked(
                "java.lang.NullPointerException",
                "Cannot read the array length because \"this.startOffsets\" is null",
            )),
        }
    }

    fn next_position(&mut self) -> Result<i32> {
        if let Some(positions) = &self.positions {
            debug_assert!(
                self.next_pos < positions.len(),
                "nextPosition() called more than freq() times!"
            );
            let position = positions[self.next_pos];
            self.next_pos += 1;
            Ok(position)
        } else {
            debug_assert!(
                self.next_pos < self.start_offsets.as_ref().map_or(0, Vec::len),
                "nextPosition() called more than freq() times!"
            );
            self.next_pos += 1;
            Ok(-1)
        }
    }

    fn start_offset(&self) -> i32 {
        match &self.start_offsets {
            None => -1,
            Some(start_offsets) => start_offsets[self.next_pos - 1],
        }
    }

    fn end_offset(&self) -> i32 {
        match &self.end_offsets {
            None => -1,
            Some(end_offsets) => end_offsets[self.next_pos - 1],
        }
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(match &self.payloads {
            None => None,
            Some(payloads) => payloads[self.next_pos - 1].as_ref().map(BytesRef::slice),
        })
    }
}

// `readString` keeps the line through `CharsRefBuilder`, like Java.
#[allow(dead_code)]
fn utf8(bytes: &[u8]) -> String {
    java_utf8_to_string(bytes)
}
