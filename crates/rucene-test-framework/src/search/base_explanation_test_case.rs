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

//! Port of `org.apache.lucene.tests.search.BaseExplanationTestCase`: tests
//! primitive queries (ie: that rewrite to themselves) to insure they match
//! the expected set of docs, and that the score of each match is equal to
//! the value of the scores explanation.
//!
//! The assumption is that if all of the "primitive" queries work well, then
//! anything that rewrites to a primitive will work well also.
//!
//! The class is declared by [`class()`] (its `@BeforeClass` and
//! `@AfterClass` hooks); it declares no test. Its static state
//! (`searcher`, `reader`, `directory`, `analyzer`) is [`STATE`], read
//! through [`searcher`]; its helpers are the functions of this module.
//!
//! **Divergence** (`afterClassTestExplanations`): `analyzer.close()` closes
//! the analyzer through its only handle; the writer and its configuration,
//! which shared it, are gone by then.

use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{Document, SortedDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::boolean_clause::{BooleanClause, Occur};
use rucene::search::boolean_query::Builder;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene::util::BytesRef;

use crate::analysis::MockAnalyzer;
use crate::index::RandomIndexWriter;
use crate::search::CheckHits;
use crate::util::java_exceptions::rethrow_java;
use crate::util::lucene_test_case::TestDirectory;
use crate::util::LuceneTestCase;

/// `KEY`.
pub const KEY: &str = "KEY";
/// `FIELD`: boost on this field is the same as the iterator for the doc.
pub const FIELD: &str = "field";
/// `ALTFIELD`: same contents, but no field boost.
pub const ALTFIELD: &str = "alt";

/// `docFields`.
pub const DOC_FIELDS: [&str; 4] = [
    "w1 w2 w3 w4 w5",
    "w1 w3 w2 w3 zz",
    "w1 xx w2 yy w3",
    "w1 w3 xx w2 yy w3 zz",
];

/// The static state of the class.
pub struct ExplanationState {
    /// `searcher`.
    pub searcher: IndexSearcher,
    /// `reader`.
    pub reader: Arc<dyn DirectoryReader>,
    /// `directory`.
    pub directory: TestDirectory,
    /// `analyzer`.
    pub analyzer: Arc<MockAnalyzer>,
}

/// `searcher`, `reader`, `directory`, `analyzer`.
pub static STATE: Mutex<Option<ExplanationState>> = Mutex::new(None);

/// The class state, locked; `searcher` is its `searcher`.
pub fn state() -> MutexGuard<'static, Option<ExplanationState>> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The declaration of `org.apache.lucene.tests.search.BaseExplanationTestCase`
/// (`public abstract class BaseExplanationTestCase extends LuceneTestCase`).
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.search.BaseExplanationTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(
                Method::new("afterClassTestExplanations")
                    .modifiers(modifier::PUBLIC | modifier::STATIC)
                    .throws(&["java.lang.Exception"])
                    .annotate(AfterClass)
                    .static_body(|| {
                        after_class_test_explanations().unwrap_or_else(|e| rethrow_java(e))
                    }),
            )
            .method(
                Method::new("beforeClassTestExplanations")
                    .modifiers(modifier::PUBLIC | modifier::STATIC)
                    .throws(&["java.lang.Exception"])
                    .annotate(BeforeClass)
                    .static_body(|| {
                        before_class_test_explanations().unwrap_or_else(|e| rethrow_java(e))
                    }),
            )
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance of a `BaseExplanationTestCase` subclass: the class keeps
/// its state in statics.
#[derive(Default)]
pub struct ExplanationTestInstance;

/// `afterClassTestExplanations()` (`@AfterClass`).
pub fn after_class_test_explanations() -> Result<()> {
    let taken = state().take();
    if let Some(ExplanationState {
        searcher,
        reader,
        mut directory,
        mut analyzer,
    }) = taken
    {
        drop(searcher);
        (Arc::clone(&reader) as Arc<dyn IndexReader>).close()?;
        drop(reader);
        directory.close()?;
        if let Some(analyzer) = Arc::get_mut(&mut analyzer) {
            analyzer.close();
        }
    }
    Ok(())
}

/// `beforeClassTestExplanations()` (`@BeforeClass`).
pub fn before_class_test_explanations() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let analyzer = Arc::new(MockAnalyzer::new(&*LuceneTestCase::random()));
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(
        Arc::clone(&analyzer) as Arc<dyn Analyzer>
    );
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(
        &*LuceneTestCase::random(),
        directory.as_directory(),
        config,
    )?;
    let result = (|| -> Result<(Arc<dyn DirectoryReader>, IndexSearcher)> {
        for i in 0..DOC_FIELDS.len() {
            writer.add_document(&create_doc(i))?;
        }
        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        Ok((reader, searcher))
    })();
    let closed = writer.close();
    drop(writer);
    let (reader, searcher) = result?;
    closed?;
    *state() = Some(ExplanationState {
        searcher,
        reader,
        directory,
        analyzer,
    });
    Ok(())
}

