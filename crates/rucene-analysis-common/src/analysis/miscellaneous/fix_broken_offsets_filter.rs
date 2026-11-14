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
//! Corrects offsets that illegally go backwards, ported from
//! `org.apache.lucene.analysis.miscellaneous.FixBrokenOffsetsFilter`.

use rucene::analysis::{OffsetAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// The body of [`FixBrokenOffsetsFilter`]: what Lucene's
/// `FixBrokenOffsetsFilter` adds to `TokenFilter`.
#[derive(Debug, Default)]
pub struct FixBrokenOffsetsFilterLogic {
    last_start_offset: i32,
}

impl FixBrokenOffsetsFilterLogic {
    /// Equivalent to the private `FixBrokenOffsetsFilter.fixOffsets()`.
    fn fix_offsets(&mut self, source: &AttributeSource) {
        let mut offset_att = source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute");
        let mut start_offset = offset_att.start_offset();
        let mut end_offset = offset_att.end_offset();
        if start_offset < self.last_start_offset {
            start_offset = self.last_start_offset;
        }
        if end_offset < start_offset {
            end_offset = start_offset;
        }
        offset_att.set_offset(start_offset, end_offset);
        self.last_start_offset = start_offset;
    }
}

impl TokenFilterLogic for FixBrokenOffsetsFilterLogic {
    /// Equivalent to `FixBrokenOffsetsFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }
        self.fix_offsets(source);
        Ok(true)
    }

    /// Equivalent to `FixBrokenOffsetsFilter.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.end()?;
        self.fix_offsets(source);
        Ok(())
    }

    /// Equivalent to `FixBrokenOffsetsFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.last_start_offset = 0;
        Ok(())
    }
}

/// A filter to correct offsets that illegally go backwards.
///
/// **Deprecated** in Lucene: fix the token filters that create broken
/// offsets in the first place.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.FixBrokenOffsetsFilter`.
pub type FixBrokenOffsetsFilter = TokenFilter<FixBrokenOffsetsFilterLogic>;

/// Equivalent to `FixBrokenOffsetsFilter(TokenStream)`, which adds the
/// `OffsetAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
#[deprecated(note = "Fix the token filters that create broken offsets in the first place.")]
pub fn new_fix_broken_offsets_filter(
    input: Box<dyn TokenStream>,
) -> Result<FixBrokenOffsetsFilter> {
    let mut filter = TokenFilter::new(input, FixBrokenOffsetsFilterLogic::default());
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn OffsetAttribute>()?;
    Ok(filter)
}
