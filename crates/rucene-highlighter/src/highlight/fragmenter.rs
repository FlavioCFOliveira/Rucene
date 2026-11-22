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

//! Port of `Fragmenter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::fmt::Debug;

use rucene::analysis::TokenStream;
use rucene::error::Result;

/// Implements the policy for breaking text into multiple fragments for
/// consideration by the `Highlighter` class.
///
/// Equivalent to `org.apache.lucene.search.highlight.Fragmenter`.
pub trait Fragmenter: Debug + Send {
    /// Initializes the Fragmenter: the attributes of interest are added to
    /// `token_stream`, so that `is_new_fragment` reads the current token.
    ///
    /// Equivalent to `start(String, TokenStream)`.
    ///
    /// # Errors
    ///
    /// What `addAttribute` raises for an attribute the stream's factory
    /// cannot create.
    fn start(&mut self, original_text: &str, token_stream: &mut dyn TokenStream) -> Result<()>;

    /// Tests if the current token starts a new fragment.
    ///
    /// Equivalent to `isNewFragment()`.
    fn is_new_fragment(&mut self) -> bool;
}