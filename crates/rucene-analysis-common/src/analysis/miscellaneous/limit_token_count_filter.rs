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
//! Limits the number of tokens a stream emits, ported from
//! `org.apache.lucene.analysis.miscellaneous.LimitTokenCountFilter`.

use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

/// The body of [`LimitTokenCountFilter`]: what Lucene's
/// `LimitTokenCountFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct LimitTokenCountFilterLogic {
    max_token_count: i32,
    consume_all_tokens: bool,
    token_count: i32,
    exhausted: bool,
}

impl TokenFilterLogic for LimitTokenCountFilterLogic {
    /// Equivalent to `LimitTokenCountFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if self.exhausted {
            Ok(false)
        } else if self.token_count < self.max_token_count {
            if input.increment_token()? {
                self.token_count += 1;
                Ok(true)
            } else {
                self.exhausted = true;
                Ok(false)
            }
        } else {
            while self.consume_all_tokens && input.increment_token()? { /* NOOP */ }
            Ok(false)
        }
    }

    /// Equivalent to `LimitTokenCountFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.token_count = 0;
        self.exhausted = false;
        Ok(())
    }
}

/// This TokenFilter limits the number of tokens while indexing. It is a
/// replacement for the maximum field length setting inside
/// `org.apache.lucene.index.IndexWriter`.
///
/// By default, this filter ignores any tokens in the wrapped `TokenStream`
/// once the limit has been reached, which can result in `reset()` being
/// called prior to `incrementToken()` returning `false`. For most
/// `TokenStream` implementations this should be acceptable, and faster then
/// consuming the full stream. If you are wrapping a `TokenStream` which
/// requires that the full stream of tokens be exhausted in order to function
/// properly, use the `consumeAllTokens` option.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.LimitTokenCountFilter`.
pub type LimitTokenCountFilter = TokenFilter<LimitTokenCountFilterLogic>;

/// Build a filter that only accepts tokens up to a maximum number. This
/// filter will not consume any tokens beyond the `max_token_count` limit.
///
/// Equivalent to `LimitTokenCountFilter(TokenStream, int)`, `this(in,
/// maxTokenCount, false)`.
///
/// # Errors
///
/// As [`new_limit_token_count_filter_consuming`].
pub fn new_limit_token_count_filter(
    input: Box<dyn TokenStream>,
    max_token_count: i32,
) -> Result<LimitTokenCountFilter> {
    new_limit_token_count_filter_consuming(input, max_token_count, false)
}

/// Build an filter that limits the maximum number of tokens per field:
/// `max_token_count` is the maximum number of tokens to return, and
/// `consume_all_tokens` whether all tokens from the input must be consumed
/// even if `max_token_count` is reached.
///
/// Equivalent to `LimitTokenCountFilter(TokenStream, int, boolean)`.
///
/// # Errors
///
/// `IllegalArgumentException` "maxTokenCount must be greater than zero".
pub fn new_limit_token_count_filter_consuming(
    input: Box<dyn TokenStream>,
    max_token_count: i32,
    consume_all_tokens: bool,
) -> Result<LimitTokenCountFilter> {
    if max_token_count < 1 {
        return Err(LuceneError::illegal_argument(
            "maxTokenCount must be greater than zero",
        ));
    }
    Ok(TokenFilter::new(
        input,
        LimitTokenCountFilterLogic {
            max_token_count,
            consume_all_tokens,
            token_count: 0,
            exhausted: false,
        },
    ))
}
