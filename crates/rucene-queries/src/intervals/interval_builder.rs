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

//! Port of `IntervalBuilder.java`, of the Java package
//! `org.apache.lucene.queries.intervals`, code adopted from ASL-licensed
//! Elasticsearch (`server/src/main/java/org/elasticsearch/index/query/IntervalBuilder.java`,
//! branch 7.10).

use std::sync::{Arc, LazyLock};

use rucene::analysis::tokenattributes::{
    PositionIncrementAttribute, PositionLengthAttribute, TermToBytesRefAttribute,
};
use rucene::analysis::{CachingTokenFilter, TokenStream};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::search::index_searcher::{IndexSearcher, TooManyClauses};
use rucene::util::graph::GraphTokenStreamFiniteStrings;
use rucene::util::{AttributeSource, BytesRef};

use crate::intervals::intervals::Intervals;
use crate::intervals::intervals_source::IntervalsSource;

/// The source of an analysed text without tokens.
///
/// Equivalent to the private static `IntervalBuilder.NO_INTERVALS`.
static NO_INTERVALS: LazyLock<Arc<dyn IntervalsSource>> =
    LazyLock::new(|| Intervals::no_intervals("No terms in analyzed text"));

/// The current term bytes of a stream, or `None` without a
/// `TermToBytesRefAttribute`.
fn term_bytes_of(atts: &AttributeSource) -> Option<BytesRef> {
    atts.get_attribute_interface::<dyn TermToBytesRefAttribute>()
        .map(|att| att.get_bytes_ref())
}

/// The current position increment; the attribute is added before it is read,
/// as Java's `addAttribute` does, so its absence is the default of `1`.
fn position_increment_of(atts: &AttributeSource) -> i32 {
    atts.get_attribute_interface::<dyn PositionIncrementAttribute>()
        .map_or(1, |att| att.get_position_increment())
}

/// The current position length, as [`position_increment_of`].
fn position_length_of(atts: &AttributeSource) -> i32 {
    atts.get_attribute_interface::<dyn PositionLengthAttribute>()
        .map_or(1, |att| att.get_position_length())
}

/// `BytesRef.deepCopyOf(bytesAtt.getBytesRef())`.
fn current_term(ts: &dyn TokenStream) -> BytesRef {
    let bytes = term_bytes_of(ts.attribute_source()).unwrap_or_default();
    BytesRef::deep_copy_of(&bytes)
}

/// Constructs an [`IntervalsSource`] based on analysed text.
///
/// Equivalent to the final class
/// `org.apache.lucene.queries.intervals.IntervalBuilder`, code adopted from
/// ASL-licensed Elasticsearch.
///
/// Java's class is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct IntervalBuilder;

impl IntervalBuilder {
    /// Equivalent to the package-private static
    /// `IntervalBuilder.analyzeText(CachingTokenFilter, int, boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates any error of the analysis chain, and the `TooManyClauses`
    /// of a graph with too many paths.
    pub fn analyze_text(
        stream: &mut CachingTokenFilter,
        max_gaps: i32,
        ordered: bool,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let has_term = stream
            .attribute_source()
            .has_attribute_interface::<dyn TermToBytesRefAttribute>();
        {
            let atts = stream.attribute_source_mut();
            atts.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
            atts.add_attribute_interface::<dyn PositionLengthAttribute>()?;
        }
        if !has_term {
            return Ok(Arc::clone(&NO_INTERVALS));
        }

        // phase 1: read through the stream and assess the situation:
        // counting the number of tokens/positions and marking if we have any
        // synonyms.

        let mut num_tokens = 0;
        let mut has_synonyms = false;
        let mut is_graph = false;

        stream.reset()?;
        while stream.increment_token()? {
            num_tokens += 1;
            let position_increment = position_increment_of(stream.attribute_source());
            if position_increment == 0 {
                has_synonyms = true;
            }
            let position_length = position_length_of(stream.attribute_source());
            if position_length > 1 {
                is_graph = true;
            }
        }

        // phase 2: based on token count, presence of synonyms, and options
        // formulate a single term, boolean, or phrase.

        if num_tokens == 0 {
            Ok(Arc::clone(&NO_INTERVALS))
        } else if num_tokens == 1 {
            // single term
            Self::analyze_term(stream)
        } else if is_graph {
            // graph
            Self::combine_sources(Self::analyze_graph(stream)?, max_gaps, ordered)
        } else {
            // phrase
            if has_synonyms {
                // phrase with single-term synonyms
                Self::analyze_synonyms(stream, max_gaps, ordered)
            } else {
                // simple phrase
                Self::combine_sources(Self::analyze_terms(stream)?, max_gaps, ordered)
            }
        }
    }

    /// Equivalent to the private static
    /// `IntervalBuilder.analyzeTerm(TokenStream)`.
    fn analyze_term(ts: &mut dyn TokenStream) -> Result<Arc<dyn IntervalsSource>> {
        ts.attribute_source_mut()
            .add_attribute_interface::<dyn TermToBytesRefAttribute>()?;
        ts.reset()?;
        ts.increment_token()?;
        Ok(Intervals::term(current_term(ts)))
    }

