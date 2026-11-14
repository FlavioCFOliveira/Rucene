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

//! Strips all characters after an apostrophe, ported from
//! `org.apache.lucene.analysis.tr.ApostropheFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// The body of [`ApostropheFilter`]: what Lucene's `ApostropheFilter` adds
/// to `TokenFilter`.
#[derive(Debug)]
pub struct ApostropheFilterLogic;

impl TokenFilterLogic for ApostropheFilterLogic {
    /// Equivalent to `ApostropheFilter.incrementToken()`. The apostrophes are
    /// BMP characters, a single UTF-16 unit and a single `char` alike, and no
    /// surrogate equals them, so the term is cut at the same character as
    /// Java's `char[]` walk cuts it.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }

        let mut term_att = source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let length = term_att.length();

        for i in 0..length {
            let c = term_att.buffer()[i];
            if c == '\'' || c == '\u{2019}' {
                term_att.set_length(i);
                return Ok(true);
            }
        }
        Ok(true)
    }
}

/// Strips all characters after an apostrophe (including the apostrophe
/// itself).
///
/// In Turkish, apostrophe is used to separate suffixes from proper names
/// (continent, sea, river, lake, mountain, upland, proper names related to
/// religion and mythology). This filter intended to be used before stem
/// filters. For more information, see [Role of Apostrophes in Turkish
/// Information Retrieval](http://www.ipcsit.com/vol57/015-ICNI2012-M021.pdf)
///
/// Equivalent to `org.apache.lucene.analysis.tr.ApostropheFilter`.
pub type ApostropheFilter = TokenFilter<ApostropheFilterLogic>;

/// Equivalent to `ApostropheFilter(TokenStream)`, whose field adds the
/// `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_apostrophe_filter(input: Box<dyn TokenStream>) -> Result<ApostropheFilter> {
    let mut filter = TokenFilter::new(input, ApostropheFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
