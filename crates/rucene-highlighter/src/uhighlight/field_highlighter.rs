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

//! Port of `FieldHighlighter.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReader;
use rucene_analysis_common::jdk::break_iterator::{BreakIterator, DONE};

use crate::java::utf16_len;
use crate::uhighlight::field_offset_strategy::FieldOffsetStrategy;
use crate::uhighlight::offsets_enum::{JavaPriorityQueue, OffsetsEnum};
use crate::uhighlight::passage::Passage;
use crate::uhighlight::passage_formatter::{FormattedPassages, PassageFormatter};
use crate::uhighlight::passage_scorer::PassageScorer;
use crate::uhighlight::unified_highlighter::{OffsetSource, PassageSortComparator};

/// The overridable methods of `FieldHighlighter` a Java subclass
/// overrides (its public `getField`, `getOffsetSource`,
/// `highlightFieldForDoc` and its protected `getSummaryPassagesNoHighlight`
/// and `highlightOffsetsEnums`, `FieldHighlighter.java:64-215`). Each
/// default is the base class's body, reachable as the highlighter's
/// `super_*` method.
pub trait FieldHighlighterOverrides: Send + Sync {
    /// Equivalent to an override of `getField()`.
    fn get_field<'a>(&self, this: &'a FieldHighlighter) -> &'a str {
        this.super_get_field()
    }

    /// Equivalent to an override of `getOffsetSource()`.
    fn get_offset_source(&self, this: &FieldHighlighter) -> OffsetSource {
        this.super_get_offset_source()
    }

    /// Equivalent to an override of `highlightFieldForDoc(LeafReader, int,
    /// String)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn highlight_field_for_doc(
        &self,
        this: &mut FieldHighlighter,
        reader: Option<&Arc<dyn LeafReader>>,
        doc_id: i32,
        content: &str,
    ) -> Result<Option<Box<dyn FormattedPassages>>> {
        this.super_highlight_field_for_doc(reader, doc_id, content)
    }

    /// Equivalent to an override of the protected
    /// `getSummaryPassagesNoHighlight(int)`.
    fn get_summary_passages_no_highlight(
        &self,
        this: &mut FieldHighlighter,
        max_passages: i32,
    ) -> Vec<Passage> {
        this.super_get_summary_passages_no_highlight(max_passages)
    }

    /// Equivalent to an override of the protected
    /// `highlightOffsetsEnums(OffsetsEnum)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn highlight_offsets_enums(
        &self,
        this: &mut FieldHighlighter,
        off: &mut dyn OffsetsEnum,
    ) -> Result<Vec<Passage>> {
        this.super_highlight_offsets_enums(off)
    }
}

/// Internal highlighter abstraction that operates on a per field basis.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.FieldHighlighter`;
/// a subclass's overrides are a [`FieldHighlighterOverrides`], set with
/// [`FieldHighlighter::set_overrides`] (typically by an override of
/// `UnifiedHighlighterOverrides::new_field_highlighter`).
pub struct FieldHighlighter {
    /// Equivalent to the protected final field `field`.
    pub field: String,
    /// Equivalent to the protected final field `fieldOffsetStrategy`.
    pub field_offset_strategy: Box<dyn FieldOffsetStrategy>,
    /// Equivalent to the protected final field `breakIterator` (note:
    /// stateful!).
    pub break_iterator: Box<dyn BreakIterator>,
    /// Equivalent to the protected final field `passageScorer`.
    pub passage_scorer: PassageScorer,
    /// Equivalent to the protected final field `maxPassages`.
    pub max_passages: i32,
    /// Equivalent to the protected final field `maxNoHighlightPassages`.
    pub max_no_highlight_passages: i32,
    /// Equivalent to the protected final field `passageFormatter`.
    pub passage_formatter: Arc<dyn PassageFormatter>,
    /// Equivalent to the protected final field `passageSortComparator`.
    pub passage_sort_comparator: PassageSortComparator,
    /// The subclass's overrides; `None` for the base class.
    overrides: Option<Arc<dyn FieldHighlighterOverrides>>,
}

impl fmt::Debug for FieldHighlighter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldHighlighter")
            .field("field", &self.field)
            .field("field_offset_strategy", &self.field_offset_strategy)
            .field("max_passages", &self.max_passages)
            .field("max_no_highlight_passages", &self.max_no_highlight_passages)
            .finish_non_exhaustive()
    }
}

fn passage_queue_order(left: &Passage, right: &Passage) -> Result<Ordering> {
    Ok(if left.get_score() < right.get_score() {
        Ordering::Less
    } else if left.get_score() > right.get_score() {
        Ordering::Greater
    } else {
        left.get_start_offset().cmp(&right.get_start_offset())
    })
}

