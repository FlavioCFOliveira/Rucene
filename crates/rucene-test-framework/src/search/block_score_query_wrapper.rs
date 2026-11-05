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

//! Port of `org.apache.lucene.tests.search.BlockScoreQueryWrapper`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::similarities::Explanation;
use rucene::search::{
    DefaultScorerSupplier, DocIdSetIterator, IndexSearcher, Query, QueryVisitor, Scorable,
    ScoreMode, Scorer, ScorerSupplier, SegmentCacheable, Weight, NO_MORE_DOCS,
};

/// Query wrapper that reduces the size of max-score blocks to more easily
/// detect problems with the max-score logic.
///
/// Equivalent to `org.apache.lucene.tests.search.BlockScoreQueryWrapper`.
#[derive(Debug, Clone)]
pub struct BlockScoreQueryWrapper {
    /// `query`.
    query: Arc<dyn Query>,
    /// `blockLength`.
    block_length: i32,
}

impl BlockScoreQueryWrapper {
    /// `new BlockScoreQueryWrapper(query, blockLength)`.
    pub fn new(query: Arc<dyn Query>, block_length: i32) -> Self {
        BlockScoreQueryWrapper {
            query,
            block_length,
        }
    }
}

impl Query for BlockScoreQueryWrapper {
    fn to_query_string(&self, field: &str) -> String {
        self.query.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.query.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<BlockScoreQueryWrapper>()
            .is_some_and(|that| {
                self.query.query_eq(&*that.query) && self.block_length == that.block_length
            })
    }

    fn query_hash(&self) -> u64 {
        let mut h = self.class_hash();
        h = h.wrapping_mul(31).wrapping_add(self.query.query_hash());
        h = h.wrapping_mul(31).wrapping_add(self.block_length as u64);
        h
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(self.query.rewrite(searcher)?.map(|rewritten| {
            Arc::new(BlockScoreQueryWrapper::new(rewritten, self.block_length)) as Arc<dyn Query>
        }))
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let in_weight = self.query.create_weight(searcher, score_mode, boost)?;
        if !score_mode.needs_scores() {
            return Ok(in_weight);
        }
        Ok(Arc::new(BlockScoreWeight {
            query: Arc::new(self.clone()),
            in_weight,
            block_length: self.block_length,
        }))
    }
}

/// The anonymous `Weight` of `createWeight`.
#[derive(Debug)]
struct BlockScoreWeight {
    query: Arc<dyn Query>,
    in_weight: Arc<dyn Weight>,
    block_length: i32,
}

impl SegmentCacheable for BlockScoreWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.in_weight.is_cacheable(ctx)
    }
}

impl Weight for BlockScoreWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.in_weight.explain(context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(mut in_scorer) = self.in_weight.scorer(context)? else {
            return Ok(None);
        };
        // The buffered documents start with -1 and end with NO_MORE_DOCS.
        let mut docs = vec![-1];
        let mut scores = vec![0.0f32];
        let mut doc = in_scorer.iterator().next_doc()?;
        loop {
            docs.push(doc);
            if doc == NO_MORE_DOCS {
                scores.push(0.0);
                break;
            }
            scores.push(in_scorer.score()?);
            doc = in_scorer.iterator().next_doc()?;
        }
        let scorer = BlockScorer {
            docs,
            scores,
            i: 0,
            block_length: self.block_length,
            last_shallow_target: -1,
        };
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }
}

/// `Math.max(float, float)`: NaN if either is NaN, and `0f` above `-0f`.
fn java_math_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() {
            b
        } else {
            a
        }
    } else if a >= b {
        a
    } else {
        b
    }
}

/// The anonymous `Scorer` of `scorerSupplier`, its own iterator view.
struct BlockScorer {
    docs: Vec<i32>,
    scores: Vec<f32>,
    i: usize,
    block_length: i32,
    /// `lastShallowTarget`.
    last_shallow_target: i32,
}

impl BlockScorer {
    /// `Arrays.binarySearch(docs, target)` mapped to its insertion point.
    fn search(&self, target: i32) -> usize {
        match self.docs.binary_search(&target) {
            Ok(i) | Err(i) => i,
        }
    }

    /// `startOfBlock(target)`.
    fn start_of_block(&self, target: i32) -> usize {
        let i = self.search(target);
        i - i % self.block_length as usize
    }

    /// `endOfBlock(target)`.
    fn end_of_block(&self, target: i32) -> usize {
        (self.start_of_block(target) + self.block_length as usize).min(self.docs.len() - 1)
    }
}

impl Scorable for BlockScorer {
    fn score(&mut self) -> Result<f32> {
        Ok(self.scores[self.i])
    }
}

impl Scorer for BlockScorer {
    fn doc_id(&self) -> i32 {
        self.docs[self.i]
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.last_shallow_target = target;
        if target == NO_MORE_DOCS {
            return Ok(NO_MORE_DOCS);
        }
        Ok(self.docs[self.end_of_block(target)] - 1)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let mut max = 0.0f32;
        let mut j = self.start_of_block(self.docs[self.i].max(self.last_shallow_target));
        loop {
            if self.docs[j] > up_to {
                break;
            }
            max = java_math_max(max, self.scores[j]);
            if j == self.docs.len() - 1 {
                break;
            }
            j += 1;
        }
        Ok(max)
    }
}

impl DocIdSetIterator for BlockScorer {
    fn doc_id(&self) -> i32 {
        self.docs[self.i]
    }

    fn next_doc(&mut self) -> Result<i32> {
        java_assert!(self.docs[self.i] != NO_MORE_DOCS);
        self.i += 1;
        Ok(self.docs[self.i])
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.i = self.search(target);
        java_assert!(self.docs[self.i] >= target);
        Ok(self.docs[self.i])
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64 - 2
    }
}
