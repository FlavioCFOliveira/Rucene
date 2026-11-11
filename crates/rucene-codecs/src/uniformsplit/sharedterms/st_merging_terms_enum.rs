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

//! Port of the package-private
//! `org.apache.lucene.codecs.uniformsplit.sharedterms.STMergingTermsEnum`.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::java_object_to_string;
use rucene::index::{ImpactsEnum, PostingsEnum, SeekStatus, TermState, TermsEnum};
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

use super::st_uniform_split_terms_writer::SegmentPostings;
use crate::blocktreeords::java_assert;

/// The reusable postings enums of a [`MultiSegmentsPostingsEnum`], one per
/// segment (Java's `reusablePostingsEnums` array).
type ReusablePostingsEnums = Rc<RefCell<Vec<Option<Box<dyn PostingsEnum>>>>>;

/// Combines `PostingsEnum` for the same term for a given field from multiple
/// segments. It is used during segment merging.
///
/// Port of the package-private class
/// `org.apache.lucene.codecs.uniformsplit.sharedterms.STMergingTermsEnum`
/// (`@lucene.experimental`).
///
/// Java's `postings(PostingsEnum, int)` returns the enum's single
/// `MultiSegmentsPostingsEnum`, reset for each term. The port's `postings`
/// returns a box the postings writer owns, so each call returns a new
/// `MultiSegmentsPostingsEnum` over the current term's segment postings;
/// what Java keeps across terms, the per-segment reusable postings enums,
/// is shared by all of them, so they are reused as Java reuses them.
///
/// Java's `attributes()` throws `UnsupportedOperationException`; the port's
/// `attributes()` returns a reference, so it returns an empty source.
pub(crate) struct STMergingTermsEnum {
    pub(crate) field_name: String,
    pub(crate) term: Option<BytesRef>,
    segment_postings_list: Vec<SegmentPostings>,
    reusable_postings_enums: ReusablePostingsEnums,
    attributes: AttributeSource,
}

fn unsupported() -> LuceneError {
    LuceneError::unsupported_operation(String::new())
}

impl STMergingTermsEnum {
    /// Constructs a [`STMergingTermsEnum`] for a given field.
    ///
    /// Port of `STMergingTermsEnum(String, int)`.
    pub(crate) fn new(field_name: &str, num_segments: usize) -> Self {
        Self {
            field_name: field_name.to_string(),
            term: None,
            segment_postings_list: Vec::new(),
            reusable_postings_enums: Rc::new(RefCell::new(
                std::iter::repeat_with(|| None).take(num_segments).collect(),
            )),
            attributes: AttributeSource::new(),
        }
    }

    /// Resets this [`STMergingTermsEnum`] with a new term and its list of
    /// segment postings to combine (sorted by segment index).
    ///
    /// Port of `STMergingTermsEnum.reset(BytesRef, List)`.
    pub(crate) fn reset(&mut self, term: BytesRef, segment_postings: Vec<SegmentPostings>) {
        self.term = Some(term);
        self.segment_postings_list = segment_postings;
    }
}

impl fmt::Display for STMergingTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.uniformsplit.sharedterms.STMergingTermsEnum",
            self,
        ))
    }
}

impl TermsEnum for STMergingTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Err(unsupported())
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(unsupported())
    }

    fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        Err(unsupported())
    }

    fn term(&self) -> Result<Option<BytesRef>> {
        Ok(self.term.clone())
    }

    fn ord(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Err(unsupported())
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Err(unsupported())
    }

    /// Port of `STMergingTermsEnum.postings(PostingsEnum, int)`.
    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        let mut multi_postings_enum = MultiSegmentsPostingsEnum::new(
            self.field_name.clone(),
            Rc::clone(&self.reusable_postings_enums),
        );
        multi_postings_enum.reset(self.segment_postings_list.clone());
        multi_postings_enum.set_posting_flags(flags);
        Ok(Some(Box::new(multi_postings_enum)))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Err(unsupported())
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Err(unsupported())
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        Err(unsupported())
    }
}

/// Combines multiple segments `PostingsEnum` as a single `PostingsEnum`, for
/// one field and one term.
///
/// This `PostingsEnum` does not extend `FilterLeafReader.FilterPostingsEnum`
/// because it updates the delegate for each segment.
///
/// Port of the protected inner class
/// `STMergingTermsEnum.MultiSegmentsPostingsEnum`. The postings enum of the
/// current segment returns to the shared reusable enums when the enum moves
/// to the next segment or is dropped, where Java's array keeps it.
pub(crate) struct MultiSegmentsPostingsEnum {
    field_name: String,
    reusable_postings_enums: ReusablePostingsEnums,
    segment_postings_list: Vec<SegmentPostings>,
    segment_index: i32,
    postings_enum: Option<Box<dyn PostingsEnum>>,
    postings_enum_segment: usize,
    postings_enum_exhausted: bool,
    doc_map: Option<rucene::index::merge::DocMap>,
    doc_id: i32,
    postings_flags: i32,
}

