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

//! Port of `LimitTokenOffsetFilter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use rucene::analysis::tokenattributes::OffsetAttribute;
use rucene::analysis::{AttributeHandle, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

/// This TokenFilter limits the number of tokens while indexing by adding up
/// the current offset.
///
/// Equivalent to the package-private
/// `org.apache.lucene.search.highlight.LimitTokenOffsetFilter`.
#[derive(Debug)]
pub(crate) struct LimitTokenOffsetFilter {
    source: AttributeSource,
    input: Box<dyn TokenStream>,
    offset_attrib: AttributeHandle<dyn OffsetAttribute>,
    max_start_offset: i32,
}

impl LimitTokenOffsetFilter {
    /// Equivalent to `new LimitTokenOffsetFilter(TokenStream, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `max_start_offset` is negative.
    pub(crate) fn new(input: Box<dyn TokenStream>, max_start_offset: i32) -> Result<Self> {
        let mut source = AttributeSource::new_from(input.attribute_source());
        let offset_attrib = AttributeHandle::add(&mut source)?;
        if max_start_offset < 0 {
            return Err(LuceneError::illegal_argument(
                "maxStartOffset must be >= zero",
            ));
        }
        Ok(Self {
            source,
            input,
            offset_attrib,
            max_start_offset,
        })
    }
}

impl TokenStream for LimitTokenOffsetFilter {
    fn increment_token(&mut self) -> Result<bool> {
        if !self.input.increment_token()? {
            return Ok(false);
        }
        if self.offset_attrib.borrow().start_offset() <= self.max_start_offset {
            return Ok(true);
        }
        Ok(false)
    }

    fn end(&mut self) -> Result<()> {
        self.input.end()
    }

    fn reset(&mut self) -> Result<()> {
        self.input.reset()
    }

    fn close(&mut self) -> Result<()> {
        self.input.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}