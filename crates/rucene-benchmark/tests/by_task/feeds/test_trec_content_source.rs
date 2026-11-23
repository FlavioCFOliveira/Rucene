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

//! Port of `org.apache.lucene.benchmark.byTask.feeds.TestTrecContentSource`.

use std::collections::HashSet;
use std::io::Read;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::DateTools;
use rucene::error::Result;
use rucene_analysis_common::jdk::charset::Charset;
use rucene_analysis_common::jdk::io::{BufferedReader, InputStreamReader};
use rucene_benchmark::by_task::feeds::content_items_source::ContentItemsSource;
use rucene_benchmark::by_task::feeds::content_source::ContentSource;
use rucene_benchmark::by_task::feeds::demo_html_parser::DemoHTMLParser;
use rucene_benchmark::by_task::feeds::doc_data::DocData;
use rucene_benchmark::by_task::feeds::no_more_data_exception::NoMoreDataException;
use rucene_benchmark::by_task::feeds::trec_content_source::{OpenNextFileState, TrecContentSource};
use rucene_benchmark::by_task::feeds::trec_doc_parser::ParsePathType;
use rucene_benchmark::by_task::utils::config::Config;
use rucene_benchmark::jdk::properties::Properties;
use rucene_queryparser::jdk::date::Date;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::benchmark_support::core_support::{class, test};
use crate::benchmark_support::{expect_no_more_data_exception, resource_bytes};

/// `private static class StringableTrecSource extends TrecContentSource`:
/// a TrecDocMaker which works on a String and not files.
fn stringable_trec_source(docs: &'static str, forever: bool) -> TrecContentSource {
    let source =
        TrecContentSource::with_open_next_file(Arc::new(move |s: &mut OpenNextFileState<'_>| {
            if s.reader.is_some() {
                if !s.forever {
                    return Err(NoMoreDataException::new());
                }
                *s.iteration += 1;
            }
            // new BufferedReader(new StringReader(docs))
            let stream: Box<dyn Read + Send> = Box::new(std::io::Cursor::new(docs.as_bytes()));
            *s.reader = Some(BufferedReader::new(InputStreamReader::with_charset(
                stream,
                Charset::for_name("UTF-8")?,
            )));
            Ok(())
        }));
    source.items_base().set_forever(forever);
    source
}

/// The overriding `setConfig(Config)`: `htmlParser = new DemoHTMLParser()`.
fn set_config_override(source: &TrecContentSource) {
    source.set_html_parser(Some(Arc::new(DemoHTMLParser::new())));
}

fn time_of(date: Option<Date>) -> Option<i64> {
    date.map(|d| d.get_time())
}

fn assert_doc_data(
    dd: &DocData,
    exp_name: &str,
    exp_title: Option<&str>,
    exp_body: &str,
    exp_date: Option<i64>,
) -> Result<()> {
    Assert::assert_equals_nullable(Some(exp_name), dd.get_name());
    Assert::assert_equals_nullable(exp_title, dd.get_title_string().as_deref());
    Assert::assert_true(dd.get_body_string().is_some_and(|b| b.contains(exp_body)));
    let date = match dd.get_date() {
        Some(d) => Some(DateTools::string_to_time(d)?),
        None => None,
    };
    Assert::assert_equals_nullable(exp_date, date);
    Ok(())
}

fn assert_no_more_data_exception(stdm: &TrecContentSource) {
    expect_no_more_data_exception(stdm.get_next_doc_data(&mut DocData::new()));
}

include!("trec_docs.inc.rs");

/// `TestTrecContentSource`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.benchmark.byTask.feeds.TestTrecContentSource",
        vec![
            test("testOneDocument", test_one_document),
            test("testTwoDocuments", test_two_documents),
            test("testMissingDate", test_missing_date),
            test("testBadDate", test_bad_date),
            test("testForever", test_forever),
            test("testTrecFeedDirAllTypes", test_trec_feed_dir_all_types),
        ],
    )
}

fn test_one_document() -> Result<()> {
    let source = stringable_trec_source(ONE_DOCUMENT, false);
    set_config_override(&source);
    let mut dd = DocData::new();
    source.get_next_doc_data(&mut dd)?;
    let exp = time_of(source.parse_date("Sun, 11 Jan 2009 08:00:00 GMT")?);
    assert_doc_data(
        &dd,
        "TEST-000_0",
        Some("TEST-000 title"),
        "TEST-000 text",
        exp,
    )?;
    assert_no_more_data_exception(&source);
    Ok(())
}

fn test_two_documents() -> Result<()> {
    let source = stringable_trec_source(TWO_DOCUMENTS, false);
    set_config_override(&source);
    let mut dd = DocData::new();
    source.get_next_doc_data(&mut dd)?;
    let exp = time_of(source.parse_date("Sun, 11 Jan 2009 08:00:00 GMT")?);
    assert_doc_data(
        &dd,
        "TEST-000_0",
        Some("TEST-000 title"),
        "TEST-000 text",
        exp,
    )?;
    source.get_next_doc_data(&mut dd)?;
    let exp = time_of(source.parse_date("Tue, 09 Dec 2003 22:39:08 GMT")?);
    assert_doc_data(
        &dd,
        "TEST-001_0",
        Some("TEST-001 title"),
        "TEST-001 text",
        exp,
    )?;
    assert_no_more_data_exception(&source);
    Ok(())
}

