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

//! The Java-oracle harness of the sprint-17 modules (rmp #628): Lucene
//! 10.5.0 `memory`, `facet`, `highlighter`, `spatial3d`, `spatial-extras`,
//! `benchmark` and `analysis/icu` run side by side with Rucene on one seed, and their dumps are
//! compared line for line.
//!
//! It follows the harness of the query modules (rmp #475,
//! `crates/rucene-queries/tests/query_oracle/mod.rs`): a single-source Java
//! program, launched through [`jvm_fixture::launch`] (compiled once per change
//! of its source), in a working directory of its own under `TMPDIR`
//! ([`WorkDir`]), so comparisons run in parallel safely; every jar is SHA-1
//! checked against the value Maven Central publishes ([`class_path`]); both
//! sides run under one `tests.seed` ([`pin_tests_seed`]).
//!
//! **Use.** A test of any crate includes this module with
//! `#[path = "../../../tests/sprint17_oracle/mod.rs"] mod sprint17_oracle;`
//! (it depends only on `rucene` and `randomizedtesting`), computes Rucene's
//! dump from a seed with `java.util.Random`-equal draws
//! (`randomizedtesting::jdk::random::JdkRandom`, and the [`corpus`] helpers),
//! and asserts it equal to [`run`]'s:
//!
//! ```ignore
//! let java = sprint17_oracle::run(Module::Memory, "smoke", SEED, &[]);
//! sprint17_oracle::assert_same(&java, &rust_dump);
//! ```
//!
//! **Java side.** One program per module,
//! `tests/fixtures/sprint17-oracle/<Module>Oracle.java`; its `main` receives
//! `seed scenario input args…` (`input` is the path of the text [`run_with_input`]
//! wrote, or `-`) and prints the scenario's dump (UTF-8) on standard output.
//! A scenario is a `case` of its `switch`; new parity tests add scenarios.
//! The `java.util.Random(seed)` draws of the Java side equal
//! `JdkRandom::new(seed)`'s.

#![allow(dead_code)]

#[path = "../jvm_fixture/mod.rs"]
pub mod jvm_fixture;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use rucene::util::java_collections::java_string_hash_code;
use rucene::util::StringHelper;

/// A module whose Lucene 10.5.0 oracle the harness runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    /// `lucene/memory`.
    Memory,
    /// `lucene/facet`.
    Facet,
    /// `lucene/highlighter`.
    Highlighter,
    /// `lucene/spatial3d`.
    Spatial3d,
    /// `lucene/spatial-extras`.
    SpatialExtras,
    /// `lucene/benchmark`.
    Benchmark,
    /// `lucene/analysis/icu` (over ICU4J 77.1).
    AnalysisIcu,
}

/// A jar of the local Maven repository: its path under
/// `~/.m2/repository` and the SHA-1 Maven Central publishes for it.
pub struct Jar {
    /// The path under `~/.m2/repository`.
    pub path: &'static str,
    /// The published SHA-1.
    pub sha1: &'static str,
}

macro_rules! jar {
    ($path:literal, $sha1:literal) => {
        Jar {
            path: $path,
            sha1: $sha1,
        }
    };
}

