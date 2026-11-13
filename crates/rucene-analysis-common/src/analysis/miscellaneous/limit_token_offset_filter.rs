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
//! Stops a stream at the first token past a start offset, ported from
//! `org.apache.lucene.analysis.miscellaneous.LimitTokenOffsetFilter`.

use rucene::analysis::{OffsetAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

/// The body of [`LimitTokenOffsetFilter`]: what Lucene's
/// `LimitTokenOffsetFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct LimitTokenOffsetFilterLogic {
    max_start_offset: i32,
    consume_all_tokens: bool,
}

impl TokenFilterLogic for LimitTokenOffsetFilterLogic {
    /// Equivalent to `LimitTokenOffsetFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }
        let start_offset = source
            .get_attribute_interface::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .start_offset();
        if start_offset <= self.max_start_offset {
            return Ok(true);
        }
        if self.consume_all_tokens {
            while input.increment_token()? {
                // no-op
            }
        }
        Ok(false)
    }
}

/// Lets all tokens pass through until it sees one with a start offset
/// <= a configured limit, which won't pass and ends the stream. This can be
/// useful to limit highlighting, for example.
///
/// By default, this filter ignores any tokens in the wrapped `TokenStream`
/// once the limit has been exceeded, which can result in `reset()` being
/// called prior to `incrementToken()` returning `false`. For most
/// `TokenStream` implementations this should be acceptable, and faster then
/// consuming the full stream. If you are wrapping a `TokenStream` which
/// requires that the full stream of tokens be exhausted in order to function
/// properly, use the `consumeAllTokens` option.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.LimitTokenOffsetFilter`.
pub type LimitTokenOffsetFilter = TokenFilter<LimitTokenOffsetFilterLogic>;

/// Lets all tokens pass through until it sees one with a start offset <=
/// `max_start_offset` which won't pass and ends the stream. It won't consume
/// any tokens afterwards.
///
/// Equivalent to `LimitTokenOffsetFilter(TokenStream, int)`, `this(input,
/// maxStartOffset, false)`.
///
/// # Errors
///
/// As [`new_limit_token_offset_filter_consuming`].
pub fn new_limit_token_offset_filter(
    input: Box<dyn TokenStream>,
    max_start_offset: i32,
) -> Result<LimitTokenOffsetFilter> {
    new_limit_token_offset_filter_consuming(input, max_start_offset, false)
}

/// Equivalent to `LimitTokenOffsetFilter(TokenStream, int, boolean)`, which
/// adds the `OffsetAttribute`.
///
/// # Errors
///
/// `IllegalArgumentException` "maxStartOffset must be >= zero", and what the
/// attribute factory raises.
pub fn new_limit_token_offset_filter_consuming(
    input: Box<dyn TokenStream>,
    max_start_offset: i32,
    consume_all_tokens: bool,
) -> Result<LimitTokenOffsetFilter> {
    let mut filter = TokenFilter::new(
        input,
        LimitTokenOffsetFilterLogic {
            max_start_offset,
            consume_all_tokens,
        },
    );
    // Java's field initializer `addAttribute` runs before the constructor body.
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn OffsetAttribute>()?;
    if max_start_offset < 0 {
        return Err(LuceneError::illegal_argument(
            "maxStartOffset must be >= zero",
        ));
    }
    Ok(filter)
}
