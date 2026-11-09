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

//! Port of `org.apache.lucene.tests.search.SearchEquivalenceTestBase`:
//! simple base class for checking search equivalence. Extend it, and write
//! tests that create [`random_term`]s (all terms are single characters
//! a-z), and use [`assert_same_set`] and [`assert_subset_of`].
//!
//! The class is declared by [`class()`] (`@SuppressCodecs("SimpleText")`,
//! its `@BeforeClass` and `@AfterClass` hooks); it declares no test. Its
//! static state (`s1`, `s2`, `directory`, `reader`, `analyzer`,
//! `stopword`) is [`STATE`]; its helpers are the functions of this module.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::sort::{Sort, SortField};
use rucene::search::term_query::TermQuery;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::util::automaton::{Automata, CharacterRunAutomaton};
use rucene::util::BytesRef;

use crate::analysis::mock_tokenizer::WHITESPACE;
use crate::analysis::MockAnalyzer;
use crate::index::RandomIndexWriter;
use crate::search::QueryUtils;
use crate::util::java_exceptions::rethrow_java;
use crate::util::lucene_test_case::{SuppressCodecs, TestDirectory, TEST_NIGHTLY};
use crate::util::{LuceneTestCase, TestUtil};

/// The static state of the class.
pub struct EquivalenceState {
    /// `s1`.
    pub s1: IndexSearcher,
    /// `s2`.
    pub s2: IndexSearcher,
    /// `directory`.
    pub directory: TestDirectory,
    /// `reader`.
    pub reader: Arc<dyn DirectoryReader>,
    /// `analyzer`.
    pub analyzer: Arc<MockAnalyzer>,
    /// `stopword`: we always pick a character as a stopword.
    pub stopword: String,
}

/// `s1`, `s2`, `directory`, `reader`, `analyzer`, `stopword`.
pub static STATE: Mutex<Option<EquivalenceState>> = Mutex::new(None);

/// The class state, locked.
pub fn state() -> MutexGuard<'static, Option<EquivalenceState>> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The declaration of `org.apache.lucene.tests.search.SearchEquivalenceTestBase`
/// (`@SuppressCodecs("SimpleText") public abstract class
/// SearchEquivalenceTestBase extends LuceneTestCase`).
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.search.SearchEquivalenceTestBase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
            .extends(&LuceneTestCase::class())
            .method(
                Method::new("beforeClass")
                    .modifiers(modifier::PUBLIC | modifier::STATIC)
                    .throws(&["java.lang.Exception"])
                    .annotate(BeforeClass)
                    .static_body(|| before_class().unwrap_or_else(|e| rethrow_java(e))),
            )
            .method(
                Method::new("afterClass")
                    .modifiers(modifier::PUBLIC | modifier::STATIC)
                    .throws(&["java.lang.Exception"])
                    .annotate(AfterClass)
                    .static_body(|| after_class().unwrap_or_else(|e| rethrow_java(e))),
            )
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance of a `SearchEquivalenceTestBase` subclass: the class keeps
/// its state in statics.
#[derive(Default)]
pub struct EquivalenceTestInstance;

/// `beforeClass()` (`@BeforeClass`).
pub fn before_class() -> Result<()> {
    let random = LuceneTestCase::random();
    let directory = LuceneTestCase::new_directory();
    let stopword = random_char().to_string();
    let stopset = Arc::new(CharacterRunAutomaton::new(Automata::make_string(
        &stopword,
    ))?);
    let analyzer = Arc::new(MockAnalyzer::with_filter(
        &*random,
        Arc::clone(&WHITESPACE),
        false,
        stopset,
    ));
    let iw = RandomIndexWriter::new(
        &*random,
        directory.as_directory(),
        Arc::clone(&analyzer) as Arc<dyn Analyzer>,
    )?;
    // index some docs
    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(1000)
    } else {
        LuceneTestCase::at_least(100)
    };
    for i in 0..num_docs {
        // Java changes the values of the document's two fields; the port's
        // document owns its fields, so each document gets fresh ones (their
        // construction draws no randomness).
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
        doc.add(Box::new(TextField::new(
            "field",
            random_field_contents(),
            Store::NO,
        )?));
        iw.add_document(&doc)?;
    }

    // delete some docs
    let num_deletes = num_docs / 20;
    for _ in 0..num_deletes {
        let to_delete = Term::new(
            "id",
            BytesRef::from_char_sequence(&random.next_int_bound(num_docs).to_string()),
        );
        if random.next_boolean() {
            iw.delete_documents(to_delete)?;
        } else {
            iw.delete_documents_by_query(Arc::new(TermQuery::new(to_delete)))?;
        }
    }

    let reader = iw.get_reader()?;
    let mut s1 = LuceneTestCase::new_searcher(Arc::clone(&reader));
    // Disable the query cache, which converts two-phase iterators to normal iterators, while we
    // want to make sure two-phase iterators are exercised.
    s1.set_query_cache(None);
    let mut s2 = LuceneTestCase::new_searcher(Arc::clone(&reader));
    s2.set_query_cache(None);
    iw.close()?;
    drop(iw);
    *state() = Some(EquivalenceState {
        s1,
        s2,
        directory,
        reader,
        analyzer,
        stopword,
    });
    Ok(())
}

