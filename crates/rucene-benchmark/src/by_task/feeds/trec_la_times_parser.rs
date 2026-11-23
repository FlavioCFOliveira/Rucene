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

//! The types declared in `TrecLATimesParser.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::error::Result;

use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::trec_content_source::TrecContentSource;
use crate::by_task::feeds::trec_doc_parser::{extract, strip_tags, strip_tags_buf, ParsePathType, TrecDocParser};
use crate::jdk;

/// `DATE`.
const DATE: &str = "<DATE>";
/// `DATE_END`.
const DATE_END: &str = "</DATE>";
/// `DATE_NOISE`: anything aftre the ','.
const DATE_NOISE: &str = "day,";
/// `SUBJECT`.
const SUBJECT: &str = "<SUBJECT>";
/// `SUBJECT_END`.
const SUBJECT_END: &str = "</SUBJECT>";
/// `HEADLINE`.
const HEADLINE: &str = "<HEADLINE>";
/// `HEADLINE_END`.
const HEADLINE_END: &str = "</HEADLINE>";

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.TrecLATimesParser`
/// (`public class TrecLATimesParser extends TrecDocParser`): parser for the
/// FT docs in trec disks 4+5 collection format.
#[derive(Debug, Default, Clone, Copy)]
pub struct TrecLATimesParser;

impl TrecDocParser for TrecLATimesParser {
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
        if let Some(mut date_str) = extract(doc_buf, DATE, DATE_END, -1, None)? {
            let units: Vec<u16> = date_str.encode_utf16().collect();
            let d2a = crate::by_task::feeds::trec_doc_parser::index_of(&units, DATE_NOISE, 0);
            if d2a > 0 {
                date_str = String::from_utf16_lossy(&units[..(d2a + 3) as usize]); // we need the "day" part
            }
            let date_str = strip_tags(&date_str, 0)?;
            date = trec_src.parse_date(jdk::java_trim(&date_str))?;
        }

        // title... first try with SUBJECT, them with HEADLINE
        let mut title = extract(doc_buf, SUBJECT, SUBJECT_END, -1, None)?;
        if title.is_none() {
            title = extract(doc_buf, HEADLINE, HEADLINE_END, -1, None)?;
        }
        if let Some(t) = title {
            title = Some(jdk::java_trim(&strip_tags(&t, 0)?).to_string());
        }

        doc_data.clear();
        doc_data.set_name(Some(name.to_string()));
        doc_data.set_date_from(date.as_ref());
        doc_data.set_title(title);
        doc_data.set_body(Some(strip_tags_buf(doc_buf, mark)?));
        Ok(())
    }
}
