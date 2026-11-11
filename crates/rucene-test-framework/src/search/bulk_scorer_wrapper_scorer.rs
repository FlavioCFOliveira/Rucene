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

use rucene::error::Result;
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
        let index = *self.buffer_length as usize;
        self.docs[index] = doc;
        self.scores[index] = scorer.score()?;
        *self.buffer_length += 1;
        Ok(())
    }
}

impl Scorable for BulkScorerWrapperScorer {
    fn score(&mut self) -> Result<f32> {
        Ok(self.scores[self.i as usize])
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
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if self.buffer_length == 0 || self.docs[(self.buffer_length - 1) as usize] < target {
            self.refill(target)?;
        }
        // `Arrays.binarySearch(docs, i + 1, bufferLength, target)`.
        let from = (self.i + 1) as usize;
        let to = self.buffer_length as usize;
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
