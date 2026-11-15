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

//! Port of `BooleanQueryTestFacade.java`.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::{directory_reader, IndexReader, LeafReaderContext};
use rucene::search::collector::{
    CollectorManager, LeafCollector, SimpleCollector, SimpleCollectorImpl,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::scorable::Scorable;
use rucene::search::score_mode::ScoreMode;
use rucene::search::CollectionResult;
use rucene_queryparser::surround::parser::query_parser::QueryParser;
use rucene_queryparser::surround::query::basic_query_factory::BasicQueryFactory;

use super::single_field_test_db::SingleFieldTestDb;

/// `public class BooleanQueryTestFacade`.
pub struct BooleanQueryTestFacade<'a> {
    query_text: String,
    expected_doc_nrs: Vec<i32>,
    d_base: &'a SingleFieldTestDb,
    field_name: String,
    qf: Arc<BasicQueryFactory>,
    verbose: bool,
}

/// The state of `class TestCollector extends SimpleCollector`, shared with
/// the `SetOnce<TestCollector>` of `doTest`.
struct TestCollectorState {
    query_text: String,
    expected_doc_nrs: Vec<i32>,
    total_matched: usize,
    encountered: Vec<bool>,
    doc_base: i32,
}

/// `class TestCollector extends SimpleCollector`.
struct TestCollector {
    state: Arc<Mutex<TestCollectorState>>,
}

impl LeafCollector for TestCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc_nr: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let score = scorer.score()?;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let doc_nr = doc_nr + st.doc_base;
        /* System.out.println(docNr + " '" + dBase.getDocs()[docNr] + "': " + score); */
        Assert::assert_true_msg(
            &format!("{}: positive score", st.query_text),
            f64::from(score) > 0.0,
        );
        Assert::assert_true_msg(
            &format!("{}: too many hits", st.query_text),
            st.total_matched < st.expected_doc_nrs.len(),
        );
        let mut i = 0;
        while i < st.expected_doc_nrs.len() {
            if !st.encountered[i] && st.expected_doc_nrs[i] == doc_nr {
                st.encountered[i] = true;
                break;
            }
            i += 1;
        }
        if i == st.expected_doc_nrs.len() {
            Assert::assert_true_msg(
                &format!("{}: doc nr for hit not expected: {doc_nr}", st.query_text),
                false,
            );
        }
        st.total_matched += 1;
        Ok(())
    }
}

impl SimpleCollectorImpl for TestCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }

    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<()> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .doc_base = context.doc_base();
        Ok(())
    }
}

/// `checkNrHits()`.
fn check_nr_hits(st: &TestCollectorState) {
    Assert::assert_equals_msg(
        Some(&format!("{}: nr of hits", st.query_text)),
        st.expected_doc_nrs.len(),
        st.total_matched,
    );
}

/// The anonymous `CollectorManager<Collector, Void>` of `doTest`, whose
/// `newCollector` sets the `SetOnce<TestCollector>`: a second collector
/// throws `AlreadySetException`.
struct Manager {
    query_text: String,
    expected_doc_nrs: Vec<i32>,
    collector: RefCell<Option<Arc<Mutex<TestCollectorState>>>>,
}

impl CollectorManager for Manager {
    type Collector = SimpleCollector<TestCollector>;
    type Output = ();

    fn new_collector(&self) -> Result<Self::Collector> {
        if self.collector.borrow().is_some() {
            randomizedtesting::jdk::exception::throw(
                "org.apache.lucene.util.SetOnce$AlreadySetException",
                "The object cannot be set twice!",
            );
        }
        let state = Arc::new(Mutex::new(TestCollectorState {
            query_text: self.query_text.clone(),
            expected_doc_nrs: self.expected_doc_nrs.clone(),
            total_matched: 0,
            encountered: vec![false; self.expected_doc_nrs.len()],
            doc_base: 0,
        }));
        *self.collector.borrow_mut() = Some(Arc::clone(&state));
        Ok(SimpleCollector::new(TestCollector { state }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<()> {
        Ok(())
    }
}

impl<'a> BooleanQueryTestFacade<'a> {
    /// `new BooleanQueryTestFacade(String queryText, int[] expectedDocNrs,
    /// SingleFieldTestDb dBase, String fieldName, Assert testCase,
    /// BasicQueryFactory qf)`.
    pub fn new(
        query_text: &str,
        expected_doc_nrs: &[i32],
        d_base: &'a SingleFieldTestDb,
        field_name: &str,
        qf: BasicQueryFactory,
    ) -> Self {
        BooleanQueryTestFacade {
            query_text: query_text.to_string(),
            expected_doc_nrs: expected_doc_nrs.to_vec(),
            d_base,
            field_name: field_name.to_string(),
            qf: Arc::new(qf),
            verbose: true,
        }
    }

    /// `setVerbose(boolean verbose)`.
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    /// `doTest()`.
    pub fn do_test(&self) -> Result<()> {
        if self.verbose {
            println!();
            println!("Query: {}", self.query_text);
        }

        let lq = QueryParser::parse(&self.query_text)?;

        /* if (verbose) System.out.println("Srnd: " + lq.toString()); */

        let query = lq.make_lucene_query_field(Some(&self.field_name), &self.qf)?;
        /* if (verbose) System.out.println("Lucene: " + query.toString()); */

        let manager = Manager {
            query_text: self.query_text.clone(),
            expected_doc_nrs: self.expected_doc_nrs.clone(),
            collector: RefCell::new(None),
        };
        let reader = directory_reader::open(self.d_base.get_db())?;
        let result = (|| -> Result<()> {
            let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            searcher.search_with_manager(query, &manager)
        })();
        let closed = reader.close();
        result?;
        closed?;
        // `collector.get().checkNrHits()`: a `null` collector (none created)
        // throws `NullPointerException`.
        match manager.collector.borrow().as_ref() {
            Some(c) => check_nr_hits(&c.lock().unwrap_or_else(|e| e.into_inner())),
            None => randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            ),
        }
        Ok(())
    }
}
