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

//! Port of `Disjunctions.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::index_searcher::IndexSearcher;

use crate::intervals::intervals::Intervals;
use crate::intervals::intervals_source::IntervalsSource;

/// Rewrites that pull disjunctions up to the top of a source tree.
///
/// Equivalent to the final class
/// `org.apache.lucene.queries.intervals.Disjunctions`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Disjunctions;

impl Disjunctions {
    /// Given a list of sources that contain disjunctions, and a combiner
    /// function, pulls the disjunctions to the top of the source tree, e.g.
    /// `FUNC(a, b, OR(c, "d e")) => [FUNC(a, b, c), FUNC(a, b, "d e")]`.
    ///
    /// Equivalent to the static `Disjunctions.pullUp(List<IntervalsSource>,
    /// Function<List<IntervalsSource>, IntervalsSource>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` ("Too many disjunctions to expand") when the
    /// expansion would exceed `IndexSearcher.getMaxClauseCount()`.
    pub(crate) fn pull_up(
        sources: &[Arc<dyn IntervalsSource>],
        function: impl Fn(Vec<Arc<dyn IntervalsSource>>) -> Arc<dyn IntervalsSource>,
    ) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        let mut rewritten: Vec<Vec<Arc<dyn IntervalsSource>>> = vec![Vec::new()];
        for source in sources {
            let disjuncts = Self::split_disjunctions(Arc::clone(source))?;
            if disjuncts.len() == 1 {
                for l in &mut rewritten {
                    l.push(Arc::clone(&disjuncts[0]));
                }
            } else {
                if (rewritten.len() as i32).wrapping_mul(disjuncts.len() as i32)
                    > IndexSearcher::get_max_clause_count()
                {
                    return Err(LuceneError::illegal_argument(
                        "Too many disjunctions to expand",
                    ));
                }
                let mut to_add = Vec::with_capacity(rewritten.len() * disjuncts.len());
                for disj in &disjuncts {
                    // clone the rewritten list, then append the disjunct
                    for sub_list in &rewritten {
                        let mut l = sub_list.clone();
                        l.push(Arc::clone(disj));
                        to_add.push(l);
                    }
                }
                rewritten = to_add;
            }
        }
        if rewritten.len() == 1 {
            let only = rewritten.pop().expect("INVARIANT: one rewritten list");
            return Ok(vec![function(only)]);
        }
        Ok(rewritten.into_iter().map(function).collect())
    }

    /// Given a source containing disjunctions, and a mapping function, pulls
    /// the disjunctions to the top of the source tree.
    ///
    /// Equivalent to the static `Disjunctions.pullUp(IntervalsSource,
    /// Function<IntervalsSource, IntervalsSource>)`.
    ///
    /// # Errors
    ///
    /// Propagates the error of splitting the disjunctions.
    pub(crate) fn pull_up_one(
        source: Arc<dyn IntervalsSource>,
        function: impl Fn(Arc<dyn IntervalsSource>) -> Arc<dyn IntervalsSource>,
    ) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        let mut disjuncts = Self::split_disjunctions(source)?;
        if disjuncts.len() == 1 {
            let only = disjuncts.pop().expect("INVARIANT: one disjunct");
            return Ok(vec![function(only)]);
        }
        Ok(disjuncts.into_iter().map(function).collect())
    }

    /// Separates out disjunctions into individual sources. Clauses that have
    /// a min extent of 1 are grouped together and treated as a single source,
    /// as any overlapping intervals of length 1 can be treated as identical,
    /// and all combinatorial sources have a min extent above 1.
    ///
    /// Equivalent to the private static
    /// `Disjunctions.splitDisjunctions(IntervalsSource)`.
    fn split_disjunctions(
        source: Arc<dyn IntervalsSource>,
    ) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        let mut singletons = Vec::new();
        let mut non_singletons = Vec::new();
        for disj in source.pull_up_disjunctions()? {
            if disj.min_extent() == 1 {
                singletons.push(disj);
            } else {
                non_singletons.push(disj);
            }
        }
        let mut split = Vec::with_capacity(1 + non_singletons.len());
        if !singletons.is_empty() {
            split.push(Intervals::or(singletons)?);
        }
        split.extend(non_singletons);
        Ok(split)
    }
}
