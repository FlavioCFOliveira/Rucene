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

//! The types declared in `TrecTopicsReader.java`, of the Java package
//! `org.apache.lucene.benchmark.quality.trec`.

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::JavaHashMap;
use rucene_analysis_common::jdk::io::{BufferedReader, ReadChars};

use crate::jdk::{self, NEW_LINE};
use crate::quality::quality_query::{sort_quality_queries, QualityQuery};

/// Equivalent to `org.apache.lucene.benchmark.quality.trec.TrecTopicsReader`:
/// read TREC topics.
///
/// Expects this topic format -
///
/// ```text
///   <top>
///   <num> Number: nnn
///
///   <title> title of the topic
///
///   <desc> Description:
///   description of the topic
///
///   <narr> Narrative:
///   "story" composed by assessors.
///
///   </top>
/// ```
///
/// Comment lines starting with '#' are ignored.
#[derive(Debug, Default)]
pub struct TrecTopicsReader;

/// `sb.substring(k + 1).trim()` over UTF-16 units, `k` being `indexOf(c)`.
fn after(sb: &[u16], c: u8) -> String {
    let k = sb.iter().position(|&u| u == u16::from(c)).map_or(-1, |p| p as i64);
    let text = String::from_utf16_lossy(&sb[(k + 1) as usize..]);
    jdk::java_trim(&text).to_string()
}

impl TrecTopicsReader {
    /// `TrecTopicsReader()`: constructor for Trec's TopicsReader.
    pub fn new() -> Self {
        TrecTopicsReader
    }

    /// `readQueries(BufferedReader reader)`: read quality queries from trec
    /// format topics file.
    ///
    /// # Errors
    ///
    /// If cannot read the queries.
    pub fn read_queries<S: ReadChars>(&self, mut reader: BufferedReader<S>) -> Result<Vec<QualityQuery>> {
        let mut res = Vec::new();
        let result = (|| -> Result<()> {
            while self.read(&mut reader, "<top>", false, false)?.is_some() {
                let mut fields: JavaHashMap<String> = JavaHashMap::new();
                // id
                let sb = self.read(&mut reader, "<num>", true, false)?.ok_or_else(LuceneError::null_pointer)?;
                let id = after(&sb, b':');
                // title
                let sb = self.read(&mut reader, "<title>", true, false)?.ok_or_else(LuceneError::null_pointer)?;
                let title = after(&sb, b'>');
                // description
                self.read(&mut reader, "<desc>", false, false)?;
                let mut sb = String::new();
                while let Some(line) = jdk::read_line(&mut reader)? {
                    if line.starts_with("<narr>") {
                        break;
                    }
                    if !sb.is_empty() {
                        sb.push(' ');
                    }
                    sb.push_str(&line);
                }
                let description = jdk::java_trim(&sb).to_string();
                // narrative
                sb.clear();
                while let Some(line) = jdk::read_line(&mut reader)? {
                    if line.starts_with("</top>") {
                        break;
                    }
                    if !sb.is_empty() {
                        sb.push(' ');
                    }
                    sb.push_str(&line);
                }
                let narrative = jdk::java_trim(&sb).to_string();
                // we got a topic!
                fields.insert("title".to_string(), title);
                fields.insert("description".to_string(), description);
                fields.insert("narrative".to_string(), narrative);
                res.push(QualityQuery::new(id, fields));
            }
            Ok(())
        })();
        let closed = reader.close();
        result?;
        closed?;
        // sort result array (by ID)
        sort_quality_queries(&mut res);
        Ok(res)
    }

    /// `read(BufferedReader reader, String prefix, StringBuilder sb, boolean
    /// collectMatchLine, boolean collectAll)`: read until finding a line that
    /// starts with the specified prefix.
    fn read<S: ReadChars>(
        &self,
        reader: &mut BufferedReader<S>,
        prefix: &str,
        collect_match_line: bool,
        collect_all: bool,
    ) -> Result<Option<Vec<u16>>> {
        let mut sb: Vec<u16> = Vec::new();
        let mut sep = "";
        loop {
            let Some(line) = jdk::read_line(reader)? else {
                return Ok(None);
            };
            if line.starts_with(prefix) {
                if collect_match_line {
                    sb.extend(sep.encode_utf16());
                    sb.extend(line.encode_utf16());
                }
                break;
            }
            if collect_all {
                sb.extend(sep.encode_utf16());
                sb.extend(line.encode_utf16());
                sep = NEW_LINE;
            }
        }
        // System.out.println("read: "+sb);
        Ok(Some(sb))
    }
}
