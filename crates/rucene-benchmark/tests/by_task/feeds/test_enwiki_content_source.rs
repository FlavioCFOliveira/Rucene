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

//! Port of `org.apache.lucene.benchmark.byTask.feeds.TestEnwikiContentSource`.

use std::io::Read;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_benchmark::by_task::feeds::content_items_source::ContentItemsSource;
use rucene_benchmark::by_task::feeds::content_source::ContentSource;
use rucene_benchmark::by_task::feeds::doc_data::DocData;
use rucene_benchmark::by_task::feeds::doc_maker::{DocMaker, DocMakerClass};
use rucene_benchmark::by_task::feeds::enwiki_content_source::EnwikiContentSource;
use rucene_benchmark::by_task::utils::config::Config;
use rucene_benchmark::jdk::properties::Properties;

use crate::benchmark_support::core_support::{class, test};
use crate::benchmark_support::expect_no_more_data_exception;

/// `private static class StringableEnwikiSource extends
/// EnwikiContentSource`: an EnwikiContentSource which works on a String and
/// not files.
fn stringable_enwiki_source(docs: String) -> EnwikiContentSource {
    EnwikiContentSource::with_open_input_stream(Arc::new(move || {
        Ok(Box::new(std::io::Cursor::new(docs.clone().into_bytes())) as Box<dyn Read + Send>)
    }))
}

fn assert_doc_data(dd: &DocData, exp_name: &str, exp_title: &str, exp_body: &str, exp_date: &str) {
    Assert::assert_equals_nullable(Some(exp_name), dd.get_name());
    Assert::assert_equals_nullable(Some(exp_title), dd.get_title());
    Assert::assert_equals_nullable(Some(exp_body), dd.get_body());
    Assert::assert_equals_nullable(Some(exp_date), dd.get_date());
}

fn assert_no_more_data_exception(stdm: &EnwikiContentSource) {
    expect_no_more_data_exception(stdm.get_next_doc_data(&mut DocData::new()));
}

const PAGE1: &str = concat!(
    "  <page>\r\n",
    "    <title>Title1</title>\r\n",
    "    <ns>0</ns>\r\n",
    "    <id>1</id>\r\n",
    "    <revision>\r\n",
    "      <id>11</id>\r\n",
    "      <parentid>111</parentid>\r\n",
    "      <timestamp>2011-09-14T11:35:09Z</timestamp>\r\n",
    "      <contributor>\r\n",
    "      <username>Mister1111</username>\r\n",
    "        <id>1111</id>\r\n",
    "      </contributor>\r\n",
    "      <minor />\r\n",
    "      <comment>/* Never mind */</comment>\r\n",
    "      <text>Some text 1 here</text>\r\n",
    "    </revision>\r\n",
    "  </page>\r\n",
);

const PAGE2: &str = concat!(
    "  <page>\r\n",
    "    <title>Title2</title>\r\n",
    "    <ns>0</ns>\r\n",
    "    <id>2</id>\r\n",
    "    <revision>\r\n",
    "      <id>22</id>\r\n",
    "      <parentid>222</parentid>\r\n",
    "      <timestamp>2022-09-14T22:35:09Z</timestamp>\r\n",
    "      <contributor>\r\n",
    "      <username>Mister2222</username>\r\n",
    "        <id>2222</id>\r\n",
    "      </contributor>\r\n",
    "      <minor />\r\n",
    "      <comment>/* Never mind */</comment>\r\n",
    "      <text>Some text 2 here</text>\r\n",
    "    </revision>\r\n",
    "  </page>\r\n",
);

/// `TestEnwikiContentSource`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.benchmark.byTask.feeds.TestEnwikiContentSource",
        vec![
            test("testOneDocument", test_one_document),
            test("testTwoDocuments", test_two_documents),
            test("testForever", test_forever),
        ],
    )
}

fn test_one_document() -> Result<()> {
    let docs = format!("<mediawiki>\r\n{PAGE1}</mediawiki>");
    let source = create_content_source(docs, false)?;
    let mut dd = DocData::new();
    source.get_next_doc_data(&mut dd)?;
    assert_doc_data(
        &dd,
        "1",
        "Title1",
        "Some text 1 here",
        "14-SEP-2011 11:35:09.000",
    );
    assert_no_more_data_exception(&source);
    Ok(())
}

fn create_content_source(docs: String, forever: bool) -> Result<Arc<EnwikiContentSource>> {
    let mut props = Properties::new();
    props.set_property("print.props", "false");
    props.set_property("content.source.forever", forever.to_string());
    let config = Arc::new(Config::from_properties(props));
    let source = Arc::new(stringable_enwiki_source(docs));
    source.set_config(Arc::clone(&config))?;
    let doc_maker = DocMaker::new();
    doc_maker.set_config(config, Arc::clone(&source) as Arc<dyn ContentSource>)?;
    DocMakerClass::reset_inputs(&doc_maker)?;
    Ok(source)
}

fn test_two_documents() -> Result<()> {
    let docs = format!("<mediawiki>\r\n{PAGE1}{PAGE2}</mediawiki>");
    let source = create_content_source(docs, false)?;
    let mut dd1 = DocData::new();
    source.get_next_doc_data(&mut dd1)?;
    assert_doc_data(
        &dd1,
        "1",
        "Title1",
        "Some text 1 here",
        "14-SEP-2011 11:35:09.000",
    );
    let mut dd2 = DocData::new();
    source.get_next_doc_data(&mut dd2)?;
    assert_doc_data(
        &dd2,
        "2",
        "Title2",
        "Some text 2 here",
        "14-SEP-2022 22:35:09.000",
    );
    assert_no_more_data_exception(&source);
    Ok(())
}

fn test_forever() -> Result<()> {
    let docs = format!("<mediawiki>\r\n{PAGE1}{PAGE2}</mediawiki>");
    let source = create_content_source(docs, true)?;
    for _ in 0..3 {
        let mut dd1 = DocData::new();
        source.get_next_doc_data(&mut dd1)?;
        assert_doc_data(
            &dd1,
            "1",
            "Title1",
            "Some text 1 here",
            "14-SEP-2011 11:35:09.000",
        );
        let mut dd2 = DocData::new();
        source.get_next_doc_data(&mut dd2)?;
        assert_doc_data(
            &dd2,
            "2",
            "Title2",
            "Some text 2 here",
            "14-SEP-2022 22:35:09.000",
        );
    }
    source.close()?;
    Ok(())
}
