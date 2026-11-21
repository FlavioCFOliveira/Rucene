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
//! An always exhausted token stream, ported from
//! `org.apache.lucene.analysis.miscellaneous.EmptyTokenStream`.

use rucene::analysis::TokenStream;
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// An always exhausted token stream.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.EmptyTokenStream`,
/// whose implicit constructor is `TokenStream()`: an attribute source over
/// the default token attribute factory.
#[derive(Debug)]
pub struct EmptyTokenStream {
    source: AttributeSource,
}

impl EmptyTokenStream {
    /// Equivalent to the implicit `EmptyTokenStream()`.
    pub fn new() -> Self {
        Self {
            source: AttributeSource::new_with_factory(
                rucene::analysis::default_token_attribute_factory(),
            ),
        }
    }
}

impl Default for EmptyTokenStream {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStream for EmptyTokenStream {
    /// Equivalent to `EmptyTokenStream.incrementToken()`: always `false`.
    fn increment_token(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
