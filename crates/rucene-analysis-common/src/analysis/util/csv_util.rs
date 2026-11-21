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

//! Utility class for parsing CSV text, ported from
//! `org.apache.lucene.analysis.util.CSVUtil`.

/// `QUOTE`.
const QUOTE: char = '"';

/// `COMMA`.
const COMMA: char = ',';

/// `ESCAPED_QUOTE`.
const ESCAPED_QUOTE: &str = "\"\"";

/// Utility class for parsing CSV text.
///
/// Equivalent to `org.apache.lucene.analysis.util.CSVUtil`, a class of
/// static methods with a private constructor ("no instance!!!"): the type has
/// no value.
#[derive(Debug)]
pub enum CSVUtil {}

impl CSVUtil {
    /// Parse CSV line.
    ///
    /// Equivalent to `CSVUtil.parse(String)`: the values of `line`, split at
    /// the commas outside quotes, each unquoted and unescaped; an odd number
    /// of quotes yields no value at all (`new String[0]`).
    pub fn parse(line: &str) -> Vec<String> {
        let mut inside_quote = false;
        let mut result = Vec::new();
        let mut quote_count = 0;
        let mut sb = String::new();
        for c in line.chars() {
            if c == QUOTE {
                inside_quote = !inside_quote;
                quote_count += 1;
            }

            if c == COMMA && !inside_quote {
                let value = Self::un_quote_un_escape(&sb);
                result.push(value);
                sb.clear();
                continue;
            }

            sb.push(c);
        }

        result.push(sb);

        // Validate
        if quote_count % 2 != 0 {
            return Vec::new();
        }

        result
    }

    /// `unQuoteUnEscape(String)`.
    fn un_quote_un_escape(original: &str) -> String {
        let mut result = original.to_string();

        // Unquote
        if result.contains('"') {
            if let Some(group1) = Self::quote_replace_pattern_group1(original) {
                result = group1.to_string();
            }

            // Unescape
            if result.contains(ESCAPED_QUOTE) {
                result = result.replace(ESCAPED_QUOTE, "\"");
            }
        }

        result
    }

    /// `QUOTE_REPLACE_PATTERN.matcher(original)`, `m.matches()` and
    /// `m.group(1)`: the pattern `^"([^"]+)"$`, matched against the whole
    /// of `original`, is a quote, one or more characters that are not
    /// quotes (line terminators included), and a closing quote that ends the
    /// input (`$` may match before a final line terminator, but `matches()`
    /// requires the match to consume the whole input). Group 1 is the text
    /// between the quotes. The fixed pattern is matched directly rather than
    /// through a regular-expression engine.
    fn quote_replace_pattern_group1(original: &str) -> Option<&str> {
        let inner = original.strip_prefix(QUOTE)?.strip_suffix(QUOTE)?;
        if inner.is_empty() || inner.contains(QUOTE) {
            None
        } else {
            Some(inner)
        }
    }

    /// Quote and escape input value for CSV.
    ///
    /// Equivalent to `CSVUtil.quoteEscape(String)`.
    pub fn quote_escape(original: &str) -> String {
        let mut result = original.to_string();

        if result.contains('"') {
            result = result.replace('"', ESCAPED_QUOTE);
        }
        if result.contains(COMMA) {
            result = format!("\"{result}\"");
        }
        result
    }
}
