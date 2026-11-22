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

//! Port of `FieldOffsetStrategy.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::{LeafReader, PostingsEnum, Terms, POSTINGS_ENUM_OFFSETS};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::score_mode::ScoreMode;
use rucene::util::BytesRef;

use crate::uhighlight::offsets_enum::{
    empty, MultiOffsetsEnum, OfMatchesIteratorWithSubs, OfPostings, OffsetsEnum,
};
use crate::uhighlight::uh_components::UHComponents;
use crate::uhighlight::unified_highlighter::{HighlightFlag, OffsetSource};

/// Ultimately returns an `OffsetsEnum` yielding potentially highlightable
/// words in the text. Needs information about the query up front.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.uhighlight.FieldOffsetStrategy`: the protected
/// `components` field is [`components`](Self::components) and the protected
/// helpers are provided methods over it.
pub trait FieldOffsetStrategy: fmt::Debug {
    /// Equivalent to the protected final field `components`; `None` where
    /// Java's is `null` (`MultiFieldsOffsetStrategy`).
    fn components(&self) -> Option<&UHComponents>;

    /// The field this strategy applies to.
    ///
    /// Equivalent to `getField()`.
    ///
    /// # Errors
    ///
    /// What an override raises (`MultiFieldsOffsetStrategy` has no single
    /// field).
    fn get_field(&self) -> Result<&str> {
        Ok(self
            .components()
            .ok_or_else(LuceneError::null_pointer)?
            .field())
    }

    /// Equivalent to the abstract `getOffsetSource()`.
    fn get_offset_source(&self) -> OffsetSource;

