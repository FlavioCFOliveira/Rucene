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

//! The executables of the five `lucene/benchmark` main classes (rmp #639),
//! run as processes.
//!
//! * `benchmark` runs the shipped `tests/resources/bin/line-docs.alg` over the
//!   `by_task` test resource `reuters.first20.lines.txt` (as
//!   `TestPerfTasksLogic` runs algorithms over it) and prints the report.
//! * Every executable run without arguments prints what the Java class prints
//!   (captured from `java -cp <lucene-benchmark 10.5.0 and its dependencies>
//!   org.apache.lucene.benchmark.<Class>` on JDK 25) and exits with Java's
//!   status.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh working directory for one process, under the target's test
/// temporary directory.
fn work_dir(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "executables-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create work dir");
    dir
}

/// Runs `exe` with `args` in `cwd`, with Rust backtraces disabled so that
/// an uncaught exception prints no `\tat` frame.
fn run(exe: &str, args: &[&str], cwd: &Path) -> Output {
    Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .output()
        .unwrap_or_else(|e| panic!("run {exe}: {e}"))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("UTF-8 output")
}

/// The `name round runCnt recsPerRun` columns of the rows of the
/// `RepSumByName` report in `stdout` (the remaining columns are timings and
/// memory, which vary per run); the `-  -` padding of alternate rows is
/// dropped.
fn report_rows(stdout: &str) -> Vec<String> {
    let mut lines = stdout
        .lines()
        .skip_while(|l| !l.starts_with("Operation        round"));
    assert!(lines.next().is_some(), "no report header in:\n{stdout}");
    lines
        .take_while(|l| !l.is_empty())
        .map(|l| {
            l.split_whitespace()
                .filter(|t| *t != "-")
                .take(4)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[test]
fn benchmark_runs_an_algorithm_file() {
    let dir = work_dir("benchmark");
    let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/resources");
    std::fs::copy(
        resources.join("by_task/reuters.first20.lines.txt"),
        dir.join("reuters.first20.lines.txt"),
    )
    .expect("copy line file");
    let alg = std::path::absolute(resources.join("bin/line-docs.alg")).expect("absolute");

    let out = run(
        env!("CARGO_BIN_EXE_benchmark"),
        &[alg.to_str().expect("UTF-8 path")],
        &dir,
    );
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(stderr, "");

    // Everything before the report is deterministic: equal to Java's output.
    let expected_head = format!(
        "Running algorithm from: {}
------------> algorithm:
Seq {{
    ResetSystemErase
    Populate {{
        CreateIndex
        MAddDocs_Exhaust {{
            AddDoc
        > * EXHAUST
        ForceMerge(1.0)
        CloseIndex
    }}
    OpenReader
    SearchSameRdr_10 {{
        Search
    > * 10
    CloseReader
    RepSumByName
}}

------------> starting task: Seq
------------> starting task: Populate
------------> starting task: OpenReader
------------> starting task: SearchSameRdr_10
------------> starting task: CloseReader

------------> Report Sum By (any) Name (8 about 8 out of 9)
Operation        round   runCnt   recsPerRun        rec/s  elapsedSec    avgUsedMem    avgTotalMem
",
        alg.display()
    );
    assert!(
        stdout.starts_with(&expected_head),
        "stdout:\n{stdout}\nexpected to start with:\n{expected_head}"
    );
    assert_eq!(
        report_rows(&stdout),
        [
            "Populate 0 1 23",
            "CreateIndex 0 1 1",
            "MAddDocs_Exhaust 0 1 20",
            "ForceMerge(1.0) 0 1 1",
            "CloseIndex 0 1 1",
            "OpenReader 0 1 1",
            "SearchSameRdr_10 0 1 10",
            "CloseReader 0 1 1",
        ]
    );
    assert!(
        stdout.ends_with("\n\n####################\n###  D O N E !!! ###\n####################\n"),
        "stdout:\n{stdout}"
    );
    // The index the algorithm wrote under `work.dir`.
    assert!(
        std::fs::read_dir(dir.join("work/index"))
            .expect("work/index")
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with("segments_")),
        "no commit in work/index"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs `exe` without arguments; asserts Java's exit status 1, stdout and
/// stderr.
fn assert_no_args(name: &str, exe: &str, stdout: &str, stderr: &str) {
    let dir = work_dir(name);
    let out = run(exe, &[], &dir);
    assert_eq!(text(&out.stdout), stdout, "{name} stdout");
    assert_eq!(text(&out.stderr), stderr, "{name} stderr");
    assert_eq!(out.status.code(), Some(1), "{name} exit status");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn benchmark_without_arguments_prints_usage() {
    assert_no_args(
        "benchmark",
        env!("CARGO_BIN_EXE_benchmark"),
        "",
        "Usage: java Benchmark <algorithm file>\n",
    );
}

#[test]
fn query_driver_without_arguments_prints_usage() {
    assert_no_args(
        "query_driver",
        env!("CARGO_BIN_EXE_query_driver"),
        "",
        "Usage: QueryDriver <topicsFile> <qrelsFile> <submissionFile> <indexDir> [querySpec]
topicsFile: input file containing queries
qrelsFile: input file containing relevance judgements
submissionFile: output submission file for trec_eval
indexDir: index directory
querySpec: string composed of fields to use in query consisting of T=title,D=description,N=narrative:
\texample: TD (query on Title + Description). The default is T (title only)
",
    );
}

#[test]
fn quality_queries_finder_without_arguments_prints_usage() {
    assert_no_args(
        "quality_queries_finder",
        env!("CARGO_BIN_EXE_quality_queries_finder"),
        "",
        "Usage: java QualityQueriesFinder <index-dir>\n",
    );
}

/// Java's `main` dereferences the `null` input path before it can print its
/// usage: the uncaught `NullPointerException` of thread `main`.
#[test]
fn extract_wikipedia_without_arguments_throws_null_pointer_exception() {
    assert_no_args(
        "extract_wikipedia",
        env!("CARGO_BIN_EXE_extract_wikipedia"),
        "",
        "Exception in thread \"main\" java.lang.NullPointerException: Cannot invoke \
         \"java.nio.file.Path.toAbsolutePath()\" because \"wikipedia\" is null\n",
    );
}

/// `Sample.initProps()` names `ReutersContentSource` as `doc.maker`: Java's
/// `PerfRunData` prints the properties, then `Class.asSubclass(DocMaker)`
/// throws the uncaught `ClassCastException`.
#[test]
fn sample_throws_class_cast_exception() {
    assert_no_args(
        "sample",
        env!("CARGO_BIN_EXE_sample"),
        "------------> config properties:
analyzer = org.apache.lucene.analysis.standard.StandardAnalyzer
compound = cmpnd:true:true:true:true:false:false:false:false
directory = FSDirectory
doc.delete.step = 8
doc.maker = org.apache.lucene.benchmark.byTask.feeds.ReutersContentSource
doc.stored = true
doc.term.vector = false
doc.tokenized = true
docs.dir = reuters-out
log.step = 2000
max.buffered = buf:10:10:100:100:10:10:100:100
merge.factor = mrg:10:100:10:100:10:100:10:100
query.maker = org.apache.lucene.benchmark.byTask.feeds.ReutersQueryMaker
task.max.depth.log = 3
-------------------------------
",
        "Exception in thread \"main\" java.lang.ClassCastException: class \
         org.apache.lucene.benchmark.byTask.feeds.ReutersContentSource\n",
    );
}
