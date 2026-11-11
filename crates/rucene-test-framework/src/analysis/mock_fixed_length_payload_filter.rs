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

//! Port of `org.apache.lucene.tests.analysis.MockFixedLengthPayloadFilter`.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

use super::attributes::{payload_id, Att};

/// The logic of [`MockFixedLengthPayloadFilter`].
pub struct MockFixedLengthPayloadFilterLogic {
    payload_att: Att,
    random: Arc<dyn Random>,
    bytes: Vec<u8>,
}

impl std::fmt::Debug for MockFixedLengthPayloadFilterLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockFixedLengthPayloadFilter")
            .field("length", &self.bytes.len())
            .finish()
    }
}

impl TokenFilterLogic for MockFixedLengthPayloadFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            self.random.next_bytes(&mut self.bytes);
            self.payload_att
                .payload_mut()
                .set_payload(Some(BytesRef::new(self.bytes.clone())));
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// TokenFilter that adds random fixed-length payloads.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockFixedLengthPayloadFilter`.
pub struct MockFixedLengthPayloadFilter;

impl MockFixedLengthPayloadFilter {
    /// `MockFixedLengthPayloadFilter(Random random, TokenStream in, int
    /// length)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (`length must be >= 0`) for a negative
    /// length, or when the stream's factory provides no `PayloadAttribute`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        random: Arc<dyn Random>,
        mut input: Box<dyn TokenStream>,
        length: i32,
    ) -> Result<TokenFilter<MockFixedLengthPayloadFilterLogic>> {
        let payload_att = Att::add(input.attribute_source_mut(), payload_id())?;
        if length < 0 {
            return Err(LuceneError::illegal_argument(
                "length must be >= 0".to_string(),
            ));
        }
        Ok(TokenFilter::new(
            input,
            MockFixedLengthPayloadFilterLogic {
                payload_att,
                random,
                bytes: vec![0u8; length as usize],
            },
        ))
    }
}
