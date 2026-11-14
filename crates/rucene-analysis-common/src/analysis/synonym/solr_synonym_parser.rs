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

//! Parser for the Solr synonyms format, ported from
//! `org.apache.lucene.analysis.synonym.SolrSynonymParser`.

use std::sync::Arc;

use rucene::analysis::{Analyzer, CharReader};
use rucene::error::Result;
use rucene::util::chars_ref::CharsRef;
use rucene::util::refs::CharsRefBuilder;

use super::synonym_map::{Builder, Parser};
use crate::jdk::io::LineNumberReader;

/// `java.text.ParseException("Invalid synonym rule at line " + n, 0)` with
/// `cause`, which the two parsers of the package raise for an
/// `IllegalArgumentException` while adding rules.
pub(crate) fn invalid_synonym_rule(
    line_number: i32,
    cause: rucene::error::LuceneError,
) -> rucene::error::LuceneError {
    rucene::error::LuceneError::from(rucene::error::LuceneErrorKind::Unchecked {
        class: rucene::util::extra::PARSE_EXCEPTION,
        message: Some(format!("Invalid synonym rule at line {line_number}")),
        cause: Some(Arc::new(cause)),
    })
}

/// Runs `body` over `br`, then closes `br`, as Java's `try { body } catch
/// (IllegalArgumentException e) { throw new ParseException(...) } finally {
/// br.close(); }` does: an exception from `close()` replaces the body's.
pub(crate) fn parse_lines<'r>(
    input: &'r mut dyn CharReader,
    body: impl FnOnce(&mut LineNumberReader<&'r mut dyn CharReader>) -> Result<()>,
) -> Result<()> {
    let mut br = LineNumberReader::new(input);
    let result = match body(&mut br) {
        Err(e) if e.is_illegal_argument_exception() => {
            Err(invalid_synonym_rule(br.get_line_number(), e))
        }
        other => other,
    };
    br.close()?;
    result
}

/// `String.trim()`: removes the leading and trailing units up to U+0020.
pub(crate) fn java_trim(s: &[u16]) -> &[u16] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && s[start] <= 0x20 {
        start += 1;
    }
    while start < end && s[end - 1] <= 0x20 {
        end -= 1;
    }
    &s[start..end]
}