    /// The primary method -- return offsets for highlightable words in the
    /// specified document. Callers are expected to close the returned
    /// OffsetsEnum when it has been finished with.
    ///
    /// Equivalent to the abstract `getOffsetsEnum(LeafReader, int, String)`;
    /// `reader` is `None` for Java's `null`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn get_offsets_enum(
        &self,
        reader: Option<&Arc<dyn LeafReader>>,
        doc_id: i32,
        content: &str,
    ) -> Result<Box<dyn OffsetsEnum>>;

    /// Equivalent to the protected `createOffsetsEnumFromReader(LeafReader,
    /// int)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn create_offsets_enum_from_reader(
        &self,
        leaf_reader: &Arc<dyn LeafReader>,
        doc: i32,
    ) -> Result<Box<dyn OffsetsEnum>> {
        let components = self.components().ok_or_else(LuceneError::null_pointer)?;
        let Some(terms_index) = leaf_reader.terms(self.get_field()?)? else {
            return Ok(empty());
        };

        let mut offsets_enums: Vec<Box<dyn OffsetsEnum>> = Vec::new();

        // Handle Weight.matches approach
        if components.highlight_flags().contains(&HighlightFlag::WEIGHT_MATCHES) {
            self.create_offsets_enums_weight_matcher(leaf_reader, doc, &mut offsets_enums)?;
        } else {
            // classic approach
            // Handle position insensitive terms (a subset of this.terms
            // field):
            let phrase_helper = components.phrase_helper();
            let terms = components.terms().ok_or_else(LuceneError::null_pointer)?;
            let insensitive_terms: Vec<BytesRef> = if phrase_helper.has_position_sensitivity() {
                let insensitive = phrase_helper.get_all_position_insensitive_terms();
                debug_assert!(
                    insensitive.len() <= terms.len(),
                    "insensitive terms should be smaller set of all terms"
                );
                insensitive
            } else {
                terms.to_vec()
            };
            if !insensitive_terms.is_empty() {
                self.create_offsets_enums_for_terms(&insensitive_terms, &terms_index, doc, &mut offsets_enums)?;
            }

            // Handle spans
            if phrase_helper.has_position_sensitivity() {
                phrase_helper.create_offsets_enums_for_spans(leaf_reader, doc, &mut offsets_enums)?;
            }

            // Handle automata
            if !components
                .automata()
                .ok_or_else(LuceneError::null_pointer)?
                .is_empty()
            {
                self.create_offsets_enums_for_automata(&terms_index, doc, &mut offsets_enums)?;
            }
        }

        match offsets_enums.len() {
            0 => Ok(empty()),
            1 => Ok(offsets_enums.pop().expect("INVARIANT: one element")),
            _ => Ok(Box::new(MultiOffsetsEnum::new(offsets_enums)?)),
        }
    }

    /// Equivalent to the protected `createOffsetsEnumsWeightMatcher(LeafReader,
    /// int, List)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn create_offsets_enums_weight_matcher(
        &self,
        leaf_reader: &Arc<dyn LeafReader>,
        doc_id: i32,
        results: &mut Vec<Box<dyn OffsetsEnum>>,
    ) -> Result<()> {
        let components = self.components().ok_or_else(LuceneError::null_pointer)?;
        // remap fieldMatcher/requireFieldMatch fields to the field we are
        // highlighting
        let leaf_reader: Arc<dyn LeafReader> = Arc::new(FieldMatcherFilterLeafReader {
            base: FilterLeafReader::new(Arc::clone(leaf_reader))?,
            components: components.clone(),
        });
        let mut index_searcher = IndexSearcher::new(Arc::clone(&leaf_reader).as_index_reader())?;
        index_searcher.set_query_cache(None);
        let context = Arc::clone(&leaf_reader)
            .as_index_reader()
            .get_context()?
            .as_leaf()
            .ok_or_else(LuceneError::null_pointer)?;
        let matches = index_searcher
            .rewrite(Arc::clone(components.query()))?
            .create_weight(&index_searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?
            .matches(&context, doc_id)?;
        let Some(matches) = matches else {
            return Ok(()); // doc doesn't match
        };
        for field in matches.fields() {
            if (components.field_matcher())(&field) {
                let Some(iterator) = matches.get_matches(&field)? else {
                    continue;
                };

                results.push(Box::new(OfMatchesIteratorWithSubs::new(iterator)?));
            }
        }
        Ok(())
    }

    /// Equivalent to the protected `createOffsetsEnumsForTerms(BytesRef[],
    /// Terms, int, List)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors; `IllegalArgumentException` for a field indexed
    /// without offsets.
    fn create_offsets_enums_for_terms(
        &self,
        source_terms: &[BytesRef],
        terms_index: &Arc<dyn Terms>,
        doc: i32,
        results: &mut Vec<Box<dyn OffsetsEnum>>,
    ) -> Result<()> {
        let mut terms_enum = terms_index.iterator()?; // does not return null
        for term in source_terms {
            if terms_enum.seek_exact(term)? {
                let postings_enum = terms_enum.postings(None, POSTINGS_ENUM_OFFSETS)?;
                let Some(mut postings_enum) = postings_enum else {
                    // no offsets or positions available
                    return Err(LuceneError::illegal_argument(format!(
                        "field '{}' was indexed without offsets, cannot highlight",
                        self.get_field()?
                    )));
                };
                if doc == postings_enum.advance(doc)? {
                    // now it's positioned, although may be exhausted
                    results.push(Box::new(OfPostings::new(term.clone(), postings_enum)?));
                }
            }
        }
        Ok(())
    }

    /// Equivalent to the protected `createOffsetsEnumsForAutomata(Terms,
    /// int, List)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn create_offsets_enums_for_automata(
        &self,
        terms_index: &Arc<dyn Terms>,
        doc: i32,
        results: &mut Vec<Box<dyn OffsetsEnum>>,
    ) -> Result<()> {
        let components = self.components().ok_or_else(LuceneError::null_pointer)?;
        let automata = components.automata().ok_or_else(LuceneError::null_pointer)?;
        let mut automata_postings: Vec<Vec<Box<dyn PostingsEnum>>> =
            (0..automata.len()).map(|_| Vec::new()).collect();

        let mut terms_enum = terms_index.iterator()?;
        loop {
            let term = match terms_enum.next()? {
                Some(term) => term.clone(),
                None => break,
            };
            for (i, automaton) in automata.iter().enumerate() {
                let chars: Vec<u16> = String::from_utf8_lossy(term.slice()).encode_utf16().collect();
                if automaton.match_chars_ref(&chars) {
                    let mut postings = terms_enum
                        .postings(None, POSTINGS_ENUM_OFFSETS)?
                        .ok_or_else(LuceneError::null_pointer)?;
                    if doc == postings.advance(doc)? {
                        automata_postings[i].push(postings);
                    }
                }
            }
        }

        for (i, postings_enums) in automata_postings.into_iter().enumerate() {
            let automaton = &automata[i];
            if postings_enums.is_empty() {
                continue;
            }
            // Build one OffsetsEnum exposing the automaton label as the term,
            // and the sum of freq
            let wildcard_term = BytesRef::new(automaton.get_label().as_bytes().to_vec());
            let mut sum_freq = 0i32;
            for postings_enum in &postings_enums {
                sum_freq = sum_freq.wrapping_add(postings_enum.freq()?);
            }
            for postings_enum in postings_enums {
                results.push(Box::new(OfPostings::with_freq(
                    wildcard_term.clone(),
                    sum_freq,
                    postings_enum,
                )?));
            }
        }
        Ok(())
    }
}

/// The anonymous `FilterLeafReader` of `createOffsetsEnumsWeightMatcher`:
/// the fields the field matcher accepts read the highlighted field.
struct FieldMatcherFilterLeafReader {
    base: FilterLeafReader,
    components: UHComponents,
}

impl fmt::Debug for FieldMatcherFilterLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldOffsetStrategy$1").finish_non_exhaustive()
    }
}

impl fmt::Display for FieldMatcherFilterLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.base, f)
    }
}

impl LeafReader for FieldMatcherFilterLeafReader {
    crate::forward_leaf_reader!(filter base;
        core,
        filter_leaf_reader_delegate,
        do_close,
        term_vectors,
        num_docs,
        max_doc,
        stored_fields,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
        get_sorted_numeric_doc_values,
        get_sorted_set_doc_values,
        get_norm_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_field_infos,
        get_live_docs,
        get_point_values,
        check_integrity,
        get_meta_data,
    );

    fn terms(&self, field: &str) -> Result<Option<Arc<dyn Terms>>> {
        if (self.components.field_matcher())(field) {
            self.base.get_delegate().terms(self.components.field())
        } else {
            self.base.get_delegate().terms(field)
        }
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}