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

//! Port of `FacetCountsWithFilterQuery.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::multi_reader::reader_util;
use rucene::index::IndexReaderContext;
use rucene::search::conjunction_utils::ConjunctionUtils;
use rucene::search::{DocIdSetIterator, IndexSearcher, Query, ScoreMode, Scorer};

use crate::facets_collector::MatchingDocs;

/// Base class for facet counts. It allows for a query to be passed in to
/// filter the match set.
///
/// Equivalent to `org.apache.lucene.facet.FacetCountsWithFilterQuery`: the
/// state and helper its subclasses inherit.
#[derive(Debug, Clone)]
pub struct FacetCountsWithFilterQuery {
    /// Optional: if specified, we first test this Query to see whether the
    /// document should be checked for matching ranges.
    pub fast_match_query: Option<Arc<dyn Query>>,
}

/// A `Scorer`'s iterator as an owned `DocIdSetIterator`, for Java's
/// `s.iterator()` added to a conjunction.
pub(crate) struct ScorerIterator {
    pub(crate) scorer: Box<dyn Scorer>,
}

impl DocIdSetIterator for ScorerIterator {
    fn doc_id(&self) -> i32 {
        self.scorer
            .iterator_ref()
            .map(|it| it.doc_id())
            .unwrap_or_else(|| self.scorer.doc_id())
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.scorer.iterator().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.scorer.iterator().advance(target)
    }

    fn cost(&self) -> i64 {
        self.scorer.iterator_ref().map_or(0, |it| it.cost())
    }
}

impl FacetCountsWithFilterQuery {
    /// Constructor.
    ///
    /// Equivalent to `FacetCountsWithFilterQuery(Query)`.
    pub fn new(fast_match_query: Option<Arc<dyn Query>>) -> Self {
        Self { fast_match_query }
    }

    /// Create a `DocIdSetIterator` from the provided `hits` that relies on
    /// `fastMatchQuery` if available for first-pass filtering; `None` for
    /// Java's `null` when no doc can match.
    ///
    /// Equivalent to `createIterator(MatchingDocs, DocIdSetIterator...)`;
    /// Java's `null` extra iterator is `None` in `iterators`.
    pub fn create_iterator(
        &self,
        hits: &MatchingDocs,
        iterators: Vec<Option<Box<dyn DocIdSetIterator>>>,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let mut all_iterators: Vec<Box<dyn DocIdSetIterator>> = Vec::new();
        all_iterators.push(hits.bits.iterator()?);
        for it in iterators {
            match it {
                Some(it) => all_iterators.push(it),
                None => return Ok(None),
            }
        }

        if let Some(fast_match_query) = &self.fast_match_query {
            let top_level_context = reader_util::get_top_level_context(
                Arc::clone(&hits.context) as Arc<dyn IndexReaderContext>
            );
            let mut searcher = IndexSearcher::from_context(top_level_context, None)?;
            searcher.set_query_cache(None);
            let fast_match_weight = searcher.create_weight(
                searcher.rewrite(Arc::clone(fast_match_query))?,
                ScoreMode::COMPLETE_NO_SCORES,
                1.0,
            )?;
            let Some(s) = fast_match_weight.scorer(&hits.context)? else {
                // We can't match any docs if the fast match query doesn't
                // match anything
                return Ok(None);
            };
            all_iterators.push(Box::new(ScorerIterator { scorer: s }));
        }

        if all_iterators.len() == 1 {
            Ok(all_iterators.pop())
        } else {
            Ok(Some(
                ConjunctionUtils::intersect_iterators(all_iterators)?.into_doc_id_set_iterator(),
            ))
        }
    }
}
