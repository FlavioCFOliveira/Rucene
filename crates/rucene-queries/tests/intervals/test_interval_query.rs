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

//! Port of `TestIntervalQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::search::boost_query::BoostQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::{query_to_string, Query};
use rucene_queries::intervals::interval_query::IntervalQuery;
use rucene_queries::intervals::intervals_source::IntervalsSource;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::intervals::one_time_interval_source::OneTimeIntervalSource;
use crate::intervals::support::{
    contained_by, containing, extend, fuzzy_term, maxgaps, maxwidth, non_overlapping,
    not_contained_by, not_containing, not_within, or, ordered, phrase, phrase_s, prefix,
    prefix_max, range, regexp, term, unordered, unordered_no_overlaps, wildcard,
};
use crate::queries_support::core_support::{random, IAE};
use crate::queries_support::search_support::{
    assert_query_equals, fixture_class, ftest, set_up, tear_down,
};

/// `field`.
pub const FIELD: &str = "field";

/// `docFields`.
const DOC_FIELDS: [&str; 13] = [
    "w1 w2 w3 w4 w5",
    "w1 w3 w2 w3",
    "w1 xx w2 w4 yy w3",
    "w1 w3 xx w2 yy w3",
    "w2 w1",
    "w2 w1 w3 w2 w4",
    "coordinate genome mapping research",
    "coordinate genome research",
    "greater new york",
    "x x x x x intend x x x message x x x message x x x addressed x x",
    "issue with intervals queries from search engine. So it's a big issue for us as we need to do ordered searches. Thank you to help us concerning that issue",
    "場外好朋友",
    "alice bob alice alice carl alice bob alice carl",
];

/// The instance of `TestIntervalQuery`.
#[derive(Default)]
pub struct IntervalQueryTest {
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    directory: Option<TestDirectory>,
}

impl IntervalQueryTest {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }

    fn check_hits(&self, query: Arc<dyn Query>, results: &[i32]) -> Result<()> {
        CheckHits::check_hits(&*random(), &query, Some(FIELD), self.searcher(), results)
    }
}

/// `new IntervalQuery(field, source)` as a `Query`.
fn iq(source: Arc<dyn IntervalsSource>) -> Arc<dyn Query> {
    Arc::new(IntervalQuery::new(FIELD, source))
}

/// `TestIntervalQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<IntervalQueryTest>(
        "org.apache.lucene.queries.intervals.TestIntervalQuery",
        vec![
            set_up(do_set_up),
            tear_down(do_tear_down),
            ftest("testPhraseQuery", test_phrase_query),
            ftest("testOrderedNearQueryWidth3", test_ordered_near_query_width3),
            ftest("testOrderedNearQueryWidth4", test_ordered_near_query_width4),
            ftest("testOrderedNearQueryGaps1", test_ordered_near_query_gaps1),
            ftest("testOrderedNearQueryGaps2", test_ordered_near_query_gaps2),
            ftest("testNestedOrderedNearQuery", test_nested_ordered_near_query),
            ftest("testUnorderedQuery", test_unordered_query),
            ftest("testNonOverlappingQuery", test_non_overlapping_query),
            ftest("testFieldInToString", test_field_in_to_string),
            ftest("testNullConstructorArgs", test_null_constructor_args),
            ftest("testNotWithinQuery", test_not_within_query),
            ftest("testNotContainingQuery", test_not_containing_query),
            ftest("testContainingQuery", test_containing_query),
            ftest("testContainedByQuery", test_contained_by_query),
            ftest("testNotContainedByQuery", test_not_contained_by_query),
            ftest("testNonExistentTerms", test_non_existent_terms),
            ftest("testNestedOr", test_nested_or),
            ftest("testNestedOrWithGaps", test_nested_or_with_gaps),
            ftest(
                "testNestedOrWithinDifference",
                test_nested_or_within_difference,
            ),
            ftest(
                "testNestedOrWithinConjunctionFilter",
                test_nested_or_within_conjunction_filter,
            ),
            ftest("testUnordered", test_unordered),
            ftest("testUnorderedNoOverlaps", test_unordered_no_overlaps),
            ftest(
                "testNestedOrInUnorderedMaxGaps",
                test_nested_or_in_unordered_max_gaps,
            ),
            ftest("testUnorderedWithNoGap", test_unordered_with_no_gap),
            ftest("testOrderedWithGaps", test_ordered_with_gaps),
            ftest("testOrderedWithGaps2", test_ordered_with_gaps2),
            ftest("testOrderedWithNoGap", test_ordered_with_no_gap),
            ftest("testNestedOrInContainedBy", test_nested_or_in_contained_by),
            ftest("testDefinedGaps", test_defined_gaps),
            ftest("testScoring", test_scoring),
            ftest("testAdvanceBehavior", test_advance_behavior),
            ftest("testUnicodePrefix", test_unicode_prefix),
            ftest("testExtendDisjunctions", test_extend_disjunctions),
            ftest("testEquality", test_equality),
        ],
    )
}

