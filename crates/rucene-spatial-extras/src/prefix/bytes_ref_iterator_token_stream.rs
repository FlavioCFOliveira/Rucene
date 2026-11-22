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

//! The types declared in `BytesRefIteratorTokenStream.java`, of the Java
//! package `org.apache.lucene.spatial.prefix`.

use std::any::Any;
use std::fmt;

use rucene::analysis::{default_token_attribute_factory, BytesTermAttributeImpl, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRefIterator;
use rucene::analysis::tokenattributes::BytesTermAttribute;

/// The iterator a [`BytesRefIteratorTokenStream`] reads; a token stream is
/// shared between threads, so the iterator must be too.
pub type SharedBytesRefIterator = Box<dyn BytesRefIterator + Send + Sync>;

/// Equivalent to `org.apache.lucene.spatial.prefix.BytesRefIteratorTokenStream`
/// (`public class BytesRefIteratorTokenStream extends TokenStream`): a
/// TokenStream used internally by
/// [`PrefixTreeStrategy`](crate::prefix::prefix_tree_strategy::PrefixTreeStrategy).
pub struct BytesRefIteratorTokenStream {
    source: AttributeSource,
    // members
    bytes_iter: Option<SharedBytesRefIterator>, // null means not initialized
}

impl fmt::Debug for BytesRefIteratorTokenStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BytesRefIteratorTokenStream")
            .field("initialized", &self.bytes_iter.is_some())
            .finish()
    }
}

impl Default for BytesRefIteratorTokenStream {
    fn default() -> Self {
        Self::new()
    }
}

impl BytesRefIteratorTokenStream {
    /// `BytesRefIteratorTokenStream()`: adds the `BytesTermAttribute`.
    pub fn new() -> Self {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        source
            .add_attribute::<BytesTermAttributeImpl>()
            .expect("the default factory makes a BytesTermAttribute");
        Self { source, bytes_iter: None }
    }

    /// `getBytesRefIterator()`.
    pub fn get_bytes_ref_iterator(&mut self) -> Option<&mut SharedBytesRefIterator> {
        self.bytes_iter.as_mut()
    }

    /// `setBytesRefIterator(BytesRefIterator iter)`.
    pub fn set_bytes_ref_iterator(&mut self, iter: SharedBytesRefIterator) -> &mut Self {
        self.bytes_iter = Some(iter);
        self
    }

    fn not_initialized() -> LuceneError {
        LuceneError::illegal_state("call setBytesRefIterator() before usage")
    }
}

impl TokenStream for BytesRefIteratorTokenStream {
    fn reset(&mut self) -> Result<()> {
        if self.bytes_iter.is_none() {
            return Err(Self::not_initialized());
        }
        Ok(())
    }

    fn increment_token(&mut self) -> Result<bool> {
        let Some(bytes_iter) = self.bytes_iter.as_mut() else {
            return Err(Self::not_initialized());
        };
        // get next
        match bytes_iter.next()? {
            None => Ok(false),
            Some(bytes) => {
                self.source.clear_attributes();
                let mut att = self
                    .source
                    .get_attribute_mut::<BytesTermAttributeImpl>()
                    .expect("added by the constructor");
                att.set_bytes_ref(bytes);
                // note: we don't bother setting posInc or type attributes.  There's no point to it.
                Ok(true)
            }
        }
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
