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

//! Port of `org.apache.lucene.tests.search.MatchesTestBase`: base class for
//! tests checking the `Weight#matches(LeafReaderContext, int)`
//! implementations.
//!
//! The class is declared by [`class()`] (its `setUp()`/`tearDown()`
//! overrides); its abstract `getDocuments()` is the trait
//! [`MatchesTestBase`]; the instance fields (`searcher`, `directory`,
//! `reader`) are [`MatchesState`], whose methods are the class's
//! `protected` helpers.
//!
//! **Divergence** (`checkLabelCount`): Java counts the distinct query
//! objects by identity (`IdentityHashMap`); the port counts the distinct
//! addresses of the `Arc<dyn Query>` each match reports.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, IntPoint, NumericDocValuesField, TextField};
use rucene::error::Result;
use rucene::index::multi_reader::reader_util::sub_index_from_leaves;
use rucene::index::{IndexOptions, IndexReader};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::{Matches, MatchesIterator};
use rucene::search::named_matches::NamedMatches;
use rucene::search::query::Query;
use rucene::search::score_mode::ScoreMode;

use crate::analysis::MockAnalyzer;
use crate::index::RandomIndexWriter;
use crate::util::java_exceptions::rethrow_java;
use crate::util::lucene_test_case::TestDirectory;
use crate::util::LuceneTestCase;

/// `FIELD_WITH_OFFSETS`.
pub const FIELD_WITH_OFFSETS: &str = "field_offsets";
/// `FIELD_NO_OFFSETS`.
pub const FIELD_NO_OFFSETS: &str = "field_no_offsets";
/// `FIELD_DOCS_ONLY`.
pub const FIELD_DOCS_ONLY: &str = "field_docs_only";
/// `FIELD_FREQS`.
pub const FIELD_FREQS: &str = "field_freqs";
/// `FIELD_POINT`.
pub const FIELD_POINT: &str = "field_point";

/// `new FieldType(TextField.TYPE_STORED)` with `options`.
fn stored_text_with(options: IndexOptions) -> FieldType {
    let t = FieldType::new_from(TextField::type_stored());
    t.set_index_options(options)
        .unwrap_or_else(|e| rethrow_java(e));
    t
}

/// `OFFSETS`.
static OFFSETS: LazyLock<FieldType> =
    LazyLock::new(|| stored_text_with(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS));
/// `DOCS`.
static DOCS: LazyLock<FieldType> = LazyLock::new(|| stored_text_with(IndexOptions::DOCS));
/// `DOCS_AND_FREQS`.
static DOCS_AND_FREQS: LazyLock<FieldType> =
    LazyLock::new(|| stored_text_with(IndexOptions::DOCS_AND_FREQS));

/// The declaration of `org.apache.lucene.tests.search.MatchesTestBase`
/// (`public abstract class MatchesTestBase extends LuceneTestCase`), with
/// its `setUp()` and `tearDown()` overrides.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.search.MatchesTestBase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(
                Method::new("tearDown")
                    .throws(&["java.lang.Exception"])
                    .body(|inst: &mut MatchesTestInstance| {
                        tear_down(inst).unwrap_or_else(|e| rethrow_java(e))
                    }),
            )
            .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
                |inst: &mut MatchesTestInstance| set_up(inst).unwrap_or_else(|e| rethrow_java(e)),
            ))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The abstract member of `MatchesTestBase`.
pub trait MatchesTestBase: Send + Sync + 'static {
    /// `getDocuments()`: an array of documents to be indexed.
    fn get_documents(&self) -> Vec<String>;
}

/// The instance of a `MatchesTestBase` subclass.
pub struct MatchesTestInstance {
    /// The subclass.
    pub case: Arc<dyn MatchesTestBase>,
    /// `searcher`, `directory`, `reader`, set by `setUp()`.
    pub state: Option<MatchesState>,
}

impl MatchesTestInstance {
    /// The instance of a subclass.
    pub fn new<T: MatchesTestBase>(case: T) -> Self {
        MatchesTestInstance {
            case: Arc::new(case),
            state: None,
        }
    }
}

/// The instance fields of `MatchesTestBase`.
pub struct MatchesState {
    /// `searcher`.
    pub searcher: IndexSearcher,
    /// `directory`.
    pub directory: TestDirectory,
    /// `reader`.
    pub reader: Arc<dyn IndexReader>,
}