/// `createDoc(int index)`.
pub fn create_doc(index: usize) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        KEY,
        &index.to_string(),
        Store::NO,
    )));
    doc.add(Box::new(SortedDocValuesField::new(
        KEY,
        BytesRef::from_char_sequence(&index.to_string()),
    )));
    let f = LuceneTestCase::new_text_field(FIELD, DOC_FIELDS[index], Store::NO);
    doc.add(Box::new(f));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        ALTFIELD,
        DOC_FIELDS[index],
        Store::NO,
    )));
    doc
}

/// `new TermQuery(new Term(field, text))`.
pub fn term_query(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::new(
        field,
        BytesRef::from_char_sequence(text),
    )))
}

/// `qtest(Query q, int[] expDocNrs)`: check the expDocNrs match and have
/// scores that match the explanations. Query may be randomly wrapped in a
/// BooleanQuery with a term that matches no documents.
pub fn qtest(q: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
    let mut q = q;
    if LuceneTestCase::random().next_boolean() {
        let mut bq = Builder::new();
        bq.add(q, Occur::SHOULD)?;
        bq.add(term_query("NEVER", "MATCH"), Occur::SHOULD)?;
        q = Arc::new(bq.build());
    }
    let state = state();
    let searcher = &state
        .as_ref()
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        })
        .searcher;
    CheckHits::check_hit_collector(
        &*LuceneTestCase::random(),
        &q,
        Some(FIELD),
        searcher,
        exp_doc_nrs,
    )
}

/// `bqtest(Query q, int[] expDocNrs)`: tests a query using qtest after
/// wrapping it with both optB and reqB.
pub fn bqtest(q: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
    qtest(req_b(Arc::clone(&q))?, exp_doc_nrs)?;
    qtest(opt_b(q)?, exp_doc_nrs)
}

/// `matchTheseItems(int[] terms)`: convenience subclass of TermsQuery.
pub fn match_these_items(terms: &[i32]) -> Result<Arc<dyn Query>> {
    let mut query = Builder::new();
    for term in terms {
        query.add_clause(BooleanClause::new(
            term_query(KEY, &term.to_string()),
            Occur::SHOULD,
        ))?;
    }
    Ok(Arc::new(query.build()))
}

/// `ta(String[] s)`: helper for generating MultiPhraseQueries.
pub fn ta(s: &[&str]) -> Vec<Term> {
    s.iter()
        .map(|s| Term::new(FIELD, BytesRef::from_char_sequence(s)))
        .collect()
}

/// `optB(Query q)`: MACRO: Wraps a Query in a BooleanQuery so that it is
/// optional, along with a second prohibited clause which will never match
/// anything.
pub fn opt_b(q: Arc<dyn Query>) -> Result<Arc<dyn Query>> {
    let mut bq = Builder::new();
    bq.add(q, Occur::SHOULD)?;
    bq.add(term_query("NEVER", "MATCH"), Occur::MUST_NOT)?;
    Ok(Arc::new(bq.build()))
}

/// `reqB(Query q)`: MACRO: Wraps a Query in a BooleanQuery so that it is
/// required, along with a second optional clause which will match
/// everything.
pub fn req_b(q: Arc<dyn Query>) -> Result<Arc<dyn Query>> {
    let mut bq = Builder::new();
    bq.add(q, Occur::MUST)?;
    bq.add(term_query(FIELD, "w1"), Occur::SHOULD)?;
    Ok(Arc::new(bq.build()))
}
