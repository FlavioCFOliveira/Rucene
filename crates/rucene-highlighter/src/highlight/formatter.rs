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

//! Port of `Formatter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::fmt::Debug;

use crate::highlight::token_group::TokenGroup;

/// Processes terms found in the original text, typically by applying some
/// form of mark-up to highlight terms in HTML search results pages.
///
/// Equivalent to `org.apache.lucene.search.highlight.Formatter`.
pub trait Formatter: Debug + Send + Sync {
    /// Equivalent to `highlightTerm(String, TokenGroup)`.
    fn highlight_term(&self, original_text: &str, token_group: &TokenGroup) -> String;
}