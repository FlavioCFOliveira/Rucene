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

//! The types declared in `TrecFTParser.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::error::Result;

use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::trec_content_source::TrecContentSource;
use crate::by_task::feeds::trec_doc_parser::{extract, strip_tags_buf, ParsePathType, TrecDocParser};

/// `DATE`.
const DATE: &str = "<DATE>";
/// `DATE_END`.
const DATE_END: &str = "</DATE>";
/// `HEADLINE`.
const HEADLINE: &str = "<HEADLINE>";
/// `HEADLINE_END`.
const HEADLINE_END: &str = "</HEADLINE>";

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.TrecFTParser`
/// (`public class TrecFTParser extends TrecDocParser`): parser for the FT
/// docs in trec disks 4+5 collection format.
#[derive(Debug, Default, Clone, Copy)]
pub struct TrecFTParser;

impl TrecDocParser for TrecFTParser {
    fn parse(
        &self,
        doc_data: &mut DocData,
        name: &str,
        trec_src: &TrecContentSource,
        doc_buf: &[u16],
        _path_type: Option<ParsePathType>,
    ) -> Result<()> {
        let mark = 0; // that much is skipped

        // date...
        let mut date = None;
        if let Some(date_str) = extract(doc_buf, DATE, DATE_END, -1, None)? {
            date = trec_src.parse_date(&date_str)?;
        }

        // title...
        let title = extract(doc_buf, HEADLINE, HEADLINE_END, -1, None)?;

        doc_data.clear();
        doc_data.set_name(Some(name.to_string()));
        doc_data.set_date_from(date.as_ref());
        doc_data.set_title(title);
        doc_data.set_body(Some(strip_tags_buf(doc_buf, mark)?));
        Ok(())
    }
}
