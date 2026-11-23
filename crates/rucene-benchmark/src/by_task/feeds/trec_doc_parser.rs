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

//! The types declared in `TrecDocParser.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::path::Path;
use std::sync::Arc;

use rucene::error::Result;

use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::trec_content_source::TrecContentSource;
use crate::by_task::feeds::trec_fbis_parser::TrecFBISParser;
use crate::by_task::feeds::trec_fr94_parser::TrecFR94Parser;
use crate::by_task::feeds::trec_ft_parser::TrecFTParser;
use crate::by_task::feeds::trec_gov2_parser::TrecGov2Parser;
use crate::by_task::feeds::trec_la_times_parser::TrecLATimesParser;
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.TrecDocParser.ParsePathType`
/// (`public enum ParsePathType`): types of trec parse paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParsePathType {
    /// Java constant `GOV2`.
    Gov2,
    /// Java constant `FBIS`.
    Fbis,
    /// Java constant `FT`.
    Ft,
    /// Java constant `FR94`.
    Fr94,
    /// Java constant `LATIMES`.
    Latimes,
}

impl ParsePathType {
    /// `values()`.
    pub const VALUES: [ParsePathType; 5] =
        [ParsePathType::Gov2, ParsePathType::Fbis, ParsePathType::Ft, ParsePathType::Fr94, ParsePathType::Latimes];

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            ParsePathType::Gov2 => "GOV2",
            ParsePathType::Fbis => "FBIS",
            ParsePathType::Ft => "FT",
            ParsePathType::Fr94 => "FR94",
            ParsePathType::Latimes => "LATIMES",
        }
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.TrecDocParser`
/// (`public abstract class TrecDocParser`): parser for trec doc content,
/// invoked on doc text excluding <DOC> and <DOCNO> which are handled in
/// TrecContentSource. Required to be stateless and hence thread safe.
///
/// The text is the UTF-16 code units Java's `StringBuilder` holds, so the
/// positions the parsers compute are Java's.
pub trait TrecDocParser: Send + Sync {
    /// `parse(DocData docData, String name, TrecContentSource trecSrc,
    /// StringBuilder docBuf, ParsePathType pathType)`: parse the text
    /// prepared in docBuf into a result DocData, no synchronization is
    /// required. `path_type` is the type of parsed file, or `None` if
    /// unknown - may be used by parsers to alter their behavior according to
    /// the file path type.
    ///
    /// # Errors
    ///
    /// The `IOException` of the HTML parser.
    fn parse(
        &self,
        doc_data: &mut DocData,
        name: &str,
        trec_src: &TrecContentSource,
        doc_buf: &[u16],
        path_type: Option<ParsePathType>,
    ) -> Result<()>;
}

/// `DEFAULT_PATH_TYPE`: trec parser type used for unknown extensions.
pub const DEFAULT_PATH_TYPE: ParsePathType = ParsePathType::Gov2;

/// `MAX_PATH_LENGTH`: max length of walk up from file to its ancestors when
/// looking for a known path type.
const MAX_PATH_LENGTH: i32 = 10;

/// `pathType2Parser.get(pathType)`.
pub fn path_type_2_parser(path_type: ParsePathType) -> Arc<dyn TrecDocParser> {
    match path_type {
        ParsePathType::Gov2 => Arc::new(TrecGov2Parser),
        ParsePathType::Fbis => Arc::new(TrecFBISParser),
        ParsePathType::Fr94 => Arc::new(TrecFR94Parser),
        ParsePathType::Ft => Arc::new(TrecFTParser),
        ParsePathType::Latimes => Arc::new(TrecLATimesParser),
    }
}

/// `pathName2Type.get(name)`.
fn path_name_2_type(name: &str) -> Option<ParsePathType> {
    ParsePathType::VALUES.into_iter().find(|p| p.name().to_uppercase() == name)
}

/// `pathType(Path f)`: compute the path type of a file by inspecting name
/// of file and its parents.
pub fn path_type(f: &Path) -> ParsePathType {
    let mut path_length = 0;
    let mut f = Some(f);
    while let Some(p) = f {
        let Some(file_name) = p.file_name() else { break };
        path_length += 1;
        if path_length >= MAX_PATH_LENGTH {
            break;
        }
        if let Some(ppt) = path_name_2_type(&file_name.to_string_lossy().to_uppercase()) {
            return ppt;
        }
        f = p.parent();
    }
    DEFAULT_PATH_TYPE
}

