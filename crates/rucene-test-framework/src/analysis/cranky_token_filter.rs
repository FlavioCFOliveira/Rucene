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

//! Port of `org.apache.lucene.tests.analysis.CrankyTokenFilter`.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

fn fake(message: &str) -> LuceneError {
    LuceneError::Io(std::io::Error::other(message.to_string()))
}

/// The logic of [`CrankyTokenFilter`].
pub struct CrankyTokenFilterLogic {
    random: Arc<dyn Random>,
    thing_to_do: i32,
}

impl std::fmt::Debug for CrankyTokenFilterLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrankyTokenFilter")
            .field("thing_to_do", &self.thing_to_do)
            .finish()
    }
}

impl TokenFilterLogic for CrankyTokenFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if self.thing_to_do == 0 && self.random.next_boolean() {
            return Err(fake("Fake IOException from TokenStream.incrementToken()"));
        }
        input.increment_token()
    }

    fn end(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.end()?;
        if self.thing_to_do == 1 && self.random.next_boolean() {
            return Err(fake("Fake IOException from TokenStream.end()"));
        }
        Ok(())
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.thing_to_do = self.random.next_int_bound(100);
        if self.thing_to_do == 2 && self.random.next_boolean() {
            return Err(fake("Fake IOException from TokenStream.reset()"));
        }
        Ok(())
    }

    fn close(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.close()?;
        if self.thing_to_do == 3 && self.random.next_boolean() {
            return Err(fake("Fake IOException from TokenStream.close()"));
        }
        Ok(())
    }
}

/// Throws IOException from random TokenStream methods.
///
/// This can be used to simulate a buggy analyzer in IndexWriter, where we
/// must delete the document but not abort everything in the buffer.
///
/// Equivalent to `org.apache.lucene.tests.analysis.CrankyTokenFilter`.
pub struct CrankyTokenFilter;

impl CrankyTokenFilter {
    /// Creates a new CrankyTokenFilter.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        input: Box<dyn TokenStream>,
        random: Arc<dyn Random>,
    ) -> TokenFilter<CrankyTokenFilterLogic> {
        TokenFilter::new(
            input,
            CrankyTokenFilterLogic {
                random,
                thing_to_do: 0,
            },
        )
    }
}
