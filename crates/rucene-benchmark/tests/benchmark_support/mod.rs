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

//! Helpers shared by the ports of the tests of Lucene 10.5.0
//! `lucene/benchmark/src/test` (sprint 17, rmp #633): the suite plumbing of
//! the sprint-9 core test ports (`core_support`, `search_support`), reused
//! by path, and the port of `org.apache.lucene.benchmark.BenchmarkTestCase`.

#![allow(dead_code, unused_imports)]

#[path = "../../../rucene-test-framework/tests/core_support/mod.rs"]
pub mod core_support;

#[path = "../../../rucene-test-framework/tests/core_search/support.rs"]
pub mod search_support;

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method};
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::error::Result;
use rucene_benchmark::by_task::benchmark::Benchmark;
use rucene_test_framework::util::lucene_test_case::{SuppressSysoutChecks, VERBOSE};
use rucene_test_framework::util::LuceneTestCase;

use search_support::static_hook;

/// The directory of the shipped test resources (`src/test` resources of
/// the Java module, by package).
pub fn resources_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/resources")
}

/// `getClass().getResourceAsStream(name)` of a class in `package_dir`
/// (`by_task`, `by_task/feeds`, `quality`): the bytes of the resource.
pub fn resource_bytes(package_dir: &str, name: &str) -> Vec<u8> {
    let path = resources_dir().join(package_dir).join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("resource {}: {e}", path.display()))
}

/// The directory of the shipped `lucene/benchmark/conf` (the `*.alg`
/// resources of `TestPerfTasksParse`).
pub fn conf_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/conf")
}

/// `private static Path WORKDIR`.
static WORKDIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// `WORKDIR` as the test framework's `Path`.
static WORKDIR_NIO: Mutex<Option<rucene::store::java_nio_file::Path>> = Mutex::new(None);

/// `getWorkDir()` as the test framework's `Path` (`newFSDirectory`
/// argument).
pub fn get_work_dir_nio() -> rucene::store::java_nio_file::Path {
    WORKDIR_NIO
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("beforeClassBenchmarkTestCase ran")
}

/// `beforeClassBenchmarkTestCase()`.
fn before_class_benchmark_test_case() -> Result<()> {
    let dir = LuceneTestCase::create_temp_dir_prefix("benchmark");
    *WORKDIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.as_std_path().to_path_buf());
    *WORKDIR_NIO.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir);
    Ok(())
}

/// `afterClassBenchmarkTestCase()`.
fn after_class_benchmark_test_case() -> Result<()> {
    *WORKDIR.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *WORKDIR_NIO.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

/// The class `org.apache.lucene.benchmark.BenchmarkTestCase`
/// (`@SuppressSysoutChecks(bugUrl = "very noisy") public abstract class
/// BenchmarkTestCase extends LuceneTestCase`).
fn benchmark_test_case_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.benchmark.BenchmarkTestCase")
            .extends(&LuceneTestCase::class())
            .annotate(SuppressSysoutChecks {
                bug_url: "very noisy".to_string(),
            })
            .method(static_hook(
                "beforeClassBenchmarkTestCase",
                BeforeClass,
                before_class_benchmark_test_case,
            ))
            .method(static_hook(
                "afterClassBenchmarkTestCase",
                AfterClass,
                after_class_benchmark_test_case,
            ))
            .build()
    });
    Arc::clone(&CLASS)
}

/// A test class `name` extending `BenchmarkTestCase`, its instance `F`.
pub fn benchmark_class<F: Any + Send + Default>(name: &str, methods: Vec<Method>) -> Arc<Class> {
    benchmark_class_annotated::<F>(name, methods, |b| b)
}

