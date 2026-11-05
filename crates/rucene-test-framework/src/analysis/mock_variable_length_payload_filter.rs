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

//! Port of `org.apache.lucene.tests.analysis.MockVariableLengthPayloadFilter`.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

use super::attributes::{payload_id, Att};

const MAXLENGTH: i32 = 129;

/// The logic of [`MockVariableLengthPayloadFilter`].
pub struct MockVariableLengthPayloadFilterLogic {
    payload_att: Att,
    random: Arc<dyn Random>,
    bytes: Vec<u8>,
}

impl std::fmt::Debug for MockVariableLengthPayloadFilterLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockVariableLengthPayloadFilter")
            .finish_non_exhaustive()
    }
}

impl TokenFilterLogic for MockVariableLengthPayloadFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            self.random.next_bytes(&mut self.bytes);
            let mut payload = BytesRef::new(self.bytes.clone());
            payload.length = self.random.next_int_bound(MAXLENGTH) as usize;
            self.payload_att.payload_mut().set_payload(Some(payload));
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// TokenFilter that adds random variable-length payloads.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockVariableLengthPayloadFilter`.
pub struct MockVariableLengthPayloadFilter;

impl MockVariableLengthPayloadFilter {
    /// `MockVariableLengthPayloadFilter(Random random, TokenStream in)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory provides no
    /// `PayloadAttribute`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        random: Arc<dyn Random>,
        mut input: Box<dyn TokenStream>,
    ) -> Result<TokenFilter<MockVariableLengthPayloadFilterLogic>> {
        let payload_att = Att::add(input.attribute_source_mut(), payload_id())?;
        Ok(TokenFilter::new(
            input,
            MockVariableLengthPayloadFilterLogic {
                payload_att,
                random,
                bytes: vec![0u8; MAXLENGTH as usize],
            },
        ))
    }
}
