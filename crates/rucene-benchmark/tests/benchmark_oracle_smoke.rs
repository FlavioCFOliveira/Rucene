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

//! The smoke oracle of `lucene/benchmark` (rmp #628):
//! `BenchmarkOracle.java smoke` and Rucene, from one seed, give the same
//! `byTask.utils.Format` rows.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene_benchmark::by_task::utils::format::Format;
use sprint17_oracle::{corpus, Module};

const SEED: i64 = 0x0628_0006;

/// `BenchmarkOracle.smoke`.
fn rust_smoke(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    for _ in 0..20 {
        let d = (r.next_double() - 0.25) * 100000.0;
        let digits = r.next_int_bound(3);
        let col = " ".repeat(r.next_int_bound(12) as usize);
        let n = r.next_int_bound(2_000_000) - 1_000_000;
        let word = corpus::word(&r);
        out.push_str(&format!(
            "row |{}|{}|{}|{}|{}|{}|{}|\n",
            Format::format_double(digits, d, &col),
            Format::format_float(digits, d as f32, &col),
            Format::format_padd_right_double(digits, d, &col),
            Format::format_padd_right_float(digits, d as f32, &col),
            Format::format_int(n, &col),
            Format::format_str(word, &col),
            Format::format_padd_left(word, &col),
        ));
    }
    out
}

#[test]
fn benchmark_smoke_oracle() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Benchmark, "smoke", SEED, &[]);
    sprint17_oracle::assert_same(&java, &rust_smoke(SEED));
}
