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

//! Tokenizes the input from an edge into n-grams, ported from
//! `org.apache.lucene.analysis.ngram.EdgeNGramTokenizer`.

use std::sync::Arc;

use rucene::analysis::default_token_attribute_factory;
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use super::ngram_tokenizer::{new_ngram_tokenizer_with_token_char, NGramTokenizer};

/// Equivalent to `EdgeNGramTokenizer.DEFAULT_MAX_GRAM_SIZE`.
pub const DEFAULT_MAX_GRAM_SIZE: i32 = 1;

/// Equivalent to `EdgeNGramTokenizer.DEFAULT_MIN_GRAM_SIZE`.
pub const DEFAULT_MIN_GRAM_SIZE: i32 = 1;

/// Tokenizes the input from an edge into n-grams of given size(s).
///
/// This `Tokenizer` create n-grams from the beginning edge of a input token.
///
/// As of Lucene 4.4, this class supports pre-tokenization (`isTokenChar(int)`)
/// and correctly handles supplementary characters.
///
/// Equivalent to `org.apache.lucene.analysis.ngram.EdgeNGramTokenizer`, an
/// `NGramTokenizer` whose `edgesOnly` is set: the same tokenizer type, built
/// by this module's constructors.
pub type EdgeNGramTokenizer = NGramTokenizer;

/// Creates EdgeNGramTokenizer that can generate n-grams in the sizes of the
/// given range
///
/// * `min_gram` the smallest n-gram to generate
/// * `max_gram` the largest n-gram to generate
///
/// Equivalent to `EdgeNGramTokenizer(int, int)`, `super(minGram, maxGram,
/// true)`.
///
/// # Errors
///
/// As [`new_ngram_tokenizer_with_token_char`].
pub fn new_edge_ngram_tokenizer(min_gram: i32, max_gram: i32) -> Result<EdgeNGramTokenizer> {
    new_edge_ngram_tokenizer_with_factory(default_token_attribute_factory(), min_gram, max_gram)
}

/// Creates EdgeNGramTokenizer that can generate n-grams in the sizes of the
/// given range
///
/// * `factory` `AttributeFactory` to use
/// * `min_gram` the smallest n-gram to generate
/// * `max_gram` the largest n-gram to generate
///
/// Equivalent to `EdgeNGramTokenizer(AttributeFactory, int, int)`,
/// `super(factory, minGram, maxGram, true)`.
///
/// # Errors
///
/// As [`new_ngram_tokenizer_with_token_char`].
pub fn new_edge_ngram_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
    min_gram: i32,
    max_gram: i32,
) -> Result<EdgeNGramTokenizer> {
    new_ngram_tokenizer_with_token_char(factory, min_gram, max_gram, true, Box::new(|_| true))
}