fn do_set_up(f: &mut IntervalQueryTest) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    for doc_field in DOC_FIELDS {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            FIELD,
            doc_field,
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    let reader = writer.get_reader()?;
    writer.close()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.directory = Some(directory);
    Ok(())
}

fn do_tear_down(f: &mut IntervalQueryTest) -> Result<()> {
    f.searcher = None;
    f.reader.take().expect("setUp ran").close()?;
    f.directory.take().expect("setUp ran").close()
}

fn test_phrase_query(f: &mut IntervalQueryTest) -> Result<()> {
    f.check_hits(iq(phrase(vec![term("w1"), term("w2")])), &[0])
}

fn test_ordered_near_query_width3(f: &mut IntervalQueryTest) -> Result<()> {
    f.check_hits(
        iq(maxwidth(3, ordered(vec![term("w1"), term("w2")]))),
        &[0, 1, 2, 5],
    )
}

fn test_ordered_near_query_width4(f: &mut IntervalQueryTest) -> Result<()> {
    f.check_hits(
        iq(maxwidth(4, ordered(vec![term("w1"), term("w2")]))),
        &[0, 1, 2, 3, 5],
    )
}

fn test_ordered_near_query_gaps1(f: &mut IntervalQueryTest) -> Result<()> {
    f.check_hits(
        iq(maxgaps(1, ordered(vec![term("w1"), term("w2")]))),
        &[0, 1, 2, 5],
    )
}

fn test_ordered_near_query_gaps2(f: &mut IntervalQueryTest) -> Result<()> {
    f.check_hits(
        iq(maxgaps(2, ordered(vec![term("w1"), term("w2")]))),
        &[0, 1, 2, 3, 5],
    )
}

fn test_nested_ordered_near_query(f: &mut IntervalQueryTest) -> Result<()> {
    // onear/1(w1, onear/2(w2, w3))
    let q = iq(ordered(vec![
        term("w1"),
        maxwidth(3, ordered(vec![term("w2"), term("w3")])),
    ]));
    f.check_hits(q, &[0, 1, 3])
}

fn test_unordered_query(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(unordered(vec![term("w1"), term("w3")]));
    f.check_hits(q, &[0, 1, 2, 3, 5])
}

fn test_non_overlapping_query(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(non_overlapping(
        unordered(vec![term("w1"), term("w3")]),
        unordered(vec![term("w2"), term("w4")]),
    ));
    f.check_hits(q, &[1, 3, 5])
}

fn test_field_in_to_string(f: &mut IntervalQueryTest) -> Result<()> {
    let field_w1: Arc<dyn Query> = Arc::new(IntervalQuery::new(FIELD, term("w1")));
    Assert::assert_true(query_to_string(&*field_w1).contains(FIELD));
    let field2 = format!("{FIELD}2");
    let f2w1 = IntervalQuery::new(field2.as_str(), term("w1"));
    Assert::assert_true(query_to_string(&f2w1).contains(&format!("{field2}:")));
    Assert::assert_false_msg(
        "suppress default field",
        f2w1.to_query_string(&field2).contains(&field2),
    );
    let doc = f.searcher().search(Arc::clone(&field_w1), 1)?.score_docs[0].doc;
    let explain = f
        .searcher()
        .explain(iq(ordered(vec![term("w1"), term("w2")])), doc)?;
    Assert::assert_true(explain.to_string().contains(FIELD));
    Ok(())
}

/// `testNullConstructorArgs`: Java's `new IntervalQuery(null, …)` and
/// `new IntervalQuery("field", null)` throw `NullPointerException` from
/// `Objects.requireNonNull`. The port's constructor takes a `String` and an
/// `Arc<dyn IntervalsSource>`, neither of which can be `null`, so both calls
/// are rejected at compile time (the declared divergence of
/// `IntervalQuery::with_score_function`): the contract holds with no call to
/// make. The method stays registered so the suite keeps Java's method list.
fn test_null_constructor_args(_f: &mut IntervalQueryTest) -> Result<()> {
    Ok(())
}

