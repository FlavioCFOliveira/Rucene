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

//! Port of `TestIntervals.java`.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::analysis::Analyzer;
use rucene::document::{
    Document, Field, FieldType, NumericDocValuesField, Store, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::doc_values::DocValues;
use rucene::index::multi_reader::reader_util::sub_index_from_leaves;
use rucene::index::{directory_reader, IndexOptions, IndexReader, LeafReaderContext, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::fuzzy_query;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;
use rucene::search::query_visitor::{EmptyQueryVisitor, QueryVisitor};
use rucene::search::term_query::TermQuery;
use rucene::store::Directory;
use rucene::util::automaton::{
    CompiledAutomaton, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene::util::java_collections::JavaHashSet;
use rucene::util::BytesRef;
use rucene_queries::intervals::interval_iterator::NO_MORE_INTERVALS;
use rucene_queries::intervals::interval_matches_iterator::IntervalMatchesIterator;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;
use rucene_queries::intervals::offset_intervals_source::OffsetIntervalsSource;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::intervals::support::{
    after, assert_source_equals, assert_source_not_equals, at_least, before, contained_by,
    containing, extend, fix_field, fuzzy_term, maxgaps, not_containing, or, ordered, overlapping,
    phrase, phrase_s, prefix, prefix_max, range, regexp, term, unordered, unordered_no_overlaps,
    wildcard, within,
};
use crate::queries_support::core_support::{class, random, unchecked};
use crate::queries_support::search_support::{assert_query_equals, static_hook};

/// The query of a `MatchesIterator.getQuery()` that the Java test compares
/// with `assertEquals(expected, mi.getQuery())`: a `null` (`None`) query
/// fails the comparison, as JUnit's `assertEquals` does.
fn non_null(query: Option<Arc<dyn Query>>) -> Arc<dyn Query> {
    query.unwrap_or_else(|| Assert::fail("expected a query but was:<null>"))
}

//   0         1         2         3         4         5         6         7         8         9
//
// 012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789
const FIELD1_DOCS: [&str; 7] = [
    "Nothing of interest to anyone here",
    "Pease porridge hot, pease porridge cold, pease porridge in the pot nine days old.  Some like it hot, some like it cold, some like it in the pot nine days old",
    "Pease porridge cold, pease porridge hot, pease porridge in the pot twelve days old.  Some like it cold, some like it hot, some like it in the fraggle",
    "Nor here, nowt hot going on in pease this one",
    "Pease porridge hot, pease porridge cold, pease porridge in the pot nine years old.  Some like it hot, some like it twelve",
    "Porridge is great",
    "",
];

//   0         1         2         3         4         5         6         7         8         9
//
// 012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789
const FIELD2_DOCS: [&str; 7] = [
    "In Xanadu did Kubla Khan a stately pleasure dome decree",
    "Where Alph the sacred river ran through caverns measureless to man",
    "a b a c b a b c",
    "So thrice five miles of fertile ground",
    "Pease hot porridge porridge",
    "w1 w2 w3 w4 w1 w6 w3 w8 w4 w7 w1 w6",
    "the quick brown fox jumps over the lazy dog",
];

/// The static fields `directory` and `searcher`.
struct Statics {
    directory: TestDirectory,
    searcher: Arc<IndexSearcher>,
    reader: Arc<dyn IndexReader>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// `analyzer`: `new StandardAnalyzer(CharArraySet.EMPTY_SET)`.
static ANALYZER: LazyLock<Arc<dyn Analyzer>> =
    LazyLock::new(|| Arc::new(StandardAnalyzer::new()) as Arc<dyn Analyzer>);

/// `FIELD_TYPE`: `TextField.TYPE_STORED` indexed with offsets.
static FIELD_TYPE: LazyLock<Arc<FieldType>> = LazyLock::new(|| {
    let t = FieldType::new_from(TextField::type_stored());
    unchecked(t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS));
    Arc::new(t)
});

fn searcher() -> Arc<IndexSearcher> {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&guard.as_ref().expect("setupIndex ran").searcher)
}

fn leaves() -> Vec<Arc<LeafReaderContext>> {
    searcher().get_leaf_contexts().to_vec()
}

/// `TestIntervals`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.intervals.TestIntervals",
        vec![
            static_hook("setupIndex", BeforeClass, setup_index),
            static_hook("teardownIndex", AfterClass, teardown_index),
            t(
                "testIntervalsOnFieldWithNoPositions",
                test_intervals_on_field_with_no_positions,
            ),
            t("testExtends", test_extends),
            t("testTermQueryIntervals", test_term_query_intervals),
            t("testOrderedNearIntervals", test_ordered_near_intervals),
            t(
                "testOrderedNearWithDuplicates",
                test_ordered_near_with_duplicates,
            ),
            t("testPhraseIntervals", test_phrase_intervals),
            t("testUnorderedNearIntervals", test_unordered_near_intervals),
            t("testUnorderedWithRepeats", test_unordered_with_repeats),
            t(
                "testUnorderedWithRepeatsAndMaxGaps",
                test_unordered_with_repeats_and_max_gaps,
            ),
            t(
                "testIntervalDisjunctionToStringStability",
                test_interval_disjunction_to_string_stability,
            ),
            t("testIntervalDisjunction", test_interval_disjunction),
            t("testCombinationDisjunction", test_combination_disjunction),
            t("testNesting", test_nesting),
            t("testOffsetIntervals", test_offset_intervals),
            t("testNesting2", test_nesting2),
            t("testInterleavedOrdered", test_interleaved_ordered),
            t("testUnorderedDistinct", test_unordered_distinct),
            t("testContainedBy", test_contained_by),
            t("testContaining", test_containing),
            t("testNotContaining", test_not_containing),
            t("testMaxGaps", test_max_gaps),
            t("testMaxGapsWithRepeats", test_max_gaps_with_repeats),
            t(
                "testMaxGapsWithOnlyRepeats",
                test_max_gaps_with_only_repeats,
            ),
            t("testNestedMaxGaps", test_nested_max_gaps),
            t("testMinimumShouldMatch", test_minimum_should_match),
            t("testMinShouldMatch2", test_min_should_match2),
            t(
                "testDegenerateMinShouldMatch",
                test_degenerate_min_should_match,
            ),
            t("testNoMatchMinShouldMatch", test_no_match_min_should_match),
            t("testDefinedGaps", test_defined_gaps),
            t("testAfter", test_after),
            t("testBefore", test_before),
            t("testWithin", test_within),
            t("testOverlapping", test_overlapping),
            t("testFixedField", test_fixed_field),
            t("testPrefix", test_prefix),
            t("testWildcard", test_wildcard),
            t("testRegexp", test_regexp),
            t("testFuzzyTerm", test_fuzzy_term),
            t("testRange", test_range),
            t("testOpenEndedRange", test_open_ended_range),
            t("testWrappedFilters", test_wrapped_filters),
            t("testMultiTerm", test_multi_term),
            t("testEquality", test_equality),
        ],
    )
}

fn t(name: &str, body: fn() -> Result<()>) -> randomizedtesting::jdk::reflect::Method {
    crate::queries_support::core_support::test(name, body)
}

