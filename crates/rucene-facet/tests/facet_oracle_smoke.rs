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

//! The smoke oracle of `lucene/facet` (rmp #628): `FacetOracle.java smoke`
//! and Rucene, from one seed, give the same `FacetsConfig.pathToString` and
//! `stringToPath` results.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene_facet::facets_config::{FacetsConfig, DELIM_CHAR};
use sprint17_oracle::{corpus, escape, Module};

const SEED: i64 = 0x0628_0002;

/// Inserts `c` before the UTF-16 unit `at` of an ASCII string
/// (`StringBuilder.insert(int, char)`).
fn insert(s: &mut String, at: usize, c: char) {
    s.insert(at, c);
}

/// `FacetOracle.smoke`.
fn rust_smoke(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    for _ in 0..20 {
        let mut path = vec![format!("dim{}", r.next_int_bound(3))];
        let n = 1 + r.next_int_bound(4);
        for _ in 0..n {
            let mut component = corpus::word(&r).to_string();
            let splice = r.next_int_bound(4);
            if splice == 1 {
                let at = r.next_int_bound(component.len() as i32 + 1) as usize;
                insert(&mut component, at, DELIM_CHAR);
            }
            if splice == 2 {
                let at = r.next_int_bound(component.len() as i32 + 1) as usize;
                insert(&mut component, at, '\u{1E}');
            }
            path.push(component);
        }
        let encoded = FacetsConfig::path_to_string(&path).unwrap();
        out.push_str(&format!("path {}\n", escape(&path.join("/"))));
        out.push_str(&format!("encoded {}\n", escape(&encoded)));
        let decoded = FacetsConfig::string_to_path(&encoded);
        out.push_str(&format!("decoded {}\n", escape(&decoded.join("/"))));
    }
    out
}

#[test]
fn facet_smoke_oracle() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Facet, "smoke", SEED, &[]);
    sprint17_oracle::assert_same(&java, &rust_smoke(SEED));
}
