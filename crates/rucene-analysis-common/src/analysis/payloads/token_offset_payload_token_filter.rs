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

//! Stores the token's offsets as its payload, ported from
//! `org.apache.lucene.analysis.payloads.TokenOffsetPayloadTokenFilter`.

use rucene::analysis::{
    OffsetAttribute, PayloadAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

use super::payload_helper::PayloadHelper;

/// The body of [`TokenOffsetPayloadTokenFilter`]: what Lucene's
/// `TokenOffsetPayloadTokenFilter` adds to `TokenFilter`.
#[derive(Debug, Default)]
pub struct TokenOffsetPayloadTokenFilterLogic;

impl TokenFilterLogic for TokenOffsetPayloadTokenFilterLogic {
    /// Equivalent to the final `TokenOffsetPayloadTokenFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut data = vec![0u8; 8];
            let (start, end) = {
                let offset = source
                    .get_attribute_interface::<dyn OffsetAttribute>()
                    .expect("INVARIANT: the constructor added the OffsetAttribute");
                (offset.start_offset(), offset.end_offset())
            };
            PayloadHelper::encode_int_into(start, &mut data, 0);
            PayloadHelper::encode_int_into(end, &mut data, 4);
            let payload = BytesRef::new(data);
            source
                .get_attribute_interface_mut::<dyn PayloadAttribute>()
                .expect("INVARIANT: the constructor added the PayloadAttribute")
                .set_payload(Some(payload));
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Adds the `OffsetAttribute#startOffset()` and
/// `OffsetAttribute#endOffset()` First 4 bytes are the start
///
/// Equivalent to `org.apache.lucene.analysis.payloads.TokenOffsetPayloadTokenFilter`.
pub type TokenOffsetPayloadTokenFilter = TokenFilter<TokenOffsetPayloadTokenFilterLogic>;

/// Equivalent to `TokenOffsetPayloadTokenFilter(TokenStream)`, which adds
/// the `OffsetAttribute` and the `PayloadAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_token_offset_payload_token_filter(
    input: Box<dyn TokenStream>,
) -> Result<TokenOffsetPayloadTokenFilter> {
    let mut filter = TokenFilter::new(input, TokenOffsetPayloadTokenFilterLogic);
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PayloadAttribute>()?;
    Ok(filter)
}
