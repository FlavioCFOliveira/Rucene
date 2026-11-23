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

//! Portability test of `rucene::util::strict_math` and
//! `rucene::util::java_math` (rmp #628; `exp` and `tanh`, rmp #647): over the edge cases and 1,000,000
//! seeded inputs per function, every [`StrictMath`] function equals JDK 25
//! `java.lang.StrictMath`, and every `java_math` function equals
//! `java.lang.Math` under `-XX:-UseLibmIntrinsic` (the divergence
//! `java_math` declares), bit for bit (`NaN`s compared as
//! `Double.doubleToLongBits`). The Java side is
//! `tests/fixtures/java-math/JavaMathOracle.java`; it prints a hash per chunk
//! of 10,000 results, which this test recomputes.

#[path = "jvm_fixture/mod.rs"]
mod jvm_fixture;

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use rucene::util::java_math;
use rucene::util::strict_math::StrictMath;

const SEED: i64 = 628;
const N: usize = 1_000_000;
const CHUNK: usize = 10_000;

const EDGES: [f64; 36] = [
    0.0,
    -0.0,
    1.0,
    -1.0,
    0.5,
    -0.5,
    2.0,
    -2.0,
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::from_bits(1),
    -f64::from_bits(1),
    f64::MAX,
    -f64::MAX,
    f64::MIN_POSITIVE,
    std::f64::consts::PI,
    -std::f64::consts::PI,
    std::f64::consts::PI / 2.0,
    std::f64::consts::PI / 4.0,
    1e-300,
    1e300,
    f64::from_bits(0x3E30_0000_0000_0000), // 0x1p-28
    f64::from_bits(0x3E20_0000_0000_0000), // 0x1p-29
    f64::from_bits(0x3C60_0000_0000_0000), // 0x1p-57
    0.975,
    0.6744,
    2.4375,
    0.4375,
    1.1875,
    2147483648.0,           // 0x1p31
    18446744073709551616.0, // 0x1p64
    73786976294838206464.0, // 0x1p66
    3.0,
    -3.0,
    1e22,
];

const CENTERS: [f64; 9] = [
    0.5,
    1.0,
    std::f64::consts::PI / 4.0,
    std::f64::consts::PI / 2.0,
    std::f64::consts::PI,
    2.4375,
    0.4375,
    1.1875,
    0.975,
];

/// `JavaMathOracle.Gen`: SplitMix64.
struct Gen {
    state: u64,
}

impl Gen {
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> i64 {
        (self.next() % bound) as i64
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 * f64::from_bits(0x3CA0_0000_0000_0000) // 0x1.0p-53
    }

    fn value(&mut self) -> f64 {
        match self.below(6) {
            0 => f64::from_bits(self.next()),
            1 => self.unit() * 2.0 - 1.0,
            2 => self.unit() * 20.0 - 10.0,
            3 => self.unit() * 2e6 - 1e6,
            4 => {
                let sign = self.next() & 0x8000_0000_0000_0000;
                let exponent = (self.below(1024) as u64) << 52;
                let mantissa = self.next() & 0x000F_FFFF_FFFF_FFFF;
                f64::from_bits(sign | exponent | mantissa)
            }
            _ => {
                let c = CENTERS[self.below(CENTERS.len() as u64) as usize];
                let j = self.below(64) - 32;
                let v = f64::from_bits((c.to_bits() as i64 + j) as u64);
                if self.next() & 1 == 0 {
                    v
                } else {
                    -v
                }
            }
        }
    }

    fn exponent(&mut self) -> f64 {
        match self.below(4) {
            0 => self.value(),
            1 => (self.below(41) - 20) as f64,
            2 => (self.below(41) - 20) as f64 + 0.5,
            _ => self.unit() * 10.0 - 5.0,
        }
    }
}

/// `Double.doubleToLongBits`: the bits, every `NaN` canonical.
fn long_bits(d: f64) -> i64 {
    if d.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        d.to_bits() as i64
    }
}

/// The per-chunk hashes of `f` over the inputs of `JavaMathOracle.unary`.
fn unary(index: i64, f: impl Fn(f64) -> f64) -> Vec<String> {
    let mut inputs: Vec<f64> = EDGES.to_vec();
    let mut g = Gen {
        state: (SEED + index) as u64,
    };
    inputs.extend((0..N).map(|_| g.value()));
    hashes(inputs.into_iter().map(f))
}

/// The per-chunk hashes of `f` over the inputs of `JavaMathOracle.binary`.
fn binary(index: i64, exponent: bool, f: impl Fn(f64, f64) -> f64) -> Vec<String> {
    let mut inputs = Vec::new();
    for x in EDGES {
        for y in EDGES {
            inputs.push((x, y));
        }
    }
    let mut g = Gen {
        state: (SEED + index) as u64,
    };
    for _ in 0..N {
        let x = g.value();
        let y = if exponent { g.exponent() } else { g.value() };
        inputs.push((x, y));
    }
    hashes(inputs.into_iter().map(|(x, y)| f(x, y)))
}

