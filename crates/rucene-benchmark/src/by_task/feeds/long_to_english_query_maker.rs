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

//! The types declared in `LongToEnglishQueryMaker.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::search::query::Query;
use rucene_queryparser::classic::query_parser::QueryParser;
use rucene_queryparser::classic::query_parser_base;

use crate::by_task::feeds::doc_maker::DocMaker;
use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::tasks::new_analyzer_task::NewAnalyzerTask;
use crate::by_task::utils::config::Config;
use crate::icu::RuleBasedNumberFormat;
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.LongToEnglishQueryMaker`
/// (`public class LongToEnglishQueryMaker implements QueryMaker`): creates
/// queries whose content is a spelled-out `long` number starting from
/// `Long.MIN_VALUE + 10`.
pub struct LongToEnglishQueryMaker {
    counter: Mutex<i64>,
    /// `parser`.
    parser: Mutex<Option<QueryParser>>,
    // TODO: we could take param to specify locale...
    rnbf: RuleBasedNumberFormat,
}

impl Default for LongToEnglishQueryMaker {
    fn default() -> Self {
        Self::new()
    }
}

impl LongToEnglishQueryMaker {
    /// `new LongToEnglishQueryMaker()`.
    pub fn new() -> Self {
        LongToEnglishQueryMaker {
            counter: Mutex::new(i64::MIN + 10),
            parser: Mutex::new(None),
            rnbf: RuleBasedNumberFormat::root_spellout(),
        }
    }

    /// `getNextCounter()`.
    fn get_next_counter(&self) -> i64 {
        let mut counter = jdk::lock(&self.counter);
        if *counter == i64::MAX {
            *counter = i64::MIN + 10;
        }
        let c = *counter;
        *counter = counter.wrapping_add(1);
        c
    }
}

impl QueryMaker for LongToEnglishQueryMaker {
    fn make_query_of_size(&self, _size: i32) -> Result<Arc<dyn Query>> {
        Err(LuceneError::unchecked_with_message(
            "java.lang.UnsupportedOperationException",
            None,
        ))
    }

    fn make_query(&self) -> Result<Arc<dyn Query>> {
        let text = self.rnbf.format(self.get_next_counter());
        let mut parser = jdk::lock(&self.parser);
        let parser = parser.as_mut().ok_or_else(LuceneError::null_pointer)?;
        query_parser_base::parse(parser, &text)
    }

    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        let anlzr = NewAnalyzerTask::create_analyzer(
            &config
                .get_str(
                    "analyzer",
                    Some("org.apache.lucene.analysis.standard.StandardAnalyzer"),
                )?
                .unwrap_or_default(),
        )?;
        *jdk::lock(&self.parser) = Some(QueryParser::new(Some(DocMaker::BODY_FIELD), Some(anlzr))?);
        Ok(())
    }

    fn reset_inputs(&self) -> Result<()> {
        *jdk::lock(&self.counter) = i64::MIN + 10;
        Ok(())
    }

    fn print_queries(&self) -> String {
        format!(
            "LongToEnglish: [{} TO {}]",
            i64::MIN,
            *jdk::lock(&self.counter)
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
