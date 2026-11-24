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

//! Port of `org.apache.lucene.benchmark.byTask.feeds.TestLineDocSource`.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene_benchmark::by_task::feeds::doc_maker::DocMaker;
use rucene_benchmark::by_task::tasks::add_doc_task::AddDocTask;
use rucene_benchmark::by_task::tasks::close_index_task::CloseIndexTask;
use rucene_benchmark::by_task::tasks::create_index_task::CreateIndexTask;
use rucene_benchmark::by_task::tasks::perf_task::PerfTask;
use rucene_benchmark::by_task::tasks::task_sequence::TaskSequence;
use rucene_benchmark::by_task::tasks::write_line_doc_task::WriteLineDocTask;
use rucene_benchmark::jdk::properties::Properties;
use rucene_benchmark::jdk::NEW_LINE;
use rucene_test_framework::util::LuceneTestCase;

use crate::benchmark_support::search_support::{ftest, tq};
use crate::benchmark_support::{benchmark_class, get_work_dir, run_data_of};

/// The binary name of `LineDocSource.HeaderLineParser`.
const HEADER_LINE_PARSER: &str =
    "org.apache.lucene.benchmark.byTask.feeds.LineDocSource$HeaderLineParser";

/// The instance of `TestLineDocSource`.
#[derive(Default)]
pub struct LineDocSourceTest;

/// `TestLineDocSource`.
pub fn class_def() -> Arc<Class> {
    benchmark_class::<LineDocSourceTest>(
        "org.apache.lucene.benchmark.byTask.feeds.TestLineDocSource",
        vec![
            ftest("testBZip2", test_bzip2),
            ftest("testBZip2NoHeaderLine", test_bzip2_no_header_line),
            ftest("testRegularFile", test_regular_file),
            ftest(
                "testRegularFileSpecialHeader",
                test_regular_file_special_header,
            ),
            ftest(
                "testRegularFileNoHeaderLine",
                test_regular_file_no_header_line,
            ),
            ftest("testInvalidFormat", test_invalid_format),
            ftest("testWithDocsName", test_with_docs_name),
            ftest("testWithProperties", test_with_properties),
        ],
    )
}

fn create_bz2_line_file(file: &Path, add_header: bool) -> Result<()> {
    let out = std::fs::File::create(file)?;
    let mut writer = bzip2::write::BzEncoder::new(out, bzip2::Compression::default());
    writer.write_all(docs_text(add_header, None).as_bytes())?;
    writer.finish()?;
    Ok(())
}

/// `writeDocsToFile(writer, addHeader, otherFields)`: the text written.
fn docs_text(add_header: bool, other_fields: Option<&Properties>) -> String {
    let sep = WriteLineDocTask::SEP;
    let mut w = String::new();
    if add_header {
        w.push_str(WriteLineDocTask::FIELDS_HEADER_INDICATOR);
        w.push(sep);
        w.push_str(DocMaker::TITLE_FIELD);
        w.push(sep);
        w.push_str(DocMaker::DATE_FIELD);
        w.push(sep);
        w.push_str(DocMaker::BODY_FIELD);
        if let Some(other_fields) = other_fields {
            for fn_ in other_fields.keys() {
                w.push(sep);
                w.push_str(fn_);
            }
        }
        w.push_str(NEW_LINE);
    }
    let mut doc = String::new();
    doc.push_str("title");
    doc.push(sep);
    doc.push_str("date");
    doc.push(sep);
    doc.push_str(DocMaker::BODY_FIELD);
    if let Some(other_fields) = other_fields {
        for (_, fv) in other_fields.entries() {
            doc.push(sep);
            doc.push_str(fv);
        }
    }
    w.push_str(&doc);
    w.push_str(NEW_LINE);
    w
}

fn create_regular_line_file(file: &Path, add_header: bool) -> Result<()> {
    std::fs::write(file, docs_text(add_header, None))?;
    Ok(())
}

fn create_regular_line_file_with_more_fields(file: &Path, extra_fields: &[&str]) -> Result<()> {
    let mut p = Properties::new();
    for f in extra_fields {
        p.set_property(*f, *f);
    }
    std::fs::write(file, docs_text(true, Some(&p)))?;
    Ok(())
}

fn do_index_and_search_test(
    file: &Path,
    line_parser_class: Option<&str>,
    stored_field: Option<&str>,
) -> Result<()> {
    do_index_and_search_test_with_repeats(file, line_parser_class, 1, stored_field)?; // no extra repetitions
    do_index_and_search_test_with_repeats(file, line_parser_class, 2, stored_field)?; // 1 extra repetition
    do_index_and_search_test_with_repeats(file, line_parser_class, 4, stored_field)
    // 3 extra repetitions
}

