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

//! Java-oracle portability tests of `lucene/benchmark` (rmp #633, harness of
//! rmp #628): Lucene 10.5.0 (`BenchmarkOracle.java`) and Rucene give equal
//! dumps for `.alg` parsing (`Algorithm.toString()` of every shipped
//! `conf/*.alg`), `DemoHTMLParser` on the s611 HTML corpus, TREC and
//! line-doc parsing, and the report text of a small run; the root ICU
//! spellout of `LongToEnglish*` equals the Java table of rmp #611.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rucene_benchmark::by_task::benchmark::Benchmark;
use rucene_benchmark::by_task::feeds::content_items_source::ContentItemsSource;
use rucene_benchmark::by_task::feeds::content_source::ContentSource;
use rucene_benchmark::by_task::feeds::demo_html_parser::Parser;
use rucene_benchmark::by_task::feeds::doc_data::DocData;
use rucene_benchmark::by_task::feeds::line_doc_source::LineDocSource;
use rucene_benchmark::by_task::feeds::no_more_data_exception::NoMoreDataException;
use rucene_benchmark::by_task::feeds::trec_content_source::TrecContentSource;
use rucene_benchmark::by_task::perf_run_data::PerfRunData;
use rucene_benchmark::by_task::stats::task_stats::TaskStats;
use rucene_benchmark::by_task::tasks::rep_sum_by_name_task::RepSumByNameTask;
use rucene_benchmark::by_task::utils::algorithm::Algorithm;
use rucene_benchmark::by_task::utils::config::Config;
use rucene_benchmark::icu::RuleBasedNumberFormat;
use rucene_benchmark::jdk::properties::Properties;
use sprint17_oracle::{escape, Module, WorkDir};

const SEED: i64 = 0x0633_0001;

/// The s611 oracle data (rmp #611): `spell.java.tsv` and `html/`.
fn s611() -> PathBuf {
    PathBuf::from("/data/dev/github.com/FlavioCFOliveira/.rucene-test-tmp/s611")
}

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn abs(p: &Path) -> String {
    std::path::absolute(p).unwrap().to_string_lossy().into_owned()
}

#[test]
fn benchmark_alg_parse_oracle() {
    sprint17_oracle::pin_tests_seed();
    let conf = manifest().join("tests/conf");
    let work = WorkDir::new("s17b-alg");
    let line_file = work.path().join("linefile.txt");
    let java = sprint17_oracle::run(
        Module::Benchmark,
        "alg",
        SEED,
        &[&abs(&conf), &abs(work.path()), &abs(&line_file)],
    );
    let mut files: Vec<PathBuf> = std::fs::read_dir(&conf)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("alg"))
        .collect();
    files.sort();
    let mut out = String::new();
    for f in &files {
        let config = Arc::new(Config::from_reader(&std::fs::read_to_string(f).unwrap()).unwrap());
        config.set("work.dir", &abs(work.path())).unwrap();
        config.set("content.source", "org.apache.lucene.benchmark.byTask.feeds.SingleDocSource").unwrap();
        config.set("directory", "ByteBuffersDirectory").unwrap();
        if config.get_str("line.file.out", None).unwrap().is_some() {
            config.set("line.file.out", &abs(&line_file)).unwrap();
        }
        if config.get_str("query.maker", None).unwrap().is_some() {
            config.set("query.maker", "org.apache.lucene.benchmark.byTask.feeds.SimpleQueryMaker").unwrap();
        }
        let alg = Algorithm::new(PerfRunData::new(config).unwrap()).unwrap();
        out.push_str(&format!("ALG {}\n{}\n", f.file_name().unwrap().to_string_lossy(), escape(&alg.to_string())));
        alg.close().unwrap();
    }
    sprint17_oracle::assert_same(&java, &out);
}

/// ISO-8859-1 text of a file (each byte one char).
fn latin1(p: &Path) -> String {
    std::fs::read(p).unwrap().iter().map(|&b| b as char).collect()
}