static CORE: Jar = jar!(
    "org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar",
    "383deeee9b44ef15daaf7541d57ea5183d52a7b5"
);
static CODECS: Jar = jar!(
    "org/apache/lucene/lucene-codecs/10.5.0/lucene-codecs-10.5.0.jar",
    "d128c42988203efb7c3a2c7fb8beb788cb578dae"
);
static ANALYSIS_COMMON: Jar = jar!(
    "org/apache/lucene/lucene-analysis-common/10.5.0/lucene-analysis-common-10.5.0.jar",
    "2b314f57f18f3d9ad855f611f6f391519ac78a8d"
);
static QUERIES: Jar = jar!(
    "org/apache/lucene/lucene-queries/10.5.0/lucene-queries-10.5.0.jar",
    "c96555ec32a411d860527ea13add388d72a614b2"
);
static QUERYPARSER: Jar = jar!(
    "org/apache/lucene/lucene-queryparser/10.5.0/lucene-queryparser-10.5.0.jar",
    "dea0c58101825f086127abf5adce13ced0bd0fc2"
);
static TEST_FRAMEWORK: Jar = jar!(
    "org/apache/lucene/lucene-test-framework/10.5.0/lucene-test-framework-10.5.0.jar",
    "fbca09b1ae940d26b6c3a90befecda40d5f1c3c2"
);
static RANDOMIZEDTESTING: Jar = jar!(
    "com/carrotsearch/randomizedtesting/randomizedtesting-runner/2.8.1/randomizedtesting-runner-2.8.1.jar",
    "55ffe691e90d31ab916746516654b5701e532d6f"
);
static JUNIT: Jar = jar!(
    "junit/junit/4.13.1/junit-4.13.1.jar",
    "cdd00374f1fee76b11e2a9d127405aa3f6be5b6a"
);
static HAMCREST: Jar = jar!(
    "org/hamcrest/hamcrest/2.2/hamcrest-2.2.jar",
    "1820c0968dba3a11a1b30669bb1f01978a91dedc"
);
static MEMORY: Jar = jar!(
    "org/apache/lucene/lucene-memory/10.5.0/lucene-memory-10.5.0.jar",
    "a18c7a57e054836d2dc7ece7dfe28a9cad61f660"
);
static HIGHLIGHTER: Jar = jar!(
    "org/apache/lucene/lucene-highlighter/10.5.0/lucene-highlighter-10.5.0.jar",
    "fccffbe10fa6fd84bc884df844eb850ffcc8d51b"
);
static FACET: Jar = jar!(
    "org/apache/lucene/lucene-facet/10.5.0/lucene-facet-10.5.0.jar",
    "cd4d1febfe5c3afa1d5baf8e2e33425784d0a7a6"
);
static SPATIAL3D: Jar = jar!(
    "org/apache/lucene/lucene-spatial3d/10.5.0/lucene-spatial3d-10.5.0.jar",
    "e2a6601ca2d16796a068589128787114d15935c3"
);
static SPATIAL_EXTRAS: Jar = jar!(
    "org/apache/lucene/lucene-spatial-extras/10.5.0/lucene-spatial-extras-10.5.0.jar",
    "900c6d78202cfd96a8e513f2271c972c4af98388"
);
static BENCHMARK: Jar = jar!(
    "org/apache/lucene/lucene-benchmark/10.5.0/lucene-benchmark-10.5.0.jar",
    "0a04a5a2ea54664dd39b89df6ee25c864bb2599d"
);
static ANALYSIS_ICU: Jar = jar!(
    "org/apache/lucene/lucene-analysis-icu/10.5.0/lucene-analysis-icu-10.5.0.jar",
    "cb67d073b767ed1d1d24a9b03117ef9bbac5c1aa"
);
static SPATIAL4J: Jar = jar!(
    "org/locationtech/spatial4j/spatial4j/0.8/spatial4j-0.8.jar",
    "f3ff59c2c540102ae157a1786cb80e82ce9c9f19"
);
static S2: Jar = jar!(
    "io/sgr/s2-geometry-library-java/1.0.0/s2-geometry-library-java-1.0.0.jar",
    "f95b25589b40b5b0965deb592445073ff3efa299"
);
static COMMONS_COMPRESS: Jar = jar!(
    "org/apache/commons/commons-compress/1.19/commons-compress-1.19.jar",
    "7e65777fb451ddab6a9c054beb879e521b7eab78"
);
static ICU4J: Jar = jar!(
    "com/ibm/icu/icu4j/77.1/icu4j-77.1.jar",
    "38693cf0b1d7362a8b726af74dc06026a7c23809"
);
static NEKOHTML: Jar = jar!(
    "net/sourceforge/nekohtml/nekohtml/1.9.17/nekohtml-1.9.17.jar",
    "39a870b0ea4cb0d2a3015c1ab569d17d83122d55"
);
static XERCES: Jar = jar!(
    "xerces/xercesImpl/2.12.2/xercesImpl-2.12.2.jar",
    "f051f988aa2c9b4d25d05f95742ab0cc3ed789e2"
);

