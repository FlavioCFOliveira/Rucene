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

//! Port of `SimpleHTMLFormatter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use crate::highlight::formatter::Formatter;
use crate::highlight::token_group::TokenGroup;

const DEFAULT_PRE_TAG: &str = "<B>";
const DEFAULT_POST_TAG: &str = "</B>";

/// Simple `Formatter` to highlight terms with a pre and post tag.
///
/// Equivalent to `org.apache.lucene.search.highlight.SimpleHTMLFormatter`.
#[derive(Debug, Clone)]
pub struct SimpleHTMLFormatter {
    pre_tag: String,
    post_tag: String,
}

impl SimpleHTMLFormatter {
    /// Equivalent to `new SimpleHTMLFormatter(String, String)`.
    pub fn with_tags(pre_tag: &str, post_tag: &str) -> Self {
        Self {
            pre_tag: pre_tag.to_string(),
            post_tag: post_tag.to_string(),
        }
    }

    /// Default constructor uses HTML: `<B>` tags to markup terms.
    ///
    /// Equivalent to `new SimpleHTMLFormatter()`.
    pub fn new() -> Self {
        Self::with_tags(DEFAULT_PRE_TAG, DEFAULT_POST_TAG)
    }
}

impl Default for SimpleHTMLFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl Formatter for SimpleHTMLFormatter {
    fn highlight_term(&self, original_text: &str, token_group: &TokenGroup) -> String {
        if token_group.get_total_score() <= 0.0 {
            return original_text.to_string();
        }

        // Allocate StringBuilder with the right number of characters from the
        // beginning, to avoid char[] allocations in the middle of appends.
        let mut return_buffer =
            String::with_capacity(self.pre_tag.len() + original_text.len() + self.post_tag.len());
        return_buffer.push_str(&self.pre_tag);
        return_buffer.push_str(original_text);
        return_buffer.push_str(&self.post_tag);
        return_buffer
    }
}