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

//! The types declared in `TrecFBISParser.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::error::Result;

use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::trec_content_source::TrecContentSource;
use crate::by_task::feeds::trec_doc_parser::{
    extract, index_of, strip_tags_buf, ParsePathType, TrecDocParser,
};

/// `HEADER`.
const HEADER: &str = "<HEADER>";
/// `HEADER_END`.
const HEADER_END: &str = "</HEADER>";
/// `HEADER_END_LENGTH`.
const HEADER_END_LENGTH: i64 = HEADER_END.len() as i64;
/// `DATE1`.
const DATE1: &str = "<DATE1>";
/// `DATE1_END`.
const DATE1_END: &str = "</DATE1>";
/// `TI`.
const TI: &str = "<TI>";
/// `TI_END`.
const TI_END: &str = "</TI>";

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.TrecFBISParser`
/// (`public class TrecFBISParser extends TrecDocParser`): parser for the
/// FBIS docs in trec disks 4+5 collection format.
#[derive(Debug, Default, Clone, Copy)]
pub struct TrecFBISParser;

impl TrecDocParser for TrecFBISParser {
    fn parse(
        &self,
        doc_data: &mut DocData,
        name: &str,
        trec_src: &TrecContentSource,
        doc_buf: &[u16],
        _path_type: Option<ParsePathType>,
    ) -> Result<()> {
        let mut mark = 0; // that much is skipped
                          // optionally skip some of the text, set date, title
        let mut date = None;
        let mut title = None;
        let h1 = index_of(doc_buf, HEADER, 0);
        if h1 >= 0 {
            let h2 = index_of(doc_buf, HEADER_END, h1);
            mark = h2 + HEADER_END_LENGTH;
            // date...
            if let Some(date_str) = extract(doc_buf, DATE1, DATE1_END, h2, None)? {
                date = trec_src.parse_date(&date_str)?;
            }
            // title...
            title = extract(doc_buf, TI, TI_END, h2, None)?;
        }
        doc_data.clear();
        doc_data.set_name(Some(name.to_string()));
        doc_data.set_date_from(date.as_ref());
        doc_data.set_title(title);
        doc_data.set_body(Some(strip_tags_buf(doc_buf, mark)?));
        Ok(())
    }
}
