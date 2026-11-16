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

//! Port of `org.apache.lucene.tests.search.BulkScorerWrapperScorer`.

use rucene::error::{LuceneError, Result};
use rucene::search::{
    BulkScorer, CollectionResult, DocIdSetIterator, LeafCollector, Scorable, Scorer, NO_MORE_DOCS,
};

/// A [`BulkScorer`]-backed scorer: it scores windows of documents into a
/// buffer and iterates over the buffer.
///
/// Equivalent to `org.apache.lucene.tests.search.BulkScorerWrapperScorer`.
/// The scorer is its own iterator view (`iterator()`).
pub struct BulkScorerWrapperScorer {
    /// `scorer`.
    scorer: Box<dyn BulkScorer>,
    /// `i`.
    i: i32,
    /// `doc`.
    doc: i32,
    /// `next`.
    next: i32,
    /// `docs`.
    docs: Vec<i32>,
    /// `scores`.
    scores: Vec<f32>,
    /// `bufferLength`.
    buffer_length: i32,
}

impl BulkScorerWrapperScorer {
    /// `new BulkScorerWrapperScorer(scorer, bufferSize)`.
    pub fn new(scorer: Box<dyn BulkScorer>, buffer_size: usize) -> Self {
        BulkScorerWrapperScorer {
            scorer,
            i: -1,
            doc: -1,
            next: 0,
            docs: vec![0; buffer_size],
            scores: vec![0.0; buffer_size],
            buffer_length: 0,
        }
    }

    /// `refill(target)`.
    fn refill(&mut self, target: i32) -> Result<()> {
        self.buffer_length = 0;
        while self.next != NO_MORE_DOCS && self.buffer_length == 0 {
            let min = target.max(self.next);
            // `min + docs.length`: Java's int addition wraps.
            let max = min.wrapping_add(self.docs.len() as i32);
            let mut collector = Buffering {
                docs: &mut self.docs,
                scores: &mut self.scores,
                buffer_length: &mut self.buffer_length,
            };
            self.next = self
                .scorer
                .score(&mut collector, None, min, max)
                .map_err(|e| e.into_lucene_error())?;
        }
        self.i = -1;
        Ok(())
    }
}

/// The anonymous `LeafCollector` of `refill`.
struct Buffering<'a> {
    docs: &'a mut [i32],
    scores: &'a mut [f32],
    buffer_length: &'a mut i32,
}

impl LeafCollector for Buffering<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let index = *self.buffer_length;
        // `docs[bufferLength] = doc`: an `ArrayIndexOutOfBoundsException` past
        // the buffer.
        *array_slot(self.docs, index)? = doc;
        let score = scorer.score()?;
        *array_slot(self.scores, index)? = score;
        *self.buffer_length += 1;
        Ok(())
    }
}

/// Java's array element access `array[index]`: the element, or the JVM's
/// `ArrayIndexOutOfBoundsException` (`Index <i> out of bounds for length
/// <n>`, JDK 25).
fn array_slot<T>(array: &mut [T], index: i32) -> Result<&mut T> {
    let length = array.len();
    usize::try_from(index)
        .ok()
        .and_then(|i| array.get_mut(i))
        .ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Index {index} out of bounds for length {length}"),
            )
        })
}

impl Scorable for BulkScorerWrapperScorer {
    fn score(&mut self) -> Result<f32> {
        // `scores[i]`: `i` is -1 before the first `nextDoc`/`advance`.
        let i = self.i;
        Ok(*array_slot(&mut self.scores, i)?)
    }
}

impl Scorer for BulkScorerWrapperScorer {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::INFINITY)
    }
}

/// The anonymous `DocIdSetIterator` of `iterator()`.
impl DocIdSetIterator for BulkScorerWrapperScorer {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        // `advance(docID() + 1)`: Java's int addition wraps.
        self.advance(self.doc.wrapping_add(1))
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if self.buffer_length == 0 || self.docs[(self.buffer_length - 1) as usize] < target {
            self.refill(target)?;
        }
        // `Arrays.binarySearch(docs, i + 1, bufferLength, target)`, whose
        // `rangeCheck` throws first (JDK 25 texts).
        let (from_index, to_index) = (self.i.wrapping_add(1), self.buffer_length);
        if from_index > to_index {
            return Err(LuceneError::illegal_argument(format!(
                "fromIndex({from_index}) > toIndex({to_index})"
            )));
        }
        if from_index < 0 || to_index as usize > self.docs.len() {
            let index = if from_index < 0 { from_index } else { to_index };
            return Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Array index out of range: {index}"),
            ));
        }
        let from = from_index as usize;
        let to = to_index as usize;
        self.i = match self.docs[from..to].binary_search(&target) {
            Ok(k) => (from + k) as i32,
            Err(k) => (from + k) as i32,
        };
        if self.i == self.buffer_length {
            self.doc = NO_MORE_DOCS;
            return Ok(NO_MORE_DOCS);
        }
        self.doc = self.docs[self.i as usize];
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.scorer.cost()
    }
}

#[cfg(test)]
mod tests {
    use rucene::util::Bits;

    use super::*;

    /// A scorable that scores every document 1.
    struct One;

    impl Scorable for One {
        fn score(&mut self) -> Result<f32> {
            Ok(1.0)
        }
    }

    /// A bulk scorer over the documents 2 and 5.
    struct TwoDocs;

    impl BulkScorer for TwoDocs {
        fn score(
            &mut self,
            collector: &mut dyn LeafCollector,
            _accept_docs: Option<&dyn Bits>,
            min: i32,
            max: i32,
        ) -> CollectionResult<i32> {
            for doc in [2, 5] {
                if doc >= min && doc < max {
                    collector.collect(doc, &mut One)?;
                }
            }
            Ok([2, 5]
                .into_iter()
                .find(|&doc| doc >= max)
                .unwrap_or(NO_MORE_DOCS))
        }

        fn cost(&self) -> i64 {
            2
        }
    }

    /// `nextDoc()` after exhaustion is `advance(NO_MORE_DOCS + 1)`, which
    /// wraps to `Integer.MIN_VALUE` in Java and answers `NO_MORE_DOCS` again.
    #[test]
    fn next_doc_after_exhaustion_wraps_as_java_int() {
        let mut scorer = BulkScorerWrapperScorer::new(Box::new(TwoDocs), 3);
        assert_eq!(scorer.next_doc().expect("nextDoc"), 2);
        assert_eq!(scorer.next_doc().expect("nextDoc"), 5);
        assert_eq!(scorer.next_doc().expect("nextDoc"), NO_MORE_DOCS);
        assert_eq!(scorer.next_doc().expect("nextDoc"), NO_MORE_DOCS);
    }

    /// `score()` before the scorer is positioned reads `scores[-1]`: the
    /// JVM's `ArrayIndexOutOfBoundsException`.
    #[test]
    fn score_before_positioning_is_an_array_index_out_of_bounds() {
        let mut scorer = BulkScorerWrapperScorer::new(Box::new(TwoDocs), 3);
        let e = scorer.score().expect_err("scores[-1]");
        match e.kind() {
            rucene::error::LuceneErrorKind::Unchecked { class, message, .. } => {
                assert_eq!(*class, "java.lang.ArrayIndexOutOfBoundsException");
                assert_eq!(
                    message.as_deref(),
                    Some("Index -1 out of bounds for length 3")
                );
            }
            other => panic!("unexpected error {other:?}"),
        }
    }
}
