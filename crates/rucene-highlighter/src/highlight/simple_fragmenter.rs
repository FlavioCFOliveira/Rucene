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

//! Port of `SimpleFragmenter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use rucene::analysis::tokenattributes::OffsetAttribute;
use rucene::analysis::{AttributeHandle, TokenStream};
use rucene::error::Result;

use crate::highlight::fragmenter::Fragmenter;

const DEFAULT_FRAGMENT_SIZE: i32 = 100;

/// `Fragmenter` implementation which breaks text up into same-size
/// fragments with no concerns over spotting sentence boundaries.
///
/// Equivalent to `org.apache.lucene.search.highlight.SimpleFragmenter`.
#[derive(Debug)]
pub struct SimpleFragmenter {
    current_num_frags: i32,
    fragment_size: i32,
    offset_att: Option<AttributeHandle<dyn OffsetAttribute>>,
}

impl Default for SimpleFragmenter {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleFragmenter {
    /// Equivalent to `new SimpleFragmenter()`.
    pub fn new() -> Self {
        Self::with_fragment_size(DEFAULT_FRAGMENT_SIZE)
    }

    /// Equivalent to `new SimpleFragmenter(int)`: `fragment_size` is the
    /// size in number of characters of each fragment.
    pub fn with_fragment_size(fragment_size: i32) -> Self {
        Self {
            current_num_frags: 0,
            fragment_size,
            offset_att: None,
        }
    }

    /// Equivalent to `getFragmentSize()`.
    pub fn get_fragment_size(&self) -> i32 {
        self.fragment_size
    }

    /// Equivalent to `setFragmentSize(int)`.
    pub fn set_fragment_size(&mut self, size: i32) {
        self.fragment_size = size;
    }
}

impl Fragmenter for SimpleFragmenter {
    fn start(&mut self, _original_text: &str, stream: &mut dyn TokenStream) -> Result<()> {
        self.offset_att = Some(AttributeHandle::add(stream.attribute_source_mut())?);
        self.current_num_frags = 1;
        Ok(())
    }

    /// # Panics
    ///
    /// Before `start`, where Java throws `NullPointerException` on the unset
    /// attribute.
    fn is_new_fragment(&mut self) -> bool {
        let end_offset = self
            .offset_att
            .as_ref()
            .expect("NullPointerException: start(String, TokenStream) was not called")
            .borrow()
            .end_offset();
        let is_new_frag = end_offset >= self.fragment_size.wrapping_mul(self.current_num_frags);
        if is_new_frag {
            self.current_num_frags += 1;
        }
        is_new_frag
    }
}
