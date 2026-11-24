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

//! Port of `OffsetLimitTokenFilter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use rucene::analysis::tokenattributes::OffsetAttribute;
use rucene::analysis::{AttributeHandle, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

/// This TokenFilter limits the number of tokens while indexing by adding up
/// the current offset.
///
/// Equivalent to `org.apache.lucene.search.highlight.OffsetLimitTokenFilter`.
/// The input is any token stream the filter may own or only borrow for its
/// own lifetime `'a` (Java holds every stream by reference).
#[derive(Debug)]
pub struct OffsetLimitTokenFilter<'a> {
    source: AttributeSource,
    input: Box<dyn TokenStream + 'a>,
    offset_count: i32,
    offset_attrib: Option<AttributeHandle<dyn OffsetAttribute>>,
    offset_limit: i32,
}

impl<'a> OffsetLimitTokenFilter<'a> {
    /// Equivalent to `new OffsetLimitTokenFilter(TokenStream, int)`.
    pub fn new(input: Box<dyn TokenStream + 'a>, offset_limit: i32) -> Self {
        let source = AttributeSource::new_from(input.attribute_source());
        let offset_attrib = AttributeHandle::get(&source);
        Self {
            source,
            input,
            offset_count: 0,
            offset_attrib,
            offset_limit,
        }
    }
}

impl TokenStream for OffsetLimitTokenFilter<'_> {
    /// # Errors
    ///
    /// As the input's `incrementToken()`; `NullPointerException` when the
    /// input has no `OffsetAttribute` (Java's `getAttribute` returned
    /// `null`).
    fn increment_token(&mut self) -> Result<bool> {
        if self.offset_count < self.offset_limit && self.input.increment_token()? {
            let att = self
                .offset_attrib
                .as_ref()
                .ok_or_else(LuceneError::null_pointer)?
                .borrow();
            let offset_length = att.end_offset() - att.start_offset();
            self.offset_count = self.offset_count.wrapping_add(offset_length);
            return Ok(true);
        }
        Ok(false)
    }

    fn end(&mut self) -> Result<()> {
        self.input.end()
    }

    fn reset(&mut self) -> Result<()> {
        self.input.reset()?;
        self.offset_count = 0;
        Ok(())
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
