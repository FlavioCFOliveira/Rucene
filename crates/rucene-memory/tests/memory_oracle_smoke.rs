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

//! The smoke oracle of `lucene/memory` (rmp #628): `MemoryOracle.java smoke`
//! and Rucene, from one seed, give the same scores and `toStringDebug()`.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::index::Term;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::TermQuery;
use rucene_memory::memory_index::MemoryIndex;
use sprint17_oracle::{corpus, Module};

const SEED: i64 = 0x0628_0001;

fn bits(f: f32) -> String {
    format!("{:x}", f.to_bits())
}

/// `MemoryOracle.smoke`.
fn rust_smoke(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let analyzer = StandardAnalyzer::new();
    let mut out = String::new();
    for doc in 0..8 {
        let n = 5 + r.next_int_bound(20) as usize;
        let text = corpus::text(&r, n);
        let mut index = MemoryIndex::with_offsets(r.next_boolean());
        index.add_field_text("body", &text, &analyzer).unwrap();
        out.push_str(&format!("doc {doc} {text}\n"));
        for _ in 0..6 {
            let word = corpus::word(&r);
            let score = index
                .search(Arc::new(TermQuery::new(Term::from_text("body", word))))
                .unwrap();
            out.push_str(&format!("term {word} {}\n", bits(score)));
        }
        let w1 = corpus::word(&r);
        let w2 = corpus::word(&r);
        let score = index
            .search(Arc::new(
                PhraseQuery::from_strings("body", &[w1, w2]).unwrap(),
            ))
            .unwrap();
        out.push_str(&format!("phrase {w1} {w2} {}\n", bits(score)));
        out.push_str(&index.to_string_debug().unwrap());
        out.push('\n');
    }
    out
}

#[test]
fn memory_smoke_oracle() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Memory, "smoke", SEED, &[]);
    sprint17_oracle::assert_same(&java, &rust_smoke(SEED));
}
