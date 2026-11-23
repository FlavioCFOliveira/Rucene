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

//! The types declared in `SimpleSloppyPhraseQueryMaker.java`, of the Java
//! package `org.apache.lucene.benchmark.byTask.feeds`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::terms::Term;
use rucene::search::PhraseQueryBuilder;
use rucene::search::query::Query;

use crate::by_task::feeds::abstract_query_maker::{AbstractQueryMaker, AbstractQueryMakerBase};
use crate::by_task::feeds::doc_maker::DocMaker;
use crate::by_task::feeds::single_doc_source::DOC_TEXT;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.SimpleSloppyPhraseQueryMaker`
/// (`public class SimpleSloppyPhraseQueryMaker extends SimpleQueryMaker`):
/// create sloppy phrase queries for performance test, in an index created
/// using simple doc maker.
#[derive(Debug, Default)]
pub struct SimpleSloppyPhraseQueryMaker {
    base: AbstractQueryMakerBase,
}

impl SimpleSloppyPhraseQueryMaker {
    /// `new SimpleSloppyPhraseQueryMaker()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl AbstractQueryMaker for SimpleSloppyPhraseQueryMaker {
    fn aqm_base(&self) -> &AbstractQueryMakerBase {
        &self.base
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.benchmark.byTask.feeds.SimpleSloppyPhraseQueryMaker"
    }

    fn prepare_queries(&self) -> Result<Vec<Arc<dyn Query>>> {
        // extract some 100 words from doc text to an array
        let words: Vec<&str> = DOC_TEXT
            .split([' ', '\t', '\n', '\r', '\u{c}'])
            .filter(|w| !w.is_empty())
            .take(100)
            .collect();

        // create queries (that would find stuff) with varying slops
        let mut queries: Vec<Arc<dyn Query>> = Vec::new();
        let len = words.len() as i32;
        for slop in 0..8 {
            for qlen in 2..6 {
                let mut wd = 0;
                while wd < len - qlen - slop {
                    // ordered
                    let mut remained_slop = slop;
                    let mut wind = wd;
                    let mut builder = PhraseQueryBuilder::new();
                    for i in 0..qlen {
                        builder.add_at(Term::from_text(DocMaker::BODY_FIELD, words[wind as usize]), i)?;
                        wind += 1;
                        if remained_slop > 0 {
                            remained_slop -= 1;
                            wind += 1;
                        }
                    }
                    builder.set_slop(slop);
                    queries.push(Arc::new(builder.build()?));

                    // reversed
                    remained_slop = slop;
                    wind = wd + qlen + remained_slop - 1;
                    let mut builder = PhraseQueryBuilder::new();
                    for i in 0..qlen {
                        builder.add_at(Term::from_text(DocMaker::BODY_FIELD, words[wind as usize]), i)?;
                        wind -= 1;
                        if remained_slop > 0 {
                            remained_slop -= 1;
                            wind -= 1;
                        }
                    }
                    builder.set_slop(slop + 2 * qlen);
                    queries.push(Arc::new(builder.build()?));
                    wd += 1;
                }
            }
        }
        Ok(queries)
    }
}
