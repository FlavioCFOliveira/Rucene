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

//! Port of `TestUnifiedHighlighterReanalysis.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::boolean_clause::Occur;
use rucene_highlighter::uhighlight::unified_highlighter::UnifiedHighlighter;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::core_support::random;
use crate::highlighter_support::ti;
use crate::uhighlight::test_unified_highlighter::bq;
use crate::uhighlight::unified_highlighter_test_base::{
    random_type_constructor, tq, uh_class, UHBase, UHTest,
};

type T = UHTest;

/// `TestUnifiedHighlighterReanalysis`.
pub fn class_def() -> Arc<Class> {
    uh_class::<T>(
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighterReanalysis",
        random_type_constructor(|ft| UHTest { b: UHBase::new(ft) }),
        vec![
            ti::<T>("testWithoutIndexSearcher", test_without_index_searcher),
            ti::<T>("testIndexSearcherNullness", test_index_searcher_nullness),
        ],
    )
}

/// `highlighter.highlightWithoutSearcher(field, query, text, 1)` as Java's
/// `Object` (its `toString()`, `null` when there is no highlight).
fn without_searcher(
    highlighter: &UnifiedHighlighter,
    field: &str,
    query: &Arc<dyn rucene::search::query::Query>,
    text: &str,
) -> Result<Option<String>> {
    Ok(highlighter
        .highlight_without_searcher(field, query, text, 1)?
        .map(|o| o.to_string()))
}

fn test_without_index_searcher(t: &mut T) -> Result<()> {
    let text = "This is a test. Just a test highlighting without a searcher. Feel free to ignore.";
    let query = bq(vec![
        (tq("body", "highlighting"), Occur::SHOULD),
        (tq("title", "test"), Occur::SHOULD),
    ])?;

    let highlighter = UnifiedHighlighter::builder_without_searcher(t.analyzer()).build();
    let snippet = without_searcher(&highlighter, "body", &query, text)?
        .unwrap_or_else(|| panic!("java.lang.NullPointerException"));

    Assert::assert_equals(
        "Just a test <b>highlighting</b> without a searcher. ".to_string(),
        snippet,
    );

    // assertEquals(String, Object, Object): the snippet object equals the String " "
    Assert::assert_equals_str(
        Some("test single space"),
        Some(" "),
        without_searcher(&highlighter, "body", &query, " ")?.as_deref(),
    );

    Assert::assert_equals_str(
        None,
        Some("Hello"),
        without_searcher(&highlighter, "nonexistent", &query, "Hello")?.as_deref(),
    );
    Ok(())
}

fn test_index_searcher_nullness(t: &mut T) -> Result<()> {
    let text = "This is a test. Just a test highlighting without a searcher. Feel free to ignore.";
    let query = tq("body", "highlighting");

    let directory = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let index_writer =
            RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
        let result = (|| -> Result<()> {
            let index_reader = index_writer.get_reader()?;
            let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&index_reader)));
            let highlighter = UnifiedHighlighter::builder(searcher, t.analyzer()).build();
            expect_throws("java.lang.IllegalStateException", || {
                highlighter.highlight_without_searcher("body", &query, text, 1)
            });
            index_reader.close()
        })();
        let closed = index_writer.close();
        result?;
        closed
    })();
    let closed = directory.close();
    result?;
    closed
}
