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

//! Port of `TestOffsetLimitTokenFilter.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::StringCharReader;
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene_highlighter::highlight::offset_limit_token_filter::OffsetLimitTokenFilter;
use rucene_test_framework::analysis::mock_analyzer::BoxedStream;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::{
    components, shared, sink, BaseTokenStreamTestCase, MockTokenizer,
};

use crate::highlighter_support::core_support::test;
use crate::highlighter_support::{btstc_class, FnAnalyzer};

/// `TestOffsetLimitTokenFilter`.
pub fn class_def() -> Arc<Class> {
    btstc_class(
        "org.apache.lucene.search.highlight.TestOffsetLimitTokenFilter",
        vec![test("testFilter", test_filter)],
    )
}

fn stream() -> Result<MockTokenizer> {
    // we disable MockTokenizer checks because we will forcefully limit the
    // tokenstream and call end() before incrementToken() returns false.
    let mut stream = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    stream.set_reader(Box::new(StringCharReader::new(
        "short toolong evenmuchlongertext a ab toolong foo",
    )))?;
    stream.set_enable_checks(false);
    Ok(stream)
}

fn test_filter() -> Result<()> {
    let mut filter = OffsetLimitTokenFilter::new(Box::new(stream()?), 10);
    BaseTokenStreamTestCase::assert_token_stream_contents(&mut filter, &["short", "toolong"])?;

    let mut filter = OffsetLimitTokenFilter::new(Box::new(stream()?), 12);
    BaseTokenStreamTestCase::assert_token_stream_contents(&mut filter, &["short", "toolong"])?;

    let mut filter = OffsetLimitTokenFilter::new(Box::new(stream()?), 30);
    BaseTokenStreamTestCase::assert_token_stream_contents(
        &mut filter,
        &["short", "toolong", "evenmuchlongertext"],
    )?;

    let analyzer = FnAnalyzer::new(
        "org.apache.lucene.search.highlight.TestOffsetLimitTokenFilter$1",
        |_| {
            let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
            tokenizer.set_enable_checks(false);
            let tokenizer = Arc::new(AtomicRefCell::new(tokenizer));
            let filter = OffsetLimitTokenFilter::new(shared(&tokenizer), 10);
            Ok(components(tokenizer, sink(BoxedStream(Box::new(filter)))))
        },
    );
    BaseTokenStreamTestCase::check_one_term(&analyzer, "llenges", "llenges")?;
    Ok(())
}
