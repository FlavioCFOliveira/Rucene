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

//! Splits each token at a delimiter into a term and a payload, ported from
//! `org.apache.lucene.analysis.payloads.DelimitedPayloadTokenFilter`.

use std::sync::Arc;

use rucene::analysis::{
    CharTermAttribute, PayloadAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::payload_encoder::PayloadEncoder;
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// Equivalent to `DelimitedPayloadTokenFilter.DEFAULT_DELIMITER`.
pub const DEFAULT_DELIMITER: u16 = b'|' as u16;

/// The body of [`DelimitedPayloadTokenFilter`]: what Lucene's
/// `DelimitedPayloadTokenFilter` adds to `TokenFilter`.
///
/// The term is scanned as UTF-16 units, as Java scans its `char[]`, and the
/// payload units are handed to the encoder as such.
#[derive(Debug)]
pub struct DelimitedPayloadTokenFilterLogic {
    delimiter: u16,
    encoder: Arc<dyn PayloadEncoder>,
}

impl TokenFilterLogic for DelimitedPayloadTokenFilterLogic {
    /// Equivalent to `DelimitedPayloadTokenFilter.incrementToken()`.
    ///
    /// # Errors
    ///
    /// What the input returns, and what the encoder returns.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let buffer = {
                let term = source
                    .get_attribute_interface::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                chars_to_utf16(&term.buffer()[..term.length()])
            };
            let length = buffer.len();
            for i in 0..length {
                if buffer[i] == self.delimiter {
                    let payload = self
                        .encoder
                        .encode_range(&buffer, i + 1, length - (i + 1))?;
                    source
                        .get_attribute_interface_mut::<dyn PayloadAttribute>()
                        .expect("INVARIANT: the constructor added the PayloadAttribute")
                        .set_payload(Some(payload));
                    // simply set a new length
                    let chars = utf16_to_term_chars(&buffer[..i]);
                    source
                        .get_attribute_interface_mut::<dyn CharTermAttribute>()
                        .expect("INVARIANT: the constructor added the CharTermAttribute")
                        .copy_buffer(&chars, 0, chars.len());
                    return Ok(true);
                }
            }
            // we have not seen the delimiter
            source
                .get_attribute_interface_mut::<dyn PayloadAttribute>()
                .expect("INVARIANT: the constructor added the PayloadAttribute")
                .set_payload(None);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Characters before the delimiter are the "token", those after are the
/// payload.
///
/// For example, if the delimiter is '|', then for the string "foo|bar", foo
/// is the token and "bar" is a payload.
///
/// Note, you can also include a `PayloadEncoder` to convert the payload in
/// an appropriate way (from characters to bytes).
///
/// Note make sure your Tokenizer doesn't split on the delimiter, or this
/// won't work
///
/// Equivalent to `org.apache.lucene.analysis.payloads.DelimitedPayloadTokenFilter`.
pub type DelimitedPayloadTokenFilter = TokenFilter<DelimitedPayloadTokenFilterLogic>;

/// Equivalent to `DelimitedPayloadTokenFilter(TokenStream, char,
/// PayloadEncoder)`, which adds the `CharTermAttribute` and the
/// `PayloadAttribute`; Java's `NullPointerException` for a `null` encoder
/// has no counterpart.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_delimited_payload_token_filter(
    input: Box<dyn TokenStream>,
    delimiter: u16,
    encoder: Arc<dyn PayloadEncoder>,
) -> Result<DelimitedPayloadTokenFilter> {
    let mut filter = TokenFilter::new(
        input,
        DelimitedPayloadTokenFilterLogic { delimiter, encoder },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn PayloadAttribute>()?;
    Ok(filter)
}
