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

//! Stores the token's type as its payload, ported from
//! `org.apache.lucene.analysis.payloads.TypeAsPayloadTokenFilter`.

use rucene::analysis::{
    PayloadAttribute, TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

/// The body of [`TypeAsPayloadTokenFilter`]: what Lucene's
/// `TypeAsPayloadTokenFilter` adds to `TokenFilter`.
#[derive(Debug, Default)]
pub struct TypeAsPayloadTokenFilterLogic;

impl TokenFilterLogic for TypeAsPayloadTokenFilterLogic {
    /// Equivalent to the final `TypeAsPayloadTokenFilter.incrementToken()`;
    /// Java's `type != null` test has no counterpart, a type being a string.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let type_value = source
                .get_attribute_interface::<dyn TypeAttribute>()
                .expect("INVARIANT: the constructor added the TypeAttribute")
                .type_value()
                .to_string();
            if !type_value.is_empty() {
                source
                    .get_attribute_interface_mut::<dyn PayloadAttribute>()
                    .expect("INVARIANT: the constructor added the PayloadAttribute")
                    .set_payload(Some(BytesRef::from_char_sequence(&type_value)));
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Makes the `TypeAttribute` a payload.
///
/// Encodes the type using `String#getBytes(String)` with "UTF-8" as the
/// encoding
///
/// Equivalent to `org.apache.lucene.analysis.payloads.TypeAsPayloadTokenFilter`.
pub type TypeAsPayloadTokenFilter = TokenFilter<TypeAsPayloadTokenFilterLogic>;

/// Equivalent to `TypeAsPayloadTokenFilter(TokenStream)`, which adds the
/// `PayloadAttribute` and the `TypeAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_type_as_payload_token_filter(
    input: Box<dyn TokenStream>,
) -> Result<TypeAsPayloadTokenFilter> {
    let mut filter = TokenFilter::new(input, TypeAsPayloadTokenFilterLogic);
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn PayloadAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(filter)
}
