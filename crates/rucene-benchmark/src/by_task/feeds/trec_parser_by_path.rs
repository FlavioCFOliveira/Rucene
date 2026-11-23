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

//! The types declared in `TrecParserByPath.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::error::{LuceneError, Result};

use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::trec_content_source::TrecContentSource;
use crate::by_task::feeds::trec_doc_parser::{path_type_2_parser, ParsePathType, TrecDocParser};

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.TrecParserByPath`
/// (`public class TrecParserByPath extends TrecDocParser`): parser for trec
/// docs which selects the parser to apply according to the source files
/// path, defaulting to `TrecGov2Parser`.
#[derive(Debug, Default, Clone, Copy)]
pub struct TrecParserByPath;

impl TrecDocParser for TrecParserByPath {
    fn parse(
        &self,
        doc_data: &mut DocData,
        name: &str,
        trec_src: &TrecContentSource,
        doc_buf: &[u16],
        path_type: Option<ParsePathType>,
    ) -> Result<()> {
        // `pathType2Parser.get(null)` is null: NullPointerException.
        let path_type = path_type.ok_or_else(LuceneError::null_pointer)?;
        path_type_2_parser(path_type).parse(doc_data, name, trec_src, doc_buf, Some(path_type))
    }
}
