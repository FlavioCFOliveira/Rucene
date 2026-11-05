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

//! Port of `org.apache.lucene.tests.analysis.MockPayloadAnalyzer`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

use super::attributes::{char_term_id, payload_id, pos_inc_id, Att};
use super::mock_tokenizer::{MockTokenizer, WHITESPACE};
use super::{components, shared, sink};

/// Wraps a whitespace tokenizer with a filter that sets the first token, and
/// odd tokens to posinc=1, and all others to 0, encoding the position as
/// pos: XXX in the payload.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockPayloadAnalyzer`.
#[derive(Debug)]
pub struct MockPayloadAnalyzer {
    state: AnalyzerState,
}

impl MockPayloadAnalyzer {
    /// `new MockPayloadAnalyzer()`.
    pub fn new() -> MockPayloadAnalyzer {
        MockPayloadAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Default for MockPayloadAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for MockPayloadAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.analysis.MockPayloadAnalyzer"
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        let result = Rc::new(RefCell::new(MockTokenizer::new(
            Arc::clone(&WHITESPACE),
            true,
        )?));
        let filter = MockPayloadFilter::new(shared(&result), field_name)?;
        Ok(components(result, sink(filter)))
    }
}

/// The logic of the package-private `MockPayloadFilter`.
#[derive(Debug)]
pub struct MockPayloadFilterLogic {
    #[allow(dead_code)]
    field_name: String,
    pos: i32,
    i: i32,
    pos_incr_attr: Att,
    payload_attr: Att,
    #[allow(dead_code)]
    term_attr: Att,
}

impl TokenFilterLogic for MockPayloadFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            self.payload_attr
                .payload_mut()
                .set_payload(Some(BytesRef::new(
                    format!("pos: {}", self.pos).into_bytes(),
                )));
            let pos_incr = if self.pos == 0 || self.i % 2 == 1 {
                1
            } else {
                0
            };
            self.pos_incr_attr
                .pos_inc_mut()
                .set_position_increment(pos_incr);
            self.pos += pos_incr;
            self.i += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.i = 0;
        self.pos = 0;
        Ok(())
    }
}

/// The package-private `MockPayloadFilter` of `MockPayloadAnalyzer.java`.
pub struct MockPayloadFilter;

impl MockPayloadFilter {
    /// `MockPayloadFilter(TokenStream input, String fieldName)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory lacks one of the
    /// attributes the filter uses.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        mut input: Box<dyn TokenStream>,
        field_name: &str,
    ) -> Result<TokenFilter<MockPayloadFilterLogic>> {
        let source = input.attribute_source_mut();
        let pos_incr_attr = Att::add(source, pos_inc_id())?;
        let payload_attr = Att::add(source, payload_id())?;
        let term_attr = Att::add(source, char_term_id())?;
        Ok(TokenFilter::new(
            input,
            MockPayloadFilterLogic {
                field_name: field_name.to_string(),
                pos: 0,
                i: 0,
                pos_incr_attr,
                payload_attr,
                term_attr,
            },
        ))
    }
}
