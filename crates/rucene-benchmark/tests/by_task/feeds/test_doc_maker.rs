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

//! Port of `org.apache.lucene.benchmark.byTask.feeds.TestDocMaker`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene_benchmark::by_task::feeds::content_items_source::{
    ContentItemsSource, ContentItemsSourceBase,
};
use rucene_benchmark::by_task::feeds::content_source::ContentSource;
use rucene_benchmark::by_task::feeds::doc_data::DocData;
use rucene_benchmark::by_task::feeds::doc_maker::{DocMaker, DocMakerClass};
use rucene_benchmark::by_task::feeds::line_doc_source::LineDocSource;
use rucene_benchmark::by_task::feeds::no_more_data_exception::NoMoreDataException;
use rucene_benchmark::by_task::tasks::add_doc_task::AddDocTask;
use rucene_benchmark::by_task::tasks::close_index_task::CloseIndexTask;
use rucene_benchmark::by_task::tasks::create_index_task::CreateIndexTask;
use rucene_benchmark::by_task::tasks::perf_task::PerfTask;
use rucene_benchmark::by_task::tasks::task_sequence::TaskSequence;
use rucene_benchmark::by_task::utils::config::Config;
use rucene_benchmark::classes;
use rucene_benchmark::jdk::properties::Properties;
use rucene_test_framework::util::LuceneTestCase;

use crate::benchmark_support::search_support::{ftest, tq};
use crate::benchmark_support::{benchmark_class, get_work_dir, run_data_of};

/// The binary name of `OneDocSource`.
pub const ONE_DOC_SOURCE: &str =
    "org.apache.lucene.benchmark.byTask.feeds.TestDocMaker$OneDocSource";

/// `public static final class OneDocSource extends ContentSource`.
#[derive(Default)]
pub struct OneDocSource {
    base: ContentItemsSourceBase,
    finish: AtomicBool,
}

