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

//! Port of `org.apache.lucene.benchmark.byTask.tasks.TestWriteLineDocTask`.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene_benchmark::by_task::feeds::doc_maker::DocMaker;
use rucene_benchmark::by_task::perf_run_data::PerfRunData;
use rucene_benchmark::by_task::tasks::perf_task::PerfTask;
use rucene_benchmark::by_task::tasks::write_line_doc_task::WriteLineDocTask;
use rucene_benchmark::by_task::utils::stream_utils::Type;
use rucene_benchmark::jdk::properties::Properties;

use crate::benchmark_support::search_support::ftest;
use crate::benchmark_support::{
    benchmark_class, get_work_dir, java_split, read_lines, run_data_of, FnDocMaker,
};

const PREFIX: &str = "org.apache.lucene.benchmark.byTask.tasks.TestWriteLineDocTask$";

fn doc(fields: &[(&str, &str)]) -> Result<Document> {
    let mut doc = Document::new();
    for (name, value) in fields {
        doc.add(StringField::new(
            name.to_string(),
            value.to_string(),
            Store::NO,
        )?);
    }
    Ok(doc)
}

/// `WriteLineDocMaker`.
fn write_line_doc_maker() -> Result<Document> {
    doc(&[
        (DocMaker::BODY_FIELD, "body"),
        (DocMaker::TITLE_FIELD, "title"),
        (DocMaker::DATE_FIELD, "date"),
    ])
}

/// `NewLinesDocMaker`.
fn new_lines_doc_maker() -> Result<Document> {
    doc(&[
        (DocMaker::BODY_FIELD, "body\r\ntext\ttwo"),
        (DocMaker::TITLE_FIELD, "title\r\ntext"),
        (DocMaker::DATE_FIELD, "date\r\ntext"),
    ])
}

/// `NoBodyDocMaker`.
fn no_body_doc_maker() -> Result<Document> {
    doc(&[
        (DocMaker::TITLE_FIELD, "title"),
        (DocMaker::DATE_FIELD, "date"),
    ])
}

/// `NoTitleDocMaker`.
fn no_title_doc_maker() -> Result<Document> {
    doc(&[
        (DocMaker::BODY_FIELD, "body"),
        (DocMaker::DATE_FIELD, "date"),
    ])
}

/// `JustDateDocMaker` and `LegalJustDateDocMaker`.
fn just_date_doc_maker() -> Result<Document> {
    doc(&[(DocMaker::DATE_FIELD, "date")])
}

/// `EmptyDocMaker`.
fn empty_doc_maker() -> Result<Document> {
    Ok(Document::new())
}

/// `ThreadingDocMaker`.
fn threading_doc_maker() -> Result<Document> {
    let name = rucene_benchmark::jdk::current_thread_name();
    doc(&[
        (DocMaker::BODY_FIELD, &format!("body_{name}")),
        (DocMaker::TITLE_FIELD, &format!("title_{name}")),
        (DocMaker::DATE_FIELD, &format!("date_{name}")),
    ])
}

fn name(simple: &str) -> String {
    format!("{PREFIX}{simple}")
}

/// The instance of `TestWriteLineDocTask`.
#[derive(Default)]
pub struct WriteLineDocTaskTest;

/// `TestWriteLineDocTask`.
pub fn class_def() -> Arc<Class> {
    FnDocMaker::register(&name("WriteLineDocMaker"), write_line_doc_maker);
    FnDocMaker::register(&name("NewLinesDocMaker"), new_lines_doc_maker);
    FnDocMaker::register(&name("NoBodyDocMaker"), no_body_doc_maker);
    FnDocMaker::register(&name("NoTitleDocMaker"), no_title_doc_maker);
    FnDocMaker::register(&name("JustDateDocMaker"), just_date_doc_maker);
    FnDocMaker::register(&name("LegalJustDateDocMaker"), just_date_doc_maker);
    FnDocMaker::register(&name("EmptyDocMaker"), empty_doc_maker);
    FnDocMaker::register(&name("ThreadingDocMaker"), threading_doc_maker);
    benchmark_class::<WriteLineDocTaskTest>(
        "org.apache.lucene.benchmark.byTask.tasks.TestWriteLineDocTask",
        vec![
            ftest("testBZip2", test_bzip2),
            ftest("testGZip", test_gzip),
            ftest("testRegularFile", test_regular_file),
            ftest("testCharsReplace", test_chars_replace),
            ftest("testEmptyBody", test_empty_body),
            ftest("testEmptyTitle", test_empty_title),
            ftest("testJustDate", test_just_date),
            ftest("testLegalJustDate", test_legal_just_date),
            ftest("testEmptyDoc", test_empty_doc),
            ftest("testMultiThreaded", test_multi_threaded),
        ],
    )
}

fn create_perf_run_data(
    file: &Path,
    allow_empty_docs: bool,
    doc_maker_name: &str,
) -> Result<Arc<PerfRunData>> {
    let mut props = Properties::new();
    props.set_property("doc.maker", doc_maker_name);
    props.set_property(
        "line.file.out",
        std::path::absolute(file)?.to_string_lossy().into_owned(),
    );
    props.set_property("directory", "ByteBuffersDirectory"); // no accidental FS dir.
    if allow_empty_docs {
        props.set_property("sufficient.fields", ",");
    }
    if doc_maker_name == name("LegalJustDateDocMaker") {
        props.set_property("line.fields", DocMaker::DATE_FIELD);
        props.set_property("sufficient.fields", DocMaker::DATE_FIELD);
    }
    run_data_of(props)
}