    /// Equivalent to the private static
    /// `IntervalBuilder.combineSources(List<IntervalsSource>, int, boolean)`.
    fn combine_sources(
        sources: Vec<Arc<dyn IntervalsSource>>,
        max_gaps: i32,
        ordered: bool,
    ) -> Result<Arc<dyn IntervalsSource>> {
        if sources.is_empty() {
            return Ok(Arc::clone(&NO_INTERVALS));
        }
        if sources.len() == 1 {
            return Ok(Arc::clone(&sources[0]));
        }
        if max_gaps == 0 && ordered {
            return Intervals::phrase(sources);
        }
        let inner = if ordered {
            Intervals::ordered(sources)
        } else {
            Intervals::unordered(sources)
        };
        if max_gaps == -1 {
            return Ok(inner);
        }
        Intervals::maxgaps(max_gaps, inner)
    }

    /// Equivalent to the private static
    /// `IntervalBuilder.analyzeTerms(TokenStream)`.
    fn analyze_terms(ts: &mut dyn TokenStream) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        let mut terms = Vec::new();
        {
            let atts = ts.attribute_source_mut();
            atts.add_attribute_interface::<dyn TermToBytesRefAttribute>()?;
            atts.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        }
        ts.reset()?;
        while ts.increment_token()? {
            let term = current_term(ts);
            let preceding_spaces = position_increment_of(ts.attribute_source()) - 1;
            terms.push(Self::extend(Intervals::term(term), preceding_spaces));
        }
        ts.end()?;
        Ok(terms)
    }

    /// Equivalent to the private static
    /// `IntervalBuilder.extend(IntervalsSource, int)`.
    fn extend(source: Arc<dyn IntervalsSource>, preceding_spaces: i32) -> Arc<dyn IntervalsSource> {
        if preceding_spaces == 0 {
            return source;
        }
        Intervals::extend(source, preceding_spaces, 0)
    }

    /// Equivalent to the private static
    /// `IntervalBuilder.analyzeSynonyms(TokenStream, int, boolean)`.
    fn analyze_synonyms(
        ts: &mut dyn TokenStream,
        max_gaps: i32,
        ordered: bool,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let mut terms: Vec<Arc<dyn IntervalsSource>> = Vec::new();
        let mut synonyms: Vec<Arc<dyn IntervalsSource>> = Vec::new();
        {
            let atts = ts.attribute_source_mut();
            atts.add_attribute_interface::<dyn TermToBytesRefAttribute>()?;
            atts.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        }
        ts.reset()?;
        let mut spaces = 0;
        while ts.increment_token()? {
            let pos_inc = position_increment_of(ts.attribute_source());
            if pos_inc > 0 {
                if synonyms.len() == 1 {
                    terms.push(Self::extend(Arc::clone(&synonyms[0]), spaces));
                } else if synonyms.len() > 1 {
                    terms.push(Self::extend(Intervals::or(synonyms.clone())?, spaces));
                }
                synonyms.clear();
                spaces = pos_inc - 1;
            }
            synonyms.push(Intervals::term(current_term(ts)));
        }
        if synonyms.len() == 1 {
            terms.push(Self::extend(Arc::clone(&synonyms[0]), spaces));
        } else {
            terms.push(Self::extend(Intervals::or(synonyms)?, spaces));
        }
        Self::combine_sources(terms, max_gaps, ordered)
    }

    /// Equivalent to the private static
    /// `IntervalBuilder.analyzeGraph(TokenStream)`.
    fn analyze_graph(source: &mut dyn TokenStream) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        source.reset()?;
        let graph = GraphTokenStreamFiniteStrings::new(source)?;

        let mut clauses = Vec::new();
        let articulation_points = graph.articulation_points()?;
        let mut last_state = 0;
        let max_clause_count = IndexSearcher::get_max_clause_count();
        for i in 0..=articulation_points.len() {
            let start = last_state;
            let mut end = -1;
            if i < articulation_points.len() {
                end = articulation_points[i];
            }
            last_state = end;
            if graph.has_side_path(start) {
                let mut paths: Vec<Arc<dyn IntervalsSource>> = Vec::new();
                let mut it = graph.get_finite_strings_between(start, end);
                while let Some(mut ts) = it.next()? {
                    let phrase = Self::combine_sources(Self::analyze_terms(&mut ts)?, 0, true)?;
                    if paths.len() as i32 >= max_clause_count {
                        return Err(LuceneError::from(TooManyClauses::new()));
                    }
                    paths.push(phrase);
                }
                if !paths.is_empty() {
                    clauses.push(Intervals::or(paths)?);
                }
            } else {
                let mut it = graph.get_finite_strings_between(start, end);
                // `it.next()` of an exhausted iterator: `NoSuchElementException`.
                let Some(mut ts) = it.next()? else {
                    return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                        class: "java.util.NoSuchElementException",
                        message: None,
                        cause: None,
                    }));
                };
                clauses.extend(Self::analyze_terms(&mut ts)?);
                debug_assert!(!it.has_next()?);
            }
        }
        Ok(clauses)
    }
}
