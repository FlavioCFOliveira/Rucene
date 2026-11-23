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

//! Port of `Highlighter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::analysis::tokenattributes::{CharTermAttribute, OffsetAttribute};
use rucene::analysis::{Analyzer, AttributeHandle, SharedTokenStream, TokenStream};
use rucene::error::Result;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};

use crate::highlight::default_encoder::DefaultEncoder;
use crate::highlight::encoder::Encoder;
use crate::highlight::formatter::Formatter;
use crate::highlight::fragmenter::Fragmenter;
use crate::highlight::invalid_token_offsets_exception::InvalidTokenOffsetsException;
use crate::highlight::query_scorer::QueryScorer;
use crate::highlight::scorer::Scorer;
use crate::highlight::simple_fragmenter::SimpleFragmenter;
use crate::highlight::simple_html_formatter::SimpleHTMLFormatter;
use crate::highlight::text_fragment::{MarkedUpText, TextFragment};
use crate::highlight::token_group::TokenGroup;
use crate::java::{substring, term_text, units};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Marks up highlighted terms found in the best sections of text, using
/// configurable `Fragmenter`, `Scorer`, `Formatter`, `Encoder` and
/// tokenizers.
///
/// This is Lucene's original Highlighter; there are others.
///
/// Equivalent to `org.apache.lucene.search.highlight.Highlighter`.
#[derive(Debug)]
pub struct Highlighter {
    formatter: Box<dyn Formatter>,
    encoder: Box<dyn Encoder>,
    fragment_scorer: Box<dyn Scorer>,
    max_doc_chars_to_analyze: i32,
    text_fragmenter: Box<dyn Fragmenter>,
}

impl Highlighter {
    /// Equivalent to `Highlighter.DEFAULT_MAX_CHARS_TO_ANALYZE`.
    pub const DEFAULT_MAX_CHARS_TO_ANALYZE: i32 = 50 * 1024;

    /// Equivalent to `new Highlighter(Scorer)`.
    pub fn new(fragment_scorer: Box<dyn Scorer>) -> Self {
        Self::with_formatter(Box::new(SimpleHTMLFormatter::new()), fragment_scorer)
    }

    /// Equivalent to `new Highlighter(Formatter, Scorer)`.
    pub fn with_formatter(formatter: Box<dyn Formatter>, fragment_scorer: Box<dyn Scorer>) -> Self {
        Self::with_formatter_and_encoder(formatter, Box::new(DefaultEncoder::new()), fragment_scorer)
    }

    /// Equivalent to `new Highlighter(Formatter, Encoder, Scorer)`; Java's
    /// `null` checks (`IllegalArgumentException`) have no counterpart, the
    /// arguments being values.
    pub fn with_formatter_and_encoder(
        formatter: Box<dyn Formatter>,
        encoder: Box<dyn Encoder>,
        fragment_scorer: Box<dyn Scorer>,
    ) -> Self {
        Self {
            formatter,
            encoder,
            fragment_scorer,
            max_doc_chars_to_analyze: Self::DEFAULT_MAX_CHARS_TO_ANALYZE,
            text_fragmenter: Box::new(SimpleFragmenter::new()),
        }
    }

    /// Highlights chosen terms in a text, extracting the most relevant
    /// section. This is a convenience method that calls
    /// `get_best_fragment_from_stream`; `None` (Java's `null`) when no
    /// fragment scored.
    ///
    /// Equivalent to `getBestFragment(Analyzer, String, String)`.
    ///
    /// # Errors
    ///
    /// `IOException`, and `InvalidTokenOffsetsException` thrown if any token's
    /// endOffset exceeds the provided text's length.
    pub fn get_best_fragment(
        &mut self,
        analyzer: &dyn Analyzer,
        field_name: &str,
        text: &str,
    ) -> Result<Option<String>> {
        let token_stream = analyzer.token_stream_from_str(field_name, text)?;
        self.get_best_fragment_from_stream(Box::new(SharedTokenStream::new(token_stream)), text)
    }

    /// Highlights chosen terms in a text, extracting the most relevant
    /// section.
    ///
    /// Equivalent to `getBestFragment(TokenStream, String)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_fragment`](Self::get_best_fragment).
    pub fn get_best_fragment_from_stream(
        &mut self,
        token_stream: Box<dyn TokenStream>,
        text: &str,
    ) -> Result<Option<String>> {
        let mut results = self.get_best_fragments_from_stream(token_stream, text, 1)?;
        if !results.is_empty() {
            return Ok(Some(results.swap_remove(0)));
        }
        Ok(None)
    }