/// `tearDown()` (`@Override`): `reader.close(); directory.close();
/// super.tearDown()`.
pub fn tear_down(inst: &mut MatchesTestInstance) -> Result<()> {
    if let Some(MatchesState {
        searcher,
        mut directory,
        reader,
    }) = inst.state.take()
    {
        drop(searcher);
        reader.close()?;
        drop(reader);
        directory.close()?;
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// `setUp()` (`@Override`).
pub fn set_up(inst: &mut MatchesTestInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    let directory = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*LuceneTestCase::random()),
    )));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(
        &*LuceneTestCase::random(),
        directory.as_directory(),
        config,
    )?;
    let doc_fields = inst.case.get_documents();
    for (i, text) in doc_fields.iter().enumerate() {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_field(
            FIELD_WITH_OFFSETS,
            text,
            &OFFSETS,
        )));
        doc.add(Box::new(LuceneTestCase::new_field(
            FIELD_NO_OFFSETS,
            text,
            TextField::type_stored(),
        )));
        doc.add(Box::new(LuceneTestCase::new_field(
            FIELD_DOCS_ONLY,
            text,
            &DOCS,
        )));
        doc.add(Box::new(LuceneTestCase::new_field(
            FIELD_FREQS,
            text,
            &DOCS_AND_FREQS,
        )));
        doc.add(Box::new(IntPoint::new(FIELD_POINT, &[10])?));
        doc.add(Box::new(NumericDocValuesField::new(FIELD_POINT, 10)));
        doc.add(Box::new(NumericDocValuesField::new("id", i as i64)));
        doc.add(Box::new(LuceneTestCase::new_field(
            "id",
            &i.to_string(),
            TextField::type_stored(),
        )));
        writer.add_document(&doc)?;
    }
    writer.force_merge(1)?;
    let reader: Arc<dyn IndexReader> = writer.get_reader()?;
    writer.close()?;
    drop(writer);
    let searcher =
        LuceneTestCase::new_searcher(LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader))?);
    inst.state = Some(MatchesState {
        searcher,
        directory,
        reader,
    });
    Ok(())
}

/// `TermMatch` (`protected static class`): encapsulates a term position,
/// start and end offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TermMatch {
    /// The position.
    pub position: i32,
    /// The start offset.
    pub start_offset: i32,
    /// The end offset.
    pub end_offset: i32,
}

impl TermMatch {
    /// Create a new TermMatch.
    pub fn new(position: i32, start_offset: i32, end_offset: i32) -> Self {
        TermMatch {
            position,
            start_offset,
            end_offset,
        }
    }
}

impl std::fmt::Display for TermMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}[{}->{}]",
            self.position, self.start_offset, self.end_offset
        )
    }
}

/// `assertNull(it)` of a matches iterator.
fn assert_null_iterator(it: &Option<Box<dyn MatchesIterator>>) {
    if it.is_some() {
        Assert::fail("expected null, but was:<MatchesIterator>");
    }
}

impl MatchesState {
    /// `w.matches(ctx, doc)` of document `global_doc`, with the
    /// `LeafReaderContext` `ReaderUtil.subIndex` finds.
    fn matches_of(
        &self,
        w: &dyn rucene::search::weight::Weight,
        global_doc: i32,
    ) -> Result<Option<Arc<dyn Matches>>> {
        let leaf_contexts = self.searcher.get_leaf_contexts();
        let ctx = &leaf_contexts[sub_index_from_leaves(global_doc, leaf_contexts) as usize];
        let doc = global_doc - ctx.doc_base();
        w.matches(ctx, doc)
    }

    /// `searcher.createWeight(searcher.rewrite(q), scoreMode, 1)`.
    fn weight(
        &self,
        q: Arc<dyn Query>,
        score_mode: ScoreMode,
    ) -> Result<Arc<dyn rucene::search::weight::Weight>> {
        let rewritten = self.searcher.rewrite(q)?;
        self.searcher.create_weight(rewritten, score_mode, 1.0)
    }

