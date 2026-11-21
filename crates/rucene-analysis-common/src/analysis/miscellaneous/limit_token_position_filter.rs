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
//! Limits the positions a stream emits, ported from
//! `org.apache.lucene.analysis.miscellaneous.LimitTokenPositionFilter`.

use rucene::analysis::{PositionIncrementAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

/// The body of [`LimitTokenPositionFilter`]: what Lucene's
/// `LimitTokenPositionFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct LimitTokenPositionFilterLogic {
    max_token_position: i32,
    consume_all_tokens: bool,
    token_position: i32,
    exhausted: bool,
}

impl TokenFilterLogic for LimitTokenPositionFilterLogic {
    /// Equivalent to `LimitTokenPositionFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if self.exhausted {
            return Ok(false);
        }
        if input.increment_token()? {
            self.token_position += source
                .get_attribute_interface::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .get_position_increment();
            if self.token_position <= self.max_token_position {
                Ok(true)
            } else {
                while self.consume_all_tokens && input.increment_token()? { /* NOOP */ }
                self.exhausted = true;
                Ok(false)
            }
        } else {
            self.exhausted = true;
            Ok(false)
        }
    }

    /// Equivalent to `LimitTokenPositionFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.token_position = 0;
        self.exhausted = false;
        Ok(())
    }
}

/// This TokenFilter limits its emitted tokens to those with positions that
/// are not greater than the configured limit.
///
/// By default, this filter ignores any tokens in the wrapped `TokenStream`
/// once the limit has been exceeded, which can result in `reset()` being
/// called prior to `incrementToken()` returning `false`. For most
/// `TokenStream` implementations this should be acceptable, and faster then
/// consuming the full stream. If you are wrapping a `TokenStream` which
/// requires that the full stream of tokens be exhausted in order to function
/// properly, use the `consumeAllTokens` option.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.LimitTokenPositionFilter`.
pub type LimitTokenPositionFilter = TokenFilter<LimitTokenPositionFilterLogic>;

/// Build a filter that only accepts tokens up to and including the given
/// maximum position. This filter will not consume any tokens with position
/// greater than the `max_token_position` limit.
///
/// Equivalent to `LimitTokenPositionFilter(TokenStream, int)`, `this(in,
/// maxTokenPosition, false)`.
///
/// # Errors
///
/// As [`new_limit_token_position_filter_consuming`].
pub fn new_limit_token_position_filter(
    input: Box<dyn TokenStream>,
    max_token_position: i32,
) -> Result<LimitTokenPositionFilter> {
    new_limit_token_position_filter_consuming(input, max_token_position, false)
}

/// Build a filter that limits the maximum position of tokens to emit.
///
/// Equivalent to `LimitTokenPositionFilter(TokenStream, int, boolean)`, which
/// adds the `PositionIncrementAttribute`.
///
/// # Errors
///
/// `IllegalArgumentException` "maxTokenPosition must be greater than zero",
/// and what the attribute factory raises.
pub fn new_limit_token_position_filter_consuming(
    input: Box<dyn TokenStream>,
    max_token_position: i32,
    consume_all_tokens: bool,
) -> Result<LimitTokenPositionFilter> {
    let mut filter = TokenFilter::new(
        input,
        LimitTokenPositionFilterLogic {
            max_token_position,
            consume_all_tokens,
            token_position: 0,
            exhausted: false,
        },
    );
    // Java's field initializer `addAttribute` runs before the constructor body.
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    if max_token_position < 1 {
        return Err(LuceneError::illegal_argument(
            "maxTokenPosition must be greater than zero",
        ));
    }
    Ok(filter)
}
