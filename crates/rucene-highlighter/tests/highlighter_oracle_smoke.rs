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

//! The smoke oracle of `lucene/highlighter` (rmp #628):
//! `HighlighterOracle.java smoke` and Rucene, from one seed, give the same
//! best fragments.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::index::Term;
use rucene::search::TermQuery;
use rucene_highlighter::highlight::highlighter::Highlighter;
use rucene_highlighter::highlight::query_scorer::QueryScorer;
use rucene_highlighter::highlight::simple_fragmenter::SimpleFragmenter;
use sprint17_oracle::{corpus, escape, Module};

const SEED: i64 = 0x0628_0003;

/// `HighlighterOracle.smoke`.
fn rust_smoke(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let analyzer = StandardAnalyzer::new();
    let mut out = String::new();
    for doc in 0..6 {
        let n = 20 + r.next_int_bound(60) as usize;
        let text = corpus::text(&r, n);
        let word = corpus::word(&r);
        let query = Arc::new(TermQuery::new(Term::from_text("f", word)));
        let mut highlighter = Highlighter::new(Box::new(QueryScorer::new(query)));
        highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(20)));
        let fragments = highlighter
            .get_best_fragments(&analyzer, "f", &text, 3)
            .unwrap();
        out.push_str(&format!("doc {doc} {word} {}\n", fragments.len()));
        for fragment in &fragments {
            out.push_str(&format!("frag {}\n", escape(fragment)));
        }
    }
    out
}

#[test]
fn highlighter_smoke_oracle() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Highlighter, "smoke", SEED, &[]);
    sprint17_oracle::assert_same(&java, &rust_smoke(SEED));
}