/// The test framework and its runtime dependencies (`MockAnalyzer`,
/// `MockTokenizer`, … for the oracles that mirror a Lucene test).
static TEST_JARS: [&Jar; 5] = [
    &TEST_FRAMEWORK,
    &CODECS,
    &RANDOMIZEDTESTING,
    &JUNIT,
    &HAMCREST,
];

impl Module {
    /// Every module, in the order of the sprint.
    pub const ALL: [Module; 7] = [
        Module::Memory,
        Module::Facet,
        Module::Highlighter,
        Module::Spatial3d,
        Module::SpatialExtras,
        Module::Benchmark,
        Module::AnalysisIcu,
    ];

    /// The name of the module's Java program (without `.java`).
    pub fn program(self) -> &'static str {
        match self {
            Module::Memory => "MemoryOracle",
            Module::Facet => "FacetOracle",
            Module::Highlighter => "HighlighterOracle",
            Module::Spatial3d => "Spatial3dOracle",
            Module::SpatialExtras => "SpatialExtrasOracle",
            Module::Benchmark => "BenchmarkOracle",
            Module::AnalysisIcu => "AnalysisIcuOracle",
        }
    }

    /// The jars of the module's class path: Lucene core, the module and its
    /// runtime dependencies as its 10.5.0 POM declares them, then the test
    /// framework ([`TEST_JARS`]).
    pub fn jars(self) -> Vec<&'static Jar> {
        let module: Vec<&'static Jar> = match self {
            Module::Memory => vec![&CORE, &MEMORY, &ANALYSIS_COMMON],
            Module::Facet => vec![&CORE, &FACET, &ANALYSIS_COMMON],
            Module::Highlighter => {
                vec![&CORE, &HIGHLIGHTER, &QUERIES, &MEMORY, &ANALYSIS_COMMON]
            }
            Module::Spatial3d => vec![&CORE, &SPATIAL3D],
            Module::SpatialExtras => vec![&CORE, &SPATIAL_EXTRAS, &SPATIAL3D, &SPATIAL4J, &S2],
            Module::Benchmark => vec![
                &CORE,
                &BENCHMARK,
                &ANALYSIS_COMMON,
                &FACET,
                &HIGHLIGHTER,
                &MEMORY,
                &QUERIES,
                &QUERYPARSER,
                &SPATIAL_EXTRAS,
                &SPATIAL3D,
                &SPATIAL4J,
                &S2,
                &COMMONS_COMPRESS,
                &ICU4J,
                &ANALYSIS_ICU,
                &NEKOHTML,
                &XERCES,
            ],
            Module::AnalysisIcu => vec![&CORE, &ANALYSIS_COMMON, &ANALYSIS_ICU, &ICU4J],
        };
        module
            .into_iter()
            .chain(TEST_JARS.iter().copied())
            .collect()
    }

    /// The module's Java program.
    pub fn source(self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/sprint17-oracle")
            .join(format!("{}.java", self.program()))
    }
}

/// The class path of `module`: each jar of [`Module::jars`] under
/// `~/.m2/repository`, checked once per process: its SHA-1 must equal both
/// the pinned value and the `.sha1` file Maven downloaded beside it.
///
/// # Panics
///
/// When a jar is missing or its digest differs.
pub fn class_path(module: Module) -> Vec<PathBuf> {
    module.jars().into_iter().map(verified).collect()
}

fn verified(jar: &'static Jar) -> PathBuf {
    static CHECKED: OnceLock<std::sync::Mutex<Vec<&'static str>>> = OnceLock::new();
    let home = std::env::var("HOME").expect("HOME is set");
    let path = Path::new(&home).join(".m2/repository").join(jar.path);
    let checked = CHECKED.get_or_init(Default::default);
    if checked.lock().unwrap().contains(&jar.path) {
        return path;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "the oracle jar is required (see crates/rucene-test-framework/README.md): {}: {e}",
            path.display()
        )
    });
    let digest = sha1(&bytes);
    assert_eq!(digest, jar.sha1, "SHA-1 of {}", path.display());
    let sidecar = path.with_extension("jar.sha1");
    let published =
        std::fs::read_to_string(&sidecar).unwrap_or_else(|e| panic!("{}: {e}", sidecar.display()));
    let published = published.split_whitespace().next().unwrap_or("");
    assert_eq!(
        digest,
        published,
        "SHA-1 of {} vs its .sha1",
        path.display()
    );
    checked.lock().unwrap().push(jar.path);
    path
}

