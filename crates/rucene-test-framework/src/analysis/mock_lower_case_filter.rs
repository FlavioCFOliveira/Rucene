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

//! Port of `org.apache.lucene.tests.analysis.MockLowerCaseFilter`.

use rucene::analysis::{to_lower_case, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::attributes::{char_term_id, Att};

/// The logic of [`MockLowerCaseFilter`].
#[derive(Debug)]
pub struct MockLowerCaseFilterLogic {
    term_att: Att,
}

impl TokenFilterLogic for MockLowerCaseFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut term = self.term_att.char_term_mut();
            let length = term.length();
            to_lower_case(term.buffer_mut(), 0, length);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// A lowercasing `TokenFilter`.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockLowerCaseFilter`.
pub struct MockLowerCaseFilter;

impl MockLowerCaseFilter {
    /// Sole constructor.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory provides no
    /// `CharTermAttribute`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(mut input: Box<dyn TokenStream>) -> Result<TokenFilter<MockLowerCaseFilterLogic>> {
        let term_att = Att::add(input.attribute_source_mut(), char_term_id())?;
        Ok(TokenFilter::new(
            input,
            MockLowerCaseFilterLogic { term_att },
        ))
    }
}