impl ContentItemsSource for OneDocSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        &self.base
    }

    fn java_simple_name(&self) -> &'static str {
        "OneDocSource"
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ContentSource for OneDocSource {
    fn get_next_doc_data(&self, doc_data: &mut DocData) -> Result<()> {
        if self.finish.load(Ordering::SeqCst) {
            return Err(NoMoreDataException::new());
        }
        doc_data.set_body(Some("body".to_string()));
        doc_data.set_date(Some("date".to_string()));
        doc_data.set_title(Some("title".to_string()));
        let mut props = Properties::new();
        props.set_property("key", "value");
        doc_data.set_props(Some(props));
        self.finish.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// The instance of `TestDocMaker`.
#[derive(Default)]
pub struct DocMakerTest;

/// `TestDocMaker`.
pub fn class_def() -> Arc<Class> {
    classes::register_content_source_class(
        ONE_DOC_SOURCE,
        Arc::new(|| Arc::new(OneDocSource::default()) as Arc<dyn ContentSource>),
    );
    benchmark_class::<DocMakerTest>(
        "org.apache.lucene.benchmark.byTask.feeds.TestDocMaker",
        vec![
            ftest("testIndexProperties", test_index_properties),
            ftest("testNorms", test_norms),
            ftest("testDocMakerLeak", test_doc_maker_leak),
        ],
    )
}

fn do_test_index_properties(
    set_index_props: bool,
    index_props_val: bool,
    num_expected_results: i64,
) -> Result<()> {
    let mut props = Properties::new();
    props.set_property(
        "analyzer",
        "org.apache.lucene.analysis.core.WhitespaceAnalyzer",
    );
    props.set_property("content.source", ONE_DOC_SOURCE);
    props.set_property("directory", "ByteBuffersDirectory");
    if set_index_props {
        props.set_property("doc.index.props", index_props_val.to_string());
    }
    let run_data = run_data_of(props)?;
    let test_name = LuceneTestCase::get_test_name().unwrap_or_default();
    let tasks = TaskSequence::new(Arc::clone(&run_data), Some(&test_name), None, false)?;
    tasks.add_task(Arc::new(CreateIndexTask::new(Arc::clone(&run_data))?));
    tasks.add_task(Arc::new(AddDocTask::new(Arc::clone(&run_data))?));
    tasks.add_task(Arc::new(CloseIndexTask::new(Arc::clone(&run_data))?));
    tasks.do_logic()?;

    let reader = directory_reader::open(run_data.get_directory().expect("directory"))?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let td = searcher.search(tq("key", "value"), 10)?;
    Assert::assert_equals(num_expected_results, td.total_hits.value());
    reader.close()?;
    Ok(())
}

fn create_test_norms_document(
    set_norms_prop: bool,
    norms_prop_val: bool,
    set_body_norms_prop: bool,
    body_norms_val: bool,
) -> Result<Document> {
    let mut props = Properties::new();
    props.set_property(
        "analyzer",
        "org.apache.lucene.analysis.core.WhitespaceAnalyzer",
    );
    props.set_property("directory", "ByteBuffersDirectory");
    if set_norms_prop {
        props.set_property("doc.tokenized.norms", norms_prop_val.to_string());
    }
    if set_body_norms_prop {
        props.set_property("doc.body.tokenized.norms", body_norms_val.to_string());
    }
    let config = Arc::new(Config::from_properties(props));
    let dm = DocMaker::new();
    dm.set_config(config, Arc::new(OneDocSource::default()))?;
    dm.make_document()
}

/// Tests doc.index.props property.
fn test_index_properties(_f: &mut DocMakerTest) -> Result<()> {
    do_test_index_properties(false, false, 0)?;
    do_test_index_properties(true, false, 0)?;
    do_test_index_properties(true, true, 1)
}

fn omit_norms(doc: &Document, name: &str) -> bool {
    doc.get_field(name)
        .expect("field")
        .field_type()
        .omit_norms()
}

/// Tests doc.tokenized.norms and doc.body.tokenized.norms properties.
fn test_norms(_f: &mut DocMakerTest) -> Result<()> {
    let doc = create_test_norms_document(false, false, false, false)?;
    Assert::assert_true(omit_norms(&doc, DocMaker::TITLE_FIELD));
    Assert::assert_false(omit_norms(&doc, DocMaker::BODY_FIELD));

    let doc = create_test_norms_document(true, false, false, false)?;
    Assert::assert_true(omit_norms(&doc, DocMaker::TITLE_FIELD));
    Assert::assert_false(omit_norms(&doc, DocMaker::BODY_FIELD));

    let doc = create_test_norms_document(true, true, false, false)?;
    Assert::assert_false(omit_norms(&doc, DocMaker::TITLE_FIELD));
    Assert::assert_false(omit_norms(&doc, DocMaker::BODY_FIELD));

    let doc = create_test_norms_document(false, false, true, false)?;
    Assert::assert_true(omit_norms(&doc, DocMaker::TITLE_FIELD));
    Assert::assert_true(omit_norms(&doc, DocMaker::BODY_FIELD));

    let doc = create_test_norms_document(false, false, true, true)?;
    Assert::assert_true(omit_norms(&doc, DocMaker::TITLE_FIELD));
    Assert::assert_false(omit_norms(&doc, DocMaker::BODY_FIELD));
    Ok(())
}

fn test_doc_maker_leak(_f: &mut DocMakerTest) -> Result<()> {
    let f = get_work_dir().join("docMakerLeak.txt");
    let line = format!(
        "one title\t{}\tsome content{}",
        LuceneTestCase::random().next_long(),
        rucene_benchmark::jdk::NEW_LINE
    );
    std::fs::write(&f, line)?;
    let mut props = Properties::new();
    props.set_property(
        "docs.file",
        std::path::absolute(&f)?.to_string_lossy().into_owned(),
    );
    props.set_property("content.source.forever", "false");
    let config = Arc::new(Config::from_properties(props));
    let source: Arc<dyn ContentSource> = Arc::new(LineDocSource::new());
    source.set_config(Arc::clone(&config))?;
    let dm = DocMaker::new();
    dm.set_config(config, source)?;
    DocMakerClass::reset_inputs(&dm)?;
    DocMakerClass::reset_inputs(&dm)?;
    dm.close()?;
    Ok(())
}