fn test_not_within_query(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(not_within(term("w1"), 1, term("w2")));
    f.check_hits(q, &[1, 2, 3])
}

fn test_not_containing_query(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(not_containing(
        unordered(vec![term("w1"), term("w2")]),
        term("w3"),
    ));
    f.check_hits(q, &[0, 2, 4, 5])
}

fn test_containing_query(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(containing(
        unordered(vec![term("w1"), term("w2")]),
        term("w3"),
    ));
    f.check_hits(q, &[1, 3, 5])
}

fn test_contained_by_query(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(contained_by(
        term("w3"),
        unordered(vec![term("w1"), term("w2")]),
    ));
    f.check_hits(q, &[1, 3, 5])
}

fn test_not_contained_by_query(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(not_contained_by(
        term("w2"),
        unordered(vec![term("w1"), term("w4")]),
    ));
    f.check_hits(q, &[1, 3, 4, 5])
}

fn test_non_existent_terms(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(ordered(vec![term("w0"), term("w2")]));
    f.check_hits(q, &[])
}

fn test_nested_or(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(phrase(vec![
        term("coordinate"),
        or(vec![phrase_s(&["genome", "mapping"]), term("genome")]),
        term("research"),
    ]));
    f.check_hits(q, &[6, 7])
}

fn test_nested_or_with_gaps(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(phrase(vec![
        term("coordinate"),
        or(vec![term("genome"), extend(term("mapping"), 1, 0)]),
        term("research"),
    ]));
    f.check_hits(q, &[6, 7])
}

fn test_nested_or_within_difference(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(phrase(vec![
        term("coordinate"),
        not_containing(
            or(vec![phrase_s(&["genome", "mapping"]), term("genome")]),
            term("wibble"),
        ),
        term("research"),
    ]));
    f.check_hits(q, &[6, 7])
}

fn test_nested_or_within_conjunction_filter(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(phrase(vec![
        term("coordinate"),
        containing(
            or(vec![phrase_s(&["genome", "mapping"]), term("genome")]),
            term("genome"),
        ),
        term("research"),
    ]));
    f.check_hits(q, &[6, 7])?;
    let q = iq(phrase(vec![
        term("greater"),
        or(vec![phrase_s(&["new", "york"]), term("york")]),
    ]));
    f.check_hits(q, &[8])
}

fn test_unordered(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(unordered(vec![
        term("w1"),
        ordered(vec![term("w3"), term("yy")]),
    ]));
    f.check_hits(q, &[3])
}

fn test_unordered_no_overlaps(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(maxgaps(
        3,
        unordered_no_overlaps(
            term("addressed"),
            maxgaps(
                5,
                unordered_no_overlaps(
                    term("message"),
                    maxgaps(3, unordered(vec![term("intend"), term("message")])),
                ),
            ),
        ),
    ));
    f.check_hits(q, &[9])?;
    let q = iq(unordered_no_overlaps(
        term("w2"),
        or(vec![term("w2"), term("w3")]),
    ));
    f.check_hits(q, &[0, 1, 2, 3, 5])
}

fn test_nested_or_in_unordered_max_gaps(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(maxgaps(
        1,
        unordered(vec![
            or(vec![
                term("coordinate"),
                phrase_s(&["coordinate", "genome"]),
            ]),
            term("research"),
        ]),
    ));
    f.check_hits(q, &[6, 7])
}

fn test_unordered_with_no_gap(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(maxgaps(
        0,
        unordered(vec![term("w3"), unordered(vec![term("w1"), term("w5")])]),
    ));
    f.check_hits(q, &[0])
}

fn test_ordered_with_gaps(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(maxgaps(
        1,
        ordered(vec![term("issue"), term("search"), term("ordered")]),
    ));
    f.check_hits(q, &[])
}

fn test_ordered_with_gaps2(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(maxgaps(
        1,
        ordered(vec![term("alice"), term("bob"), term("carl")]),
    ));
    f.check_hits(q, &[12])
}

fn test_ordered_with_no_gap(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(maxgaps(
        0,
        ordered(vec![ordered(vec![term("w1"), term("w4")]), term("w5")]),
    ));
    f.check_hits(q, &[0])
}

