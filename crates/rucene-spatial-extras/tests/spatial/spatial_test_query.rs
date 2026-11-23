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

//! Port of `SpatialTestQuery.java` (package `org.apache.lucene.spatial`):
//! helper class to execute queries.

use std::fmt;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_args_parser::SpatialArgsParser;
use spatial4j::context::spatial_context::SpatialContext;

/// `SpatialTestQuery` (`public class`).
pub struct SpatialTestQuery {
    /// `testname`.
    pub testname: Option<String>,
    /// `line`.
    pub line: Option<String>,
    /// `lineNumber`.
    pub line_number: i32,
    /// `args`.
    pub args: SpatialArgs,
    /// `ids`.
    pub ids: Vec<String>,
}

impl SpatialTestQuery {
    /// `getTestQueries(SpatialArgsParser parser, SpatialContext ctx, String
    /// name, InputStream in)`: get Test Queries. The InputStream is closed.
    ///
    /// # Errors
    ///
    /// Java's `RuntimeException("invalid query line: …", ex)` for a line
    /// that does not parse; the I/O errors of reading.
    pub fn get_test_queries(
        parser: &SpatialArgsParser,
        ctx: &SpatialContext,
        _name: &str,
        input: &[u8],
    ) -> Result<Vec<SpatialTestQuery>> {
        let mut results = Vec::new();
        for (index, original) in read_lines(input)?.into_iter().enumerate() {
            let line_number = index as i32 + 1;
            let parse = || -> Result<SpatialTestQuery> {
                let mut line: &str = &original;
                // skip a comment
                if line.starts_with('[') {
                    if let Some(idx) = line.find(']') {
                        if idx > 0 {
                            line = &line[idx + 1..];
                        }
                    }
                }

                let idx = line.find('@').ok_or_else(|| {
                    // `line.substring(0, -1)`
                    LuceneError::unchecked(
                        "java.lang.StringIndexOutOfBoundsException",
                        format!("begin 0, end -1, length {}", line.encode_utf16().count()),
                    )
                })?;
                let mut ids = Vec::new();
                for token in string_tokenizer(&line[..idx]) {
                    ids.push(token.trim().to_string());
                }
                let args = parser.parse(line[idx + 1..].trim(), ctx)?;
                Ok(SpatialTestQuery {
                    testname: None,
                    line: Some(original.clone()),
                    line_number,
                    args,
                    ids,
                })
            };
            match parse() {
                Ok(test) => results.push(test),
                Err(ex) => {
                    return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                        class: "java.lang.RuntimeException",
                        message: Some(format!("invalid query line: {original}")),
                        cause: Some(std::sync::Arc::new(ex)),
                    }))
                }
            }
        }
        Ok(results)
    }
}

impl fmt::Display for SpatialTestQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = &self.line {
            return f.write_str(line);
        }
        write!(f, "{} [{}]", self.args, self.ids.join(", "))
    }
}

/// `BufferedReader.readLine()` over the UTF-8 decoding of `input`, until
/// the end: lines end at `\n`, `\r` or `\r\n`.
///
/// # Errors
///
/// Java's `MalformedInputException` is never raised by `InputStreamReader`
/// (it replaces malformed input); the port does the same.
pub fn read_lines(input: &[u8]) -> Result<Vec<String>> {
    let text = String::from_utf8_lossy(input);
    let mut lines = Vec::new();
    let mut rest: &str = &text;
    while !rest.is_empty() {
        match rest.find(['\n', '\r']) {
            Some(i) => {
                lines.push(rest[..i].to_string());
                let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[i + skip..];
            }
            None => {
                lines.push(rest.to_string());
                rest = "";
            }
        }
    }
    Ok(lines)
}

/// `String.split(String regex)` for a one-character literal separator:
/// trailing empty strings are removed.
pub fn java_split(s: &str, sep: char) -> Vec<String> {
    let mut parts: Vec<String> = s.split(sep).map(str::to_string).collect();
    while parts.len() > 1 && parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    if parts.len() == 1 && parts[0].is_empty() && !s.is_empty() {
        parts.clear();
    }
    parts
}

/// `new StringTokenizer(str)`: the tokens between the default delimiters
/// `" \t\n\r\f"`.
pub fn string_tokenizer(s: &str) -> Vec<&str> {
    s.split([' ', '\t', '\n', '\r', '\u{c}'])
        .filter(|t| !t.is_empty())
        .collect()
}
