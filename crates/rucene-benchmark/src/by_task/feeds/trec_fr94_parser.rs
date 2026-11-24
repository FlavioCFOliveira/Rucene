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

//! The types declared in `TrecFR94Parser.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::error::Result;

use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::trec_content_source::TrecContentSource;
use crate::by_task::feeds::trec_doc_parser::{
    extract, index_of, strip_tags, strip_tags_buf, ParsePathType, TrecDocParser,
};
use crate::jdk;

/// `TEXT`.
const TEXT: &str = "<TEXT>";
/// `TEXT_LENGTH`.
const TEXT_LENGTH: i64 = TEXT.len() as i64;
/// `TEXT_END`.
const TEXT_END: &str = "</TEXT>";
/// `DATE`.
const DATE: &str = "<DATE>";
/// `DATE_NOISE_PREFIXES`.
const DATE_NOISE_PREFIXES: [&str; 3] = [
    "DATE:", "date:", // TODO improve date extraction for this format
    "t.c.",
];
/// `DATE_END`.
const DATE_END: &str = "</DATE>";

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.TrecFR94Parser`
/// (`public class TrecFR94Parser extends TrecDocParser`): parser for the FR94
/// docs in trec disks 4+5 collection format.
#[derive(Debug, Default, Clone, Copy)]
pub struct TrecFR94Parser;

impl TrecDocParser for TrecFR94Parser {
    // TODO can we also extract title for this format?
    fn parse(
        &self,
        doc_data: &mut DocData,
        name: &str,
        trec_src: &TrecContentSource,
        doc_buf: &[u16],
        _path_type: Option<ParsePathType>,
    ) -> Result<()> {
        let mut mark = 0; // that much is skipped
                          // optionally skip some of the text, set date (no title?)
        let mut date = None;
        let h1 = index_of(doc_buf, TEXT, 0);
        if h1 >= 0 {
            let h2 = index_of(doc_buf, TEXT_END, h1);
            mark = h1 + TEXT_LENGTH;
            // date...
            if let Some(date_str) =
                extract(doc_buf, DATE, DATE_END, h2, Some(&DATE_NOISE_PREFIXES))?
            {
                let date_str = strip_tags(&date_str, 0)?;
                date = trec_src.parse_date(jdk::java_trim(&date_str))?;
            }
        }
        doc_data.clear();
        doc_data.set_name(Some(name.to_string()));
        doc_data.set_date_from(date.as_ref());
        doc_data.set_body(Some(strip_tags_buf(doc_buf, mark)?));
        Ok(())
    }
}