    /// Highlights chosen terms in a text, extracting the most relevant
    /// sections.
    ///
    /// Equivalent to `getBestFragments(Analyzer, String, String, int)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_fragment`](Self::get_best_fragment).
    pub fn get_best_fragments(
        &mut self,
        analyzer: &dyn Analyzer,
        field_name: &str,
        text: &str,
        max_num_fragments: i32,
    ) -> Result<Vec<String>> {
        let token_stream = analyzer.token_stream_from_str(field_name, text)?;
        self.get_best_fragments_from_stream(
            Box::new(SharedTokenStream::new(token_stream)),
            text,
            max_num_fragments,
        )
    }

    /// Highlights chosen terms in a text, extracting the most relevant
    /// sections.
    ///
    /// Equivalent to `getBestFragments(TokenStream, String, int)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_fragment`](Self::get_best_fragment).
    pub fn get_best_fragments_from_stream(
        &mut self,
        token_stream: Box<dyn TokenStream>,
        text: &str,
        max_num_fragments: i32,
    ) -> Result<Vec<String>> {
        let max_num_fragments = max_num_fragments.max(1); // sanity check

        let frag = self.get_best_text_fragments(token_stream, text, true, max_num_fragments)?;

        // Get text
        let mut frag_texts = Vec::new();
        for f in &frag {
            if f.get_score() > 0.0 {
                frag_texts.push(f.to_string());
            }
        }
        Ok(frag_texts)
    }

    /// Low level api to get the most relevant (formatted) sections of the
    /// document. This method has been made public to allow visibility of
    /// score information held in TextFragment objects. Thanks to Jason Calabrese
    /// for help in redefining the interface.
    ///
    /// Equivalent to `getBestTextFragments(TokenStream, String, boolean,
    /// int)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_fragment`](Self::get_best_fragment).
    pub fn get_best_text_fragments(
        &mut self,
        token_stream: Box<dyn TokenStream>,
        text: &str,
        merge_contiguous_fragments: bool,
        max_num_fragments: i32,
    ) -> Result<Vec<TextFragment>> {
        let mut token_stream = token_stream;
        let mut doc_frags: Vec<TextFragment> = Vec::new();
        let new_text: MarkedUpText = Arc::new(Mutex::new(Vec::new()));

        let term_att: AttributeHandle<dyn CharTermAttribute> =
            AttributeHandle::add(token_stream.attribute_source_mut())?;
        let offset_att: AttributeHandle<dyn OffsetAttribute> =
            AttributeHandle::add(token_stream.attribute_source_mut())?;
        let mut current_frag =
            TextFragment::new(Arc::clone(&new_text), lock(&new_text).len() as i32, doc_frags.len() as i32);

        if let Some(query_scorer) = self
            .fragment_scorer
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<QueryScorer>())
        {
            query_scorer.set_max_doc_chars_to_analyze(self.max_doc_chars_to_analyze);
        }

        let mut token_stream = self.fragment_scorer.init(token_stream)?;
        self.fragment_scorer.start_fragment(&current_frag);
        doc_frags.push(current_frag.clone());

        let mut frag_queue = PriorityQueue::new(max_num_fragments.max(0) as usize, FragmentQueue)?;

