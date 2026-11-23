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

//! Port of `UHComponents.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use rucene::search::query::Query;
use rucene::util::BytesRef;

use crate::uhighlight::labelled_char_array_matcher::LabelledCharArrayMatcher;
use crate::uhighlight::phrase_helper::PhraseHelper;
use crate::uhighlight::unified_highlighter::HighlightFlag;

/// A field name predicate: Java's `Predicate<String>`.
pub type FieldMatcher = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// A parameter object to hold the components a `FieldOffsetStrategy`
/// needs.
///
/// Equivalent to the record `org.apache.lucene.search.uhighlight.UHComponents`;
/// `terms` and `automata` are `None` for Java's `null` (when the highlighter
/// did not extract them).
#[derive(Clone)]
pub struct UHComponents {
    field: String,
    field_matcher: FieldMatcher,
    query: Arc<dyn Query>,
    terms: Option<Vec<BytesRef>>,
    phrase_helper: Arc<PhraseHelper>,
    automata: Option<Vec<Arc<dyn LabelledCharArrayMatcher>>>,
    has_unrecognized_query_part: bool,
    highlight_flags: HashSet<HighlightFlag>,
}

impl fmt::Debug for UHComponents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UHComponents")
            .field("field", &self.field)
            .field("query", &self.query)
            .field("terms", &self.terms)
            .field(
                "has_unrecognized_query_part",
                &self.has_unrecognized_query_part,
            )
            .field("highlight_flags", &self.highlight_flags)
            .finish_non_exhaustive()
    }
}

impl UHComponents {
    /// The canonical record constructor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        field: &str,
        field_matcher: FieldMatcher,
        query: Arc<dyn Query>,
        terms: Option<Vec<BytesRef>>,
        phrase_helper: Arc<PhraseHelper>,
        automata: Option<Vec<Arc<dyn LabelledCharArrayMatcher>>>,
        has_unrecognized_query_part: bool,
        highlight_flags: HashSet<HighlightFlag>,
    ) -> Self {
        Self {
            field: field.to_string(),
            field_matcher,
            query,
            terms,
            phrase_helper,
            automata,
            has_unrecognized_query_part,
            highlight_flags,
        }
    }

    /// The record accessor `field()`.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// The record accessor `fieldMatcher()`.
    pub fn field_matcher(&self) -> &FieldMatcher {
        &self.field_matcher
    }

    /// The record accessor `query()`.
    pub fn query(&self) -> &Arc<dyn Query> {
        &self.query
    }

    /// The record accessor `terms()`.
    pub fn terms(&self) -> Option<&[BytesRef]> {
        self.terms.as_deref()
    }

    /// The record accessor `phraseHelper()`.
    pub fn phrase_helper(&self) -> &Arc<PhraseHelper> {
        &self.phrase_helper
    }

    /// The record accessor `automata()`.
    pub fn automata(&self) -> Option<&[Arc<dyn LabelledCharArrayMatcher>]> {
        self.automata.as_deref()
    }

    /// The record accessor `hasUnrecognizedQueryPart()`.
    pub fn has_unrecognized_query_part(&self) -> bool {
        self.has_unrecognized_query_part
    }

    /// The record accessor `highlightFlags()`.
    pub fn highlight_flags(&self) -> &HashSet<HighlightFlag> {
        &self.highlight_flags
    }
}