impl FieldHighlighter {
    /// Equivalent to `new FieldHighlighter(String, FieldOffsetStrategy,
    /// BreakIterator, PassageScorer, int, int, PassageFormatter,
    /// Comparator)`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        field: &str,
        field_offset_strategy: Box<dyn FieldOffsetStrategy>,
        break_iterator: Box<dyn BreakIterator>,
        passage_scorer: PassageScorer,
        max_passages: i32,
        max_no_highlight_passages: i32,
        passage_formatter: Arc<dyn PassageFormatter>,
        passage_sort_comparator: PassageSortComparator,
    ) -> Self {
        Self {
            field: field.to_string(),
            field_offset_strategy,
            break_iterator,
            passage_scorer,
            max_passages,
            max_no_highlight_passages,
            passage_formatter,
            passage_sort_comparator,
            overrides: None,
        }
    }

    /// Installs a subclass's overrides (Java: constructing the subclass).
    pub fn set_overrides(&mut self, overrides: Arc<dyn FieldHighlighterOverrides>) {
        self.overrides = Some(overrides);
    }

    /// Equivalent to `getField()`.
    pub fn get_field(&self) -> &str {
        match &self.overrides {
            Some(o) => o.get_field(self),
            None => self.super_get_field(),
        }
    }

    /// The base class's `getField()`.
    pub fn super_get_field(&self) -> &str {
        &self.field
    }

    /// Equivalent to `getOffsetSource()`.
    pub fn get_offset_source(&self) -> OffsetSource {
        match &self.overrides {
            Some(o) => o.get_offset_source(self),
            None => self.super_get_offset_source(),
        }
    }

    /// The base class's `getOffsetSource()`.
    pub fn super_get_offset_source(&self) -> OffsetSource {
        self.field_offset_strategy.get_offset_source()
    }

    /// The primary method -- highlight this doc, assuming a specific field
    /// and given this content. Returns `None` when there is nothing to
    /// highlight.
    ///
    /// Equivalent to `highlightFieldForDoc(LeafReader, int, String)`;
    /// `reader` is `None` for Java's `null`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn highlight_field_for_doc(
        &mut self,
        reader: Option<&Arc<dyn LeafReader>>,
        doc_id: i32,
        content: &str,
    ) -> Result<Option<Box<dyn FormattedPassages>>> {
        match self.overrides.clone() {
            Some(o) => o.highlight_field_for_doc(self, reader, doc_id, content),
            None => self.super_highlight_field_for_doc(reader, doc_id, content),
        }
    }

    /// The base class's `highlightFieldForDoc(LeafReader, int, String)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn super_highlight_field_for_doc(
        &mut self,
        reader: Option<&Arc<dyn LeafReader>>,
        doc_id: i32,
        content: &str,
    ) -> Result<Option<Box<dyn FormattedPassages>>> {
        // TODO accept LeafReaderContext from UnifiedHighlighter to make it
        // easier to use, but it's annoying since there are some multi-reader
        // scenarios.

        if content.is_empty() {
            return Ok(None); // nothing to do
        }

        self.break_iterator.set_text_str(content);

        let mut offsets_enums = self
            .field_offset_strategy
            .get_offsets_enum(reader, doc_id, content)?;
        let result = (|| -> Result<Option<Box<dyn FormattedPassages>>> {
            let mut passages = self.highlight_offsets_enums(&mut *offsets_enums)?; // and breakIterator & scorer

            // Format the resulting Passages.
            if passages.is_empty() {
                // no passages were returned, so ask for a default summary
                passages = self.get_summary_passages_no_highlight(
                    if self.max_no_highlight_passages == -1 {
                        self.max_passages
                    } else {
                        self.max_no_highlight_passages
                    },
                );
            }

            if !passages.is_empty() {
                Ok(Some(self.passage_formatter.format(&passages, content)?))
            } else {
                Ok(None)
            }
        })();
        // try-with-resources: the enum is closed; a close failure is added
        // to a body failure as suppressed, or thrown.
        match (result, offsets_enums.close()) {
            (Ok(v), Ok(())) => Ok(v),
            (Ok(_), Err(e)) => Err(e),
            (Err(mut e), Err(c)) => {
                e.add_suppressed(c);
                Err(e)
            }
            (Err(e), Ok(())) => Err(e),
        }
    }

    /// Called to summarize a document when no highlights were found. By
    /// default this just returns the first `max_passages` sentences;
    /// subclasses can override to customize. The state of `break_iterator`
    /// should be at the beginning.
    ///
    /// Equivalent to the protected `getSummaryPassagesNoHighlight(int)`.
    pub fn get_summary_passages_no_highlight(&mut self, max_passages: i32) -> Vec<Passage> {
        match self.overrides.clone() {
            Some(o) => o.get_summary_passages_no_highlight(self, max_passages),
            None => self.super_get_summary_passages_no_highlight(max_passages),
        }
    }

    /// The base class's `getSummaryPassagesNoHighlight(int)`.
    pub fn super_get_summary_passages_no_highlight(&mut self, max_passages: i32) -> Vec<Passage> {
        debug_assert!(self.break_iterator.current() == self.break_iterator.first());

        let mut passages: Vec<Passage> = Vec::with_capacity(max_passages.clamp(0, 10) as usize);
        let mut pos = self.break_iterator.current();
        debug_assert!(pos == 0);
        while (passages.len() as i64) < max_passages as i64 {
            let next = self.break_iterator.next();
            if next == DONE {
                break;
            }
            let mut passage = Passage::new();
            passage.set_start_offset(pos);
            passage.set_end_offset(next);
            passages.push(passage);
            pos = next;
        }

        passages
    }

    // algorithm: treat sentence snippets as miniature documents
    // we can intersect these with the postings lists via BreakIterator.preceding(offset),s
    // score each sentence as norm(sentenceStartOffset) * sum(weight * tf(freq))
    /// Equivalent to the protected `highlightOffsetsEnums(OffsetsEnum)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors; `IllegalArgumentException` for a field indexed
    /// without offsets.
    pub fn highlight_offsets_enums(&mut self, off: &mut dyn OffsetsEnum) -> Result<Vec<Passage>> {
        match self.overrides.clone() {
            Some(o) => o.highlight_offsets_enums(self, off),
            None => self.super_highlight_offsets_enums(off),
        }
    }

    /// The base class's `highlightOffsetsEnums(OffsetsEnum)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors; `IllegalArgumentException` for a field indexed
    /// without offsets.
    pub fn super_highlight_offsets_enums(
        &mut self,
        off: &mut dyn OffsetsEnum,
    ) -> Result<Vec<Passage>> {
        let content_length = self.break_iterator.get_text().get_end_index();

        if !off.next_position()? {
            return Ok(Vec::new());
        }

        let mut passage_queue: JavaPriorityQueue<Passage> =
            JavaPriorityQueue::new(passage_queue_order);
        let mut passage = Passage::new(); // the current passage in-progress.  Will either get reset or added to queue.
        let mut last_passage_end = 0;

        loop {
            'body: {
                let start = off.start_offset()?;
                if start == -1 {
                    return Err(LuceneError::illegal_argument(format!(
                        "field '{}' was indexed without offsets, cannot highlight",
                        self.field
                    )));
                }
                let end = off.end_offset()?;
                if start < content_length && end > content_length {
                    break 'body;
                }
                // See if this term should be part of a new passage.
                if start >= passage.get_end_offset() {
                    passage =
                        self.maybe_add_passage(&mut passage_queue, passage, content_length)?;
                    // if we exceed limit, we are done
                    if start >= content_length {
                        // `break` of the do-while
                        let _ =
                            self.maybe_add_passage(&mut passage_queue, passage, content_length)?;
                        return self.finish(passage_queue);
                    }
                    // find fragment from the middle of the match, so the
                    // result's length may be closer to fragsize
                    let center = start + (end - start) / 2;
                    // advance breakIterator
                    passage.set_start_offset(
                        start.min(
                            self.break_iterator
                                .preceding((start + 1).max(center))?
                                .max(last_passage_end),
                        ),
                    );
                    last_passage_end = end.max(
                        self.break_iterator
                            .following((end - 1).min(center))?
                            .min(content_length),
                    );
                    passage.set_end_offset(last_passage_end);
                }
                // Add this term to the passage.
                let term = off.get_term()?.ok_or_else(LuceneError::null_pointer)?; // a reference; safe to refer to
                passage.add_match(start, end, term, off.freq()?);
            }
            if !off.next_position()? {
                break;
            }
        }
        let _ = self.maybe_add_passage(&mut passage_queue, passage, content_length)?;
        self.finish(passage_queue)
    }

    /// The tail of `highlightOffsetsEnums`: the queue's passages sorted by
    /// the passage sort comparator.
    fn finish(&self, passage_queue: JavaPriorityQueue<Passage>) -> Result<Vec<Passage>> {
        let mut passages = passage_queue.into_vec();
        // sort in ascending order
        let comparator = Arc::clone(&self.passage_sort_comparator);
        passages.sort_by(|a, b| comparator(a, b));
        Ok(passages)
    }

    fn maybe_add_passage(
        &self,
        passage_queue: &mut JavaPriorityQueue<Passage>,
        mut passage: Passage,
        content_length: i32,
    ) -> Result<Passage> {
        if passage.get_start_offset() == -1 {
            // empty passage, we can ignore it
            return Ok(passage);
        }
        passage.set_score(self.passage_scorer.score(&passage, content_length));
        // new sentence: first add 'passage' to queue
        if passage_queue.len() as i64 == self.max_passages as i64
            && passage.get_score()
                < passage_queue
                    .peek()
                    .ok_or_else(LuceneError::null_pointer)?
                    .get_score()
        {
            passage.reset(); // can't compete, just reset it
            Ok(passage)
        } else {
            passage_queue.offer(passage)?;
            if passage_queue.len() as i64 > self.max_passages as i64 {
                let mut polled = passage_queue
                    .poll()?
                    .expect("INVARIANT: the queue is not empty");
                polled.reset();
                Ok(polled)
            } else {
                Ok(Passage::new())
            }
        }
    }
}

#[allow(unused_imports)]
use utf16_len as _;