fn setup_index() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&ANALYZER)));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    for i in 0..FIELD1_DOCS.len() {
        let mut doc = Document::new();
        doc.add(Field::new(
            "field1",
            FIELD1_DOCS[i].to_string(),
            Arc::clone(&FIELD_TYPE),
        )?);
        doc.add(Field::new(
            "field2",
            FIELD2_DOCS[i].to_string(),
            Arc::clone(&FIELD_TYPE),
        )?);
        doc.add(StringField::new("id", i.to_string(), Store::NO)?);
        doc.add(NumericDocValuesField::new("id", i as i64));
        writer.add_document(&doc)?;
    }
    writer.close()?;
    let dir: Arc<dyn Directory> = directory.as_directory();
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir)? as Arc<dyn IndexReader>;
    let searcher = Arc::new(IndexSearcher::new(Arc::clone(&reader))?);
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        directory,
        searcher,
        reader,
    });
    Ok(())
}

fn teardown_index() -> Result<()> {
    let taken = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(Statics {
        directory,
        searcher,
        reader,
    }) = taken
    {
        // IOUtils.close(searcher.getIndexReader(), directory)
        drop(searcher);
        let closed = reader.close();
        let dir_closed = directory.close();
        closed?;
        dir_closed?;
    }
    Ok(())
}

/// `assertTrue(mi.next())` and friends dereference `mi`: Java's `null` is a
/// `NullPointerException`.
fn nn<T>(it: Option<T>) -> T {
    it.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    })
}

/// `assertNotNull(object)`.
fn assert_not_null<T>(it: &Option<T>) {
    if it.is_none() {
        Assert::fail("");
    }
}

/// `assertNull([message,] object)` of an iterator: JUnit's `expected null,
/// but was:<…>`, the object printed as `<class>@<identity hash>` in Java,
/// which no run reproduces (as `MatchesTestBase`'s port prints it).
fn assert_null_iterator<T>(message: Option<&str>, it: &Option<T>) {
    if it.is_some() {
        let prefix = message.map_or_else(String::new, |m| format!("{m} "));
        Assert::fail(&format!("{prefix}expected null, but was:<MatchesIterator>"));
    }
}

/// `checkIntervals(source, field, expectedMatchCount, expectedIntervals)`.
///
/// `expected` is an array of expected position intervals for each document.
/// Each sub-array contains a number of pairs corresponding to (start, end)
/// positions of each interval.
fn check_intervals(
    source: &Arc<dyn IntervalsSource>,
    field: &str,
    expected_match_count: i32,
    expected: &[&[i32]],
) -> Result<()> {
    let mut matched_docs = 0;
    for ctx in leaves() {
        let fake = source.intervals(&format!("{field}fake"), &ctx)?;
        if fake.is_some() {
            Assert::fail("expected null, but was:<IntervalIterator>");
        }
        let leaf = ctx.leaf_reader();
        let mut ids = DocValues::get_numeric(&*leaf, "id")?;
        let Some(mut intervals) = source.intervals(field, &ctx)? else {
            continue;
        };
        for doc in 0..leaf.max_doc() {
            ids.advance(doc)?;
            let mut mi = source.matches(field, &ctx, doc)?;
            let id = ids.long_value()? as i32;
            if intervals.doc_id() == doc
                || (intervals.doc_id() < doc && intervals.advance(doc)? == doc)
            {
                let mut i = 0usize;
                Assert::assert_equals(-1, intervals.start());
                Assert::assert_equals(-1, intervals.end());
                loop {
                    let pos = intervals.next_interval()?;
                    if pos == NO_MORE_INTERVALS {
                        break;
                    }
                    let exp = expected[id as usize];
                    if i >= exp.len() {
                        Assert::fail(&format!(
                            "Unexpected match in doc {id}: {}",
                            intervals.interval_to_string()
                        ));
                    }
                    Assert::assert_equals_msg(
                        Some(&format!(
                            "{}: wrong start value in doc {id}",
                            source.to_source_string()
                        )),
                        exp[i],
                        pos,
                    );
                    Assert::assert_equals_msg(
                        Some("start() != pos returned from nextInterval()"),
                        exp[i],
                        intervals.start(),
                    );
                    Assert::assert_equals_msg(
                        Some(&format!("Wrong end value in doc {id}")),
                        exp[i + 1],
                        intervals.end(),
                    );
                    i += 2;
                    let m = nn(mi.as_mut());
                    Assert::assert_true(m.next()?);
                    Assert::assert_equals_msg(
                        Some(&format!(
                            "{}: wrong start value in match in doc {id}",
                            source.to_source_string()
                        )),
                        intervals.start(),
                        m.start_position(),
                    );
                    Assert::assert_equals_msg(
                        Some(&format!(
                            "{}: wrong end value in match in doc {id}",
                            source.to_source_string()
                        )),
                        intervals.end(),
                        m.end_position(),
                    );
                }
                Assert::assert_equals_msg(
                    Some(&format!(
                        "{}: wrong number of endpoints in doc {id}",
                        source.to_source_string()
                    )),
                    expected[id as usize].len(),
                    i,
                );
                Assert::assert_equals(NO_MORE_INTERVALS, intervals.start());
                Assert::assert_equals(NO_MORE_INTERVALS, intervals.end());
                if i > 0 {
                    matched_docs += 1;
                    Assert::assert_false(nn(mi.as_mut()).next()?);
                } else {
                    assert_null_iterator(
                        Some(&format!("Expected null matches iterator on doc {id}")),
                        &mi,
                    );
                }
            } else {
                // Assume empty matches if the expected array of IDs is shorter
                // than the number of docs.
                if expected.len() > id as usize {
                    Assert::assert_equals(0usize, expected[id as usize].len());
                }
                assert_null_iterator(None, &mi);
            }
        }
    }
    Assert::assert_equals(expected_match_count, matched_docs);
    Ok(())
}

/// The anonymous `QueryVisitor` of `checkVisits`.
struct CountingVisitor<'a> {
    actual_terms: &'a mut JavaHashSet,
    visited_sources: &'a mut i32,
}

impl QueryVisitor for CountingVisitor<'_> {
    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        *self.visited_sources += 1;
        for t in terms {
            self.actual_terms.insert(t.text());
        }
    }

    fn visit_leaf(&mut self, _query: &dyn Query) {
        *self.visited_sources += 1;
    }

    fn get_sub_visitor<'b>(
        &'b mut self,
        occur: Occur,
        _parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'b>
    where
        Self: 'b,
    {
        *self.visited_sources += 1;
        // super.getSubVisitor(occur, parent)
        if occur == Occur::MUST_NOT {
            return Box::new(EmptyQueryVisitor);
        }
        Box::new(self)
    }
}

/// `AbstractCollection.toString()` of a `HashSet<String>`.
fn set_to_string(set: &JavaHashSet) -> String {
    let items: Vec<&str> = set.iter().map(String::as_str).collect();
    format!("[{}]", items.join(", "))
}