        let result = self.best_text_fragments_body(
            &mut *token_stream,
            text,
            merge_contiguous_fragments,
            &term_att,
            &offset_att,
            &new_text,
            &mut current_frag,
            &mut doc_frags,
            &mut frag_queue,
        );
        // finally: the stream is ended and closed, ignoring their failures
        let _ = token_stream.end();
        let _ = token_stream.close();
        result
    }

    /// The `try` block of `getBestTextFragments`.
    #[allow(clippy::too_many_arguments)]
    fn best_text_fragments_body(
        &mut self,
        token_stream: &mut dyn TokenStream,
        text: &str,
        merge_contiguous_fragments: bool,
        term_att: &AttributeHandle<dyn CharTermAttribute>,
        offset_att: &AttributeHandle<dyn OffsetAttribute>,
        new_text: &MarkedUpText,
        current_frag: &mut TextFragment,
        doc_frags: &mut Vec<TextFragment>,
        frag_queue: &mut PriorityQueue<TextFragment, FragmentQueue>,
    ) -> Result<Vec<TextFragment>> {
        let text_units = units(text);
        let text_length = text_units.len() as i32;
        let new_text_len = || lock(new_text).len() as i32;
        let append = |s: &str| lock(new_text).extend(s.encode_utf16());

        let mut last_end_offset = 0;
        self.text_fragmenter.start(text, token_stream)?;

        let mut token_group = TokenGroup::new(token_stream)?;

        token_stream.reset()?;
        let mut next = token_stream.increment_token()?;
        while next && offset_att.borrow().start_offset() < self.max_doc_chars_to_analyze {
            let (start, end) = {
                let att = offset_att.borrow();
                (att.start_offset(), att.end_offset())
            };
            if (end > text_length) || (start > text_length) {
                return Err(InvalidTokenOffsetsException::error(format!(
                    "Token {} exceeds length of provided text sized {}",
                    term_text(&*term_att.borrow()),
                    text_length
                )));
            }
            if (token_group.get_num_tokens() > 0) && token_group.is_distinct() {
                // the current token is distinct from previous tokens -
                // markup the cached token group info
                let start_offset = token_group.get_start_offset();
                let end_offset = token_group.get_end_offset();
                let token_text = substring(&text_units, start_offset, end_offset)?;
                let marked_up_text = self
                    .formatter
                    .highlight_term(&self.encoder.encode_text(&token_text), &token_group);
                // store any whitespace etc from between this and last group
                if start_offset > last_end_offset {
                    append(&self.encoder.encode_text(&substring(
                        &text_units,
                        last_end_offset,
                        start_offset,
                    )?));
                }
                append(&marked_up_text);
                last_end_offset = end_offset.max(last_end_offset);
                token_group.clear();

                // check if current token marks the start of a new fragment
                if self.text_fragmenter.is_new_fragment() {
                    current_frag.set_score(self.fragment_scorer.get_fragment_score());
                    // record stats for a new fragment
                    current_frag.set_text_end_pos(new_text_len());
                    *current_frag =
                        TextFragment::new(Arc::clone(new_text), new_text_len(), doc_frags.len() as i32);
                    self.fragment_scorer.start_fragment(current_frag);
                    doc_frags.push(current_frag.clone());
                }
            }

            token_group.add_token(self.fragment_scorer.get_token_score()?);

            next = token_stream.increment_token()?;
        }
        current_frag.set_score(self.fragment_scorer.get_fragment_score());

        if token_group.get_num_tokens() > 0 {
            // flush the accumulated text (same code as in above loop)
            let start_offset = token_group.get_start_offset();
            let end_offset = token_group.get_end_offset();
            let token_text = substring(&text_units, start_offset, end_offset)?;
            let marked_up_text = self
                .formatter
                .highlight_term(&self.encoder.encode_text(&token_text), &token_group);
            // store any whitespace etc from between this and last group
            if start_offset > last_end_offset {
                append(&self.encoder.encode_text(&substring(
                    &text_units,
                    last_end_offset,
                    start_offset,
                )?));
            }
            append(&marked_up_text);
            last_end_offset = last_end_offset.max(end_offset);
        }

        // Test what remains of the original text beyond the point where we
        // stopped analyzing
        if (last_end_offset < text_length) && (text_length <= self.max_doc_chars_to_analyze) {
            // append it to the last fragment
            append(&self.encoder.encode_text(&substring(
                &text_units,
                last_end_offset,
                text_length,
            )?));
        }

        current_frag.set_text_end_pos(new_text_len());

        // sort the most relevant sections of the text
        for frag in doc_frags.iter() {
            frag_queue.insert_with_overflow(frag.clone());
        }

        // return the most relevant fragments
        let mut frag: Vec<Option<TextFragment>> = vec![None; frag_queue.size()];
        for i in (0..frag.len()).rev() {
            frag[i] = frag_queue.pop();
        }

        // merge any contiguous fragments to improve readability
        if merge_contiguous_fragments {
            Self::merge_contiguous_fragments(&mut frag);
            let mut frag_texts = Vec::new();
            for f in frag.into_iter().flatten() {
                if f.get_score() > 0.0 {
                    frag_texts.push(f);
                }
            }
            return Ok(frag_texts);
        }

        Ok(frag.into_iter().flatten().collect())
    }

    /// Improves readability of a score-sorted list of TextFragments by
    /// merging any fragments that were contiguous in the original text into
    /// one larger fragment with the correct order. This will leave a
    /// "null" in the array entry for the lesser scored fragment.
    ///
    /// Equivalent to the private `mergeContiguousFragments(TextFragment[])`.
    fn merge_contiguous_fragments(frag: &mut [Option<TextFragment>]) {
        let mut merging_still_being_done;
        if frag.len() > 1 {
            loop {
                merging_still_being_done = false; // initialise loop control flag
                // for each fragment, scan other frags looking for contiguous
                // blocks
                for i in 0..frag.len() {
                    if frag[i].is_none() {
                        continue;
                    }
                    // merge any contiguous blocks
                    for x in 0..frag.len() {
                        if frag[x].is_none() {
                            continue;
                        }
                        if frag[i].is_none() {
                            break;
                        }
                        let fi = frag[i].clone().expect("INVARIANT: checked above");
                        let fx = frag[x].clone().expect("INVARIANT: checked above");
                        let mut frag1 = None;
                        let mut frag2 = None;
                        let mut frag1_num = 0;
                        let mut frag2_num = 0;
                        // if blocks are contiguous....
                        if fi.follows(&fx) {
                            frag1 = Some(fx);
                            frag1_num = x;
                            frag2 = Some(fi);
                            frag2_num = i;
                        } else if fx.follows(&fi) {
                            frag1 = Some(fi);
                            frag1_num = i;
                            frag2 = Some(fx);
                            frag2_num = x;
                        }
                        // merging required..
                        if let (Some(frag1), Some(frag2)) = (frag1, frag2) {
                            let best_scoring_frag_num;
                            let worst_scoring_frag_num;
                            if frag1.get_score() > frag2.get_score() {
                                best_scoring_frag_num = frag1_num;
                                worst_scoring_frag_num = frag2_num;
                            } else {
                                best_scoring_frag_num = frag2_num;
                                worst_scoring_frag_num = frag1_num;
                            }
                            frag1.merge(&frag2);
                            frag[worst_scoring_frag_num] = None;
                            merging_still_being_done = true;
                            frag[best_scoring_frag_num] = Some(frag1);
                        }
                    }
                }
                if !merging_still_being_done {
                    break;
                }
            }
        }
    }

    /// Highlights terms in the text, extracting the most relevant sections
    /// and concatenating the chosen fragments with a separator (typically
    /// "...").
    ///
    /// Equivalent to `getBestFragments(TokenStream, String, int, String)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_fragment`](Self::get_best_fragment).
    pub fn get_best_fragments_with_separator(
        &mut self,
        token_stream: Box<dyn TokenStream>,
        text: &str,
        max_num_fragments: i32,
        separator: &str,
    ) -> Result<String> {
        let sections = self.get_best_fragments_from_stream(token_stream, text, max_num_fragments)?;
        let mut result = String::new();
        for (i, section) in sections.iter().enumerate() {
            if i > 0 {
                result.push_str(separator);
            }
            result.push_str(section);
        }
        Ok(result)
    }

    /// Equivalent to `getMaxDocCharsToAnalyze()`.
    pub fn get_max_doc_chars_to_analyze(&self) -> i32 {
        self.max_doc_chars_to_analyze
    }

    /// Equivalent to `setMaxDocCharsToAnalyze(int)`.
    pub fn set_max_doc_chars_to_analyze(&mut self, max_doc_chars_to_analyze: i32) {
        self.max_doc_chars_to_analyze = max_doc_chars_to_analyze;
    }

    /// Equivalent to `getTextFragmenter()`.
    pub fn get_text_fragmenter(&self) -> &dyn Fragmenter {
        &*self.text_fragmenter
    }

    /// Equivalent to `setTextFragmenter(Fragmenter)`.
    pub fn set_text_fragmenter(&mut self, fragmenter: Box<dyn Fragmenter>) {
        self.text_fragmenter = fragmenter;
    }

    /// Returns the Object used to score each text fragment.
    ///
    /// Equivalent to `getFragmentScorer()`.
    pub fn get_fragment_scorer(&self) -> &dyn Scorer {
        &*self.fragment_scorer
    }

    /// Equivalent to `setFragmentScorer(Scorer)`.
    pub fn set_fragment_scorer(&mut self, scorer: Box<dyn Scorer>) {
        self.fragment_scorer = scorer;
    }

    /// Equivalent to `getEncoder()`.
    pub fn get_encoder(&self) -> &dyn Encoder {
        &*self.encoder
    }

    /// Equivalent to `setEncoder(Encoder)`.
    pub fn set_encoder(&mut self, encoder: Box<dyn Encoder>) {
        self.encoder = encoder;
    }
}

/// The order of the fragment queue.
///
/// Equivalent to the package-private nested class
/// `Highlighter.FragmentQueue`, a `PriorityQueue<TextFragment>`: the port's
/// queue is `PriorityQueue<TextFragment, FragmentQueue>`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FragmentQueue;

impl PriorityQueueComparator<TextFragment> for FragmentQueue {
    fn less_than(&self, frag_a: &TextFragment, frag_b: &TextFragment) -> bool {
        if frag_a.get_score() == frag_b.get_score() {
            frag_a.get_frag_num() > frag_b.get_frag_num()
        } else {
            frag_a.get_score() < frag_b.get_score()
        }
    }
}