/// `afterClass()` (`@AfterClass`).
pub fn after_class() -> Result<()> {
    let taken = state().take();
    if let Some(EquivalenceState {
        s1,
        s2,
        mut directory,
        reader,
        mut analyzer,
        ..
    }) = taken
    {
        drop(s1);
        drop(s2);
        (Arc::clone(&reader) as Arc<dyn IndexReader>).close()?;
        drop(reader);
        directory.close()?;
        if let Some(analyzer) = Arc::get_mut(&mut analyzer) {
            analyzer.close();
        }
    }
    Ok(())
}

/// `randomFieldContents()`: populate a field with random contents. terms
/// should be single characters in lowercase (a-z) tokenization can be
/// assumed to be on whitespace.
pub fn random_field_contents() -> String {
    let mut sb = String::new();
    let num_terms = LuceneTestCase::random().next_int_bound(15);
    for _ in 0..num_terms {
        if !sb.is_empty() {
            sb.push(' '); // whitespace
        }
        sb.push(random_char());
    }
    sb
}

/// `randomChar()`: returns random character (a-z).
pub fn random_char() -> char {
    let mut c = TestUtil::next_int(&*LuceneTestCase::random(), 'a' as i32, 'z' as i32);
    if LuceneTestCase::random().next_boolean() {
        // bias towards earlier chars, so that chars have a ~ zipfian distribution with earlier
        // chars having a higher frequency
        c = TestUtil::next_int(&*LuceneTestCase::random(), 'a' as i32, c);
    }
    char::from_u32(c as u32).unwrap_or('a')
}

/// `randomTerm()`: returns a term suitable for searching. terms are single
/// characters in lowercase (a-z).
pub fn random_term() -> Term {
    Term::new(
        "field",
        BytesRef::from_char_sequence(&random_char().to_string()),
    )
}

/// `randomFilter()`: returns a random filter over the document set.
pub fn random_filter() -> Result<Arc<dyn Query>> {
    if LuceneTestCase::random().next_boolean() {
        let upper = random_char().to_string();
        Ok(Arc::new(TermRangeQuery::new_string_range(
            "field",
            Some("a"),
            Some(&upper),
            true,
            true,
        )?))
    } else {
        // use a query with a two-phase approximation
        let a = random_char().to_string();
        let b = random_char().to_string();
        Ok(Arc::new(PhraseQuery::with_slop(100, "field", &[&a, &b])?))
    }
}

/// `new BooleanQuery.Builder().add(q, MUST).add(filter, FILTER).build()`.
fn with_filter(q: Arc<dyn Query>, filter: Arc<dyn Query>) -> Result<Arc<dyn Query>> {
    let mut b = Builder::new();
    b.add(q, Occur::MUST)?;
    b.add(filter, Occur::FILTER)?;
    Ok(Arc::new(b.build()))
}

/// `assertSameSet(Query q1, Query q2)`: asserts that the documents returned
/// by `q1` are the same as of those returned by `q2`.
pub fn assert_same_set(q1: Arc<dyn Query>, q2: Arc<dyn Query>) -> Result<()> {
    assert_subset_of(Arc::clone(&q1), Arc::clone(&q2))?;
    assert_subset_of(q2, q1)
}

/// `assertSubsetOf(Query q1, Query q2)`: asserts that the documents
/// returned by `q1` are a subset of those returned by `q2`.
pub fn assert_subset_of(q1: Arc<dyn Query>, q2: Arc<dyn Query>) -> Result<()> {
    // test without a filter
    assert_subset_of_filtered(Arc::clone(&q1), Arc::clone(&q2), None)?;

    // test with some filters (this will sometimes cause advance'ing enough to test it)
    let num_filters = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(10)
    } else {
        LuceneTestCase::at_least(3)
    };
    for _ in 0..num_filters {
        let filter = random_filter()?;
        // incorporate the filter in different ways.
        assert_subset_of_filtered(Arc::clone(&q1), Arc::clone(&q2), Some(Arc::clone(&filter)))?;
        assert_subset_of_filtered(
            filtered_query(Arc::clone(&q1), Arc::clone(&filter))?,
            filtered_query(Arc::clone(&q2), filter)?,
            None,
        )?;
    }
    Ok(())
}