/// `checkVisits(source, expectedVisitCount, expectedTerms...)`.
fn check_visits(
    source: &Arc<dyn IntervalsSource>,
    expected_visit_count: i32,
    expected_terms: &[&str],
) {
    let mut actual_terms = JavaHashSet::new();
    let mut visited_sources = 0;
    Arc::clone(source).visit(
        "field",
        &mut CountingVisitor {
            actual_terms: &mut actual_terms,
            visited_sources: &mut visited_sources,
        },
    );
    let mut expected_set: JavaHashSet = expected_terms.iter().map(|s| s.to_string()).collect();
    let actual_snapshot: Vec<String> = actual_terms.iter().cloned().collect();
    for a in &actual_snapshot {
        expected_set.remove(a);
    }
    for e in expected_terms {
        actual_terms.remove(e);
    }
    Assert::assert_equals(expected_visit_count, visited_sources);
    Assert::assert_true_msg(
        &format!(
            "Unexpected terms collected: {}",
            set_to_string(&actual_terms)
        ),
        actual_terms.is_empty(),
    );
    Assert::assert_true_msg(
        &format!("Missing expected terms: {}", set_to_string(&expected_set)),
        expected_set.is_empty(),
    );
}

/// `getMatches(source, doc, field)`.
fn get_matches(
    source: &Arc<dyn IntervalsSource>,
    doc: i32,
    field: &str,
) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
    let leaves = leaves();
    let ord = sub_index_from_leaves(doc, &leaves);
    let ctx = &leaves[ord as usize];
    source.matches(field, ctx, doc - ctx.doc_base())
}

/// `assertMatch(mi, start, end, startOffset, endOffset)`.
fn assert_match(
    mi: &mut dyn MatchesIterator,
    start: i32,
    end: i32,
    start_offset: i32,
    end_offset: i32,
) -> Result<()> {
    Assert::assert_true(mi.next()?);
    Assert::assert_equals_msg(Some("Start position"), start, mi.start_position());
    Assert::assert_equals_msg(Some("End position"), end, mi.end_position());
    Assert::assert_equals_msg(Some("Start offset"), start_offset, mi.start_offset()?);
    Assert::assert_equals_msg(Some("End offset"), end_offset, mi.end_offset()?);
    Ok(())
}

/// `assertGaps(source, doc, field, expectedGaps)`.
fn assert_gaps(
    source: &Arc<dyn IntervalsSource>,
    doc: i32,
    field: &str,
    expected_gaps: &[i32],
) -> Result<()> {
    let leaves = leaves();
    let ord = sub_index_from_leaves(doc, &leaves);
    let ctx = &leaves[ord as usize];
    let mut it = nn(source.intervals(field, ctx)?);
    let doc = doc - ctx.doc_base();
    Assert::assert_equals(doc, it.advance(doc)?);
    for &expected_gap in expected_gaps {
        if it.next_interval()? == NO_MORE_INTERVALS {
            Assert::fail(&format!("Unexpected interval {}", it.interval_to_string()));
        }
        Assert::assert_equals(expected_gap, it.gaps());
    }
    Ok(())
}

/// The match of `getMatches(...)` as the `MatchesIterator` the assertions
/// take (Java dereferences it, so `null` is a `NullPointerException`).
fn mi_of(mi: &mut Option<Box<dyn IntervalMatchesIterator>>) -> &mut dyn MatchesIterator {
    let it: &mut Box<dyn IntervalMatchesIterator> = nn(mi.as_mut());
    &mut **it
}

/// `mi.getSubMatches()` dereferenced.
fn sub_of(mi: &dyn MatchesIterator) -> Result<Box<dyn MatchesIterator>> {
    Ok(nn(mi.get_sub_matches()?))
}

/// `new TermQuery(new Term(field, text))`.
fn tq(field: &str, text: &str) -> TermQuery {
    TermQuery::new(Term::from_text(field, text))
}

/// `int[][]` with Java's array syntax.
macro_rules! iv {
    ($([$($x:expr),*]),* $(,)?) => { &[$(&[$($x),*][..]),*] };
}

fn test_intervals_on_field_with_no_positions() -> Result<()> {
    let e = expect_throws("java.lang.IllegalArgumentException", || {
        term("wibble").intervals("id", &leaves()[0])
    });
    Assert::assert_equals_str(
        None,
        Some("Cannot create an IntervalIterator over field id because it has no indexed positions"),
        e.message(),
    );
    Ok(())
}

fn test_extends() -> Result<()> {
    let source = extend(term("fox"), 1, 2);
    check_intervals(&source, "field2", 1, iv![[], [], [], [], [], [], [2, 5]])?;
    let mut mi = get_matches(&source, 6, "field2")?;
    // LUCENE-10229: we can't report offsets for the "extended" position range because this
    // information
    // is not available from term positions index alone. Report the truth (-1 - not available).
    assert_match(mi_of(&mut mi), 2, 5, -1, -1)
}

fn test_term_query_intervals() -> Result<()> {
    let source = term("porridge");
    check_intervals(
        &source,
        "field1",
        4,
        iv![
            [],
            [1, 1, 4, 4, 7, 7],
            [1, 1, 4, 4, 7, 7],
            [],
            [1, 1, 4, 4, 7, 7],
            [0, 0]
        ],
    )?;
    assert_null_iterator(None, &get_matches(&source, 0, "field1")?);
    assert_null_iterator(None, &get_matches(&source, 2, "no_such_field")?);
    let mut mi = get_matches(&source, 2, "field1")?;
    assert_match(mi_of(&mut mi), 1, 1, 6, 14)?;
    let porridge = tq("field1", "porridge");
    assert_query_equals(&porridge, &*non_null(mi_of(&mut mi).get_query()));
    assert_match(mi_of(&mut mi), 4, 4, 27, 35)?;
    assert_query_equals(&porridge, &*non_null(mi_of(&mut mi).get_query()));
    assert_match(mi_of(&mut mi), 7, 7, 47, 55)?;
    assert_query_equals(&porridge, &*non_null(mi_of(&mut mi).get_query()));
    Assert::assert_false(mi_of(&mut mi).next()?);
    Assert::assert_equals(1, source.min_extent());
    check_visits(&source, 1, &["porridge"]);
    Ok(())
}

fn test_ordered_near_intervals() -> Result<()> {
    let source = ordered(vec![term("pease"), term("hot")]);
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [0, 2, 6, 17], [3, 5, 6, 21], [], [0, 2, 6, 17], []],
    )?;
    assert_null_iterator(None, &get_matches(&source, 3, "field1")?);
    let mut mi = get_matches(&source, 4, "field1")?;
    assert_match(mi_of(&mut mi), 0, 2, 0, 18)?;
    let mut sub = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *sub, 0, 0, 0, 5)?;
    assert_match(&mut *sub, 2, 2, 15, 18)?;
    Assert::assert_false(sub.next()?);
    assert_match(mi_of(&mut mi), 6, 17, 41, 100)?;
    let mut sub = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *sub, 6, 6, 41, 46)?;
    assert_match(&mut *sub, 17, 17, 97, 100)?;
    Assert::assert_false(sub.next()?);
    Assert::assert_false(mi_of(&mut mi).next()?);
    Assert::assert_equals(2, source.min_extent());
    check_visits(&source, 3, &["pease", "hot"]);
    Ok(())
}

