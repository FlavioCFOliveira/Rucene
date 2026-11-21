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

//! Wraps a term and boost, ported from
//! `org.apache.lucene.analysis.synonym.word2vec.TermAndBoost`.

use rucene::util::BytesRef;

/// Wraps a term and boost: `term` the term, `boost` the boost.
///
/// Equivalent to the record `org.apache.lucene.analysis.synonym.word2vec.TermAndBoost`.
#[derive(Debug, Clone, PartialEq)]
pub struct TermAndBoost {
    term: BytesRef,
    boost: f32,
}

impl TermAndBoost {
    /// Creates a new TermAndBoost.
    ///
    /// Equivalent to the compact constructor, which keeps
    /// `BytesRef.deepCopyOf(term)`.
    pub fn new(term: &BytesRef, boost: f32) -> Self {
        Self {
            term: BytesRef::deep_copy_of(term),
            boost,
        }
    }

    /// Equivalent to the record accessor `term()`.
    pub fn term(&self) -> &BytesRef {
        &self.term
    }

    /// Equivalent to the record accessor `boost()`.
    pub fn boost(&self) -> f32 {
        self.boost
    }
}