    /// `checkMatches(Query q, String field, int[][] expected)`: for a given
    /// query and field, check that expected matches are retrieved.
    pub fn check_matches(
        &self,
        q: Arc<dyn Query>,
        field: &str,
        expected: &[Vec<i32>],
    ) -> Result<()> {
        let w = self.weight(q, ScoreMode::COMPLETE)?;
        for e in expected {
            let Some(matches) = self.matches_of(&*w, e[0])? else {
                Assert::assert_equals(e.len(), 1);
                continue;
            };
            let it = matches.get_matches(field)?;
            if e.len() == 1 {
                assert_null_iterator(&it);
                continue;
            }
            let mut it = it.unwrap_or_else(|| Assert::fail_without_message());
            self.check_field_matches(&mut *it, e)?;
            let mut again = matches
                .get_matches(field)?
                .unwrap_or_else(|| Assert::fail_without_message());
            self.check_field_matches(&mut *again, e)?; // test multiple calls
        }
        Ok(())
    }

    /// `checkLabelCount(Query q, String field, int[] expected)`: for a given
    /// query and field, check that the expected numbers of query labels were
    /// found.
    pub fn check_label_count(
        &self,
        q: Arc<dyn Query>,
        field: &str,
        expected: &[i32],
    ) -> Result<()> {
        let w = self.weight(q, ScoreMode::COMPLETE)?;
        for (i, &e) in expected.iter().enumerate() {
            let Some(matches) = self.matches_of(&*w, i as i32)? else {
                Assert::assert_equals_msg(
                    Some(&format!("Expected to get matches on document {i}")),
                    0,
                    e,
                );
                continue;
            };
            let it = matches.get_matches(field)?;
            if e == 0 {
                assert_null_iterator(&it);
                continue;
            }
            let mut it = it.unwrap_or_else(|| Assert::fail_without_message());
            let mut labels: HashSet<usize> = HashSet::new();
            while it.next()? {
                labels.insert(Arc::as_ptr(&it.get_query()) as *const () as usize);
            }
            Assert::assert_equals(e as usize, labels.len());
        }
        Ok(())
    }

    /// `checkFieldMatches(MatchesIterator it, int[] expected)`: given a
    /// MatchesIterator, check that it has the expected set of start and end
    /// positions.
    pub fn check_field_matches(
        &self,
        it: &mut dyn MatchesIterator,
        expected: &[i32],
    ) -> Result<()> {
        let mut pos = 1;
        while it.next()? {
            Assert::assert_equals_msg(
                Some("Wrong start position"),
                expected[pos],
                it.start_position(),
            );
            Assert::assert_equals_msg(
                Some("Wrong end position"),
                expected[pos + 1],
                it.end_position(),
            );
            Assert::assert_equals_msg(
                Some("Wrong start offset"),
                expected[pos + 2],
                it.start_offset()?,
            );
            Assert::assert_equals_msg(
                Some("Wrong end offset"),
                expected[pos + 3],
                it.end_offset()?,
            );
            pos += 4;
        }
        Assert::assert_equals(expected.len(), pos);
        Ok(())
    }

    /// `checkNoPositionsMatches(Query q, String field, boolean[] expected)`.
    pub fn check_no_positions_matches(
        &self,
        q: Arc<dyn Query>,
        field: &str,
        expected: &[bool],
    ) -> Result<()> {
        let w = self.weight(q, ScoreMode::COMPLETE)?;
        for (i, &e) in expected.iter().enumerate() {
            let matches = self.matches_of(&*w, i as i32)?;
            if e {
                let matches = matches.unwrap_or_else(|| {
                    randomizedtesting::jdk::exception::throw_without_message(
                        "java.lang.NullPointerException",
                    )
                });
                let mut mi = matches.get_matches(field)?.unwrap_or_else(|| {
                    randomizedtesting::jdk::exception::throw_without_message(
                        "java.lang.NullPointerException",
                    )
                });
                Assert::assert_true(mi.next()?);
                Assert::assert_equals(-1, mi.start_position());
                while mi.next()? {
                    Assert::assert_equals(-1, mi.start_position());
                }
            } else if matches.is_some() {
                Assert::fail("expected null, but was:<Matches>");
            }
        }
        Ok(())
    }