fn test_ordered_near_with_duplicates() -> Result<()> {
    let source = ordered(vec![term("pease"), term("pease"), term("porridge")]);
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [0, 4, 3, 7], [0, 4, 3, 7], [], [0, 4, 3, 7], []],
    )?;
    assert_gaps(&source, 1, "field1", &[2, 2])?;

    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 0, 4, 0, 34)?;
    let sub = mi_of(&mut mi).get_sub_matches()?;
    assert_not_null(&sub);
    let mut sub = nn(sub);
    assert_match(&mut *sub, 0, 0, 0, 5)?;
    assert_match(&mut *sub, 3, 3, 20, 25)?;
    assert_match(&mut *sub, 4, 4, 26, 34)?;
    assert_match(mi_of(&mut mi), 3, 7, 20, 55)?;
    Assert::assert_false(mi_of(&mut mi).next()?);
    Ok(())
}

fn test_phrase_intervals() -> Result<()> {
    let source = phrase_s(&["pease", "porridge"]);
    check_intervals(
        &source,
        "field1",
        3,
        iv![
            [],
            [0, 1, 3, 4, 6, 7],
            [0, 1, 3, 4, 6, 7],
            [],
            [0, 1, 3, 4, 6, 7],
            []
        ],
    )?;
    assert_null_iterator(None, &get_matches(&source, 0, "field1")?);
    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 0, 1, 0, 14)?;
    assert_match(mi_of(&mut mi), 3, 4, 20, 34)?;
    let mut sub = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *sub, 3, 3, 20, 25)?;
    assert_query_equals(&tq("field1", "pease"), &*non_null(sub.get_query()));
    assert_match(&mut *sub, 4, 4, 26, 34)?;
    assert_query_equals(&tq("field1", "porridge"), &*non_null(sub.get_query()));
    Assert::assert_false(sub.next()?);
    assert_match(mi_of(&mut mi), 6, 7, 41, 55)?;
    let mut sub = sub_of(mi_of(&mut mi))?;
    Assert::assert_true(sub.next()?);
    assert_query_equals(&tq("field1", "pease"), &*non_null(sub.get_query()));
    Assert::assert_true(sub.next()?);
    assert_query_equals(&tq("field1", "porridge"), &*non_null(sub.get_query()));
    Assert::assert_false(sub.next()?);
    Assert::assert_equals(2, source.min_extent());
    check_visits(&source, 3, &["pease", "porridge"]);
    Ok(())
}

fn test_unordered_near_intervals() -> Result<()> {
    let source = unordered(vec![term("pease"), term("hot")]);
    check_intervals(
        &source,
        "field1",
        4,
        iv![
            [],
            [0, 2, 2, 3, 6, 17],
            [3, 5, 5, 6, 6, 21],
            [3, 7],
            [0, 2, 2, 3, 6, 17],
            []
        ],
    )?;
    assert_null_iterator(None, &get_matches(&source, 0, "field1")?);
    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 0, 2, 0, 18)?;
    assert_match(mi_of(&mut mi), 2, 3, 15, 25)?;
    assert_match(mi_of(&mut mi), 6, 17, 41, 99)?;
    let mut sub = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *sub, 6, 6, 41, 46)?;
    assert_match(&mut *sub, 17, 17, 96, 99)?;
    Assert::assert_false(sub.next()?);
    Assert::assert_false(mi_of(&mut mi).next()?);
    assert_gaps(&source, 1, "field1", &[1, 0, 10])?;
    Assert::assert_equals(2, source.min_extent());
    check_visits(&source, 3, &["pease", "hot"]);
    Ok(())
}

fn test_unordered_with_repeats() -> Result<()> {
    let source = unordered(vec![term("pease"), term("pease"), term("hot")]);
    check_intervals(
        &source,
        "field1",
        3,
        iv![
            [],
            [0, 3, 2, 6, 3, 17],
            [0, 5, 3, 6],
            [],
            [0, 3, 2, 6, 3, 17],
            []
        ],
    )?;
    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 0, 3, 0, 25)?;
    let sub = mi_of(&mut mi).get_sub_matches()?;
    assert_not_null(&sub);
    let mut sub = nn(sub);
    assert_match(&mut *sub, 0, 0, 0, 5)?;
    assert_match(&mut *sub, 2, 2, 15, 18)?;
    assert_match(&mut *sub, 3, 3, 20, 25)
}

fn test_unordered_with_repeats_and_max_gaps() -> Result<()> {
    let source = maxgaps(
        2,
        unordered(vec![term("pease"), term("pease"), term("hot")]),
    );
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [0, 3, 2, 6], [3, 6], [], [0, 3, 2, 6], []],
    )
}

fn test_interval_disjunction_to_string_stability() -> Result<()> {
    /*
    Sanity check that the subclauses of a disjunction are presented in sorted order via the toString() method.
    The exact order is irrelevant, but ensuring stability of output makes the output more useful; e.g., for external
    comparison across different JVMs, etc...
     */
    let size = random().next_int_bound(22) + 4; // ensure a reasonably large minimum number of clauses
    let mut terms: Vec<String> = (0..size)
        .map(|i| char::from(b'a' + i as u8).to_string())
        .collect();
    let expected = format!("or({})", terms.join(","));
    /*
    NOTE: shuffling below shouldn't matter at the moment (because the disjunction subSources are destined for a
    HashMap, so will be reordered anyway); but it might matter if the internal implementation of
    DisjunctionIntervalsSource changes.
     */
    shuffle(&mut terms, &*random());
    let source = or(terms.iter().map(|t| term(t)).collect());
    Assert::assert_equals_str(None, Some(&expected), Some(&source.to_source_string()));
    Ok(())
}

fn test_interval_disjunction() -> Result<()> {
    let source = or(vec![term("pease"), term("hot"), term("notMatching")]);
    check_intervals(
        &source,
        "field1",
        4,
        iv![
            [],
            [0, 0, 2, 2, 3, 3, 6, 6, 17, 17],
            [0, 0, 3, 3, 5, 5, 6, 6, 21, 21],
            [3, 3, 7, 7],
            [0, 0, 2, 2, 3, 3, 6, 6, 17, 17],
            []
        ],
    )?;
    assert_null_iterator(None, &get_matches(&source, 0, "field1")?);
    let mut mi = get_matches(&source, 3, "field1")?;
    assert_match(mi_of(&mut mi), 3, 3, 15, 18)?;
    assert_query_equals(&tq("field1", "hot"), &*non_null(mi_of(&mut mi).get_query()));
    assert_null_iterator(None, &mi_of(&mut mi).get_sub_matches()?);
    assert_match(mi_of(&mut mi), 7, 7, 31, 36)?;
    assert_query_equals(
        &tq("field1", "pease"),
        &*non_null(mi_of(&mut mi).get_query()),
    );
    assert_null_iterator(None, &mi_of(&mut mi).get_sub_matches()?);
    Assert::assert_false(mi_of(&mut mi).next()?);
    Assert::assert_equals(1, source.min_extent());
    check_visits(&source, 4, &["pease", "hot", "notMatching"]);
    Ok(())
}

fn test_combination_disjunction() -> Result<()> {
    let source = ordered(vec![
        or(vec![term("alph"), term("sacred")]),
        term("measureless"),
    ]);
    check_intervals(&source, "field2", 1, iv![[], [3, 8], [], [], [], []])?;
    Assert::assert_equals(2, source.min_extent());
    check_visits(&source, 5, &["alph", "sacred", "measureless"]);
    Ok(())
}

