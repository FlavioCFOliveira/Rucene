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

//! Port of `org.apache.lucene.benchmark.byTask.tasks.TestWriteEnwikiLineDocTask`.

use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene_benchmark::by_task::feeds::doc_maker::DocMaker;
use rucene_benchmark::by_task::perf_run_data::PerfRunData;
use rucene_benchmark::by_task::tasks::perf_task::PerfTask;
use rucene_benchmark::by_task::tasks::write_enwiki_line_doc_task::WriteEnwikiLineDocTask;
use rucene_benchmark::by_task::tasks::write_line_doc_task::WriteLineDocTask;
use rucene_benchmark::jdk::properties::Properties;

use super::test_write_line_doc_task::assert_header_line;
use crate::benchmark_support::search_support::ftest;
use crate::benchmark_support::{
    benchmark_class, get_work_dir, java_split, read_lines, run_data_of, FnDocMaker,
};

const WRITE_LINE_CATEGORY_DOC_MAKER: &str =
    "org.apache.lucene.benchmark.byTask.tasks.TestWriteEnwikiLineDocTask$WriteLineCategoryDocMaker";

/// `WriteLineCategoryDocMaker`: interleaves category docs with regular docs
/// (`AtomicInteger flip`, one per maker instance).
fn register_write_line_category_doc_maker() {
    rucene_benchmark::classes::register_doc_maker_class(
        WRITE_LINE_CATEGORY_DOC_MAKER,
        Arc::new(|| {
            let flip = AtomicI32::new(0);
            Arc::new(FnDocMaker::new(move || {
                let is_category = (flip.fetch_add(1, Ordering::SeqCst) + 1) % 2 == 0;
                let mut doc = Document::new();
                doc.add(StringField::new(
                    DocMaker::BODY_FIELD,
                    "body text".to_string(),
                    Store::NO,
                )?);
                doc.add(StringField::new(
                    DocMaker::TITLE_FIELD,
                    if is_category {
                        "Category:title text"
                    } else {
                        "title text"
                    }
                    .to_string(),
                    Store::NO,
                )?);
                doc.add(StringField::new(
                    DocMaker::DATE_FIELD,
                    "date text".to_string(),
                    Store::NO,
                )?);
                Ok(doc)
            })) as Arc<dyn rucene_benchmark::by_task::feeds::doc_maker::DocMakerClass>
        }),
    );
}

/// The instance of `TestWriteEnwikiLineDocTask`.
#[derive(Default)]
pub struct WriteEnwikiLineDocTaskTest;

/// `TestWriteEnwikiLineDocTask`.
pub fn class_def() -> Arc<Class> {
    register_write_line_category_doc_maker();
    benchmark_class::<WriteEnwikiLineDocTaskTest>(
        "org.apache.lucene.benchmark.byTask.tasks.TestWriteEnwikiLineDocTask",
        vec![ftest("testCategoryLines", test_category_lines)],
    )
}

fn create_perf_run_data(file: &Path, doc_maker_name: &str) -> Result<Arc<PerfRunData>> {
    let mut props = Properties::new();
    props.set_property("doc.maker", doc_maker_name);
    props.set_property(
        "line.file.out",
        std::path::absolute(file)?.to_string_lossy().into_owned(),
    );
    props.set_property("directory", "ByteBuffersDirectory"); // no accidental FS dir.
    run_data_of(props)
}

fn do_read_test(
    file: &Path,
    exp_title: &str,
    exp_date: &str,
    exp_body: Option<&str>,
) -> Result<()> {
    do_read_test_n(2, file, exp_title, exp_date, exp_body)?;
    let categories_file = WriteEnwikiLineDocTask::categories_line_file(file);
    do_read_test_n(
        2,
        &categories_file,
        &format!("Category:{exp_title}"),
        exp_date,
        exp_body,
    )
}

fn do_read_test_n(
    n: usize,
    file: &Path,
    exp_title: &str,
    exp_date: &str,
    exp_body: Option<&str>,
) -> Result<()> {
    let lines = read_lines(std::fs::File::open(file)?)?;
    let mut it = lines.iter();
    assert_header_line(it.next().map(String::as_str));
    for _ in 0..n {
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
    }
    Assert::assert_true(it.next().is_none());
    Ok(())
}

fn test_category_lines(_f: &mut WriteEnwikiLineDocTaskTest) -> Result<()> {
    let file = get_work_dir().join("two-lines-each.txt");
    let run_data = create_perf_run_data(&file, WRITE_LINE_CATEGORY_DOC_MAKER)?;
    let wldt: Box<dyn PerfTask> = Box::new(WriteEnwikiLineDocTask::new(run_data)?);
    for _ in 0..4 {
        // four times so that each file should have 2 lines.
        wldt.do_logic()?;
    }
    wldt.close()?;
    do_read_test(&file, "title text", "date text", Some("body text"))
}