    /// `checkSubMatches(Query q, String[][] expectedNames)`: given a query,
    /// check that matches contain the expected NamedQuery wrapper names.
    pub fn check_sub_matches(&self, q: Arc<dyn Query>, expected_names: &[Vec<&str>]) -> Result<()> {
        let w = self.weight(q, ScoreMode::COMPLETE_NO_SCORES)?;
        for (i, names) in expected_names.iter().enumerate() {
            let Some(matches) = self.matches_of(&*w, i as i32)? else {
                Assert::assert_equals_msg(
                    Some(&format!("Expected to get no matches on document {i}")),
                    0,
                    names.len(),
                );
                continue;
            };
            let expected_queries: HashSet<String> = names.iter().map(|s| s.to_string()).collect();
            let actual_queries: HashSet<String> = NamedMatches::find_named_matches(matches)
                .iter()
                .filter_map(|m| {
                    m.as_any()
                        .downcast_ref::<NamedMatches>()
                        .map(|n| n.get_name().to_string())
                })
                .collect();
            let unexpected: Vec<&String> = actual_queries.difference(&expected_queries).collect();
            Assert::assert_equals_msg(
                Some(&format!("Unexpected matching leaf queries: {unexpected:?}")),
                0,
                unexpected.len(),
            );
            let missing: Vec<&String> = expected_queries.difference(&actual_queries).collect();
            Assert::assert_equals_msg(
                Some(&format!("Missing matching leaf queries: {missing:?}")),
                0,
                missing.len(),
            );
        }
        Ok(())
    }

    /// `assertIsLeafMatch(Query q, String field)`: assert that query matches
    /// from a field are all leaf matches and do not contain sub matches.
    pub fn assert_is_leaf_match(&self, q: Arc<dyn Query>, field: &str) -> Result<()> {
        let w = self.weight(q, ScoreMode::COMPLETE)?;
        for i in 0..self.searcher.get_index_reader().max_doc() {
            let Some(matches) = self.matches_of(&*w, i)? else {
                return Ok(());
            };
            let Some(mut mi) = matches.get_matches(field)? else {
                return Ok(());
            };
            while mi.next()? {
                if mi.get_sub_matches()?.is_some() {
                    Assert::fail("expected null, but was:<MatchesIterator>");
                }
            }
        }
        Ok(())
    }

    /// `checkTermMatches(Query q, String field, TermMatch[][][] expected)`:
    /// for a query and field, check that each document's submatches conform
    /// to an expected TermMatch.
    pub fn check_term_matches(
        &self,
        q: Arc<dyn Query>,
        field: &str,
        expected: &[Vec<Vec<TermMatch>>],
    ) -> Result<()> {
        let w = self.weight(q, ScoreMode::COMPLETE)?;
        for (i, e) in expected.iter().enumerate() {
            let Some(matches) = self.matches_of(&*w, i as i32)? else {
                Assert::assert_equals(e.len(), 0);
                continue;
            };
            let it = matches.get_matches(field)?;
            if e.is_empty() {
                assert_null_iterator(&it);
                continue;
            }
            let mut it = it.unwrap_or_else(|| Assert::fail_without_message());
            check_terms(e, &mut *it)?;
        }
        Ok(())
    }
}

/// `checkTerms(TermMatch[][] expected, MatchesIterator it)` (`private`).
fn check_terms(expected: &[Vec<TermMatch>], it: &mut dyn MatchesIterator) -> Result<()> {
    let mut up_to = 0;
    while it.next()? {
        let mut expected_matches: HashSet<TermMatch> = expected[up_to].iter().copied().collect();
        let mut submatches = it.get_sub_matches()?.unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        });
        while submatches.next()? {
            let tm = TermMatch::new(
                submatches.start_position(),
                submatches.start_offset()?,
                submatches.end_offset()?,
            );
            if !expected_matches.remove(&tm) {
                Assert::fail(&format!("Unexpected term match: {tm}"));
            }
        }
        if !expected_matches.is_empty() {
            let missing: Vec<String> = expected_matches.iter().map(|m| m.to_string()).collect();
            Assert::fail(&format!("Missing term matches: {}", missing.join(", ")));
        }
        up_to += 1;
    }
    if up_to + 1 < expected.len() {
        Assert::fail("Missing expected match");
    }
    Ok(())
}