fn test_nesting() -> Result<()> {
    let source = unordered(vec![
        term("pease"),
        term("porridge"),
        or(vec![term("hot"), term("cold")]),
    ]);
    check_intervals(
        &source,
        "field1",
        3,
        iv![
            [],
            [0, 2, 1, 3, 2, 4, 3, 5, 4, 6, 5, 7, 6, 17],
            [0, 2, 1, 3, 2, 4, 3, 5, 4, 6, 5, 7, 6, 17],
            [],
            [0, 2, 1, 3, 2, 4, 3, 5, 4, 6, 5, 7, 6, 17],
            []
        ],
    )?;
    Assert::assert_equals(3, source.min_extent());

    assert_null_iterator(None, &get_matches(&source, 0, "field1")?);
    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 0, 2, 0, 18)?;
    assert_match(mi_of(&mut mi), 1, 3, 6, 25)?;
    assert_match(mi_of(&mut mi), 2, 4, 15, 34)?;
    assert_match(mi_of(&mut mi), 3, 5, 20, 39)?;
    let mut sub = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *sub, 3, 3, 20, 25)?;
    assert_match(&mut *sub, 4, 4, 26, 34)?;
    assert_match(&mut *sub, 5, 5, 35, 39)?;
    Assert::assert_false(sub.next()?);
    assert_match(mi_of(&mut mi), 4, 6, 26, 46)?;
    assert_match(mi_of(&mut mi), 5, 7, 35, 55)?;
    assert_match(mi_of(&mut mi), 6, 17, 41, 99)?;
    Assert::assert_false(mi_of(&mut mi).next()?);

    assert_gaps(&source, 1, "field1", &[0, 0, 0, 0, 0, 0, 9])
}

fn test_offset_intervals() -> Result<()> {
    let source = unordered(vec![
        term("pease"),
        term("porridge"),
        or(vec![term("hot"), term("cold")]),
    ]);

    let before_s: Arc<dyn IntervalsSource> =
        Arc::new(OffsetIntervalsSource::new(Arc::clone(&source), true));
    check_intervals(
        &before_s,
        "field1",
        3,
        iv![
            [],
            [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
            [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
            [],
            [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
            []
        ],
    )?;

    let after_s: Arc<dyn IntervalsSource> =
        Arc::new(OffsetIntervalsSource::new(Arc::clone(&source), false));
    check_intervals(
        &after_s,
        "field1",
        3,
        iv![
            [],
            [3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 18, 18],
            [3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 18, 18],
            [],
            [3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 18, 18],
            []
        ],
    )?;

    check_visits(&before_s, 7, &["pease", "porridge", "hot", "cold"]);
    Ok(())
}

fn test_nesting2() -> Result<()> {
    let source = unordered(vec![
        ordered(vec![term("like"), term("it"), term("cold")]),
        term("pease"),
    ]);
    check_intervals(&source, "field1", 2, iv![[], [6, 21], [6, 17], [], [], []])?;
    assert_null_iterator(None, &get_matches(&source, 0, "field1")?);
    let mut it = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut it), 6, 21, 41, 118)?;
    let mut sub = sub_of(mi_of(&mut it))?;
    assert_match(&mut *sub, 6, 6, 41, 46)?;
    assert_query_equals(&tq("field1", "pease"), &*non_null(sub.get_query()));
    assert_match(&mut *sub, 19, 19, 106, 110)?;
    assert_query_equals(&tq("field1", "like"), &*non_null(sub.get_query()));
    assert_match(&mut *sub, 20, 20, 111, 113)?;
    assert_query_equals(&tq("field1", "it"), &*non_null(sub.get_query()));
    assert_match(&mut *sub, 21, 21, 114, 118)?;
    assert_query_equals(&tq("field1", "cold"), &*non_null(sub.get_query()));
    Assert::assert_false(sub.next()?);
    Assert::assert_false(mi_of(&mut it).next()?);
    Assert::assert_equals(4, source.min_extent());
    Ok(())
}

fn test_interleaved_ordered() -> Result<()> {
    let source = ordered(vec![term("a"), term("b"), term("c")]);
    check_intervals(&source, "field2", 1, iv![[], [], [0, 3, 5, 7], [], [], []])?;
    assert_gaps(&source, 2, "field2", &[1, 0])
}

fn test_unordered_distinct() -> Result<()> {
    check_intervals(
        &unordered_no_overlaps(term("pease"), term("pease")),
        "field1",
        3,
        iv![[], [0, 3, 3, 6], [0, 3, 3, 6], [], [0, 3, 3, 6], []],
    )?;
    check_intervals(
        &unordered_no_overlaps(
            unordered(vec![term("pease"), term("porridge"), term("hot")]),
            term("porridge"),
        ),
        "field1",
        3,
        iv![
            [],
            [1, 4, 2, 7, 4, 17],
            [1, 5, 4, 7],
            [],
            [1, 4, 2, 7, 4, 17],
            []
        ],
    )?;
    check_intervals(
        &unordered_no_overlaps(
            unordered(vec![term("pease"), term("porridge"), term("hot")]),
            term("porridge"),
        ),
        "field2",
        1,
        iv![[], [], [], [], [0, 3], []],
    )?;
    let source = unordered_no_overlaps(
        term("porridge"),
        unordered(vec![term("pease"), term("porridge")]),
    );
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [1, 4, 4, 7], [1, 4, 4, 7], [], [1, 4, 4, 7], []],
    )?;
    // automatic rewrites mean that we end up with 11 sources to visit
    check_visits(&source, 11, &["porridge", "pease"]);
    Ok(())
}

fn test_contained_by() -> Result<()> {
    let source = contained_by(term("porridge"), ordered(vec![term("pease"), term("cold")]));
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [4, 4, 7, 7], [1, 1, 7, 7], [], [4, 4], []],
    )?;
    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 4, 4, 26, 34)?;
    let mut subs = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *subs, 4, 4, 26, 34)?;
    Assert::assert_false(subs.next()?);
    assert_match(mi_of(&mut mi), 7, 7, 47, 55)?;
    let mut subs = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *subs, 7, 7, 47, 55)?;
    Assert::assert_false(subs.next()?);
    Assert::assert_false(mi_of(&mut mi).next()?);
    Assert::assert_equals(1, source.min_extent());
    check_visits(&source, 5, &["porridge", "pease", "cold"]);
    Ok(())
}

fn test_containing() -> Result<()> {
    let source = containing(ordered(vec![term("pease"), term("cold")]), term("porridge"));
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [3, 5, 6, 21], [0, 2, 6, 17], [], [3, 5], []],
    )?;
    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 3, 5, 20, 39)?;
    let mut subs = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *subs, 3, 3, 20, 25)?;
    assert_match(&mut *subs, 4, 4, 26, 34)?;
    assert_match(&mut *subs, 5, 5, 35, 39)?;
    Assert::assert_false(subs.next()?);
    assert_match(mi_of(&mut mi), 6, 21, 41, 118)?;
    let mut subs = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *subs, 6, 6, 41, 46)?;
    assert_match(&mut *subs, 7, 7, 47, 55)?;
    assert_match(&mut *subs, 21, 21, 114, 118)?;
    Assert::assert_false(subs.next()?);
    Assert::assert_false(mi_of(&mut mi).next()?);
    Assert::assert_equals(2, source.min_extent());
    Ok(())
}