/// [`benchmark_class`] with class-level annotations.
pub fn benchmark_class_annotated<F: Any + Send + Default>(
    name: &str,
    methods: Vec<Method>,
    annotate: impl FnOnce(ClassBuilder) -> ClassBuilder,
) -> Arc<Class> {
    let mut builder = annotate(
        Class::builder(name)
            .extends(&benchmark_test_case_class())
            .constructor(Constructor::no_args(F::default)),
    );
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// `getWorkDir()`.
pub fn get_work_dir() -> PathBuf {
    WORKDIR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("beforeClassBenchmarkTestCase ran")
}

/// `copyToWorkDir(resourceName)` of a class whose resources are in
/// `package_dir`.
pub fn copy_to_work_dir(package_dir: &str, resource_name: &str) -> Result<()> {
    let target = get_work_dir().join(resource_name);
    if target.exists() {
        std::fs::remove_file(&target)?;
    }
    std::fs::write(&target, resource_bytes(package_dir, resource_name))?;
    Ok(())
}

/// `toAbsolutePath().toString().replace("\\", "/")`.
fn absolute_slashed(p: &Path) -> String {
    std::path::absolute(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

/// `getWorkDirResourcePath(resourceName)`.
pub fn get_work_dir_resource_path(resource_name: &str) -> String {
    absolute_slashed(&get_work_dir().join(resource_name))
}

/// `getWorkDirPath()`.
pub fn get_work_dir_path() -> String {
    absolute_slashed(&get_work_dir())
}

/// `NEW_LINE`: `System.getProperty("line.separator")`.
pub const NEW_LINE: &str = rucene_benchmark::jdk::NEW_LINE;

/// `propLines`: properties in effect in all tests here.
fn prop_lines() -> Vec<String> {
    vec![
        format!("work.dir={}", get_work_dir_path()),
        "directory=ByteBuffersDirectory".to_string(),
        "print.props=false".to_string(),
    ]
}

/// `algLinesToText(algLines)`: catenate alg lines to make the alg text.
fn alg_lines_to_text(alg_lines: &[String]) -> String {
    let indent = "  ";
    let mut sb = String::new();
    for line in prop_lines() {
        sb.push_str(indent);
        sb.push_str(&line);
        sb.push_str(NEW_LINE);
    }
    for line in alg_lines {
        sb.push_str(indent);
        sb.push_str(line);
        sb.push_str(NEW_LINE);
    }
    sb
}

/// `logTstLogic(txt)`.
fn log_tst_logic(txt: &str) {
    if !*VERBOSE {
        return;
    }
    println!("Test logic of:");
    println!("{txt}");
}

/// `execBenchmark(algLines)`: create the benchmark and execute it.
pub fn exec_benchmark<S: AsRef<str>>(alg_lines: &[S]) -> Result<Benchmark> {
    let lines: Vec<String> = alg_lines.iter().map(|s| s.as_ref().to_string()).collect();
    let alg_text = alg_lines_to_text(&lines);
    log_tst_logic(&alg_text);
    let benchmark = Benchmark::new(&alg_text)?;
    benchmark.execute()?;
    Ok(benchmark)
}

/// `new Properties()` → `new Config(props)` → `new PerfRunData(config)`.
pub fn run_data_of(
    props: rucene_benchmark::jdk::properties::Properties,
) -> Result<Arc<rucene_benchmark::by_task::perf_run_data::PerfRunData>> {
    let config = Arc::new(rucene_benchmark::by_task::utils::config::Config::from_properties(props));
    rucene_benchmark::by_task::perf_run_data::PerfRunData::new(config)
}

/// `expectThrows(NoMoreDataException.class, runnable)` over the result of
/// the runnable.
pub fn expect_no_more_data_exception<T>(result: Result<T>) {
    use rucene_benchmark::by_task::feeds::no_more_data_exception::NoMoreDataException;
    match result {
        Err(e) if NoMoreDataException::is(&e) => {}
        Err(e) => rucene_test_framework::util::java_exceptions::rethrow_java(e),
        Ok(_) => randomizedtesting::junit::Assert::fail(
            "Expected exception NoMoreDataException but no exception was thrown",
        ),
    }
}

/// Java's `String.split(regex)` of a one-character literal separator:
/// trailing empty strings are removed.
pub fn java_split(s: &str, sep: char) -> Vec<String> {
    let mut parts: Vec<String> = s.split(sep).map(str::to_string).collect();
    while parts.len() > 1 && parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    if parts.len() == 1 && parts[0].is_empty() && !s.is_empty() {
        parts.clear();
    }
    parts
}

/// A `DocMaker` subclass overriding `makeDocument()`.
pub struct FnDocMaker {
    dm: rucene_benchmark::by_task::feeds::doc_maker::DocMaker,
    make: Box<dyn Fn() -> Result<rucene::document::Document> + Send + Sync>,
}

impl FnDocMaker {
    /// A `DocMaker` whose `makeDocument()` is `make`.
    pub fn new(
        make: impl Fn() -> Result<rucene::document::Document> + Send + Sync + 'static,
    ) -> Self {
        FnDocMaker {
            dm: rucene_benchmark::by_task::feeds::doc_maker::DocMaker::new(),
            make: Box::new(make),
        }
    }

    /// Puts the class `name` on the "classpath".
    pub fn register(name: &str, make: fn() -> Result<rucene::document::Document>) {
        rucene_benchmark::classes::register_doc_maker_class(
            name,
            Arc::new(move || {
                Arc::new(FnDocMaker::new(make))
                    as Arc<dyn rucene_benchmark::by_task::feeds::doc_maker::DocMakerClass>
            }),
        );
    }
}

impl rucene_benchmark::by_task::feeds::doc_maker::DocMakerClass for FnDocMaker {
    fn doc_maker(&self) -> &rucene_benchmark::by_task::feeds::doc_maker::DocMaker {
        &self.dm
    }

    fn make_document(&self) -> Result<rucene::document::Document> {
        (self.make)()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Reads all lines of a UTF-8 text stream (`BufferedReader.readLine` until
/// `null`).
pub fn read_lines(input: impl std::io::Read) -> Result<Vec<String>> {
    use std::io::BufRead;
    let mut out = Vec::new();
    for line in std::io::BufReader::new(input).lines() {
        out.push(line?);
    }
    Ok(out)
}

/// `assertEquals(message, expected, actual)`.
pub fn assert_equals_msg<T: PartialEq + std::fmt::Display>(message: &str, expected: T, actual: T) {
    randomizedtesting::junit::Assert::assert_equals_msg(Some(message), expected, actual)
}

/// `assertEquals(message, expected, actual, delta)` of doubles.
pub fn assert_equals_delta_msg(message: &str, expected: f64, actual: f64, delta: f64) {
    randomizedtesting::junit::Assert::assert_equals_double_msg(
        Some(message),
        expected,
        actual,
        delta,
    )
}