/// `indexOf(str, fromIndex)` over UTF-16 units, Java's semantics.
pub fn index_of(buf: &[u16], s: &str, from: i64) -> i64 {
    let pat: Vec<u16> = s.encode_utf16().collect();
    let from = from.max(0) as usize;
    if from > buf.len() {
        return if pat.is_empty() { buf.len() as i64 } else { -1 };
    }
    if pat.is_empty() {
        return from as i64;
    }
    if pat.len() > buf.len() {
        return -1;
    }
    (from..=buf.len() - pat.len()).find(|&i| buf[i..i + pat.len()] == pat[..]).map_or(-1, |i| i as i64)
}

/// `substring(begin, end)` over UTF-16 units.
///
/// # Errors
///
/// `StringIndexOutOfBoundsException` as Java throws it.
pub fn substring(buf: &[u16], begin: i64, end: i64) -> Result<String> {
    if begin < 0 || end > buf.len() as i64 || begin > end {
        return Err(rucene::error::LuceneError::unchecked(
            "java.lang.StringIndexOutOfBoundsException",
            format!("begin {begin}, end {end}, length {}", buf.len()),
        ));
    }
    Ok(String::from_utf16_lossy(&buf[begin as usize..end as usize]))
}

/// `stripTags(StringBuilder buf, int start)`: strip tags from `buf`: each
/// tag is replaced by a single blank. Returns the text obtained when
/// stripping all tags from `buf` (input is unmodified).
///
/// # Errors
///
/// `StringIndexOutOfBoundsException` for a `start` out of range.
pub fn strip_tags_buf(buf: &[u16], start: i64) -> Result<String> {
    let s = substring(buf, start, buf.len() as i64)?;
    strip_tags(&s, 0)
}

/// `stripTags(String buf, int start)`: `buf.replaceAll("<[^>]*>", " ")`
/// after skipping `start` units.
///
/// # Errors
///
/// `StringIndexOutOfBoundsException` for a `start` out of range.
pub fn strip_tags(buf: &str, start: i64) -> Result<String> {
    let units: Vec<u16> = buf.encode_utf16().collect();
    let units = if start > 0 {
        let s = substring(&units, start, units.len() as i64)?;
        s.encode_utf16().collect()
    } else {
        units
    };
    let mut out: Vec<u16> = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        if units[i] == u16::from(b'<') {
            if let Some(end) = units[i + 1..].iter().position(|&c| c == u16::from(b'>')) {
                out.push(u16::from(b' '));
                i += end + 2;
                continue;
            }
        }
        out.push(units[i]);
        i += 1;
    }
    Ok(String::from_utf16_lossy(&out))
}

/// `extract(StringBuilder buf, String startTag, String endTag, int maxPos,
/// String[] noisePrefixes)`: extract from `buf` the text of interest within
/// specified tags; `max_pos`, if >= 0, sets a limit on start of text of
/// interest. Returns the text of interest or `None` if not found.
/// # Errors
///
/// `StringIndexOutOfBoundsException` when a noise prefix ends past the end
/// tag.
pub fn extract(
    buf: &[u16],
    start_tag: &str,
    end_tag: &str,
    max_pos: i64,
    noise_prefixes: Option<&[&str]>,
) -> Result<Option<String>> {
    let mut k1 = index_of(buf, start_tag, 0);
    if k1 >= 0 && (max_pos < 0 || k1 < max_pos) {
        k1 += jdk::utf16_len(start_tag) as i64;
        let k2 = index_of(buf, end_tag, k1);
        if k2 >= 0 && (max_pos < 0 || k2 < max_pos) {
            // found end tag with allowed range
            if let Some(noise_prefixes) = noise_prefixes {
                for noise in noise_prefixes {
                    let k1a = index_of(buf, noise, k1);
                    if k1a >= 0 && k1a < k2 {
                        k1 = k1a + jdk::utf16_len(noise) as i64;
                    }
                }
            }
            let text = substring(buf, k1, k2)?;
            return Ok(Some(jdk::java_trim(&text).to_string()));
        }
    }
    Ok(None)
}