fn test_not_containing() -> Result<()> {
    let source = not_containing(ordered(vec![term("porridge"), term("pease")]), term("hot"));
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [4, 6], [1, 3], [], [4, 6], []],
    )?;
    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 4, 6, 26, 46)?;
    let mut subs = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *subs, 4, 4, 26, 34)?;
    assert_match(&mut *subs, 6, 6, 41, 46)?;
    Assert::assert_false(subs.next()?);
    Assert::assert_false(mi_of(&mut mi).next()?);
    Assert::assert_equals(2, source.min_extent());
    Ok(())
}

fn test_max_gaps() -> Result<()> {
    let source = maxgaps(1, unordered(vec![term("w1"), term("w3"), term("w4")]));
    check_intervals(
        &source,
        "field2",
        1,
        iv![[], [], [], [], [], [0, 3, 2, 4, 3, 6]],
    )?;

    let mut mi = get_matches(&source, 5, "field2")?;
    assert_match(mi_of(&mut mi), 0, 3, 0, 11)?;

    Assert::assert_equals(3, source.min_extent());
    assert_source_equals(&source, &source);
    assert_source_equals(
        &source,
        &maxgaps(1, unordered(vec![term("w1"), term("w3"), term("w4")])),
    );
    assert_source_not_equals(
        &source,
        &maxgaps(2, unordered(vec![term("w1"), term("w3"), term("w4")])),
    );
    Ok(())
}

fn test_max_gaps_with_repeats() -> Result<()> {
    let source = maxgaps(11, ordered(vec![term("pease"), term("pease"), term("hot")]));
    check_intervals(&source, "field1", 1, iv![[], [], [0, 5], [], [], []])?;
    assert_gaps(&source, 2, "field1", &[3])
}

fn test_max_gaps_with_only_repeats() -> Result<()> {
    let source = maxgaps(
        1,
        ordered(vec![
            or(vec![term("pease"), term("hot")]),
            or(vec![term("pease"), term("hot")]),
        ]),
    );
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [0, 2, 2, 3], [3, 5, 5, 6], [], [0, 2, 2, 3], []],
    )?;
    assert_gaps(&source, 1, "field1", &[1, 0])
}

fn test_nested_max_gaps() -> Result<()> {
    let source = maxgaps(
        1,
        unordered(vec![ordered(vec![term("w1"), term("w3")]), term("w4")]),
    );
    check_intervals(
        &source,
        "field2",
        1,
        iv![[], [], [], [], [], [0, 3, 3, 6, 4, 8]],
    )?;

    assert_gaps(&source, 5, "field2", &[0, 0, 1])?;

    let mut mi = get_matches(&source, 5, "field2")?;
    assert_match(mi_of(&mut mi), 0, 3, 0, 11)?;
    assert_match(mi_of(&mut mi), 3, 6, 9, 20)?;
    assert_match(mi_of(&mut mi), 4, 8, 12, 26)?;

    Assert::assert_equals(3, source.min_extent());
    Ok(())
}

fn test_minimum_should_match() -> Result<()> {
    let source = at_least(
        3,
        vec![
            term("porridge"),
            term("hot"),
            term("twelve"),
            term("nine"),
            term("pease"),
        ],
    );
    check_intervals(
        &source,
        "field1",
        3,
        iv![
            [],
            [0, 2, 1, 3, 2, 4, 6, 11, 7, 17],
            [3, 5, 4, 6, 5, 7, 6, 11, 7, 21],
            [],
            [0, 2, 1, 3, 2, 4, 6, 11, 7, 17, 11, 21],
            []
        ],
    )?;

    assert_gaps(&source, 1, "field1", &[0, 0, 0, 3, 8])?;

    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 0, 2, 0, 18)?;
    let subs = mi_of(&mut mi).get_sub_matches()?;
    assert_not_null(&subs);
    let mut subs = nn(subs);
    assert_match(&mut *subs, 0, 0, 0, 5)?;
    assert_match(&mut *subs, 1, 1, 6, 14)?;
    assert_match(&mut *subs, 2, 2, 15, 18)?;
    Assert::assert_false(subs.next()?);
    Assert::assert_true(mi_of(&mut mi).next()?);
    Assert::assert_true(mi_of(&mut mi).next()?);
    assert_match(mi_of(&mut mi), 6, 11, 41, 71)?;
    let mut subs = sub_of(mi_of(&mut mi))?;
    assert_match(&mut *subs, 6, 6, 41, 46)?;
    assert_match(&mut *subs, 7, 7, 47, 55)?;
    assert_match(&mut *subs, 11, 11, 67, 71)?;

    Assert::assert_equals(3, source.min_extent());
    Ok(())
}

fn test_min_should_match2() -> Result<()> {
    let source = at_least(
        2,
        vec![
            unordered(vec![term("alph"), term("ran")]),
            term("where"),
            term("river"),
        ],
    );
    let mut mi = get_matches(&source, 1, "field2")?;
    assert_match(mi_of(&mut mi), 0, 4, 0, 27)?; // contains "where" and "river"
    let subs = mi_of(&mut mi).get_sub_matches()?;
    assert_not_null(&subs);
    let mut subs = nn(subs);
    assert_match(&mut *subs, 0, 0, 0, 5)?;
    assert_match(&mut *subs, 4, 4, 22, 27)?;
    Assert::assert_false(subs.next()?);
    assert_match(mi_of(&mut mi), 1, 5, 6, 31)?; // contains "river" and unordered("alph", "run")
    let subs = mi_of(&mut mi).get_sub_matches()?;
    assert_not_null(&subs);
    let mut subs = nn(subs);
    assert_match(&mut *subs, 1, 1, 6, 10)?;
    assert_match(&mut *subs, 4, 4, 22, 27)?;
    assert_match(&mut *subs, 5, 5, 28, 31)?;
    Assert::assert_false(subs.next()?);
    Assert::assert_false(mi_of(&mut mi).next()?);
    Ok(())
}

fn test_degenerate_min_should_match() -> Result<()> {
    let source = ordered(vec![
        at_least(1, vec![term("interest")]),
        at_least(1, vec![term("anyone")]),
    ]);

    let mut mi = get_matches(&source, 0, "field1")?;
    assert_match(mi_of(&mut mi), 2, 4, 11, 29)?;
    let subs = mi_of(&mut mi).get_sub_matches()?;
    assert_not_null(&subs);
    let mut subs = nn(subs);
    assert_match(&mut *subs, 2, 2, 11, 19)?;
    assert_match(&mut *subs, 4, 4, 23, 29)?;
    Assert::assert_false(subs.next()?);
    Assert::assert_false(mi_of(&mut mi).next()?);
    Ok(())
}

fn test_no_match_min_should_match() -> Result<()> {
    let source = at_least(4, vec![term("a"), term("b")]);
    check_intervals(&source, "field", 0, &[])
}