/// Parser for the Solr synonyms format.
///
/// 1. Blank lines and lines starting with '#' are comments.
/// 2. Explicit mappings match any token sequence on the LHS of "=&gt;" and
///    replace with all alternatives on the RHS. These types of mappings
///    ignore the expand parameter in the constructor. Example:
///    `i-pod, i pod => ipod`
/// 3. Equivalent synonyms may be separated with commas and give no explicit
///    mapping. In this case the mapping behavior will be taken from the
///    expand parameter in the constructor. This allows the same synonym file
///    to be used in different synonym handling strategies. Example:
///    `ipod, i-pod, i pod`
/// 4. Multiple synonym mapping entries are merged. Example:
///    `foo => foo bar` and `foo => baz` is equivalent to
///    `foo => foo bar, baz`
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SolrSynonymParser`
/// (`@lucene.experimental`), which extends `SynonymMap.Parser`. Lines are
/// read as UTF-16 units, Java's strings.
#[derive(Debug)]
pub struct SolrSynonymParser {
    builder: Builder,
    analyzer: Arc<dyn Analyzer>,
    expand: bool,
}

impl SolrSynonymParser {
    /// Equivalent to `SolrSynonymParser(boolean, boolean, Analyzer)`.
    pub fn new(dedup: bool, expand: bool, analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            builder: Builder::with_dedup(dedup),
            analyzer,
            expand,
        }
    }

    /// Equivalent to `addInternal(BufferedReader)`.
    fn add_internal(&mut self, input: &mut LineNumberReader<&mut dyn CharReader>) -> Result<()> {
        while let Some(line) = input.read_line()? {
            if line.is_empty() || line[0] == u16::from(b'#') {
                continue; // ignore empty lines and comments
            }

            // TODO: we could process this more efficiently.
            let sides = Self::split(&line, "=>");
            if sides.len() > 1 {
                // explicit mapping
                if sides.len() != 2 {
                    return Err(rucene::error::LuceneError::illegal_argument(
                        "more than one explicit mapping specified on the same line",
                    ));
                }
                let input_strings = Self::split(&sides[0], ",");
                let mut inputs: Vec<CharsRef> = Vec::with_capacity(input_strings.len());
                for input_string in &input_strings {
                    inputs.push(self.analyze(
                        java_trim(&Self::unescape(input_string)),
                        &mut CharsRefBuilder::new(),
                    )?);
                }

                let output_strings = Self::split(&sides[1], ",");
                let mut outputs: Vec<CharsRef> = Vec::with_capacity(output_strings.len());
                for output_string in &output_strings {
                    outputs.push(self.analyze(
                        java_trim(&Self::unescape(output_string)),
                        &mut CharsRefBuilder::new(),
                    )?);
                }
                // these mappings are explicit and never preserve original
                for input in &inputs {
                    for output in &outputs {
                        self.builder.add(input, output, false)?;
                    }
                }
            } else {
                let input_strings = Self::split(&line, ",");
                let mut inputs: Vec<CharsRef> = Vec::with_capacity(input_strings.len());
                for input_string in &input_strings {
                    inputs.push(self.analyze(
                        java_trim(&Self::unescape(input_string)),
                        &mut CharsRefBuilder::new(),
                    )?);
                }
                if self.expand {
                    // all pairs
                    for i in 0..inputs.len() {
                        for j in 0..inputs.len() {
                            if i != j {
                                self.builder.add(&inputs[i], &inputs[j], true)?;
                            }
                        }
                    }
                } else {
                    // all subsequent inputs map to first one; we also add inputs[0] here
                    // so that we "effectively" (because we remove the original input and
                    // add back a synonym with the same text) change that token's type to
                    // SYNONYM (matching legacy behavior):
                    for input in &inputs {
                        self.builder.add(input, &inputs[0], false)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `split(String, String)`: splits `s` at `separator`,
    /// keeping a backslash and the unit it escapes, and dropping empty
    /// parts.
    fn split(s: &[u16], separator: &str) -> Vec<Vec<u16>> {
        let separator: Vec<u16> = separator.encode_utf16().collect();
        let mut list: Vec<Vec<u16>> = Vec::with_capacity(2);
        let mut sb: Vec<u16> = Vec::new();
        let mut pos = 0;
        let end = s.len();
        while pos < end {
            if s[pos..].starts_with(&separator) {
                if !sb.is_empty() {
                    list.push(std::mem::take(&mut sb));
                }
                pos += separator.len();
                continue;
            }

            let mut ch = s[pos];
            pos += 1;
            if ch == u16::from(b'\\') {
                sb.push(ch);
                if pos >= end {
                    break; // ERROR, or let it go?
                }
                ch = s[pos];
                pos += 1;
            }

            sb.push(ch);
        }

        if !sb.is_empty() {
            list.push(sb);
        }

        list
    }

    /// Equivalent to `unescape(String)`: drops each backslash that is not
    /// the last unit, keeping the unit it escapes.
    fn unescape(s: &[u16]) -> Vec<u16> {
        if s.contains(&u16::from(b'\\')) {
            let mut sb = Vec::with_capacity(s.len());
            let mut i = 0;
            while i < s.len() {
                let ch = s[i];
                if ch == u16::from(b'\\') && i < s.len() - 1 {
                    i += 1;
                    sb.push(s[i]);
                } else {
                    sb.push(ch);
                }
                i += 1;
            }
            return sb;
        }
        s.to_vec()
    }
}

impl Parser for SolrSynonymParser {
    fn builder(&self) -> &Builder {
        &self.builder
    }

    fn builder_mut(&mut self) -> &mut Builder {
        &mut self.builder
    }

    fn analyzer(&self) -> &dyn Analyzer {
        self.analyzer.as_ref()
    }

    /// Equivalent to `SolrSynonymParser.parse(Reader)`: the lines of a
    /// `LineNumberReader` over `input`, which is closed at the end; an
    /// `IllegalArgumentException` becomes `java.text.ParseException("Invalid
    /// synonym rule at line <n>")` with that cause.
    fn parse(&mut self, input: &mut dyn CharReader) -> Result<()> {
        parse_lines(input, |br| self.add_internal(br))
    }
}