#[test]
fn benchmark_html_parser_oracle() {
    sprint17_oracle::pin_tests_seed();
    let dir = s611().join("html");
    let java = sprint17_oracle::run(Module::Benchmark, "html", SEED, &[&abs(&dir)]);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().ends_with(".html"))
        .collect();
    files.sort();
    let mut out = String::new();
    for f in &files {
        out.push_str(&format!("FILE {}\n", f.file_name().unwrap().to_string_lossy()));
        let p = match Parser::from_reader(&latin1(f)) {
            Ok(p) => p,
            Err(e) => {
                out.push_str(&format!("ERROR {}\n", java_exception(&e)));
                continue;
            }
        };
        out.push_str(&format!("TITLE {}\nBODY {}\n", escape(&p.title), escape(&p.body)));
        let mut meta: Vec<(String, String)> =
            p.meta_tags.entries().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        meta.sort_by(|a, b| sprint17_oracle_compare(&a.0, &b.0));
        for (k, v) in meta {
            out.push_str(&format!("META {}={}\n", escape(&k), escape(&v)));
        }
    }
    assert_eq!(files.len(), 49, "the s611 corpus has 49 documents");
    // Every differing line (not only the first), so one run shows the whole
    // divergence.
    let (jl, rl): (Vec<&str>, Vec<&str>) = (java.lines().collect(), out.lines().collect());
    assert_eq!(jl.len(), rl.len(), "line counts differ");
    let diffs: Vec<String> = jl
        .iter()
        .zip(&rl)
        .enumerate()
        .filter(|(_, (j, r))| j != r)
        .map(|(i, (j, r))| format!("line {}: java {j} | rust {r} (after {})", i + 1, jl[i.saturating_sub(2)]))
        .collect();
    assert!(diffs.is_empty(), "{} lines differ:\n{}", diffs.len(), diffs.join("\n"));
}

/// `e.getClass().getName() + ": " + escape(e.getMessage())` of a parser
/// exception.
fn java_exception(e: &nekohtml::java::Exception) -> String {
    format!("{}: {}", e.class_name(), e.get_message().as_deref().map_or_else(|| "null".to_string(), escape))
}

/// `String.compareTo` (UTF-16 order), the order of Java's `TreeMap`.
fn sprint17_oracle_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn opt(s: Option<&str>) -> String {
    s.map_or_else(|| "null".to_string(), escape)
}

fn dump_doc_data(dd: &DocData, out: &mut String) {
    out.push_str(&format!(
        "DOC name={} title={} date={} id={}\nBODY {}\n",
        opt(dd.get_name()),
        opt(dd.get_title()),
        opt(dd.get_date()),
        dd.get_id(),
        opt(dd.get_body())
    ));
    if let Some(props) = dd.get_props() {
        let mut sorted: Vec<(String, String)> = props.entries().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        sorted.sort_by(|a, b| sprint17_oracle_compare(&a.0, &b.0));
        for (k, v) in sorted {
            out.push_str(&format!("PROP {}={}\n", escape(&k), escape(&v)));
        }
    }
}