fn test_defined_gaps() -> Result<()> {
    let source = phrase(vec![
        term("pease"),
        extend(term("cold"), 1, 1),
        term("porridge"),
    ]);
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [3, 7], [0, 4], [], [3, 7], []],
    )?;
    Assert::assert_equals(5, source.min_extent());

    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 3, 7, -1, -1)?;
    let sub = mi_of(&mut mi).get_sub_matches()?;
    assert_not_null(&sub);
    let mut sub = nn(sub);
    assert_match(&mut *sub, 3, 3, 20, 25)?;
    assert_match(&mut *sub, 4, 6, -1, -1)?;
    assert_match(&mut *sub, 7, 7, 47, 55)?;

    let source = extend(term("w1"), 5, i32::MAX);
    check_intervals(
        &source,
        "field2",
        1,
        iv![
            [],
            [],
            [],
            [],
            [],
            [0, i32::MAX - 1, 0, i32::MAX - 1, 5, i32::MAX - 1]
        ],
    )?;

    Assert::assert_equals(i32::MAX, source.min_extent());
    Ok(())
}

fn test_after() -> Result<()> {
    let source = after(term("porridge"), ordered(vec![term("pease"), term("cold")]));
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [7, 7], [4, 4, 7, 7], [], [7, 7], []],
    )?;

    let mut mi = get_matches(&source, 1, "field1")?;
    assert_match(mi_of(&mut mi), 7, 7, 47, 55)?;
    let sub = mi_of(&mut mi).get_sub_matches()?;
    assert_not_null(&sub);
    let mut sub = nn(sub);
    assert_match(&mut *sub, 7, 7, 47, 55)?;
    Assert::assert_false(sub.next()?);

    Assert::assert_equals(1, source.min_extent());
    Ok(())
}

fn test_before() -> Result<()> {
    let source = before(term("cold"), term("twelve"));
    check_intervals(&source, "field1", 2, iv![[], [], [2, 2], [], [5, 5], []])?;
    Assert::assert_equals(1, source.min_extent());
    Ok(())
}

fn test_within() -> Result<()> {
    let source = within(term("hot"), 6, or(vec![term("porridge"), term("fraggle")]));
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [2, 2], [5, 5, 21, 21], [], [2, 2], []],
    )?;
    Assert::assert_equals(1, source.min_extent());
    Ok(())
}

fn test_overlapping() -> Result<()> {
    let source = overlapping(
        unordered(vec![term("hot"), term("porridge")]),
        unordered(vec![term("cold"), term("pease")]),
    );
    check_intervals(
        &source,
        "field1",
        3,
        iv![[], [2, 4, 7, 17], [5, 7, 7, 21], [], [2, 4], []],
    )?;

    assert_gaps(&source, 2, "field1", &[1, 13])?;

    let mut mi = get_matches(&source, 1, "field1")?;
    assert_not_null(&mi);
    assert_match(mi_of(&mut mi), 2, 4, 15, 34)?;
    let sub = mi_of(&mut mi).get_sub_matches()?;
    assert_not_null(&sub);
    let mut sub = nn(sub);
    assert_match(&mut *sub, 2, 2, 15, 18)?;
    assert_match(&mut *sub, 4, 4, 26, 34)?;
    Assert::assert_false(sub.next()?);
    assert_match(mi_of(&mut mi), 7, 17, 47, 99)?;

    Assert::assert_equals(2, source.min_extent());
    Ok(())
}

fn test_fixed_field() -> Result<()> {
    let source = phrase(vec![term("alph"), fix_field("field1", term("hot"))]);

    // We search in field2, but 'hot' will report intervals from field1
    check_intervals(&source, "field2", 1, iv![[], [1, 2], [], [], [], []])?;

    let mut mi = get_matches(&source, 1, "field2")?;
    assert_not_null(&mi);
    assert_match(mi_of(&mut mi), 1, 2, 6, 18)
}

/// `expectThrows(IllegalStateException.class, () -> { for (ctx : leaves) s.intervals("field1",
/// ctx); })`, returning the message.
fn expansion_error(s: impl FnOnce() -> Result<Arc<dyn IntervalsSource>>) -> Option<String> {
    let e = expect_throws("java.lang.IllegalStateException", || {
        let s = s()?;
        for ctx in leaves() {
            s.intervals("field1", &ctx)?;
        }
        Ok(())
    });
    e.message().map(str::to_string)
}

fn test_prefix() -> Result<()> {
    let source = prefix("p");
    check_intervals(
        &source,
        "field1",
        5,
        iv![
            [],
            [0, 0, 1, 1, 3, 3, 4, 4, 6, 6, 7, 7, 10, 10, 27, 27],
            [0, 0, 1, 1, 3, 3, 4, 4, 6, 6, 7, 7, 10, 10],
            [7, 7],
            [0, 0, 1, 1, 3, 3, 4, 4, 6, 6, 7, 7, 10, 10],
            [0, 0]
        ],
    )?;
    let mut mi = get_matches(&source, 1, "field1")?;
    assert_not_null(&mi);
    assert_match(mi_of(&mut mi), 0, 0, 0, 5)?;
    assert_match(mi_of(&mut mi), 1, 1, 6, 14)?;

    let no_such = prefix("qqq");
    check_intervals(&no_such, "field1", 0, &[])?;

    let s = prefix_max("p", 1);
    let message = expansion_error(|| Ok(s));
    Assert::assert_equals_str(
        None,
        Some("Automaton [p*] expanded to too many terms (limit 1)"),
        message.as_deref(),
    );

    check_visits(&prefix("p"), 1, &[]);
    Ok(())
}

fn test_wildcard() -> Result<()> {
    let source = wildcard("?ot");
    check_intervals(
        &source,
        "field1",
        4,
        iv![
            [],
            [2, 2, 10, 10, 17, 17, 27, 27],
            [5, 5, 10, 10, 21, 21],
            [3, 3],
            [2, 2, 10, 10, 17, 17],
            []
        ],
    )?;
    let mut mi = get_matches(&source, 4, "field1")?;
    assert_not_null(&mi);
    assert_match(mi_of(&mut mi), 2, 2, 15, 18)?;
    assert_match(mi_of(&mut mi), 10, 10, 63, 66)?;
    assert_match(mi_of(&mut mi), 17, 17, 97, 100)?;

    let message = expansion_error(|| {
        Intervals::wildcard_with_max_expansions(&BytesRef::from_char_sequence("?ot"), 1)
    });
    Assert::assert_equals_str(
        None,
        Some("Automaton [?ot] expanded to too many terms (limit 1)"),
        message.as_deref(),
    );

    check_visits(&wildcard("p??"), 1, &[]);
    Ok(())
}

fn test_regexp() -> Result<()> {
    let source = regexp(".ot");
    check_intervals(
        &source,
        "field1",
        4,
        iv![
            [],
            [2, 2, 10, 10, 17, 17, 27, 27],
            [5, 5, 10, 10, 21, 21],
            [3, 3],
            [2, 2, 10, 10, 17, 17],
            []
        ],
    )?;
    let mut mi = get_matches(&source, 4, "field1")?;
    assert_not_null(&mi);
    assert_match(mi_of(&mut mi), 2, 2, 15, 18)?;
    assert_match(mi_of(&mut mi), 10, 10, 63, 66)?;
    assert_match(mi_of(&mut mi), 17, 17, 97, 100)?;

    let message = expansion_error(|| {
        Intervals::regexp_with_max_expansions(&BytesRef::from_char_sequence(".ot"), 1)
    });
    Assert::assert_equals_str(
        None,
        Some("Automaton [.ot] expanded to too many terms (limit 1)"),
        message.as_deref(),
    );

    check_visits(&regexp("p.*"), 1, &[]);
    Ok(())
}

