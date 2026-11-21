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

//! Port of `org.apache.lucene.tests.analysis.SimplePayloadFilter`.

use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

use super::attributes::{char_term_id, payload_id, Att};

/// The logic of [`SimplePayloadFilter`].
#[derive(Debug)]
pub struct SimplePayloadFilterLogic {
    pos: i32,
    payload_attr: Att,
    #[allow(dead_code)]
    term_attr: Att,
}

impl TokenFilterLogic for SimplePayloadFilterLogic {
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
            self.pos += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.pos = 0;
        Ok(())
    }
}

/// Simple payload filter that sets the payload as pos: XXXX.
///
/// Equivalent to `org.apache.lucene.tests.analysis.SimplePayloadFilter`.
pub struct SimplePayloadFilter;

impl SimplePayloadFilter {
    /// `SimplePayloadFilter(TokenStream input)`: adds the attributes to the
    /// input, as Java's `input.addAttribute` does.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory provides no
    /// `PayloadAttribute` or `CharTermAttribute`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(mut input: Box<dyn TokenStream>) -> Result<TokenFilter<SimplePayloadFilterLogic>> {
        let source = input.attribute_source_mut();
        let payload_attr = Att::add(source, payload_id())?;
        let term_attr = Att::add(source, char_term_id())?;
        Ok(TokenFilter::new(
            input,
            SimplePayloadFilterLogic {
                pos: 0,
                payload_attr,
                term_attr,
            },
        ))
    }
}