fn test_nested_or_in_contained_by(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(contained_by(
        term("genome"),
        or(vec![
            term("coordinate"),
            ordered(vec![term("coordinate"), term("research")]),
        ]),
    ));
    f.check_hits(q, &[6, 7])
}

fn test_defined_gaps(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(phrase(vec![term("w1"), extend(term("w2"), 1, 0)]));
    f.check_hits(q, &[1, 2, 5])
}

fn test_scoring(f: &mut IntervalQueryTest) -> Result<()> {
    let source = ordered(vec![or(vec![term("w1"), term("w2")]), term("w3")]);
    let q = iq(Arc::clone(&source));
    let td = f.searcher().search(Arc::clone(&q), 10)?;
    Assert::assert_equals(5i64, td.total_hits.value());
    Assert::assert_equals(1, td.score_docs[0].doc);
    Assert::assert_equals(3, td.score_docs[1].doc);
    Assert::assert_equals(0, td.score_docs[2].doc);
    Assert::assert_equals(5, td.score_docs[3].doc);
    Assert::assert_equals(2, td.score_docs[4].doc);

    let boost_q: Arc<dyn Query> = Arc::new(BoostQuery::new(Arc::clone(&q), 2.0)?);
    let boost_td = f.searcher().search(boost_q, 10)?;
    Assert::assert_equals(5i64, boost_td.total_hits.value());
    for i in 0..5 {
        Assert::assert_equals_float_msg(
            None,
            td.score_docs[i].score * 2.0,
            boost_td.score_docs[i].score,
            0.0,
        );
    }

    // change the pivot - order should remain the same
    let q1: Arc<dyn Query> = Arc::new(IntervalQuery::with_pivot(FIELD, Arc::clone(&source), 2.0)?);
    let td1 = f.searcher().search(q1, 10)?;
    Assert::assert_equals(5i64, td1.total_hits.value());
    Assert::assert_equals_float_msg(None, 0.5, td1.score_docs[0].score, 0.0); // freq=pivot
    for i in 0..5 {
        Assert::assert_equals(td.score_docs[i].doc, td1.score_docs[i].doc);
    }

    // increase the exp, docs higher than pivot should get a higher score, and vice versa
    let q2: Arc<dyn Query> = Arc::new(IntervalQuery::with_pivot_and_exp(
        FIELD,
        Arc::clone(&source),
        1.2,
        2.0,
    )?);
    let td2 = f.searcher().search(q2, 10)?;
    Assert::assert_equals(5i64, td2.total_hits.value());
    for i in 0..5 {
        Assert::assert_equals(td.score_docs[i].doc, td2.score_docs[i].doc);
        if i < 2 {
            Assert::assert_true(td.score_docs[i].score < td2.score_docs[i].score);
        } else {
            Assert::assert_true(td.score_docs[i].score > td2.score_docs[i].score);
        }
    }

    // check valid bounds
    expect_throws(IAE, || {
        IntervalQuery::with_pivot(FIELD, Arc::clone(&source), -1.0)
    });
    expect_throws(IAE, || {
        IntervalQuery::with_pivot_and_exp(FIELD, Arc::clone(&source), 1.0, -1.0)
    });
    Ok(())
}

fn test_advance_behavior(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(containing(term("w1"), Arc::new(OneTimeIntervalSource)));
    f.check_hits(q, &[0, 1, 2, 3])
}

fn test_unicode_prefix(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(prefix("場"));
    f.check_hits(q, &[11])
}

fn test_extend_disjunctions(f: &mut IntervalQueryTest) -> Result<()> {
    let q = iq(or(vec![
        term("XXX"),
        containing(extend(term("message"), 0, 10), term("intend")),
    ]));
    f.check_hits(q, &[])
}

fn test_equality(_f: &mut IntervalQueryTest) -> Result<()> {
    let fq = |s: Arc<dyn IntervalsSource>| IntervalQuery::new("f", s);
    assert_query_equals(&fq(regexp(".*foo")), &fq(regexp(".*foo")));
    assert_query_equals(&fq(prefix_max("p", 1)), &fq(prefix_max("p", 1)));
    assert_query_equals(&fq(fuzzy_term("kot", 1)), &fq(fuzzy_term("kot", 1)));
    assert_query_equals(&fq(wildcard("*.txt")), &fq(wildcard("*.txt")));
    assert_query_equals(
        &fq(range(Some("cold"), Some("hot"), true, true)),
        &fq(range(Some("cold"), Some("hot"), true, true)),
    );
    Ok(())
}
