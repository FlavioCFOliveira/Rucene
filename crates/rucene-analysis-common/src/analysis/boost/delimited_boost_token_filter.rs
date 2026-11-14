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

//! Splits each token at a delimiter into a term and a boost, ported from
//! `org.apache.lucene.analysis.boost.DelimitedBoostTokenFilter`.

use std::any::TypeId;

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::search::{BoostAttribute, BoostAttributeImpl};
use rucene::util::attribute::AttributeSource;

use crate::analysis::synonym::word2vec::dl4j_model_reader::parse_float;
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`DelimitedBoostTokenFilter`]: what Lucene's
/// `DelimitedBoostTokenFilter` adds to `TokenFilter`.
///
/// The term is scanned as UTF-16 units, as Java scans its `char[]`. The
/// `BoostAttribute` is set through its implementation, `BoostAttributeImpl`,
/// which `rucene`'s default attribute factory provides for it.
#[derive(Debug)]
pub struct DelimitedBoostTokenFilterLogic {
    delimiter: u16,
}

impl TokenFilterLogic for DelimitedBoostTokenFilterLogic {
    /// Equivalent to `DelimitedBoostTokenFilter.incrementToken()`.
    ///
    /// # Errors
    ///
    /// What the input returns, and the `NumberFormatException` of
    /// `Float.parseFloat` for a boost that is not a float.
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
                    let boost = parse_float(&String::from_utf16_lossy(&buffer[i + 1..length]))?;
                    source
                        .get_attribute_mut::<BoostAttributeImpl>()
                        .expect("INVARIANT: the constructor added the BoostAttribute")
                        .set_boost(boost);
                    let chars = utf16_to_term_chars(&buffer[..i]);
                    source
                        .get_attribute_interface_mut::<dyn CharTermAttribute>()
                        .expect("INVARIANT: the constructor added the CharTermAttribute")
                        .copy_buffer(&chars, 0, chars.len());
                    return Ok(true);
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Characters before the delimiter are the "token", those after are the
/// boost.
///
/// For example, if the delimiter is '|', then for the string "foo|0.7", foo
/// is the token and 0.7 is the boost.
///
/// Note make sure your Tokenizer doesn't split on the delimiter, or this
/// won't work
///
/// Equivalent to `org.apache.lucene.analysis.boost.DelimitedBoostTokenFilter`.
pub type DelimitedBoostTokenFilter = TokenFilter<DelimitedBoostTokenFilterLogic>;

/// Equivalent to `DelimitedBoostTokenFilter(TokenStream, char)`, which adds
/// the `CharTermAttribute` and the `BoostAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_delimited_boost_token_filter(
    input: Box<dyn TokenStream>,
    delimiter: u16,
) -> Result<DelimitedBoostTokenFilter> {
    let mut filter = TokenFilter::new(input, DelimitedBoostTokenFilterLogic { delimiter });
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_by_id(TypeId::of::<dyn BoostAttribute>())?;
    Ok(filter)
}