fn do_index_and_search_test_with_repeats(
    file: &Path,
    line_parser_class: Option<&str>,
    num_adds: i32,
    stored_field: Option<&str>,
) -> Result<()> {
    let mut props = Properties::new();
    props.set_property(
        "docs.file",
        std::path::absolute(file)?.to_string_lossy().into_owned(),
    );
    if let Some(line_parser_class) = line_parser_class {
        props.set_property("line.parser", line_parser_class);
    }
    props.set_property(
        "analyzer",
        "org.apache.lucene.analysis.core.WhitespaceAnalyzer",
    );
    props.set_property(
        "content.source",
        "org.apache.lucene.benchmark.byTask.feeds.LineDocSource",
    );
    props.set_property("directory", "ByteBuffersDirectory");
    props.set_property("doc.stored", "true");
    props.set_property("doc.index.props", "true");
    let run_data = run_data_of(props)?;
    let mut reader = None;
    let result = (|| -> Result<()> {
        let tasks = TaskSequence::new(Arc::clone(&run_data), Some("testBzip2"), None, false)?;
        tasks.add_task(Arc::new(CreateIndexTask::new(Arc::clone(&run_data))?));
        for _ in 0..num_adds {
            tasks.add_task(Arc::new(AddDocTask::new(Arc::clone(&run_data))?));
        }
        tasks.add_task(Arc::new(CloseIndexTask::new(Arc::clone(&run_data))?));
        let logic = tasks.do_logic();
        let closed = tasks.close();
        logic?;
        closed?;

        let r = directory_reader::open(run_data.get_directory().expect("directory"))?;
        reader = Some(Arc::clone(&r));
        let searcher = LuceneTestCase::new_searcher(r);
        let td = searcher.search(tq("body", "body"), 10)?;
        Assert::assert_equals(i64::from(num_adds), td.total_hits.value());
        Assert::assert_true(!td.score_docs.is_empty()); // assertNotNull(td.scoreDocs[0])
                                                        // added to all docs and satisfies field-name == value
        let stored_field = stored_field.unwrap_or(DocMaker::BODY_FIELD);
        Assert::assert_equals_nullable_msg(
            Some("Wrong field value"),
            Some(stored_field.to_string()),
            searcher.stored_fields()?.document(0)?.get(stored_field),
        );
        Ok(())
    })();
    // IOUtils.close(reader, runData)
    if let Some(r) = reader {
        r.close()?;
    }
    run_data.close()?;
    result
}

/// Tests LineDocSource with a bzip2 input stream.
fn test_bzip2(_f: &mut LineDocSourceTest) -> Result<()> {
    let file = get_work_dir().join("one-line.bz2");
    create_bz2_line_file(&file, true)?;
    do_index_and_search_test(&file, None, None)
}

fn test_bzip2_no_header_line(_f: &mut LineDocSourceTest) -> Result<()> {
    let file = get_work_dir().join("one-line.bz2");
    create_bz2_line_file(&file, false)?;
    do_index_and_search_test(&file, None, None)
}

fn test_regular_file(_f: &mut LineDocSourceTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    create_regular_line_file(&file, true)?;
    do_index_and_search_test(&file, None, None)
}

fn test_regular_file_special_header(_f: &mut LineDocSourceTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    create_regular_line_file(&file, true)?;
    do_index_and_search_test(&file, Some(HEADER_LINE_PARSER), None)
}

fn test_regular_file_no_header_line(_f: &mut LineDocSourceTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    create_regular_line_file(&file, false)?;
    do_index_and_search_test(&file, None, None)
}

fn test_invalid_format(_f: &mut LineDocSourceTest) -> Result<()> {
    let sep = WriteLineDocTask::SEP;
    let test_cases = [
        String::new(),             // empty line
        "title".to_string(),       // just title
        format!("title{sep}"),     // title + SEP
        format!("title{sep}body"), // title + SEP + body
    ];
    for test_case in &test_cases {
        let file = get_work_dir().join("one-line");
        std::fs::write(&file, format!("{test_case}{NEW_LINE}"))?;
        rucene_test_framework::util::java_exceptions::expect_throws("java.lang.Exception", || {
            do_index_and_search_test(&file, None, None)
        });
    }
    Ok(())
}

/// Doc Name is not part of the default header.
fn test_with_docs_name(_f: &mut LineDocSourceTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    create_regular_line_file_with_more_fields(&file, &[DocMaker::NAME_FIELD])?;
    do_index_and_search_test(&file, None, Some(DocMaker::NAME_FIELD))
}

/// Use fields names that are not defined in Docmaker and so will go to
/// Properties.
fn test_with_properties(_f: &mut LineDocSourceTest) -> Result<()> {
    let file = get_work_dir().join("one-line");
    let special_field = "mySpecialField";
    create_regular_line_file_with_more_fields(&file, &[special_field])?;
    do_index_and_search_test(&file, None, Some(special_field))
}
