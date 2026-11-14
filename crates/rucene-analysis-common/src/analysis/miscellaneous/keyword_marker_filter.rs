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
//! Marks terms as keywords via the `KeywordAttribute`, ported from
//! `org.apache.lucene.analysis.miscellaneous.KeywordMarkerFilter`.

use std::fmt::Debug;

use rucene::analysis::{KeywordAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// The abstract `isKeyword()` of [`KeywordMarkerFilter`], which a subclass
/// supplies.
pub trait KeywordMarkerFilterLogic: Debug + Send + Sync {
    /// Returns `true` iff the current token is a keyword, otherwise `false`.
    ///
    /// Equivalent to the abstract `KeywordMarkerFilter.isKeyword()`.
    ///
    /// # Errors
    ///
    /// What the subclass's check raises.
    fn is_keyword(&mut self, source: &AttributeSource) -> Result<bool>;
}

/// The body of [`KeywordMarkerFilter`]: what Lucene's `KeywordMarkerFilter`
/// adds to `TokenFilter`, around the subclass's
/// [`is_keyword`](KeywordMarkerFilterLogic::is_keyword).
#[derive(Debug)]
pub struct KeywordMarkerFilterAdapter<T: KeywordMarkerFilterLogic> {
    logic: T,
}

impl<T: KeywordMarkerFilterLogic> KeywordMarkerFilterAdapter<T> {
    /// The subclass's logic.
    pub fn logic(&self) -> &T {
        &self.logic
    }
}

impl<T: KeywordMarkerFilterLogic> TokenFilterLogic for KeywordMarkerFilterAdapter<T> {
    /// Equivalent to the final `KeywordMarkerFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            if self.logic.is_keyword(source)? {
                source
                    .get_attribute_interface_mut::<dyn KeywordAttribute>()
                    .expect("INVARIANT: the constructor added the KeywordAttribute")
                    .set_keyword(true);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Marks terms as keywords via the `KeywordAttribute`.
///
/// Equivalent to the abstract `org.apache.lucene.analysis.miscellaneous.KeywordMarkerFilter`:
/// the
/// subclass is `T`.
pub type KeywordMarkerFilter<T> = TokenFilter<KeywordMarkerFilterAdapter<T>>;

/// Creates a new [`KeywordMarkerFilter`] over `input`, whose `isKeyword()` is
/// `logic`.
///
/// Equivalent to the protected `KeywordMarkerFilter(TokenStream)`, which adds
/// the `KeywordAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_keyword_marker_filter<T: KeywordMarkerFilterLogic>(
    input: Box<dyn TokenStream>,
    logic: T,
) -> Result<KeywordMarkerFilter<T>> {
    let mut filter = TokenFilter::new(input, KeywordMarkerFilterAdapter { logic });
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn KeywordAttribute>()?;
    Ok(filter)
}
