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

//! The types declared in `EnwikiQueryMaker.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::terms::Term;
use rucene::search::multi_term_query::constant_score_blended_rewrite;
use rucene::search::query::Query;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::util::automaton::operations::DEFAULT_DETERMINIZE_WORK_LIMIT;

use crate::by_task::feeds::abstract_query_maker::{AbstractQueryMaker, AbstractQueryMakerBase};
use crate::by_task::feeds::doc_maker::DocMaker;
use crate::by_task::feeds::reuters_query_maker::{create_queries, span_prebuilt_queries, QueryItem};
use crate::by_task::tasks::new_analyzer_task::NewAnalyzerTask;

/// `STANDARD_QUERIES`: common and a few uncommon queries from wikipedia
/// search logs.
const STANDARD_QUERIES: [&str; 87] = [
    "Images catbox gif",
    "Imunisasi haram",
    "Favicon ico",
    "Michael jackson",
    "Unknown artist",
    "Lily Thai",
    "Neda",
    "The Last Song",
    "Metallica",
    "Nicola Tesla",
    "Max B",
    "Skil Corporation",
    "\"The 100 Greatest Artists of All Time\"",
    "\"Top 100 Global Universities\"",
    "Pink floyd",
    "Bolton Sullivan",
    "Frank Lucas Jr",
    "Drake Woods",
    "Radiohead",
    "George Freeman",
    "Oksana Grigorieva",
    "The Elder Scrolls V",
    "Deadpool",
    "Green day",
    "\"Red hot chili peppers\"",
    "Jennifer Bini Taylor",
    "The Paradiso Girls",
    "Queen",
    "3Me4Ph",
    "Paloma Jimenez",
    "AUDI A4",
    "Edith Bouvier Beale: A Life In Pictures",
    "\"Skylar James Deleon\"",
    "Simple Explanation",
    "Juxtaposition",
    "The Woody Show",
    "London WITHER",
    "In A Dark Place",
    "George Freeman",
    "LuAnn de Lesseps",
    "Muhammad.",
    "U2",
    "List of countries by GDP",
    "Dean Martin Discography",
    "Web 3.0",
    "List of American actors",
    "The Expendables",
    "\"100 Greatest Guitarists of All Time\"",
    "Vince Offer.",
    "\"List of ZIP Codes in the United States\"",
    "Blood type diet",
    "Jennifer Gimenez",
    "List of hobbies",
    "The beatles",
    "Acdc",
    "Nightwish",
    "Iron maiden",
    "Murder Was the Case",
    "Pelvic hernia",
    "Naruto Shippuuden",
    "campaign",
    "Enthesopathy of hip region",
    "operating system",
    "mouse",
    "List of Xbox 360 games without region encoding",
    "Shakepearian sonnet",
    "\"The Monday Night Miracle\"",
    "India",
    "Dad's Army",
    "Solanum melanocerasum",
    "\"List of PlayStation Portable Wi-Fi games\"",
    "Little Pixie Geldof",
    "Planes, Trains & Automobiles",
    "Freddy Ingalls",
    "The Return of Chef",
    "Nehalem",
    "Turtle",
    "Calculus",
    "Superman-Prime",
    "\"The Losers\"",
    "pen-pal",
    "Audio stream input output",
    "lifehouse",
    "50 greatest gunners",
    "Polyfecalia",
    "freeloader",
    "The Filthy Youth",
];

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.EnwikiQueryMaker`
/// (`public class EnwikiQueryMaker extends AbstractQueryMaker`): a QueryMaker
/// that uses common and uncommon actual Wikipedia queries for searching the
/// English Wikipedia collection. 90 queries total.
#[derive(Debug, Default)]
pub struct EnwikiQueryMaker {
    base: AbstractQueryMakerBase,
}

impl EnwikiQueryMaker {
    /// `new EnwikiQueryMaker()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `getPrebuiltQueries(String field)`.
    fn get_prebuilt_queries(field: &str) -> Result<Vec<Arc<dyn Query>>> {
        let wcq = WildcardQuery::with_rewrite_method(
            Term::from_text(field.to_string(), "fo*"),
            DEFAULT_DETERMINIZE_WORK_LIMIT,
            constant_score_blended_rewrite(),
        )?;
        span_prebuilt_queries(field, Arc::new(wcq))
    }
}

impl AbstractQueryMaker for EnwikiQueryMaker {
    fn aqm_base(&self) -> &AbstractQueryMakerBase {
        &self.base
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.benchmark.byTask.feeds.EnwikiQueryMaker"
    }

    fn prepare_queries(&self) -> Result<Vec<Arc<dyn Query>>> {
        let config = self.base.config().ok_or_else(rucene::error::LuceneError::null_pointer)?;
        // analyzer (default is standard analyzer)
        let anlzr = NewAnalyzerTask::create_analyzer(
            &config
                .get_str("analyzer", Some("org.apache.lucene.analysis.standard.StandardAnalyzer"))?
                .unwrap_or_default(),
        )?;
        let mut query_list: Vec<QueryItem> = STANDARD_QUERIES.iter().map(|s| QueryItem::Text(s.to_string())).collect();
        if !config.get_bool("enwikiQueryMaker.disableSpanQueries", false)? {
            query_list.extend(Self::get_prebuilt_queries(DocMaker::BODY_FIELD)?.into_iter().map(QueryItem::Query));
        }
        create_queries(query_list, anlzr)
    }
}
