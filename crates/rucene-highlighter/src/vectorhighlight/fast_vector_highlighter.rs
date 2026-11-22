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

//! Port of `FastVectorHighlighter.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::IndexReader;
use rucene::search::query::Query;

use crate::highlight::encoder::Encoder;
use crate::vectorhighlight::field_frag_list::FieldFragList;
use crate::vectorhighlight::field_phrase_list::FieldPhraseList;
use crate::vectorhighlight::field_query::FieldQuery;
use crate::vectorhighlight::field_term_stack::FieldTermStack;
use crate::vectorhighlight::frag_list_builder::FragListBuilder;
use crate::vectorhighlight::fragments_builder::FragmentsBuilder;
use crate::vectorhighlight::score_order_fragments_builder::ScoreOrderFragmentsBuilder;
use crate::vectorhighlight::simple_frag_list_builder::SimpleFragListBuilder;

/// Another highlighter implementation.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.FastVectorHighlighter`.
#[derive(Debug)]
pub struct FastVectorHighlighter {
    /// Equivalent to the protected final field `phraseHighlight`.
    pub phrase_highlight: bool,
    /// Equivalent to the protected final field `fieldMatch`.
    pub field_match: bool,
    frag_list_builder: Box<dyn FragListBuilder>,
    fragments_builder: Box<dyn FragmentsBuilder>,
    phrase_limit: i32,
}

impl Default for FastVectorHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl FastVectorHighlighter {
    /// Equivalent to `FastVectorHighlighter.DEFAULT_PHRASE_HIGHLIGHT`.
    pub const DEFAULT_PHRASE_HIGHLIGHT: bool = true;
    /// Equivalent to `FastVectorHighlighter.DEFAULT_FIELD_MATCH`.
    pub const DEFAULT_FIELD_MATCH: bool = true;

    /// the default constructor.
    ///
    /// Equivalent to `new FastVectorHighlighter()`.
    pub fn new() -> Self {
        Self::with_options(Self::DEFAULT_PHRASE_HIGHLIGHT, Self::DEFAULT_FIELD_MATCH)
    }

    /// a constructor. Using `SimpleFragListBuilder` and
    /// `ScoreOrderFragmentsBuilder`.
    ///
    /// Equivalent to `new FastVectorHighlighter(boolean, boolean)`.
    pub fn with_options(phrase_highlight: bool, field_match: bool) -> Self {
        Self::with_builders(
            phrase_highlight,
            field_match,
            Box::new(SimpleFragListBuilder::new()),
            Box::new(ScoreOrderFragmentsBuilder::new()),
        )
    }

    /// a constructor. A `FragListBuilder` and a `FragmentsBuilder` can be
    /// specified (plugins).
    ///
    /// Equivalent to `new FastVectorHighlighter(boolean, boolean,
    /// FragListBuilder, FragmentsBuilder)`.
    pub fn with_builders(
        phrase_highlight: bool,
        field_match: bool,
        frag_list_builder: Box<dyn FragListBuilder>,
        fragments_builder: Box<dyn FragmentsBuilder>,
    ) -> Self {
        Self {
            phrase_highlight,
            field_match,
            frag_list_builder,
            fragments_builder,
            phrase_limit: i32::MAX,
        }
    }

    /// create a `FieldQuery` object.
    ///
    /// Equivalent to `getFieldQuery(Query)`: an `IOException` becomes a
    /// `RuntimeException`, as Java wraps it.
    ///
    /// # Errors
    ///
    /// What [`FieldQuery::new`] raises.
    pub fn get_field_query(&self, query: &Arc<dyn Query>) -> Result<FieldQuery> {
        // TODO: should we deprecate this? because if there is no reader then
        // we cannot correctly expand MultiTermQuery
        self.get_field_query_with_reader(query, None).map_err(|e| {
            if e.is_io_exception() {
                LuceneError::runtime_exception(e)
            } else {
                e
            }
        })
    }

