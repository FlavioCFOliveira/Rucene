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

//! Port of `SimpleHTMLEncoder.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use crate::highlight::encoder::Encoder;

/// Simple `Encoder` implementation to escape text for HTML output.
///
/// Equivalent to `org.apache.lucene.search.highlight.SimpleHTMLEncoder`.
#[derive(Debug, Default, Clone)]
pub struct SimpleHTMLEncoder;

impl SimpleHTMLEncoder {
    /// Equivalent to `new SimpleHTMLEncoder()`.
    pub fn new() -> Self {
        Self
    }

    /// Encode string into HTML; `None` (Java's `null`) and the empty string
    /// encode to the empty string.
    ///
    /// Equivalent to the static `htmlEncode(String)`.
    pub fn html_encode(plain_text: Option<&str>) -> String {
        let Some(plain_text) = plain_text else {
            return String::new();
        };
        if plain_text.is_empty() {
            return String::new();
        }

        let mut result = String::with_capacity(plain_text.len());

        for ch in plain_text.chars() {
            match ch {
                '"' => result.push_str("&quot;"),
                '&' => result.push_str("&amp;"),
                '<' => result.push_str("&lt;"),
                '>' => result.push_str("&gt;"),
                '\'' => result.push_str("&#x27;"),
                '/' => result.push_str("&#x2F;"),
                _ => result.push(ch),
            }
        }

        result
    }
}

impl Encoder for SimpleHTMLEncoder {
    fn encode_text(&self, original_text: &str) -> String {
        Self::html_encode(Some(original_text))
    }
}