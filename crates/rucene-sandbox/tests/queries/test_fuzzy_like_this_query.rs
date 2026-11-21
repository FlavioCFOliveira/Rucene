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

//! Port of `TestFuzzyLikeThisQuery.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::Term;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::term_collector;
use rucene_sandbox::queries::fuzzy_like_this_query::FuzzyLikeThisQuery;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::random;
use crate::search_support::{fixture_class, ftest, set_up, tear_down};

/// The instance of `TestFuzzyLikeThisQuery`.
#[derive(Default)]
pub struct FuzzyLikeThisTest {
    directory: Option<TestDirectory>,
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    analyzer: Option<Arc<dyn Analyzer>>,
}

impl FuzzyLikeThisTest {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }

    fn analyzer(&self) -> Arc<dyn Analyzer> {
        Arc::clone(self.analyzer.as_ref().expect("setUp ran"))
    }
}

/// `TestFuzzyLikeThisQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<FuzzyLikeThisTest>(
        "org.apache.lucene.sandbox.queries.TestFuzzyLikeThisQuery",
        vec![
            set_up(do_set_up),
            tear_down(do_tear_down),
            ftest(
                "testClosestEditDistanceMatchComesFirst",
                test_closest_edit_distance_match_comes_first,
            ),
            ftest("testMultiWord", test_multi_word),
            ftest("testNonExistingField", test_non_existing_field),
            ftest("testNoMatchFirstWordBug", test_no_match_first_word_bug),
            ftest(
                "testFuzzyLikeThisQueryEquals",
                test_fuzzy_like_this_query_equals,
            ),
        ],
    )
}

fn do_set_up(f: &mut FuzzyLikeThisTest) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let directory = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;

    // Add series of docs with misspelt names
    add_doc(&writer, "jonathon smythe", "1")?;
    add_doc(&writer, "jonathan smith", "2")?;
    add_doc(&writer, "johnathon smyth", "3")?;
    add_doc(&writer, "johnny smith", "4")?;
    add_doc(&writer, "jonny smith", "5")?;
    add_doc(&writer, "johnathon smythe", "6")?;
    let reader = writer.get_reader()?;
    writer.close()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.directory = Some(directory);
    f.analyzer = Some(analyzer);
    Ok(())
}

fn do_tear_down(f: &mut FuzzyLikeThisTest) -> Result<()> {
    // IOUtils.close(reader, directory, analyzer)
    f.searcher = None;
    let r = f.reader.take().expect("setUp ran").close();
    let d = f.directory.take().expect("setUp ran").close();
    f.analyzer.take().expect("setUp ran").close();
    r?;
    d
}

fn add_doc(writer: &RandomIndexWriter, name: &str, id: &str) -> Result<()> {
    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field("name", name, Store::YES));
    doc.add(LuceneTestCase::new_text_field("id", id, Store::YES));
    writer.add_document(&doc)?;
    Ok(())
}

/// `flt.rewrite(searcher)` then `q.visit(QueryVisitor.termCollector(queryTerms))`.
fn query_terms(f: &FuzzyLikeThisTest, flt: &FuzzyLikeThisQuery) -> Result<HashSet<Term>> {
    let q: Arc<dyn Query> = match flt.rewrite(f.searcher())? {
        Some(rewritten) => rewritten,
        None => Arc::new(flt.clone()),
    };
    let mut query_terms = HashSet::new();
    q.visit(&mut term_collector(&mut query_terms));
    Ok(query_terms)
}

/// The top hit of `searcher.search(flt, 1)` must be the document of id
/// `expected`.
fn assert_top_hit(f: &FuzzyLikeThisTest, flt: FuzzyLikeThisQuery, message: &str) -> Result<()> {
    let top_docs = f.searcher().search(Arc::new(flt), 1)?;
    let sd = &top_docs.score_docs;
    Assert::assert_true_msg("score docs must match 1 doc", !sd.is_empty());
    let doc = f.searcher().stored_fields()?.document(sd[0].doc)?;
    Assert::assert_equals_str(Some(message), Some("2"), doc.get("id").as_deref());
    Ok(())
}