// If a Date: attribute is missing, make sure the document is not skipped, but
// rather that null Data is assigned.
fn test_missing_date() -> Result<()> {
    let source = stringable_trec_source(MISSING_DATE, false);
    set_config_override(&source);
    let mut dd = DocData::new();
    source.get_next_doc_data(&mut dd)?;
    assert_doc_data(
        &dd,
        "TEST-000_0",
        Some("TEST-000 title"),
        "TEST-000 text",
        None,
    )?;
    source.get_next_doc_data(&mut dd)?;
    let exp = time_of(source.parse_date("Sun, 11 Jan 2009 08:01:00 GMT")?);
    assert_doc_data(
        &dd,
        "TEST-001_0",
        Some("TEST-001 title"),
        "TEST-001 text",
        exp,
    )?;
    assert_no_more_data_exception(&source);
    Ok(())
}

// When a 'bad date' is input (unparsable date), make sure the DocData date is
// assigned null.
fn test_bad_date() -> Result<()> {
    let source = stringable_trec_source(BAD_DATE, false);
    set_config_override(&source);
    let mut dd = DocData::new();
    source.get_next_doc_data(&mut dd)?;
    assert_doc_data(
        &dd,
        "TEST-000_0",
        Some("TEST-000 title"),
        "TEST-000 text",
        None,
    )?;
    assert_no_more_data_exception(&source);
    Ok(())
}

fn test_forever() -> Result<()> {
    let source = stringable_trec_source(FOREVER, true);
    set_config_override(&source);
    let mut dd = DocData::new();
    source.get_next_doc_data(&mut dd)?;
    let exp = time_of(source.parse_date("Sun, 11 Jan 2009 08:00:00 GMT")?);
    assert_doc_data(
        &dd,
        "TEST-000_0",
        Some("TEST-000 title"),
        "TEST-000 text",
        exp,
    )?;
    // same document, but the second iteration changes the name.
    source.get_next_doc_data(&mut dd)?;
    assert_doc_data(
        &dd,
        "TEST-000_1",
        Some("TEST-000 title"),
        "TEST-000 text",
        exp,
    )?;
    source.close()?;
    // Don't test that NoMoreDataException is thrown, since the forever flag is
    // turned on.
    Ok(())
}

/// Open a trec content source over a directory with files of all trec path
/// types and all supported formats - bzip, gzip, txt.
fn test_trec_feed_dir_all_types() -> Result<()> {
    let data_dir = LuceneTestCase::create_temp_dir_prefix("trecFeedAllTypes");
    TestUtil::unzip(
        &resource_bytes("by_task/feeds", "trecdocs.zip"),
        data_dir.as_std_path(),
    )?;
    let tcs = TrecContentSource::new();
    let mut props = Properties::new();
    props.set_property("print.props", "false");
    props.set_property("content.source.verbose", "false");
    props.set_property("content.source.excludeIteration", "true");
    props.set_property(
        "docs.dir",
        std::fs::canonicalize(data_dir.as_std_path())?
            .to_string_lossy()
            .replace('\\', "/"),
    );
    props.set_property(
        "trec.doc.parser",
        "org.apache.lucene.benchmark.byTask.feeds.TrecParserByPath",
    );
    props.set_property("content.source.forever", "false");
    tcs.set_config(Arc::new(Config::from_properties(props)))?;
    tcs.reset_inputs()?;
    let mut dd = DocData::new();
    let mut n = 0;
    let mut got_expected_exception = false;
    let mut unseen_types: HashSet<ParsePathType> = [
        ParsePathType::Gov2,
        ParsePathType::Fbis,
        ParsePathType::Ft,
        ParsePathType::Fr94,
        ParsePathType::Latimes,
    ]
    .into_iter()
    .collect();
    let result = (|| -> Result<()> {
        while n < 100 {
            // arbiterary limit to prevent looping forever in case of test failure
            tcs.get_next_doc_data(&mut dd)?;
            n += 1;
            let curr = tcs.curr_path_type();
            if let Some(curr) = curr {
                unseen_types.remove(&curr);
            }
            match curr {
                Some(ParsePathType::Gov2) => assert_doc_data(
                    &dd,
                    "TEST-000",
                    Some("TEST-000 title"),
                    "TEST-000 text",
                    time_of(tcs.parse_date("Sun, 11 Jan 2009 08:00:00 GMT")?),
                )?,
                Some(ParsePathType::Fbis) => assert_doc_data(
                    &dd,
                    "TEST-001",
                    Some("TEST-001 Title"),
                    "TEST-001 text",
                    time_of(tcs.parse_date("1 January 1991")?),
                )?,
                Some(ParsePathType::Fr94) => {
                    // no title extraction in this source for now
                    assert_doc_data(
                        &dd,
                        "TEST-002",
                        None,
                        "DEPARTMENT OF SOMETHING",
                        time_of(tcs.parse_date("February 3, 1994")?),
                    )?
                }
                Some(ParsePathType::Ft) => assert_doc_data(
                    &dd,
                    "TEST-003",
                    Some("Test-003 title"),
                    "Some pub text",
                    time_of(tcs.parse_date("980424")?),
                )?,
                Some(ParsePathType::Latimes) => assert_doc_data(
                    &dd,
                    "TEST-004",
                    Some("Test-004 Title"),
                    "Some paragraph",
                    time_of(tcs.parse_date("January 17, 1997, Sunday")?),
                )?,
                _ => Assert::assert_true_msg("Should never get here!", false),
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        if !NoMoreDataException::is(&e) {
            return Err(e);
        }
        got_expected_exception = true;
    }
    Assert::assert_true_msg(
        "Should have gotten NoMoreDataException!",
        got_expected_exception,
    );
    crate::benchmark_support::assert_equals_msg(
        "Wrong number of documents created by source!",
        5,
        n,
    );
    Assert::assert_true_msg("Did not see all types!", unseen_types.is_empty());
    Ok(())
}