fn test_fuzzy_term() -> Result<()> {
    let source = fuzzy_term("kot", 1); // matches 'pot'
    check_intervals(
        &source,
        "field1",
        4,
        iv![
            [],
            [2, 2, 10, 10, 17, 17, 27, 27],
            [5, 5, 10, 10, 21, 21],
            [3, 3],
            [2, 2, 10, 10, 17, 17],
            []
        ],
    )?;
    let mut mi = get_matches(&source, 4, "field1")?;
    assert_not_null(&mi);
    assert_match(mi_of(&mut mi), 2, 2, 15, 18)?;
    assert_match(mi_of(&mut mi), 10, 10, 63, 66)?;
    assert_match(mi_of(&mut mi), 17, 17, 97, 100)?;

    // Check limits.
    let message = expansion_error(|| {
        Intervals::fuzzy_term_with(
            "kot",
            1,
            fuzzy_query::DEFAULT_PREFIX_LENGTH,
            fuzzy_query::DEFAULT_TRANSPOSITIONS,
            1,
        )
    });
    Assert::assert_equals_str(
        None,
        Some("Automaton [kot~1] expanded to too many terms (limit 1)"),
        message.as_deref(),
    );

    check_visits(&fuzzy_term("kot", fuzzy_query::DEFAULT_MAX_EDITS), 1, &[]);
    Ok(())
}

fn test_range() -> Result<()> {
    let source = range(Some("cold"), Some("hot"), true, true);
    check_intervals(
        &source,
        "field1",
        6,
        iv![
            [5, 5],
            [2, 2, 5, 5, 12, 12, 17, 17, 21, 21, 29, 29],
            [2, 2, 5, 5, 12, 12, 17, 17, 21, 21, 27, 27],
            [1, 1, 3, 3, 4, 4],
            [2, 2, 5, 5, 17, 17],
            [2, 2]
        ],
    )?;
    let mut mi = get_matches(&source, 3, "field1")?;
    assert_not_null(&mi);
    assert_match(mi_of(&mut mi), 1, 1, 4, 8)?;
    assert_match(mi_of(&mut mi), 3, 3, 15, 18)?;
    assert_match(mi_of(&mut mi), 4, 4, 19, 24)?;

    let message = expansion_error(|| {
        Intervals::range_with_max_expansions(
            Some(&BytesRef::from_char_sequence("cold")),
            Some(&BytesRef::from_char_sequence("hot")),
            true,
            true,
            1,
        )
    });
    Assert::assert_equals_str(
        None,
        Some("Automaton [{cold,hot}] expanded to too many terms (limit 1)"),
        message.as_deref(),
    );

    check_visits(&source, 1, &[]);
    Ok(())
}

fn test_open_ended_range() -> Result<()> {
    {
        let source = range(Some("porridge"), None, false, false);
        check_intervals(
            &source,
            "field1",
            5,
            iv![
                [3, 3],
                [9, 9, 10, 10, 14, 14, 18, 18, 22, 22, 26, 26, 27, 27],
                [9, 9, 10, 10, 11, 11, 14, 14, 18, 18, 22, 22, 26, 26],
                [8, 8],
                [9, 9, 10, 10, 12, 12, 14, 14, 18, 18, 21, 21],
                []
            ],
        )?;
        let mut mi = get_matches(&source, 3, "field1")?;
        assert_not_null(&mi);
        assert_match(mi_of(&mut mi), 8, 8, 37, 41)?;
    }

    {
        let source = range(None, Some("anyone"), false, true);
        check_intervals(&source, "field1", 1, iv![[4, 4], [], [], [], [], []])?;
        let mut mi = get_matches(&source, 0, "field1")?;
        assert_not_null(&mi);
        assert_match(mi_of(&mut mi), 4, 4, 23, 29)?;
    }
    Ok(())
}

fn test_wrapped_filters() -> Result<()> {
    let source = or(vec![
        term("nine"),
        maxgaps(
            1,
            or(vec![
                ordered(vec![term("pease"), term("hot")]),
                ordered(vec![term("pease"), term("cold")]),
            ]),
        ),
    ]);
    check_intervals(
        &source,
        "field1",
        3,
        iv![
            [],
            [0, 2, 3, 5, 11, 11, 28, 28],
            [0, 2, 3, 5],
            [],
            [0, 2, 3, 5, 11, 11],
            []
        ],
    )
}

fn test_multi_term() -> Result<()> {
    let re = RegExp::new("p.*e")?;
    let automaton = unchecked(
        Operations::determinize(&re.to_automaton()?, DEFAULT_DETERMINIZE_WORK_LIMIT)
            .map_err(rucene::error::LuceneError::from),
    );

    let source = Intervals::multiterm(
        CompiledAutomaton::from_automaton(automaton.clone())?,
        &re.to_string(),
    )?;

    check_intervals(
        &source,
        "field1",
        5,
        iv![
            [],
            [0, 0, 1, 1, 3, 3, 4, 4, 6, 6, 7, 7],
            [0, 0, 1, 1, 3, 3, 4, 4, 6, 6, 7, 7],
            [7, 7],
            [0, 0, 1, 1, 3, 3, 4, 4, 6, 6, 7, 7],
            [0, 0]
        ],
    )?;

    let message = expansion_error(|| {
        Intervals::multiterm_with_max_expansions(
            CompiledAutomaton::from_automaton(automaton.clone())?,
            1,
            &re.to_string(),
        )
    });
    Assert::assert_equals_str(
        None,
        Some("Automaton [\\p(.)*\\e] expanded to too many terms (limit 1)"),
        message.as_deref(),
    );

    check_visits(&source, 1, &[]);
    Ok(())
}

// basic test for equality and inequality of instances created by the factories
fn test_equality() -> Result<()> {
    assert_source_equals(&term("wibble"), &term("wibble"));
    assert_source_equals(&prefix_max("p", 1), &prefix_max("p", 1));
    assert_source_equals(&fuzzy_term("kot", 1), &fuzzy_term("kot", 1));
    assert_source_equals(&regexp(".*ot"), &regexp(".*ot"));
    assert_source_equals(&wildcard("*.txt"), &wildcard("*.txt"));
    assert_source_equals(
        &range(Some("cold"), Some("hot"), true, true),
        &range(Some("cold"), Some("hot"), true, true),
    );

    assert_source_not_equals(&term("wibble"), &term("wobble"));
    assert_source_not_equals(&prefix_max("p", 1), &prefix_max("b", 1));
    assert_source_not_equals(&fuzzy_term("kot", 1), &fuzzy_term("kof", 1));
    assert_source_not_equals(&regexp(".*ot"), &regexp(".*at"));
    assert_source_not_equals(&wildcard("*.txt"), &wildcard("*.tat"));
    assert_source_not_equals(
        &range(Some("warm"), Some("hot"), true, true),
        &range(Some("cold"), Some("hot"), true, true),
    );
    Ok(())
}