    /// create a `FieldQuery` object.
    ///
    /// Equivalent to `getFieldQuery(Query, IndexReader)`.
    ///
    /// # Errors
    ///
    /// What [`FieldQuery::new`] raises.
    pub fn get_field_query_with_reader(
        &self,
        query: &Arc<dyn Query>,
        reader: Option<&Arc<dyn IndexReader>>,
    ) -> Result<FieldQuery> {
        FieldQuery::new(query, reader, self.phrase_highlight, self.field_match)
    }

    /// return the best fragment.
    ///
    /// Equivalent to `getBestFragment(FieldQuery, IndexReader, int, String,
    /// int)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn get_best_fragment(
        &self,
        field_query: &FieldQuery,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        frag_char_size: i32,
    ) -> Result<Option<String>> {
        let mut field_frag_list = self.get_field_frag_list(
            &*self.frag_list_builder,
            field_query,
            reader,
            doc_id,
            field_name,
            frag_char_size,
        )?;
        self.fragments_builder
            .create_fragment(reader, doc_id, field_name, &mut *field_frag_list)
    }

    /// return the best fragments.
    ///
    /// Equivalent to `getBestFragments(FieldQuery, IndexReader, int, String,
    /// int, int)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn get_best_fragments(
        &self,
        field_query: &FieldQuery,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        frag_char_size: i32,
        max_num_fragments: i32,
    ) -> Result<Option<Vec<String>>> {
        let mut field_frag_list = self.get_field_frag_list(
            &*self.frag_list_builder,
            field_query,
            reader,
            doc_id,
            field_name,
            frag_char_size,
        )?;
        self.fragments_builder.create_fragments(
            reader,
            doc_id,
            field_name,
            &mut *field_frag_list,
            max_num_fragments,
        )
    }

    /// return the best fragment.
    ///
    /// Equivalent to `getBestFragment(FieldQuery, IndexReader, int, String,
    /// int, FragListBuilder, FragmentsBuilder, String[], String[], Encoder)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    #[allow(clippy::too_many_arguments)]
    pub fn get_best_fragment_with(
        &self,
        field_query: &FieldQuery,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        frag_char_size: i32,
        frag_list_builder: &dyn FragListBuilder,
        fragments_builder: &dyn FragmentsBuilder,
        pre_tags: &[String],
        post_tags: &[String],
        encoder: &dyn Encoder,
    ) -> Result<Option<String>> {
        let mut field_frag_list = self.get_field_frag_list(
            frag_list_builder,
            field_query,
            reader,
            doc_id,
            field_name,
            frag_char_size,
        )?;
        fragments_builder.create_fragment_with_tags(
            reader,
            doc_id,
            field_name,
            &mut *field_frag_list,
            pre_tags,
            post_tags,
            encoder,
        )
    }

    /// return the best fragments.
    ///
    /// Equivalent to `getBestFragments(FieldQuery, IndexReader, int, String,
    /// int, int, FragListBuilder, FragmentsBuilder, String[], String[],
    /// Encoder)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    #[allow(clippy::too_many_arguments)]
    pub fn get_best_fragments_with(
        &self,
        field_query: &FieldQuery,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        frag_char_size: i32,
        max_num_fragments: i32,
        frag_list_builder: &dyn FragListBuilder,
        fragments_builder: &dyn FragmentsBuilder,
        pre_tags: &[String],
        post_tags: &[String],
        encoder: &dyn Encoder,
    ) -> Result<Option<Vec<String>>> {
        let mut field_frag_list = self.get_field_frag_list(
            frag_list_builder,
            field_query,
            reader,
            doc_id,
            field_name,
            frag_char_size,
        )?;
        fragments_builder.create_fragments_with_tags(
            reader,
            doc_id,
            field_name,
            &mut *field_frag_list,
            max_num_fragments,
            pre_tags,
            post_tags,
            encoder,
        )
    }

    /// Return the best fragments. Matches are scanned from matchedFields and
    /// turned into fragments against storedField. The highlighting may not
    /// make sense if matchedFields has matches with offsets that don't
    /// correspond features in storedField. It will outright throw an
    /// `IndexOutOfBoundsException` if matchedFields produces offsets outside
    /// of storedField. As such it is advisable that all matchedFields share
    /// the same source as storedField or are at least a prefix of it.
    ///
    /// Equivalent to `getBestFragments(FieldQuery, IndexReader, int, String,
    /// Set, int, int, FragListBuilder, FragmentsBuilder, String[], String[],
    /// Encoder)`; `matched_fields` is the set in its iteration order.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors; `IllegalArgumentException` when
    /// `matched_fields` is empty.
    #[allow(clippy::too_many_arguments)]
    pub fn get_best_fragments_matched_fields(
        &self,
        field_query: &FieldQuery,
        reader: &dyn IndexReader,
        doc_id: i32,
        stored_field: &str,
        matched_fields: &[&str],
        frag_char_size: i32,
        max_num_fragments: i32,
        frag_list_builder: &dyn FragListBuilder,
        fragments_builder: &dyn FragmentsBuilder,
        pre_tags: &[String],
        post_tags: &[String],
        encoder: &dyn Encoder,
    ) -> Result<Option<Vec<String>>> {
        let mut field_frag_list = self.get_field_frag_list_matched(
            frag_list_builder,
            field_query,
            reader,
            doc_id,
            matched_fields,
            frag_char_size,
        )?;
        fragments_builder.create_fragments_with_tags(
            reader,
            doc_id,
            stored_field,
            &mut *field_frag_list,
            max_num_fragments,
            pre_tags,
            post_tags,
            encoder,
        )
    }

    /// Build a FieldFragList for one field.
    fn get_field_frag_list(
        &self,
        frag_list_builder: &dyn FragListBuilder,
        field_query: &FieldQuery,
        reader: &dyn IndexReader,
        doc_id: i32,
        matched_field: &str,
        frag_char_size: i32,
    ) -> Result<Box<dyn FieldFragList>> {
        let mut field_term_stack = FieldTermStack::new(reader, doc_id, matched_field, field_query)?;
        let field_phrase_list =
            FieldPhraseList::with_phrase_limit(&mut field_term_stack, field_query, self.phrase_limit);
        frag_list_builder.create_field_frag_list(&field_phrase_list, frag_char_size)
    }

    /// Build a FieldFragList for more than one field.
    fn get_field_frag_list_matched(
        &self,
        frag_list_builder: &dyn FragListBuilder,
        field_query: &FieldQuery,
        reader: &dyn IndexReader,
        doc_id: i32,
        matched_fields: &[&str],
        frag_char_size: i32,
    ) -> Result<Box<dyn FieldFragList>> {
        if matched_fields.is_empty() {
            return Err(LuceneError::illegal_argument(
                "matchedFields must contain at least on field name.",
            ));
        }
        let mut to_merge: Vec<FieldPhraseList> = Vec::with_capacity(matched_fields.len());
        for matched_field in matched_fields {
            let mut stack = FieldTermStack::new(reader, doc_id, matched_field, field_query)?;
            to_merge.push(FieldPhraseList::with_phrase_limit(
                &mut stack,
                field_query,
                self.phrase_limit,
            ));
        }
        frag_list_builder.create_field_frag_list(&FieldPhraseList::from_merge(&to_merge), frag_char_size)
    }

    /// return whether phraseHighlight or not.
    ///
    /// Equivalent to `isPhraseHighlight()`.
    pub fn is_phrase_highlight(&self) -> bool {
        self.phrase_highlight
    }

    /// return whether fieldMatch or not.
    ///
    /// Equivalent to `isFieldMatch()`.
    pub fn is_field_match(&self) -> bool {
        self.field_match
    }

    /// Equivalent to `getPhraseLimit()`: the maximum number of phrases to
    /// analyze when searching for the highest-scoring phrase.
    pub fn get_phrase_limit(&self) -> i32 {
        self.phrase_limit
    }

    /// set the maximum number of phrases to analyze when searching for the
    /// highest-scoring phrase. The default is unlimited (i32::MAX).
    ///
    /// Equivalent to `setPhraseLimit(int)`.
    pub fn set_phrase_limit(&mut self, phrase_limit: i32) {
        self.phrase_limit = phrase_limit;
    }
}