/// Reads docs until `NoMoreDataException`, then "END".
fn drain(source: &dyn ContentSource, fresh: bool, out: &mut String) {
    let mut dd = DocData::new();
    loop {
        if fresh {
            dd = DocData::new();
        }
        match source.get_next_doc_data(&mut dd) {
            Ok(()) => dump_doc_data(&dd, out),
            Err(e) if NoMoreDataException::is(&e) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
    out.push_str("END\n");
}

#[test]
fn benchmark_trec_oracle() {
    sprint17_oracle::pin_tests_seed();
    let work = WorkDir::new("s17b-trec");
    let zip = std::fs::read(manifest().join("tests/resources/by_task/feeds/trecdocs.zip")).unwrap();
    rucene_test_framework::util::test_util::TestUtil::unzip(&zip, work.path()).unwrap();
    let dir = std::fs::canonicalize(work.path()).unwrap().to_string_lossy().into_owned();
    let java = sprint17_oracle::run(Module::Benchmark, "trec", SEED, &[&dir]);
    let tcs = TrecContentSource::new();
    let mut props = Properties::new();
    props.set_property("print.props", "false");
    props.set_property("content.source.verbose", "false");
    props.set_property("content.source.excludeIteration", "true");
    props.set_property("docs.dir", dir.as_str());
    props.set_property("trec.doc.parser", "org.apache.lucene.benchmark.byTask.feeds.TrecParserByPath");
    props.set_property("content.source.forever", "false");
    tcs.set_config(Arc::new(Config::from_properties(props))).unwrap();
    tcs.reset_inputs().unwrap();
    let mut out = String::new();
    drain(&tcs, false, &mut out);
    tcs.close().unwrap();
    sprint17_oracle::assert_same(&java, &out);
}

fn line_doc_case(file: &Path, parser: Option<&str>) {
    let java = sprint17_oracle::run(Module::Benchmark, "linedoc", SEED, &[&abs(file), parser.unwrap_or("-")]);
    let src = LineDocSource::new();
    let mut props = Properties::new();
    props.set_property("docs.file", abs(file));
    props.set_property("content.source.forever", "false");
    if let Some(parser) = parser {
        props.set_property("line.parser", parser);
    }
    src.set_config(Arc::new(Config::from_properties(props))).unwrap();
    src.reset_inputs().unwrap();
    let mut out = String::new();
    drain(&src, true, &mut out);
    src.close().unwrap();
    sprint17_oracle::assert_same(&java, &out);
}

#[test]
fn benchmark_line_doc_oracle() {
    sprint17_oracle::pin_tests_seed();
    let reuters = manifest().join("tests/resources/by_task/reuters.first20.lines.txt");
    line_doc_case(&reuters, None);
    // a header line with extra fields, read by the HeaderLineParser
    let work = WorkDir::new("s17b-linedoc");
    let f = work.path().join("header.txt");
    std::fs::write(
        &f,
        "FIELDS_HEADER_INDICATOR###\tdoctitle\tdocdate\tbody\tdocname\tmySpecialField\n\
         t1\td1\tb1 x\tn1\ts1\nt2\t\tb2\tn2\ts2\n",
    )
    .unwrap();
    line_doc_case(&f, Some("org.apache.lucene.benchmark.byTask.feeds.LineDocSource$HeaderLineParser"));
    line_doc_case(&f, None);
}

/// `BenchmarkOracle.maskReportLine`: the rate, elapsed and memory columns
/// of a report data line become `#`.
fn mask_report_line(line: &str) -> String {
    let t: Vec<&str> = line.split_whitespace().collect();
    let n = t.len();
    let grouped = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == ',');
    let fraction = |s: &str| {
        s.split_once('.').is_some_and(|(a, b)| a.chars().all(|c| c.is_ascii_digit() || c == ',') && grouped(b) && !b.contains(','))
    };
    let nums: Vec<usize> = (0..n).filter(|&i| t[i] != "-").collect();
    let k = nums.len();
    if k < 4 || !grouped(t[nums[k - 1]]) || !grouped(t[nums[k - 2]]) {
        return line.to_string();
    }
    t.iter()
        .enumerate()
        .filter(|(_, s)| **s != "-")
        .map(|(i, s)| {
            if i == nums[k - 1] || i == nums[k - 2] || fraction(s) {
                "#".to_string()
            } else {
                s.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn benchmark_report_oracle() {
    sprint17_oracle::pin_tests_seed();
    let work = WorkDir::new("s17b-report");
    let reuters = manifest().join("tests/resources/by_task/reuters.first20.lines.txt");
    let alg = format!(
        "work.dir={}\ndirectory=ByteBuffersDirectory\nprint.props=false\n\
         content.source=org.apache.lucene.benchmark.byTask.feeds.LineDocSource\n\
         docs.file={}\ncontent.source.forever=false\nlog.step=-1\n\
         {{ \"Rounds\"\n  ResetSystemErase\n  CreateIndex\n  {{ \"AddDocs\" AddDoc > : *\n  CloseIndex\n  NewRound\n}} : 2\n",
        abs(work.path()).replace('\\', "/"),
        abs(&reuters).replace('\\', "/")
    );
    let java = sprint17_oracle::run(Module::Benchmark, "report", SEED, &[&alg]);
    let b = Benchmark::new(&alg).unwrap();
    b.execute().unwrap();
    let stats: Vec<TaskStats> =
        b.get_run_data().get_points().task_stats().iter().map(|s| s.lock().unwrap().clone()).collect();
    let rep = RepSumByNameTask::new(Arc::clone(b.get_run_data())).unwrap().report_sum_by_name(&stats).unwrap();
    let mut out = format!("outOf={} size={} reported={}\n", rep.get_out_of(), rep.get_size(), rep.get_reported());
    for line in rep.get_text().split('\n') {
        out.push_str(&escape(&mask_report_line(line)));
        out.push('\n');
    }
    b.get_run_data().close().unwrap();
    sprint17_oracle::assert_same(&java, &out);
}

#[test]
fn benchmark_spellout_oracle() {
    // `spell.java.tsv`: Java's `new RuleBasedNumberFormat(Locale.ROOT,
    // SPELLOUT).format(n)` (icu4j 77.1) for 38,871 values (rmp #611).
    let tsv = std::fs::read_to_string(s611().join("spell.java.tsv")).unwrap();
    let f = RuleBasedNumberFormat::root_spellout();
    let mut rows = 0;
    let mut diffs = Vec::new();
    for line in tsv.lines() {
        let (n, expected) = line.split_once('\t').expect("n<TAB>text");
        let n: i64 = n.parse().unwrap();
        let actual = f.format(n);
        if actual != expected {
            diffs.push(format!("{n}: java={expected:?} rust={actual:?}"));
        }
        rows += 1;
    }
    assert_eq!(rows, 38_871);
    assert!(diffs.is_empty(), "{} rows differ, first: {:?}", diffs.len(), &diffs[..diffs.len().min(5)]);
}
