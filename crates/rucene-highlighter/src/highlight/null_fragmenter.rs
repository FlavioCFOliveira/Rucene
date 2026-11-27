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

//! Port of `NullFragmenter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use rucene::analysis::TokenStream;
use rucene::error::Result;

use crate::highlight::fragmenter::Fragmenter;

/// `Fragmenter` implementation which does not fragment the text. This is
/// useful for highlighting the entire content of a document or field.
///
/// Equivalent to `org.apache.lucene.search.highlight.NullFragmenter`.
#[derive(Debug, Default, Clone)]
pub struct NullFragmenter;

impl NullFragmenter {
    /// Equivalent to `new NullFragmenter()`.
    pub fn new() -> Self {
        Self
    }
}

impl Fragmenter for NullFragmenter {
    fn start(&mut self, _s: &str, _token_stream: &mut dyn TokenStream) -> Result<()> {
        Ok(())
    }

    fn is_new_fragment(&mut self) -> bool {
        false
    }
}
