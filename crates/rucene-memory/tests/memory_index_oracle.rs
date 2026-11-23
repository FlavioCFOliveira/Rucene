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

//! The MemoryIndex search parity of rmp #629: `MemoryOracle.java search` and
//! Rucene, from one seed, index the same two-field documents (offsets and
//! `ClassicSimilarity` drawn) and give the same `toString()` and score bits
//! for every term, phrase, sloppy phrase, boolean and prefix query, and the
//! same `toStringDebug()`.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::{query_to_string, Query};
use rucene::search::similarities::ClassicSimilarity;
use rucene::search::TermQuery;
use rucene_memory::memory_index::MemoryIndex;
use sprint17_oracle::{bits32, corpus, Module};

const SEEDS: [i64; 3] = [0x0629_0001, 0x0629_0002, -0x0629_0003];

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(field.to_string(), text)))
}

/// `MemoryOracle.query`.
fn query(r: &dyn Random) -> Arc<dyn Query> {
    match r.next_int_bound(5) {
        0 => {
            let field = if r.next_boolean() { "title" } else { "body" };
            tq(field, corpus::word(r))
        }
        1 => {
            let (w1, w2) = (corpus::word(r), corpus::word(r));
            Arc::new(PhraseQuery::from_strings("body", &[w1, w2]).unwrap())
        }
        2 => {
            let slop = r.next_int_bound(4);
            let (w1, w2) = (corpus::word(r), corpus::word(r));
            Arc::new(PhraseQuery::with_slop(slop, "body", &[w1, w2]).unwrap())
        }
        3 => {
            let mut b = BooleanQueryBuilder::new();
            b.add(tq("title", corpus::word(r)), Occur::SHOULD).unwrap();
            b.add(tq("body", corpus::word(r)), Occur::SHOULD).unwrap();
            let q = tq("body", corpus::word(r));
            let occur = if r.next_boolean() {
                Occur::MUST
            } else {
                Occur::MUST_NOT
            };
            b.add(q, occur).unwrap();
            Arc::new(b.build())
        }
        _ => {
            let w = corpus::word(r);
            let n = 1 + r.next_int_bound(w.len() as i32) as usize;
            Arc::new(PrefixQuery::new(Term::from_text("body", &w[..n])).unwrap())
        }
    }
}

/// `MemoryOracle.search`.
fn rust_search(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let analyzer = StandardAnalyzer::new();
    let mut out = String::new();
    for doc in 0..12 {
        let n = 3 + r.next_int_bound(8) as usize;
        let title = corpus::text(&r, n);
        let n = 10 + r.next_int_bound(50) as usize;
        let body = corpus::text(&r, n);
        let mut index = MemoryIndex::with_offsets(r.next_boolean());
        let classic = r.next_boolean();
        if classic {
            index
                .set_similarity(Arc::new(ClassicSimilarity::new()))
                .unwrap();
        }
        index.add_field_text("title", &title, &analyzer).unwrap();
        index.add_field_text("body", &body, &analyzer).unwrap();
        out.push_str(&format!("doc {doc} classic={classic}\n"));
        out.push_str(&format!("title {title}\nbody {body}\n"));
        for _ in 0..8 {
            let query = query(&r);
            let text = query_to_string(&*query);
            let score = index.search(query).unwrap();
            out.push_str(&format!("query {text} {}\n", bits32(score)));
        }
        out.push_str(&index.to_string_debug().unwrap());
        out.push('\n');
    }
    out
}

#[test]
fn memory_index_search_parity() {
    sprint17_oracle::pin_tests_seed();
    for seed in SEEDS {
        let java = sprint17_oracle::run(Module::Memory, "search", seed, &[]);
        sprint17_oracle::assert_same(&java, &rust_search(seed));
    }
}

/// Positive control: a perturbed Rust dump is reported as different.
#[test]
fn memory_index_search_parity_detects_a_difference() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Memory, "search", SEEDS[0], &[]);
    let rust = rust_search(SEEDS[0]).replacen("query ", "query x", 1);
    assert!(sprint17_oracle::compare(&java, &rust).is_err());
}