/// SHA-1 (FIPS 180-4) in lower-case hexadecimal.
pub fn sha1(data: &[u8]) -> String {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let mut message = data.to_vec();
    let bit_length = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

/// The `tests.seed` both sides run under when the environment sets none.
pub const HASH_SEED: &str = "DEADBEEF";

/// Pins the `tests.seed` both sides run under and returns it: the
/// environment's `tests.seed`, else [`HASH_SEED`] (set in the environment,
/// where Rucene reads it). The JVM gets it as `-Dtests.seed`.
///
/// Lucene seeds `StringHelper.GOOD_FAST_HASH_SEED` (the `BytesRef` hash, so
/// the iteration order of `BytesRefHash`-backed sets) from that property
/// (`StringHelper.java:143`); one seed on both sides makes those orders equal.
///
/// # Panics
///
/// When `GOOD_FAST_HASH_SEED` was computed before the pin.
pub fn pin_tests_seed() -> String {
    static SEED: OnceLock<String> = OnceLock::new();
    SEED.get_or_init(|| {
        let seed = std::env::var("tests.seed").unwrap_or_else(|_| {
            std::env::set_var("tests.seed", HASH_SEED);
            HASH_SEED.to_string()
        });
        assert_eq!(
            StringHelper::good_fast_hash_seed(),
            java_string_hash_code(&seed),
            "GOOD_FAST_HASH_SEED was computed before the harness pinned tests.seed"
        );
        seed
    })
    .clone()
}

/// A directory of its own under `TMPDIR`, removed on drop.
pub struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    /// A new, empty directory named after `prefix`, the process and a
    /// counter.
    pub fn new(prefix: &str) -> WorkDir {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let path = std::env::temp_dir().join(format!(
            "rucene-{prefix}-{}-{}-{nanos}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create the work directory");
        WorkDir { path }
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Runs `scenario` of `module`'s oracle with `seed` and `args` and returns
/// its dump.
///
/// # Panics
///
/// When the JVM cannot run or the oracle fails.
pub fn run(module: Module, scenario: &str, seed: i64, args: &[&str]) -> String {
    run_inner(module, scenario, seed, args, None)
}

/// [`run`] with `input` written to a file of the working directory, whose
/// path the oracle receives as its `input` argument.
///
/// # Panics
///
/// As [`run`].
pub fn run_with_input(
    module: Module,
    scenario: &str,
    seed: i64,
    args: &[&str],
    input: &str,
) -> String {
    run_inner(module, scenario, seed, args, Some(input))
}

fn run_inner(
    module: Module,
    scenario: &str,
    seed: i64,
    args: &[&str],
    input: Option<&str>,
) -> String {
    let tests_seed = pin_tests_seed();
    let class_path = std::env::join_paths(class_path(module)).expect("a class path");
    let work = WorkDir::new("s17-oracle");
    let input_arg = match input {
        Some(text) => {
            let path = work.path().join("input.txt");
            std::fs::write(&path, text).expect("write the input file");
            path.display().to_string()
        }
        None => "-".to_string(),
    };
    let mut command = Command::new("java");
    command
        .current_dir(work.path())
        .arg(format!("-Djava.io.tmpdir={}", work.path().display()))
        .arg("-Xmx1g")
        // Lucene's test JVMs run with assertions on (`tests.asserts`,
        // `gradle/testing/randomization.gradle:76,182-183`), as Rucene's test
        // builds evaluate Java's `assert`.
        .arg("-ea")
        .arg("-esa")
        // `Math.sin`, `cos`, `tan` and `pow` as fdlibm computes them, which
        // `rucene::util::java_math` reproduces (its declared divergence from
        // HotSpot's x86_64 LIBM intrinsics, rmp #628).
        .arg("-XX:+UnlockDiagnosticVMOptions")
        .arg("-XX:-UseLibmIntrinsic")
        .arg(format!("-Dtests.seed={tests_seed}"))
        .arg("-cp")
        .arg(class_path)
        .arg(module.source())
        .arg(seed.to_string())
        .arg(scenario)
        .arg(input_arg)
        .args(args);
    let output = jvm_fixture::launch(&command)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "{}.java {scenario} failed:\n{}",
        module.program(),
        java_errors(&String::from_utf8_lossy(&output.stderr))
    );
    String::from_utf8(output.stdout).expect("the oracle prints UTF-8")
}

/// The stderr lines of a Java run without the JVM's start-up warnings.
pub fn java_errors(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| {
            !(l.starts_with("WARNING:")
                || l.contains("VectorizationProvider lookup")
                || l.trim().is_empty())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `escape` of the Java programs: printable ASCII except the backslash as
/// is, every other UTF-16 unit as `\uXXXX`.
pub fn escape(s: &str) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    escape_utf16(&units)
}

/// `escape` of a Java String held as UTF-16 code units (an unpaired
/// surrogate is printed as its `\uXXXX` code unit, as Java prints it).
pub fn escape_utf16(s: &[u16]) -> String {
    let mut out = String::new();
    for &unit in s {
        if (0x20..=0x7e).contains(&unit) && unit != u16::from(b'\\') {
            out.push(char::from(unit as u8));
        } else {
            out.push_str(&format!("\\u{unit:04X}"));
        }
    }
    out
}

/// `bits(double)` of the Java programs: `Long.toHexString(Double.doubleToRawLongBits(d))`.
pub fn bits64(d: f64) -> String {
    format!("{:x}", d.to_bits())
}

/// `bits(float)` of the Java programs: `Integer.toHexString(Float.floatToRawIntBits(f))`.
pub fn bits32(f: f32) -> String {
    format!("{:x}", f.to_bits())
}

/// Compares two dumps line by line: the first differing line with its
/// number, both lines and the line before, or the differing line counts.
pub fn compare(java: &str, rust: &str) -> Result<(), String> {
    let (j, r): (Vec<&str>, Vec<&str>) = (java.lines().collect(), rust.lines().collect());
    for i in 0..j.len().max(r.len()) {
        let (jl, rl) = (j.get(i).copied(), r.get(i).copied());
        if jl != rl {
            let before = if i > 0 { j[i - 1] } else { "" };
            return Err(format!(
                "dumps differ at line {}:\n  before {before}\n  Java   {}\n  Rust   {}\n\
                 (Java {} lines, Rust {} lines)",
                i + 1,
                jl.unwrap_or("<none>"),
                rl.unwrap_or("<none>"),
                j.len(),
                r.len()
            ));
        }
    }
    Ok(())
}

/// Asserts that the dumps are equal and not empty ([`compare`]).
///
/// # Panics
///
/// When they differ or the Java dump is empty.
pub fn assert_same(java: &str, rust: &str) {
    assert!(!java.trim().is_empty(), "the Java oracle printed nothing");
    if let Err(e) = compare(java, rust) {
        panic!("{e}");
    }
}

/// Seeded generators both sides share. Each Java program carries the same
/// functions (`Corpus` in `<Module>Oracle.java`), drawing from
/// `java.util.Random` in the same order.
pub mod corpus {
    use randomizedtesting::jdk::random::Random;

    /// The words of the generated texts, most frequent first.
    pub const VOCABULARY: [&str; 30] = [
        "the", "of", "and", "to", "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
        "hotel", "india", "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec",
        "romeo", "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
    ];

    /// `Corpus.word(r)`: `VOCABULARY[r.nextInt(30)]`.
    pub fn word(r: &dyn Random) -> &'static str {
        VOCABULARY[r.next_int_bound(VOCABULARY.len() as i32) as usize]
    }

    /// `Corpus.text(r, n)`: `n` [`word`]s joined by single spaces.
    pub fn text(r: &dyn Random, n: usize) -> String {
        let mut out = String::new();
        for i in 0..n {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(word(r));
        }
        out
    }
}
