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

//! The types declared in `FileBasedQueryMaker.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::path::Path;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::query::Query;
use rucene_queryparser::classic::parse_exception::ParseException;
use rucene_queryparser::classic::query_parser::QueryParser;
use rucene_queryparser::classic::query_parser_base;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;

use crate::by_task::feeds::abstract_query_maker::{AbstractQueryMaker, AbstractQueryMakerBase};
use crate::by_task::feeds::doc_maker::DocMaker;
use crate::by_task::tasks::new_analyzer_task::NewAnalyzerTask;
use crate::by_task::utils::config::read_lines;
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.FileBasedQueryMaker`
/// (`public class FileBasedQueryMaker extends AbstractQueryMaker`): create
/// queries from a FileReader. One per line, pass them through the
/// QueryParser. Lines beginning with # are treated as comments
///
/// File can be specified as a absolute, relative or resource. Two
/// properties can be set: file.query.maker.file=<Full path to file
/// containing queries> and file.query.maker.default.field=<Name of default
/// field - Default value is "body">
///
/// Example:
///
/// ```text
/// file.query.maker.file=c:/myqueries.txt
/// file.query.maker.default.field=body
/// ```
#[derive(Debug, Default)]
pub struct FileBasedQueryMaker {
    base: AbstractQueryMakerBase,
}

impl FileBasedQueryMaker {
    /// `new FileBasedQueryMaker()`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// `Files.newBufferedReader(path, UTF_8)` read whole, or
/// `getResourceAsStream(name)` of the benchmark's class loader decoded as
/// UTF-8; `None` when neither exists.
fn read_query_file(file_name: &str) -> Result<Option<String>> {
    let path = Path::new(file_name);
    // note: we use a decoding reader, so if your queries are screwed up you know
    if path.exists() {
        let bytes = std::fs::read(path)?;
        return String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| jdk::io_exception("Input length = 1"));
    }
    // see if we can find it as a resource
    match rucene_analysis_common::resources::class_loader_resources().get(file_name) {
        Some(bytes) => String::from_utf8(bytes.to_vec())
            .map(Some)
            .map_err(|_| jdk::io_exception("Input length = 1")),
        None => Ok(None),
    }
}

impl AbstractQueryMaker for FileBasedQueryMaker {
    fn aqm_base(&self) -> &AbstractQueryMakerBase {
        &self.base
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.benchmark.byTask.feeds.FileBasedQueryMaker"
    }

    fn prepare_queries(&self) -> Result<Vec<Arc<dyn Query>>> {
        let config = self.base.config().ok_or_else(LuceneError::null_pointer)?;
        let anlzr = NewAnalyzerTask::create_analyzer(
            &config
                .get_str("analyzer", Some("org.apache.lucene.analysis.standard.StandardAnalyzer"))?
                .unwrap_or_default(),
        )?;
        let default_field = config
            .get_str("file.query.maker.default.field", Some(DocMaker::BODY_FIELD))?
            .unwrap_or_default();
        let mut qp = QueryParser::new(Some(&default_field), Some(anlzr))?;
        qp.set_allow_leading_wildcard(true)?;

        let mut qq: Vec<Arc<dyn Query>> = Vec::new();
        if let Some(file_name) = config.get_str("file.query.maker.file", None)? {
            match read_query_file(&file_name)? {
                Some(text) => {
                    for (line_num, line) in read_lines(&text).iter().enumerate() {
                        let line = jdk::java_trim(line);
                        if !line.is_empty() && !line.starts_with('#') {
                            match query_parser_base::parse(&mut qp, line) {
                                Ok(q) => qq.push(q),
                                Err(e) if ParseException::of(&e).is_some() => {
                                    eprintln!(
                                        "Exception: {} occurred while parsing line: {line_num} Text: {line}",
                                        jdk::get_message(&e).unwrap_or_else(|| "null".to_string())
                                    );
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    }
                }
                None => eprintln!("No Reader available for: {file_name}"),
            }
        }
        Ok(qq)
    }
}