fn hashes(results: impl Iterator<Item = f64>) -> Vec<String> {
    let mut out = Vec::new();
    let mut h: i64 = 0;
    let mut count = 0;
    for r in results {
        h = h.wrapping_mul(31).wrapping_add(long_bits(r));
        count += 1;
        if count % CHUNK == 0 {
            out.push(format!("{count} {h:x}"));
            h = 0;
        }
    }
    if count % CHUNK != 0 {
        out.push(format!("{count} {h:x}"));
    }
    out
}

/// The oracle's chunks: per function, `(StrictMath, Math)` hashes per chunk,
/// and its `fn` line.
struct Oracle {
    strict: HashMap<String, Vec<String>>,
    math: HashMap<String, Vec<String>>,
    summary: HashMap<String, String>,
}

fn oracle() -> Oracle {
    let work = std::env::temp_dir().join(format!("rucene-java-math-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-math/JavaMathOracle.java");
    let output = jvm_fixture::launch(
        Command::new("java")
            .current_dir(&work)
            .arg(format!("-Djava.io.tmpdir={}", work.display()))
            // `java_math` reproduces `Math` without HotSpot's LIBM intrinsics
            // (its declared divergence, rmp #628).
            .arg("-XX:+UnlockDiagnosticVMOptions")
            .arg("-XX:-UseLibmIntrinsic")
            .arg(source)
            .arg(SEED.to_string())
            .arg(N.to_string()),
    )
    .output()
    .expect("a JDK is required to run the java.lang.Math oracle");
    let _ = std::fs::remove_dir_all(&work);
    assert!(
        output.status.success(),
        "JavaMathOracle.java failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut oracle = Oracle {
        strict: HashMap::new(),
        math: HashMap::new(),
        summary: HashMap::new(),
    };
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        let parts: Vec<&str> = line.split(' ').collect();
        match parts[0] {
            "chunk" => {
                let name = parts[1].to_string();
                oracle
                    .strict
                    .entry(name.clone())
                    .or_default()
                    .push(format!("{} {}", parts[2], parts[3]));
                oracle
                    .math
                    .entry(name)
                    .or_default()
                    .push(format!("{} {}", parts[2], parts[4]));
            }
            "fn" => {
                oracle
                    .summary
                    .insert(parts[1].to_string(), line.to_string());
            }
            _ => panic!("unexpected oracle line: {line}"),
        }
    }
    oracle
}

fn check(oracle: &Oracle, name: &str, side: &str, rust: &[String]) {
    let java = if side == "StrictMath" {
        &oracle.strict[name]
    } else {
        &oracle.math[name]
    };
    assert_eq!(java.len(), rust.len(), "{side}.{name}: chunk count");
    for (j, r) in java.iter().zip(rust) {
        assert_eq!(
            j, r,
            "{side}.{name}: the chunk ending at the input of this count differs ({})",
            oracle.summary[name]
        );
    }
}

#[test]
fn strict_math_and_java_math_equal_the_jdk() {
    let oracle = oracle();

    // StrictMath: every function, against java.lang.StrictMath.
    check(&oracle, "sin", "StrictMath", &unary(0, StrictMath::sin));
    check(&oracle, "cos", "StrictMath", &unary(1, StrictMath::cos));
    check(&oracle, "tan", "StrictMath", &unary(2, StrictMath::tan));
    check(&oracle, "asin", "StrictMath", &unary(3, StrictMath::asin));
    check(&oracle, "acos", "StrictMath", &unary(4, StrictMath::acos));
    check(&oracle, "atan", "StrictMath", &unary(5, StrictMath::atan));
    check(&oracle, "sqrt", "StrictMath", &unary(6, StrictMath::sqrt));
    check(
        &oracle,
        "atan2",
        "StrictMath",
        &binary(7, false, StrictMath::atan2),
    );
    check(
        &oracle,
        "pow",
        "StrictMath",
        &binary(8, true, StrictMath::pow),
    );
    check(
        &oracle,
        "IEEEremainder",
        "StrictMath",
        &binary(9, false, StrictMath::ieee_remainder),
    );
    check(&oracle, "exp", "StrictMath", &unary(10, StrictMath::exp));
    check(&oracle, "tanh", "StrictMath", &unary(11, StrictMath::tanh));

    // java_math: every function, against java.lang.Math (without the LIBM
    // intrinsics).
    check(&oracle, "sin", "Math", &unary(0, java_math::sin));
    check(&oracle, "cos", "Math", &unary(1, java_math::cos));
    check(&oracle, "tan", "Math", &unary(2, java_math::tan));
    check(&oracle, "asin", "Math", &unary(3, java_math::asin));
    check(&oracle, "acos", "Math", &unary(4, java_math::acos));
    check(&oracle, "atan", "Math", &unary(5, java_math::atan));
    check(&oracle, "sqrt", "Math", &unary(6, java_math::sqrt));
    check(
        &oracle,
        "atan2",
        "Math",
        &binary(7, false, java_math::atan2),
    );
    check(&oracle, "pow", "Math", &binary(8, true, java_math::pow));
    check(
        &oracle,
        "IEEEremainder",
        "Math",
        &binary(9, false, java_math::ieee_remainder),
    );
    check(&oracle, "exp", "Math", &unary(10, java_math::exp));
    check(&oracle, "tanh", "Math", &unary(11, java_math::tanh));
}