/// `assertSubsetOf(Query q1, Query q2, Query filter)`: both queries will be
/// filtered by `filter`.
pub fn assert_subset_of_filtered(
    q1: Arc<dyn Query>,
    q2: Arc<dyn Query>,
    filter: Option<Arc<dyn Query>>,
) -> Result<()> {
    QueryUtils::check(&*q1);
    QueryUtils::check(&*q2);

    let (q1, q2) = match filter {
        Some(filter) => (
            with_filter(q1, Arc::clone(&filter))?,
            with_filter(q2, filter)?,
        ),
        None => (q1, q2),
    };
    let state = state();
    let state = state.as_ref().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    let max_doc = state.reader.max_doc();
    // we test both INDEXORDER and RELEVANCE because we want to test needsScores=true/false
    let index_order = Sort::new_fields(vec![SortField::FIELD_DOC.clone()])?;
    for sort in [index_order, Sort::new()] {
        // not efficient, but simple!
        let td1 = state.s1.search_with_sort(Arc::clone(&q1), max_doc, &sort)?;
        let td2 = state.s2.search_with_sort(Arc::clone(&q2), max_doc, &sort)?;
        Assert::assert_true_msg(
            &format!(
                "too many hits: {} > {}",
                td1.total_hits.value(),
                td2.total_hits.value()
            ),
            td1.total_hits.value() <= td2.total_hits.value(),
        );

        // fill the superset into a bitset
        let bitset: HashSet<i32> = td2.score_docs.iter().map(|d| d.score_doc.doc).collect();

        // check in the subset, that every bit was set by the super
        for d in &td1.score_docs {
            Assert::assert_true(bitset.contains(&d.score_doc.doc));
        }
    }
    Ok(())
}

/// `assertSameScores(Query q1, Query q2)`: assert that two queries return
/// the same documents and with the same scores.
pub fn assert_same_scores(q1: Arc<dyn Query>, q2: Arc<dyn Query>) -> Result<()> {
    assert_same_set(Arc::clone(&q1), Arc::clone(&q2))?;

    assert_same_scores_filtered(Arc::clone(&q1), Arc::clone(&q2), None)?;
    // also test with some filters to test advancing
    let num_filters = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(10)
    } else {
        LuceneTestCase::at_least(3)
    };
    for _ in 0..num_filters {
        let filter = random_filter()?;
        // incorporate the filter in different ways.
        assert_same_scores_filtered(Arc::clone(&q1), Arc::clone(&q2), Some(Arc::clone(&filter)))?;
        assert_same_scores_filtered(
            filtered_query(Arc::clone(&q1), Arc::clone(&filter))?,
            filtered_query(Arc::clone(&q2), filter)?,
            None,
        )?;
    }
    Ok(())
}

/// `assertSameScores(Query q1, Query q2, Query filter)`.
pub fn assert_same_scores_filtered(
    q1: Arc<dyn Query>,
    q2: Arc<dyn Query>,
    filter: Option<Arc<dyn Query>>,
) -> Result<()> {
    // not efficient, but simple!
    let (q1, q2) = match filter {
        Some(filter) => (
            with_filter(q1, Arc::clone(&filter))?,
            with_filter(q2, filter)?,
        ),
        None => (q1, q2),
    };
    let state = state();
    let state = state.as_ref().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    let max_doc = state.reader.max_doc();
    let td1 = state.s1.search(q1, max_doc)?;
    let td2 = state.s2.search(q2, max_doc)?;
    Assert::assert_equals(td1.total_hits.value(), td2.total_hits.value());
    for (a, b) in td1.score_docs.iter().zip(&td2.score_docs) {
        Assert::assert_equals(a.doc, b.doc);
        Assert::assert_equals_float_msg(None, a.score, b.score, 10e-5);
    }
    Ok(())
}

/// `filteredQuery(Query query, Query filter)`.
pub fn filtered_query(query: Arc<dyn Query>, filter: Arc<dyn Query>) -> Result<Arc<dyn Query>> {
    with_filter(query, filter)
}
