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

//! Port of `Scorer.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::any::Any;
use std::fmt::Debug;

use rucene::analysis::TokenStream;
use rucene::error::Result;

use crate::highlight::text_fragment::TextFragment;

/// A Scorer is responsible for scoring a stream of tokens. These token scores
/// can then be used to compute `TextFragment` scores.
///
/// Equivalent to `org.apache.lucene.search.highlight.Scorer`.
pub trait Scorer: Debug + Send {
    /// Called to init the Scorer with a `TokenStream`. You can grab
    /// references to the attributes you are interested in here and access
    /// them from `get_token_score()`.
    ///
    /// Equivalent to `init(TokenStream)`. Java returns either `null`, to keep
    /// reading `token_stream`, or a new stream to read instead; the port takes
    /// the stream and returns the one to read: `token_stream` itself where
    /// Java returns `null`, the new stream (which may own `token_stream`)
    /// otherwise.
    ///
    /// # Errors
    ///
    /// Propagates the scorer's I/O and attribute errors.
    fn init(&mut self, token_stream: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>;

    /// Called when a new fragment is started for consideration.
    ///
    /// Equivalent to `startFragment(TextFragment)`.
    fn start_fragment(&mut self, new_fragment: &TextFragment);

    /// Called for each token in the current fragment.
    ///
    /// Equivalent to `getTokenScore()`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions Java's implementations raise, as the
    /// `NullPointerException` of a scorer used before `init` /
    /// `startFragment`.
    fn get_token_score(&mut self) -> Result<f32>;

    /// Called when the `Highlighter` has no more tokens for the current
    /// fragment - the Scorer returns the weighting it has derived for the
    /// most recent fragment, typically based on the results of
    /// `get_token_score()`.
    ///
    /// Equivalent to `getFragmentScore()`.
    fn get_fragment_score(&self) -> f32;

    /// This scorer as `Any`, for the `instanceof` tests Java performs on a
    /// `Scorer` (`Highlighter` tests `instanceof QueryScorer`).
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
}