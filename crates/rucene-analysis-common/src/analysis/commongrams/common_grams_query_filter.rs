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

//! Keeps only the bigrams and the unigrams outside bigrams of a
//! `CommonGramsFilter`, ported from
//! `org.apache.lucene.analysis.commongrams.CommonGramsQueryFilter`.

use rucene::analysis::{
    PositionIncrementAttribute, PositionLengthAttribute, TokenFilter, TokenFilterLogic,
    TokenStream, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};

use super::common_grams_filter::{CommonGramsFilter, GRAM_TYPE};

/// The body of [`CommonGramsQueryFilter`]: what Lucene's
/// `CommonGramsQueryFilter` adds to `TokenFilter`.
#[derive(Debug, Default)]
pub struct CommonGramsQueryFilterLogic {
    previous: Option<CapturedState>,
    previous_type: Option<String>,
    exhausted: bool,
}

/// Convenience method to check if the current type is a gram type
///
/// Equivalent to `CommonGramsQueryFilter.isGramType()`.
pub fn is_gram_type(source: &AttributeSource) -> bool {
    source
        .get_attribute_interface::<dyn TypeAttribute>()
        .expect("INVARIANT: the constructor added the TypeAttribute")
        .type_value()
        == GRAM_TYPE
}

/// Sets the position increment and length of a gram emitted as a unigram.
fn reconnect(source: &AttributeSource) {
    source
        .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
        .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
        .set_position_increment(1);
    // We must set this back to 1 (from e.g. 2 or higher) otherwise the token graph is
    // disconnected:
    source
        .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
        .expect("INVARIANT: the constructor added the PositionLengthAttribute")
        .set_position_length(1);
}

impl TokenFilterLogic for CommonGramsQueryFilterLogic {
    /// Output bigrams whenever possible to optimize queries. Only output
    /// unigrams when they are not a member of a bigram. Example:
    ///
    /// * input: "the rain in spain falls mainly"
    /// * output:"the-rain", "rain-in" ,"in-spain", "falls", "mainly"
    ///
    /// Equivalent to `CommonGramsQueryFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        while !self.exhausted && input.increment_token()? {
            let current = source.capture_state();
            if self.previous.is_some() && !is_gram_type(source) {
                if let Some(previous) = &self.previous {
                    source.restore_state(previous)?;
                }
                self.previous = current;
                self.previous_type = Some(
                    source
                        .get_attribute_interface::<dyn TypeAttribute>()
                        .expect("INVARIANT: the constructor added the TypeAttribute")
                        .type_value()
                        .to_string(),
                );

                if is_gram_type(source) {
                    reconnect(source);
                }
                return Ok(true);
            }

            self.previous = current;
        }

        self.exhausted = true;

        if self.previous.is_none() || self.previous_type.as_deref() == Some(GRAM_TYPE) {
            return Ok(false);
        }

        if let Some(previous) = self.previous.take() {
            source.restore_state(&previous)?;
        }

        if is_gram_type(source) {
            reconnect(source);
        }
        Ok(true)
    }

    /// Equivalent to `CommonGramsQueryFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.previous = None;
        self.previous_type = None;
        self.exhausted = false;
        Ok(())
    }
}

/// Wrap a CommonGramsFilter optimizing phrase queries by only returning
/// single words when they are not a member of a bigram.
///
/// Example:
///
/// * query input to CommonGramsFilter: "the rain in spain falls mainly"
/// * output of CommomGramsFilter/input to CommonGramsQueryFilter: |"the,
///   "the-rain"|"rain" "rain-in"|"in, "in-spain"|"spain"|"falls"|"mainly"
/// * output of CommonGramsQueryFilter:"the-rain", "rain-in" ,"in-spain",
///   "falls", "mainly"
///
/// Equivalent to `org.apache.lucene.analysis.commongrams.CommonGramsQueryFilter`.
pub type CommonGramsQueryFilter = TokenFilter<CommonGramsQueryFilterLogic>;

/// Constructs a new CommonGramsQueryFilter based on the provided
/// CommomGramsFilter
///
/// * `input` CommonGramsFilter the QueryFilter will use
///
/// Equivalent to `CommonGramsQueryFilter(CommonGramsFilter)`, which adds the
/// `TypeAttribute`, `PositionIncrementAttribute` and
/// `PositionLengthAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_common_grams_query_filter(input: CommonGramsFilter) -> Result<CommonGramsQueryFilter> {
    let mut filter = TokenFilter::new(Box::new(input), CommonGramsQueryFilterLogic::default());
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    Ok(filter)
}