impl MultiSegmentsPostingsEnum {
    /// Port of `MultiSegmentsPostingsEnum(int)`.
    fn new(field_name: String, reusable_postings_enums: ReusablePostingsEnums) -> Self {
        Self {
            field_name,
            reusable_postings_enums,
            segment_postings_list: Vec::new(),
            segment_index: -1,
            postings_enum: None,
            postings_enum_segment: 0,
            postings_enum_exhausted: true,
            doc_map: None,
            doc_id: -1,
            postings_flags: 0,
        }
    }

    /// Resets/reuse this `PostingsEnum`; `segment_postings_list` is ordered
    /// by segment index.
    ///
    /// Port of `MultiSegmentsPostingsEnum.reset(List)`.
    fn reset(&mut self, segment_postings_list: Vec<SegmentPostings>) {
        self.segment_postings_list = segment_postings_list;
        self.segment_index = -1;
        self.postings_enum_exhausted = true;
        self.doc_id = -1;
    }

    /// Port of `MultiSegmentsPostingsEnum.setPostingFlags(int)`.
    fn set_posting_flags(&mut self, flags: i32) {
        self.postings_flags = flags;
    }

    /// Returns the current segment's postings enum to the reusable ones.
    fn release_current(&mut self) {
        if let Some(postings_enum) = self.postings_enum.take() {
            if let Some(slot) = self
                .reusable_postings_enums
                .borrow_mut()
                .get_mut(self.postings_enum_segment)
            {
                *slot = Some(postings_enum);
            }
        }
    }

    /// Port of `MultiSegmentsPostingsEnum.getPostings(SegmentPostings)`: the
    /// field is present in the segment because it is one of the segments
    /// provided in the `reset()` method.
    fn get_postings(
        &mut self,
        segment_postings: &SegmentPostings,
    ) -> Result<Box<dyn PostingsEnum>> {
        let reuse = self
            .reusable_postings_enums
            .borrow_mut()
            .get_mut(segment_postings.segment_index)
            .and_then(Option::take);
        segment_postings.get_postings(&self.field_name, reuse, self.postings_flags)
    }

    fn current(&self) -> &dyn PostingsEnum {
        self.postings_enum
            .as_deref()
            .expect("INVARIANT: the postings writer reads a posting after nextDoc() found one")
    }
}

impl Drop for MultiSegmentsPostingsEnum {
    fn drop(&mut self) {
        self.release_current();
    }
}

impl fmt::Display for MultiSegmentsPostingsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.uniformsplit.sharedterms.STMergingTermsEnum$MultiSegmentsPostingsEnum",
            self,
        ))
    }
}

impl DocIdSetIterator for MultiSegmentsPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.postings_enum_exhausted {
                if self.segment_index == self.segment_postings_list.len() as i32 - 1 {
                    self.doc_id = NO_MORE_DOCS;
                    return Ok(self.doc_id);
                } else {
                    self.segment_index += 1;
                    let segment_postings =
                        self.segment_postings_list[self.segment_index as usize].clone();
                    self.release_current();
                    self.postings_enum = Some(self.get_postings(&segment_postings)?);
                    self.postings_enum_segment = segment_postings.segment_index;
                    self.postings_enum_exhausted = false;
                    self.doc_map = Some(segment_postings.doc_map.clone());
                }
            }
            let Some(postings_enum) = self.postings_enum.as_mut() else {
                return Err(crate::uniformsplit::npe(
                    "Cannot invoke \"org.apache.lucene.index.PostingsEnum.nextDoc()\" because \
                     \"this.postingsEnum\" is null",
                ));
            };
            let doc_id = postings_enum.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                self.postings_enum_exhausted = true;
            } else {
                let doc_id = match &self.doc_map {
                    Some(doc_map) => doc_map(doc_id),
                    None => doc_id,
                };
                if doc_id != -1 {
                    java_assert!(
                        doc_id > self.doc_id,
                        "next docId={doc_id}, current docId={}",
                        self.doc_id
                    );
                    self.doc_id = doc_id;
                    return Ok(doc_id);
                }
            }
        }
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(unsupported())
    }

    fn cost(&self) -> i64 {
        0 // Cost is not useful here.
    }
}

impl PostingsEnum for MultiSegmentsPostingsEnum {
    fn freq(&self) -> Result<i32> {
        self.current().freq()
    }

    fn next_position(&mut self) -> Result<i32> {
        match self.postings_enum.as_mut() {
            Some(postings_enum) => postings_enum.next_position(),
            None => Err(crate::uniformsplit::npe(
                "Cannot invoke \"org.apache.lucene.index.PostingsEnum.nextPosition()\" because \
                 \"this.postingsEnum\" is null",
            )),
        }
    }

    fn start_offset(&self) -> i32 {
        self.current().start_offset()
    }

    fn end_offset(&self) -> i32 {
        self.current().end_offset()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.current().get_payload()
    }
}
