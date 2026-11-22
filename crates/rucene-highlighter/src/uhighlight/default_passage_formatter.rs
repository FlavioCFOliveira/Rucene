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

//! Port of `DefaultPassageFormatter.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use rucene::error::Result;

use crate::uhighlight::passage::Passage;
use crate::uhighlight::passage_formatter::{FormattedPassages, PassageFormatter};

/// Creates a formatted snippet from the top passages.
///
/// The default implementation marks the query terms as bold, and places
/// ellipses between unconnected passages.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.DefaultPassageFormatter`.
#[derive(Debug, Clone)]
pub struct DefaultPassageFormatter {
    /// text that will appear before highlighted terms
    pub pre_tag: String,
    /// text that will appear after highlighted terms
    pub post_tag: String,
    /// text that will appear between two unconnected passages
    pub ellipsis: String,
    /// true if we should escape for html
    pub escape: bool,
}

impl Default for DefaultPassageFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultPassageFormatter {
    /// Creates a new DefaultPassageFormatter with the default tags.
    ///
    /// Equivalent to `new DefaultPassageFormatter()`.
    pub fn new() -> Self {
        Self::with_tags("<b>", "</b>", "... ", false)
    }

    /// Creates a new DefaultPassageFormatter with custom tags.
    ///
    /// Equivalent to `new DefaultPassageFormatter(String, String, String,
    /// boolean)`; Java's `null` checks have no counterpart.
    pub fn with_tags(pre_tag: &str, post_tag: &str, ellipsis: &str, escape: bool) -> Self {
        Self {
            pre_tag: pre_tag.to_string(),
            post_tag: post_tag.to_string(),
            ellipsis: ellipsis.to_string(),
            escape,
        }
    }

    /// Equivalent to `format(Passage[], String)`, returning the `String`.
    pub fn format_string(&self, passages: &[Passage], content: &str) -> String {
        let content: Vec<u16> = content.encode_utf16().collect();
        let mut sb: Vec<u16> = Vec::new();
        let mut pos = 0;
        for passage in passages {
            // don't add ellipsis if its the first one, or if its connected.
            if !sb.is_empty() && passage.get_start_offset() != pos {
                sb.extend(self.ellipsis.encode_utf16());
            }
            pos = passage.get_start_offset();
            let mut i = 0;
            while i < passage.get_num_matches() as usize {
                let start = passage.get_match_starts()[i];
                debug_assert!(start >= pos && start < passage.get_end_offset());
                // append content before this start
                self.append(&mut sb, &content, pos, start);

                let mut end = passage.get_match_ends()[i];
                debug_assert!(end > start);
                // Look ahead to expand 'end' past all overlapping:
                while i + 1 < passage.get_num_matches() as usize
                    && passage.get_match_starts()[i + 1] < end
                {
                    i += 1;
                    end = end.max(passage.get_match_ends()[i]);
                }
                end = end.min(passage.get_end_offset()); // in case match straddles past passage

                sb.extend(self.pre_tag.encode_utf16());
                self.append(&mut sb, &content, start, end);
                sb.extend(self.post_tag.encode_utf16());

                pos = end;
                i += 1;
            }
            // its possible a "term" from the analyzer could span a sentence
            // boundary.
            self.append(&mut sb, &content, pos, pos.max(passage.get_end_offset()));
            pos = passage.get_end_offset();
        }
        String::from_utf16_lossy(&sb)
    }

    /// Appends original text to the response.
    ///
    /// Equivalent to the protected `append(StringBuilder, String, int,
    /// int)`: `dest` and `content` are UTF-16.
    ///
    /// # Panics
    ///
    /// When the range is outside `content`, where Java throws
    /// `StringIndexOutOfBoundsException`.
    pub fn append(&self, dest: &mut Vec<u16>, content: &[u16], start: i32, end: i32) {
        if self.escape {
            // note: these are the rules from owasp.org
            for &ch in &content[start as usize..end as usize] {
                match ch {
                    0x26 => dest.extend("&amp;".encode_utf16()),
                    0x3C => dest.extend("&lt;".encode_utf16()),
                    0x3E => dest.extend("&gt;".encode_utf16()),
                    0x22 => dest.extend("&quot;".encode_utf16()),
                    0x27 => dest.extend("&#x27;".encode_utf16()),
                    0x2F => dest.extend("&#x2F;".encode_utf16()),
                    _ => dest.push(ch),
                }
            }
        } else {
            dest.extend_from_slice(&content[start as usize..end as usize]);
        }
    }
}

impl PassageFormatter for DefaultPassageFormatter {
    fn format(&self, passages: &[Passage], content: &str) -> Result<Box<dyn FormattedPassages>> {
        Ok(Box::new(self.format_string(passages, content)))
    }
}