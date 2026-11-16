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

use rucene::error::{LuceneError, LuceneErrorKind, Result};
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

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.search.BlockScoreQueryWrapper"
    }

    /// `BlockScoreQueryWrapper.hashCode()`.
    fn query_hash(&self) -> i32 {
        let mut h = self.class_hash();
        h = h.wrapping_mul(31).wrapping_add(self.query.query_hash());
        h.wrapping_mul(31).wrapping_add(self.block_length)
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
///
/// Its index arithmetic is Java's `int` arithmetic: `startOfBlock` keeps the
/// sign rule of `%` for a negative `blockLength`, throws
/// `ArithmeticException: / by zero` for a zero one, and an index out of the
/// arrays is the JVM's `ArrayIndexOutOfBoundsException`.
struct BlockScorer {
    docs: Vec<i32>,
    scores: Vec<f32>,
    /// `i`.
    i: i32,
    block_length: i32,
    /// `lastShallowTarget`.
    last_shallow_target: i32,
}

/// The JVM's `ArrayIndexOutOfBoundsException` for `array[index]` (JDK 25
/// text).
fn index_out_of_bounds(index: i32, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// `array[index]`.
fn element<T: Copy>(array: &[T], index: i32) -> Result<T> {
    usize::try_from(index)
        .ok()
        .and_then(|i| array.get(i).copied())
        .ok_or_else(|| index_out_of_bounds(index, array.len()))
}

impl BlockScorer {
    /// `Arrays.binarySearch(docs, target)` mapped to its insertion point.
    fn search(&self, target: i32) -> i32 {
        match self.docs.binary_search(&target) {
            Ok(i) | Err(i) => i as i32,
        }
    }

    /// `startOfBlock(target)`: `i - i % blockLength`.
    fn start_of_block(&self, target: i32) -> Result<i32> {
        let i = self.search(target);
        if self.block_length == 0 {
            return Err(LuceneError::from(LuceneErrorKind::Arithmetic(
                "/ by zero".to_string(),
            )));
        }
        Ok(i.wrapping_sub(i.wrapping_rem(self.block_length)))
    }

    /// `endOfBlock(target)`: `Math.min(startOfBlock(target) + blockLength,
    /// docs.length - 1)`.
    fn end_of_block(&self, target: i32) -> Result<i32> {
        Ok(self
            .start_of_block(target)?
            .wrapping_add(self.block_length)
            .min(self.docs.len() as i32 - 1))
    }

    /// `docs[i]` where Java's accessor cannot declare an exception: the
    /// JVM's `ArrayIndexOutOfBoundsException` is thrown.
    fn current_doc(&self) -> i32 {
        element(&self.docs, self.i)
            .unwrap_or_else(|e| crate::util::java_exceptions::rethrow_java(e))
    }
}

impl Scorable for BlockScorer {
    fn score(&mut self) -> Result<f32> {
        element(&self.scores, self.i)
    }
}

impl Scorer for BlockScorer {
    fn doc_id(&self) -> i32 {
        self.current_doc()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.last_shallow_target = target;
        if target == NO_MORE_DOCS {
            return Ok(NO_MORE_DOCS);
        }
        Ok(element(&self.docs, self.end_of_block(target)?)?.wrapping_sub(1))
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let mut max = 0.0f32;
        let mut j =
            self.start_of_block(element(&self.docs, self.i)?.max(self.last_shallow_target))?;
        loop {
            if element(&self.docs, j)? > up_to {
                break;
            }
            max = java_math_max(max, element(&self.scores, j)?);
            if j == self.docs.len() as i32 - 1 {
                break;
            }
            j = j.wrapping_add(1);
        }
        Ok(max)
    }
}

impl DocIdSetIterator for BlockScorer {
    fn doc_id(&self) -> i32 {
        self.current_doc()
    }

    fn next_doc(&mut self) -> Result<i32> {
        java_assert!(element(&self.docs, self.i)? != NO_MORE_DOCS);
        // `docs[++i]`
        self.i = self.i.wrapping_add(1);
        element(&self.docs, self.i)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.i = self.search(target);
        java_assert!(element(&self.docs, self.i)? >= target);
        element(&self.docs, self.i)
    }

    fn cost(&self) -> i64 {
        // `docs.length - 2`: an `int`.
        i64::from(self.docs.len() as i32 - 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scorer(docs: Vec<i32>, block_length: i32) -> BlockScorer {
        let scores = vec![1.0; docs.len()];
        BlockScorer {
            docs,
            scores,
            i: 0,
            block_length,
            last_shallow_target: -1,
        }
    }

    /// `startOfBlock` is `int` arithmetic: with a negative `blockLength`,
    /// `2 - 2 % -2` is 2 and `endOfBlock` is `min(2 + -2, 3)` = 0, so
    /// `advanceShallow(5)` answers `docs[0] - 1`.
    #[test]
    fn negative_block_length_follows_java_int_remainder() {
        let mut s = scorer(vec![-1, 0, 5, NO_MORE_DOCS], -2);
        assert_eq!(s.advance_shallow(5).expect("advanceShallow"), -2);
    }

    /// A zero `blockLength` is Java's `ArithmeticException: / by zero`.
    #[test]
    fn zero_block_length_is_an_arithmetic_exception() {
        let mut s = scorer(vec![-1, 0, 5, NO_MORE_DOCS], 0);
        let e = s.advance_shallow(5).expect_err("/ by zero");
        assert!(
            matches!(e.kind(), LuceneErrorKind::Arithmetic(m) if m == "/ by zero"),
            "{e:?}"
        );
    }
}
