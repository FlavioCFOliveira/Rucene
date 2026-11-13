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
//! Drops tokens carrying a given combination of flags, ported from
//! `org.apache.lucene.analysis.miscellaneous.DropIfFlaggedFilter`.

use rucene::analysis::{
    new_filtering_token_filter, FilteringTokenFilter, FilteringTokenFilterLogic, FlagsAttribute,
    TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// The `accept` of [`DropIfFlaggedFilter`]: what Lucene's
/// `DropIfFlaggedFilter` adds to `FilteringTokenFilter`.
#[derive(Debug)]
pub struct DropIfFlaggedFilterLogic {
    drop_flags: i32,
}

impl FilteringTokenFilterLogic for DropIfFlaggedFilterLogic {
    /// Equivalent to `DropIfFlaggedFilter.accept()`.
    fn accept(&mut self, source: &AttributeSource) -> Result<bool> {
        let flags = source
            .get_attribute_interface::<dyn FlagsAttribute>()
            .expect("INVARIANT: the constructor added the FlagsAttribute")
            .get_flags();
        Ok((flags & self.drop_flags) != self.drop_flags)
    }
}

/// Allows Tokens with a given combination of flags to be dropped. If all
/// flags specified are present the token is dropped, otherwise it is
/// retained.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.DropIfFlaggedFilter`
/// (since 8.8.0).
pub type DropIfFlaggedFilter = FilteringTokenFilter<DropIfFlaggedFilterLogic>;

/// Construct a token stream filtering the given input: `drop_flags` is a
/// combination of flags that indicates that the token should be dropped.
///
/// Equivalent to `DropIfFlaggedFilter(TokenStream, int)`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_drop_if_flagged_filter(
    input: Box<dyn TokenStream>,
    drop_flags: i32,
) -> Result<DropIfFlaggedFilter> {
    let mut filter = new_filtering_token_filter(input, DropIfFlaggedFilterLogic { drop_flags })?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn FlagsAttribute>()?;
    Ok(filter)
}