// Tests that idf ranking is not favouring rare mis-spellings over a strong edit-distance match
fn test_closest_edit_distance_match_comes_first(f: &mut FuzzyLikeThisTest) -> Result<()> {
    let mut flt = FuzzyLikeThisQuery::new(10, Some(f.analyzer()));
    flt.add_terms(Some("smith".to_string()), Some("name".to_string()), 2.0, 1)?;
    let query_terms = query_terms(f, &flt)?;
    Assert::assert_true_msg(
        "Should have variant smythe",
        query_terms.contains(&Term::from_text("name", "smythe")),
    );
    Assert::assert_true_msg(
        "Should have variant smith",
        query_terms.contains(&Term::from_text("name", "smith")),
    );
    Assert::assert_true_msg(
        "Should have variant smyth",
        query_terms.contains(&Term::from_text("name", "smyth")),
    );
    assert_top_hit(f, flt, "Should match most similar not most rare variant")
}

// Test multiple input words are having variants produced
fn test_multi_word(f: &mut FuzzyLikeThisTest) -> Result<()> {
    let mut flt = FuzzyLikeThisQuery::new(10, Some(f.analyzer()));
    flt.add_terms(
        Some("jonathin smoth".to_string()),
        Some("name".to_string()),
        2.0,
        1,
    )?;
    let query_terms = query_terms(f, &flt)?;
    Assert::assert_true_msg(
        "Should have variant jonathan",
        query_terms.contains(&Term::from_text("name", "jonathan")),
    );
    Assert::assert_true_msg(
        "Should have variant smith",
        query_terms.contains(&Term::from_text("name", "smith")),
    );
    assert_top_hit(f, flt, "Should match most similar when using 2 words")
}

// LUCENE-4809
fn test_non_existing_field(f: &mut FuzzyLikeThisTest) -> Result<()> {
    let mut flt = FuzzyLikeThisQuery::new(10, Some(f.analyzer()));
    flt.add_terms(
        Some("jonathin smoth".to_string()),
        Some("name".to_string()),
        2.0,
        1,
    )?;
    flt.add_terms(
        Some("jonathin smoth".to_string()),
        Some("this field does not exist".to_string()),
        2.0,
        1,
    )?;
    // don't fail here just because the field doesn't exits
    let query_terms = query_terms(f, &flt)?;
    Assert::assert_true_msg(
        "Should have variant jonathan",
        query_terms.contains(&Term::from_text("name", "jonathan")),
    );
    Assert::assert_true_msg(
        "Should have variant smith",
        query_terms.contains(&Term::from_text("name", "smith")),
    );
    assert_top_hit(f, flt, "Should match most similar when using 2 words")
}

// Test bug found when first query word does not match anything
fn test_no_match_first_word_bug(f: &mut FuzzyLikeThisTest) -> Result<()> {
    let mut flt = FuzzyLikeThisQuery::new(10, Some(f.analyzer()));
    flt.add_terms(
        Some("fernando smith".to_string()),
        Some("name".to_string()),
        2.0,
        1,
    )?;
    let query_terms = query_terms(f, &flt)?;
    Assert::assert_true_msg(
        "Should have variant smith",
        query_terms.contains(&Term::from_text("name", "smith")),
    );
    assert_top_hit(f, flt, "Should match most similar when using 2 words")
}

fn test_fuzzy_like_this_query_equals(_f: &mut FuzzyLikeThisTest) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let mut fltq1 = FuzzyLikeThisQuery::new(10, Some(Arc::clone(&analyzer)));
    fltq1.add_terms(
        Some("javi".to_string()),
        Some("subject".to_string()),
        2.0,
        2,
    )?;
    let mut fltq2 = FuzzyLikeThisQuery::new(10, Some(Arc::clone(&analyzer)));
    fltq2.add_terms(
        Some("javi".to_string()),
        Some("subject".to_string()),
        2.0,
        2,
    )?;
    // assertEquals(message, fltq1, fltq2): `toString()` is `null` for both
    Assert::assert_true_msg(
        "FuzzyLikeThisQuery with same attributes is not equal expected:<null> but was:<null>",
        fltq1.query_eq(&fltq2),
    );
    Ok(())
}
