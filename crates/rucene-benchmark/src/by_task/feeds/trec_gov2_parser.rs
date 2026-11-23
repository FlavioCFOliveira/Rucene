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

//! The types declared in `TrecGov2Parser.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::error::{LuceneError, Result};

use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::trec_content_source::TrecContentSource;
use crate::by_task::feeds::trec_doc_parser::{extract, index_of, substring, ParsePathType, TrecDocParser};
use crate::jdk;

/// `DATE`.
const DATE: &str = "Date: ";
/// `DATE_END`: `TrecContentSource.NEW_LINE`.
const DATE_END: &str = TrecContentSource::NEW_LINE;
/// `DOCHDR`.
const DOCHDR: &str = "<DOCHDR>";
/// `TERMINATING_DOCHDR`.
const TERMINATING_DOCHDR: &str = "</DOCHDR>";

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.TrecGov2Parser`
/// (`public class TrecGov2Parser extends TrecDocParser`): parser for the
/// GOV2 collection format.
#[derive(Debug, Default, Clone, Copy)]
pub struct TrecGov2Parser;

impl TrecDocParser for TrecGov2Parser {
    fn parse(
        &self,
        doc_data: &mut DocData,
        name: &str,
        trec_src: &TrecContentSource,
        doc_buf: &[u16],
        _path_type: Option<ParsePathType>,
    ) -> Result<()> {
        // skip some of the non-html text, optionally set date
        let mut date = None;
        let mut start = 0;
        let h1 = index_of(doc_buf, DOCHDR, 0);
        if h1 >= 0 {
            let h2 = index_of(doc_buf, TERMINATING_DOCHDR, h1);
            if let Some(date_str) = extract(doc_buf, DATE, DATE_END, h2, None)? {
                date = trec_src.parse_date(&date_str)?;
            }
            start = h2 + jdk::utf16_len(TERMINATING_DOCHDR) as i64;
        }
        let html = substring(doc_buf, start, doc_buf.len() as i64)?;
        let parser = trec_src.get_html_parser().ok_or_else(LuceneError::null_pointer)?;
        parser.parse(doc_data, Some(name), date.as_ref(), &html, Some(trec_src))
    }
}
