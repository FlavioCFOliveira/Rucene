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

//! Assigns a numeric payload to tokens of a type, ported from
//! `org.apache.lucene.analysis.payloads.NumericPayloadTokenFilter`.

use rucene::analysis::{
    PayloadAttribute, TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

use super::payload_helper::PayloadHelper;

/// The body of [`NumericPayloadTokenFilter`]: what Lucene's
/// `NumericPayloadTokenFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct NumericPayloadTokenFilterLogic {
    type_match: String,
    the_payload: BytesRef,
}

impl TokenFilterLogic for NumericPayloadTokenFilterLogic {
    /// Equivalent to the final `NumericPayloadTokenFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let matches = source
                .get_attribute_interface::<dyn TypeAttribute>()
                .expect("INVARIANT: the constructor added the TypeAttribute")
                .type_value()
                == self.type_match;
            if matches {
                source
                    .get_attribute_interface_mut::<dyn PayloadAttribute>()
                    .expect("INVARIANT: the constructor added the PayloadAttribute")
                    .set_payload(Some(self.the_payload.clone()));
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Assigns a payload to a token based on the `TypeAttribute`
///
/// Equivalent to `org.apache.lucene.analysis.payloads.NumericPayloadTokenFilter`.
pub type NumericPayloadTokenFilter = TokenFilter<NumericPayloadTokenFilterLogic>;

/// Equivalent to `NumericPayloadTokenFilter(TokenStream, float, String)`,
/// which adds the `PayloadAttribute` and the `TypeAttribute` and encodes the
/// payload with `PayloadHelper.encodeFloat`; Java's `NullPointerException`
/// for a `null` type has no counterpart.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_numeric_payload_token_filter(
    input: Box<dyn TokenStream>,
    payload: f32,
    type_match: &str,
) -> Result<NumericPayloadTokenFilter> {
    let logic = NumericPayloadTokenFilterLogic {
        type_match: type_match.to_string(),
        // Need to encode the payload
        the_payload: BytesRef::new(PayloadHelper::encode_float(payload)),
    };
    let mut filter = TokenFilter::new(input, logic);
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn PayloadAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(filter)
}