fn do_read_test(
    file: &Path,
    file_type: Type,
    exp_title: &str,
    exp_date: &str,
    exp_body: Option<&str>,
) -> Result<()> {
    let raw = std::fs::File::open(file)?;
    let input: Box<dyn Read> = match file_type {
        Type::Bzip2 => Box::new(bzip2::read::BzDecoder::new(raw)),
        Type::Gzip => Box::new(flate2::read::GzDecoder::new(raw)),
        Type::Plain => Box::new(raw), // nothing to do
    };
    let lines = read_lines(input)?;
    let mut it = lines.iter();
    assert_header_line(it.next().map(String::as_str));
    let line = it.next();
    Assert::assert_true(line.is_some());
    let parts = java_split(line.unwrap(), WriteLineDocTask::SEP);
    let num_exp_parts = if exp_body.is_none() { 2 } else { 3 };
    Assert::assert_equals(num_exp_parts, parts.len());
    Assert::assert_equals(exp_title, parts[0].as_str());
    Assert::assert_equals(exp_date, parts[1].as_str());
    if let Some(exp_body) = exp_body {
        Assert::assert_equals(exp_body, parts[2].as_str());
    }
    Assert::assert_true(it.next().is_none());
    Ok(())
}

/// `assertHeaderLine(line)`.
pub fn assert_header_line(line: Option<&str>) {
    Assert::assert_true_msg(
        "First line should be a header line",
        line.expect("NullPointerException")
            .starts_with(WriteLineDocTask::FIELDS_HEADER_INDICATOR),
    );
}

fn write_one(file: &Path, allow_empty_docs: bool, maker: &str) -> Result<()> {
    let run_data = create_perf_run_data(file, allow_empty_docs, &name(maker))?;
    let wldt = WriteLineDocTask::new(run_data)?;
    wldt.do_logic()?;
    wldt.close()
}

/// Tests WriteLineDocTask with a bzip2 format.
fn test_bzip2(_f: &mut WriteLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("one-line.bz2");
    write_one(&file, false, "WriteLineDocMaker")?;
    do_read_test(&file, Type::Bzip2, "title", "date", Some("body"))
}

/// Tests WriteLineDocTask with a gzip format.
fn test_gzip(_f: &mut WriteLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("one-line.gz");
    write_one(&file, false, "WriteLineDocMaker")?;
    do_read_test(&file, Type::Gzip, "title", "date", Some("body"))
}

fn test_regular_file(_f: &mut WriteLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    write_one(&file, false, "WriteLineDocMaker")?;
    do_read_test(&file, Type::Plain, "title", "date", Some("body"))
}

fn test_chars_replace(_f: &mut WriteLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    write_one(&file, false, "NewLinesDocMaker")?;
    do_read_test(
        &file,
        Type::Plain,
        "title text",
        "date text",
        Some("body text two"),
    )
}

fn test_empty_body(_f: &mut WriteLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    write_one(&file, false, "NoBodyDocMaker")?;
    do_read_test(&file, Type::Plain, "title", "date", None)
}

fn test_empty_title(_f: &mut WriteLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    write_one(&file, false, "NoTitleDocMaker")?;
    do_read_test(&file, Type::Plain, "", "date", Some("body"))
}

/// Fail by default when there's only date.
fn test_just_date(_f: &mut WriteLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    write_one(&file, false, "JustDateDocMaker")?;
    let lines = read_lines(std::fs::File::open(&file)?)?;
    assert_header_line(lines.first().map(String::as_str));
    Assert::assert_true(lines.get(1).is_none());
    Ok(())
}

fn test_legal_just_date(_f: &mut WriteLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    write_one(&file, false, "LegalJustDateDocMaker")?;
    let lines = read_lines(std::fs::File::open(&file)?)?;
    assert_header_line(lines.first().map(String::as_str));
    Assert::assert_true(lines.get(1).is_some());
    Ok(())
}

fn test_empty_doc(_f: &mut WriteLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    write_one(&file, true, "EmptyDocMaker")?;
    let lines = read_lines(std::fs::File::open(&file)?)?;
    assert_header_line(lines.first().map(String::as_str));
    Assert::assert_true(lines.get(1).is_some());
    Ok(())
}

fn test_multi_threaded(_f: &mut WriteLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    let run_data = create_perf_run_data(&file, false, &name("ThreadingDocMaker"))?;
    let wldt = Arc::new(WriteLineDocTask::new(run_data)?);
    let mut threads = Vec::new();
    for i in 0..10 {
        let wldt = Arc::clone(&wldt);
        threads.push(
            std::thread::Builder::new()
                .name(format!("t{i}"))
                .spawn(move || {
                    if let Err(e) = wldt.do_logic() {
                        panic!("RuntimeException: {e}");
                    }
                })?,
        );
    }
    let n = threads.len();
    for t in threads {
        if let Err(p) = t.join() {
            std::panic::resume_unwind(p);
        }
    }
    wldt.close()?;
    let mut ids = std::collections::HashSet::new();
    let lines = read_lines(std::fs::File::open(&file)?)?;
    let mut it = lines.iter();
    // header line is written once, no matter how many threads there are
    assert_header_line(it.next().map(String::as_str));
    for _ in 0..n {
        let line = it.next().expect("NullPointerException");
        let parts = java_split(line, WriteLineDocTask::SEP);
        Assert::assert_equals(3usize, parts.len());
        let tname = parts[0][parts[0].find('_').unwrap()..].to_string();
        ids.insert(tname.clone());
        Assert::assert_equals(tname.as_str(), &parts[1][parts[1].find('_').unwrap()..]);
        Assert::assert_equals(tname.as_str(), &parts[2][parts[2].find('_').unwrap()..]);
    }
    Assert::assert_true(it.next().is_none());
    Assert::assert_equals(n, ids.len());
    Ok(())
}
