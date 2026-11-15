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

//! Port of `TestBasics.java`: tests basic search capabilities.
//!
//! Uses a collection of 1000 documents, each the english rendition of their
//! document number. For example, the document numbered 333 has text "three
//! hundred thirty three".
//!
//! Tests are each a single query, and its hits are checked to ensure that
//! all and only the correct documents are returned, thus providing
//! end-to-end testing of the indexing and search code.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::english::English;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::queries_support::core_support::{assert_equals_msg, class, random, test};
use crate::queries_support::search_support::static_hook;
use crate::queries_support::span_test_util::{
    self, span_first_query, span_near_ordered_query_terms, span_near_unordered_query_terms,
    span_not_query, span_not_query_pre_post, span_or_query_terms, span_position_range_query,
    span_term_query,
};

/// The static fields `searcher`, `reader` and `directory`.
struct Statics {
    searcher: Arc<IndexSearcher>,
    reader: Arc<dyn DirectoryReader>,
    directory: TestDirectory,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn searcher() -> Arc<IndexSearcher> {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&guard.as_ref().expect("beforeClass ran").searcher)
}

/// `TestBasics`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestBasics",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testTerm", test_term),
            test("testTerm2", test_term2),
            test("testPhrase", test_phrase),
            test("testPhrase2", test_phrase2),
            test("testBoolean", test_boolean),
            test("testBoolean2", test_boolean2),
            test("testSpanNearExact", test_span_near_exact),
            test("testSpanTermQuery", test_span_term_query),
            test("testSpanNearUnordered", test_span_near_unordered),
            test("testSpanNearOrdered", test_span_near_ordered),
            test("testSpanNot", test_span_not),
            test(
                "testSpanNotNoOverflowOnLargeSpans",
                test_span_not_no_overflow_on_large_spans,
            ),
            test(
                "testSpanWithMultipleNotSingle",
                test_span_with_multiple_not_single,
            ),
            test(
                "testSpanWithMultipleNotMany",
                test_span_with_multiple_not_many,
            ),
            test(
                "testNpeInSpanNearWithSpanNot",
                test_npe_in_span_near_with_span_not,
            ),
            test(
                "testNpeInSpanNearInSpanFirstInSpanNot",
                test_npe_in_span_near_in_span_first_in_span_not,
            ),
            test("testSpanNotWindowOne", test_span_not_window_one),
            test(
                "testSpanNotWindowTwoBefore",
                test_span_not_window_two_before,
            ),
            test("testSpanNotWindowNegPost", test_span_not_window_neg_post),
            test("testSpanNotWindowNegPre", test_span_not_window_neg_pre),
            test(
                "testSpanNotWindowDoubleExcludesBefore",
                test_span_not_window_double_excludes_before,
            ),
            test("testSpanFirst", test_span_first),
            test("testSpanPositionRange", test_span_position_range),
            test("testSpanOr", test_span_or),
            test("testSpanExactNested", test_span_exact_nested),
            test("testSpanNearOr", test_span_near_or),
            test("testSpanComplex1", test_span_complex1),
            test("testBooleanSpanQuery", test_boolean_span_query),
            test("testDismaxSpanQuery", test_dismax_span_query),
        ],
    )
}

fn before_class() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&SIMPLE),
        true,
    ));
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_max_buffered_docs(TestUtil::next_int(&*random(), 100, 1000))?;
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    // writer.infoStream = System.out;
    for i in 0..2000 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            &English::int_to_english(i),
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    let reader = writer.get_reader()?;
    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    writer.close()?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        searcher,
        reader,
        directory,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    let taken = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(Statics {
        searcher,
        reader,
        directory,
    }) = taken
    {
        reader.close()?;
        directory.close()?;
        drop(searcher);
    }
    Ok(())
}

fn check_hits(query: Arc<dyn Query>, results: &[i32]) -> Result<()> {
    CheckHits::check_hits(&*random(), &query, Some("field"), &searcher(), results)
}

/// `assertTrue(searcher.explain(query, doc).getValue().doubleValue() > 0.0f)`.
fn assert_explain_positive(query: &Arc<dyn SpanQuery>, doc: i32) -> Result<()> {
    let q: Arc<dyn Query> = Arc::clone(query) as Arc<dyn Query>;
    Assert::assert_true(searcher().explain(q, doc)?.value().double_value() > 0.0);
    Ok(())
}

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(field, text)))
}

fn test_term() -> Result<()> {
    let query = tq("field", "seventy");
    check_hits(
        query,
        &[
            70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 170, 171, 172, 173, 174, 175, 176, 177, 178,
            179, 270, 271, 272, 273, 274, 275, 276, 277, 278, 279, 370, 371, 372, 373, 374, 375,
            376, 377, 378, 379, 470, 471, 472, 473, 474, 475, 476, 477, 478, 479, 570, 571, 572,
            573, 574, 575, 576, 577, 578, 579, 670, 671, 672, 673, 674, 675, 676, 677, 678, 679,
            770, 771, 772, 773, 774, 775, 776, 777, 778, 779, 870, 871, 872, 873, 874, 875, 876,
            877, 878, 879, 970, 971, 972, 973, 974, 975, 976, 977, 978, 979, 1070, 1071, 1072,
            1073, 1074, 1075, 1076, 1077, 1078, 1079, 1170, 1171, 1172, 1173, 1174, 1175, 1176,
            1177, 1178, 1179, 1270, 1271, 1272, 1273, 1274, 1275, 1276, 1277, 1278, 1279, 1370,
            1371, 1372, 1373, 1374, 1375, 1376, 1377, 1378, 1379, 1470, 1471, 1472, 1473, 1474,
            1475, 1476, 1477, 1478, 1479, 1570, 1571, 1572, 1573, 1574, 1575, 1576, 1577, 1578,
            1579, 1670, 1671, 1672, 1673, 1674, 1675, 1676, 1677, 1678, 1679, 1770, 1771, 1772,
            1773, 1774, 1775, 1776, 1777, 1778, 1779, 1870, 1871, 1872, 1873, 1874, 1875, 1876,
            1877, 1878, 1879, 1970, 1971, 1972, 1973, 1974, 1975, 1976, 1977, 1978, 1979,
        ],
    )
}

fn test_term2() -> Result<()> {
    let query = tq("field", "seventish");
    check_hits(query, &[])
}

fn test_phrase() -> Result<()> {
    let query: Arc<dyn Query> =
        Arc::new(PhraseQuery::from_strings("field", &["seventy", "seven"])?);
    check_hits(
        query,
        &[
            77, 177, 277, 377, 477, 577, 677, 777, 877, 977, 1077, 1177, 1277, 1377, 1477, 1577,
            1677, 1777, 1877, 1977,
        ],
    )
}

fn test_phrase2() -> Result<()> {
    let query: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings(
        "field",
        &["seventish", "sevenon"],
    )?);
    check_hits(query, &[])
}

fn test_boolean() -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq("field", "seventy"), Occur::MUST)?;
    query.add(tq("field", "seven"), Occur::MUST)?;
    check_hits(
        Arc::new(query.build()),
        &[
            77, 177, 277, 377, 477, 577, 677, 770, 771, 772, 773, 774, 775, 776, 777, 778, 779,
            877, 977, 1077, 1177, 1277, 1377, 1477, 1577, 1677, 1770, 1771, 1772, 1773, 1774, 1775,
            1776, 1777, 1778, 1779, 1877, 1977,
        ],
    )
}

fn test_boolean2() -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq("field", "sevento"), Occur::MUST)?;
    query.add(tq("field", "sevenly"), Occur::MUST)?;
    check_hits(Arc::new(query.build()), &[])
}

fn test_span_near_exact() -> Result<()> {
    let query = span_near_ordered_query_terms("field", 0, &["seventy", "seven"])?;
    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[
            77, 177, 277, 377, 477, 577, 677, 777, 877, 977, 1077, 1177, 1277, 1377, 1477, 1577,
            1677, 1777, 1877, 1977,
        ],
    )?;
    assert_explain_positive(&query, 77)?;
    assert_explain_positive(&query, 977)
}

fn test_span_term_query() -> Result<()> {
    check_hits(
        span_term_query("field", "seventy"),
        &[
            70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 170, 171, 172, 173, 174, 175, 176, 177, 178,
            179, 270, 271, 272, 273, 274, 275, 276, 277, 278, 279, 370, 371, 372, 373, 374, 375,
            376, 377, 378, 379, 470, 471, 472, 473, 474, 475, 476, 477, 478, 479, 570, 571, 572,
            573, 574, 575, 576, 577, 578, 579, 670, 671, 672, 673, 674, 675, 676, 677, 678, 679,
            770, 771, 772, 773, 774, 775, 776, 777, 778, 779, 870, 871, 872, 873, 874, 875, 876,
            877, 878, 879, 970, 971, 972, 973, 974, 975, 976, 977, 978, 979, 1070, 1071, 1072,
            1073, 1074, 1075, 1076, 1077, 1078, 1079, 1170, 1270, 1370, 1470, 1570, 1670, 1770,
            1870, 1970, 1171, 1172, 1173, 1174, 1175, 1176, 1177, 1178, 1179, 1271, 1272, 1273,
            1274, 1275, 1276, 1277, 1278, 1279, 1371, 1372, 1373, 1374, 1375, 1376, 1377, 1378,
            1379, 1471, 1472, 1473, 1474, 1475, 1476, 1477, 1478, 1479, 1571, 1572, 1573, 1574,
            1575, 1576, 1577, 1578, 1579, 1671, 1672, 1673, 1674, 1675, 1676, 1677, 1678, 1679,
            1771, 1772, 1773, 1774, 1775, 1776, 1777, 1778, 1779, 1871, 1872, 1873, 1874, 1875,
            1876, 1877, 1878, 1879, 1971, 1972, 1973, 1974, 1975, 1976, 1977, 1978, 1979,
        ],
    )
}

fn test_span_near_unordered() -> Result<()> {
    check_hits(
        span_near_unordered_query_terms("field", 4, &["nine", "six"])?,
        &[
            609, 629, 639, 649, 659, 669, 679, 689, 699, 906, 926, 936, 946, 956, 966, 976, 986,
            996, 1609, 1629, 1639, 1649, 1659, 1669, 1679, 1689, 1699, 1906, 1926, 1936, 1946,
            1956, 1966, 1976, 1986, 1996,
        ],
    )
}

fn test_span_near_ordered() -> Result<()> {
    check_hits(
        span_near_ordered_query_terms("field", 4, &["nine", "six"])?,
        &[
            906, 926, 936, 946, 956, 966, 976, 986, 996, 1906, 1926, 1936, 1946, 1956, 1966, 1976,
            1986, 1996,
        ],
    )
}

fn test_span_not() -> Result<()> {
    let near = span_near_ordered_query_terms("field", 4, &["eight", "one"])?;
    let query = span_not_query(near, span_term_query("field", "forty"))?;
    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[
            801, 821, 831, 851, 861, 871, 881, 891, 1801, 1821, 1831, 1851, 1861, 1871, 1881, 1891,
        ],
    )?;
    assert_explain_positive(&query, 801)?;
    assert_explain_positive(&query, 891)
}

fn test_span_not_no_overflow_on_large_spans() -> Result<()> {
    let near = span_near_ordered_query_terms("field", 4, &["eight", "one"])?;
    let query =
        span_not_query_pre_post(near, span_term_query("field", "forty"), i32::MAX, i32::MAX)?;
    check_hits(
        query,
        &[
            801, 821, 831, 851, 861, 871, 881, 891, 1801, 1821, 1831, 1851, 1861, 1871, 1881, 1891,
        ],
    )
}

fn test_span_with_multiple_not_single() -> Result<()> {
    let near = span_near_ordered_query_terms("field", 4, &["eight", "one"])?;
    let or = span_or_query_terms("field", &["forty"])?;
    let query = span_not_query(near, or)?;
    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[
            801, 821, 831, 851, 861, 871, 881, 891, 1801, 1821, 1831, 1851, 1861, 1871, 1881, 1891,
        ],
    )?;
    assert_explain_positive(&query, 801)?;
    assert_explain_positive(&query, 891)
}

fn test_span_with_multiple_not_many() -> Result<()> {
    let near = span_near_ordered_query_terms("field", 4, &["eight", "one"])?;
    let or = span_or_query_terms("field", &["forty", "sixty", "eighty"])?;
    let query = span_not_query(near, or)?;
    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[
            801, 821, 831, 851, 871, 891, 1801, 1821, 1831, 1851, 1871, 1891,
        ],
    )?;
    assert_explain_positive(&query, 801)?;
    assert_explain_positive(&query, 891)
}

fn test_npe_in_span_near_with_span_not() -> Result<()> {
    let near = span_near_ordered_query_terms("field", 4, &["eight", "one"])?;
    let exclude = span_near_ordered_query_terms("field", 1, &["hundred", "forty"])?;
    let query = span_not_query(near, exclude)?;
    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[
            801, 821, 831, 851, 861, 871, 881, 891, 1801, 1821, 1831, 1851, 1861, 1871, 1881, 1891,
        ],
    )?;
    assert_explain_positive(&query, 801)?;
    assert_explain_positive(&query, 891)
}

fn test_npe_in_span_near_in_span_first_in_span_not() -> Result<()> {
    let n = 5;
    let include = span_first_query(span_term_query("field", "forty"), n);
    let near = span_near_ordered_query_terms("field", n - 1, &["hundred", "forty"])?;
    let exclude = span_first_query(near, n - 1);
    let q = span_not_query(include, exclude)?;
    check_hits(
        q,
        &[
            40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047,
            1048, 1049, 1140, 1141, 1142, 1143, 1144, 1145, 1146, 1147, 1148, 1149, 1240, 1241,
            1242, 1243, 1244, 1245, 1246, 1247, 1248, 1249, 1340, 1341, 1342, 1343, 1344, 1345,
            1346, 1347, 1348, 1349, 1440, 1441, 1442, 1443, 1444, 1445, 1446, 1447, 1448, 1449,
            1540, 1541, 1542, 1543, 1544, 1545, 1546, 1547, 1548, 1549, 1640, 1641, 1642, 1643,
            1644, 1645, 1646, 1647, 1648, 1649, 1740, 1741, 1742, 1743, 1744, 1745, 1746, 1747,
            1748, 1749, 1840, 1841, 1842, 1843, 1844, 1845, 1846, 1847, 1848, 1849, 1940, 1941,
            1942, 1943, 1944, 1945, 1946, 1947, 1948, 1949,
        ],
    )
}

fn test_span_not_window_one() -> Result<()> {
    let near = span_near_ordered_query_terms("field", 4, &["eight", "forty"])?;
    let query = span_not_query_pre_post(near, span_term_query("field", "one"), 1, 1)?;
    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[
            840, 842, 843, 844, 845, 846, 847, 848, 849, 1840, 1842, 1843, 1844, 1845, 1846, 1847,
            1848, 1849,
        ],
    )?;
    assert_explain_positive(&query, 840)?;
    assert_explain_positive(&query, 1842)
}

fn test_span_not_window_two_before() -> Result<()> {
    let near = span_near_ordered_query_terms("field", 4, &["eight", "forty"])?;
    let query = span_not_query_pre_post(near, span_term_query("field", "one"), 2, 0)?;
    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[840, 841, 842, 843, 844, 845, 846, 847, 848, 849],
    )?;
    assert_explain_positive(&query, 840)?;
    assert_explain_positive(&query, 849)
}

fn test_span_not_window_neg_post() -> Result<()> {
    // test handling of invalid window < 0
    let near = span_near_ordered_query_terms("field", 4, &["eight", "one"])?;
    let or = span_or_query_terms("field", &["forty"])?;
    let query = span_not_query_pre_post(Arc::clone(&near), Arc::clone(&or), 0, -1)?;
    check_hits(
        query,
        &[
            801, 821, 831, 851, 861, 871, 881, 891, 1801, 1821, 1831, 1851, 1861, 1871, 1881, 1891,
        ],
    )?;

    let query = span_not_query_pre_post(near, or, 0, -2)?;
    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[
            801, 821, 831, 841, 851, 861, 871, 881, 891, 1801, 1821, 1831, 1841, 1851, 1861, 1871,
            1881, 1891,
        ],
    )?;

    assert_explain_positive(&query, 801)?;
    assert_explain_positive(&query, 891)
}

fn test_span_not_window_neg_pre() -> Result<()> {
    // test handling of invalid window < 0
    let near = span_near_ordered_query_terms("field", 4, &["eight", "one"])?;
    let or = span_or_query_terms("field", &["forty"])?;
    let query = span_not_query_pre_post(Arc::clone(&near), Arc::clone(&or), -2, 0)?;
    check_hits(
        query,
        &[
            801, 821, 831, 851, 861, 871, 881, 891, 1801, 1821, 1831, 1851, 1861, 1871, 1881, 1891,
        ],
    )?;

    let query = span_not_query_pre_post(near, or, -3, 0)?;
    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[
            801, 821, 831, 841, 851, 861, 871, 881, 891, 1801, 1821, 1831, 1841, 1851, 1861, 1871,
            1881, 1891,
        ],
    )?;

    assert_explain_positive(&query, 801)?;
    assert_explain_positive(&query, 891)
}

fn test_span_not_window_double_excludes_before() -> Result<()> {
    // test hitting two excludes before an include
    let near = span_near_ordered_query_terms("field", 2, &["forty", "two"])?;
    let exclude = span_term_query("field", "one");
    let query = span_not_query_pre_post(near, exclude, 4, 1)?;

    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[42, 242, 342, 442, 542, 642, 742, 842, 942],
    )?;

    assert_explain_positive(&query, 242)?;
    assert_explain_positive(&query, 942)
}

/// The hits of `spanFirstQuery(spanTermQuery("field", "five"), 1)`.
const FIVE_FIRST: [i32; 101] = [
    5, 500, 501, 502, 503, 504, 505, 506, 507, 508, 509, 510, 511, 512, 513, 514, 515, 516, 517,
    518, 519, 520, 521, 522, 523, 524, 525, 526, 527, 528, 529, 530, 531, 532, 533, 534, 535, 536,
    537, 538, 539, 540, 541, 542, 543, 544, 545, 546, 547, 548, 549, 550, 551, 552, 553, 554, 555,
    556, 557, 558, 559, 560, 561, 562, 563, 564, 565, 566, 567, 568, 569, 570, 571, 572, 573, 574,
    575, 576, 577, 578, 579, 580, 581, 582, 583, 584, 585, 586, 587, 588, 589, 590, 591, 592, 593,
    594, 595, 596, 597, 598, 599,
];

fn test_span_first() -> Result<()> {
    let query = span_first_query(span_term_query("field", "five"), 1);
    check_hits(Arc::clone(&query) as Arc<dyn Query>, &FIVE_FIRST)?;

    assert_explain_positive(&query, 5)?;
    assert_explain_positive(&query, 599)
}

fn test_span_position_range() -> Result<()> {
    let term1 = span_term_query("field", "five");
    let query = span_position_range_query(Arc::clone(&term1), 1, 2);

    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[25, 35, 45, 55, 65, 75, 85, 95],
    )?;
    assert_explain_positive(&query, 25)?;
    assert_explain_positive(&query, 95)?;

    let query = span_position_range_query(Arc::clone(&term1), 0, 1);
    check_hits(query, &FIVE_FIRST)?;

    let query = span_position_range_query(term1, 6, 7);
    check_hits(query, &[])
}

fn test_span_or() -> Result<()> {
    let near1 = span_near_ordered_query_terms("field", 0, &["thirty", "three"])?;
    let near2 = span_near_ordered_query_terms("field", 0, &["forty", "seven"])?;
    let query = span_test_util::span_or_query(vec![near1, near2])?;

    check_hits(
        Arc::clone(&query) as Arc<dyn Query>,
        &[
            33, 47, 133, 147, 233, 247, 333, 347, 433, 447, 533, 547, 633, 647, 733, 747, 833, 847,
            933, 947, 1033, 1047, 1133, 1147, 1233, 1247, 1333, 1347, 1433, 1447, 1533, 1547, 1633,
            1647, 1733, 1747, 1833, 1847, 1933, 1947,
        ],
    )?;

    assert_explain_positive(&query, 33)?;
    assert_explain_positive(&query, 947)
}

fn test_span_exact_nested() -> Result<()> {
    let near1 = span_near_ordered_query_terms("field", 0, &["three", "hundred"])?;
    let near2 = span_near_ordered_query_terms("field", 0, &["thirty", "three"])?;
    let query = span_test_util::span_near_ordered_query(0, vec![near1, near2])?;

    check_hits(Arc::clone(&query) as Arc<dyn Query>, &[333, 1333])?;

    assert_explain_positive(&query, 333)
}

/// The hits of `testSpanNearOr` and `testSpanComplex1`.
const SIX_SEVEN: [i32; 72] = [
    606, 607, 626, 627, 636, 637, 646, 647, 656, 657, 666, 667, 676, 677, 686, 687, 696, 697, 706,
    707, 726, 727, 736, 737, 746, 747, 756, 757, 766, 767, 776, 777, 786, 787, 796, 797, 1606,
    1607, 1626, 1627, 1636, 1637, 1646, 1647, 1656, 1657, 1666, 1667, 1676, 1677, 1686, 1687, 1696,
    1697, 1706, 1707, 1726, 1727, 1736, 1737, 1746, 1747, 1756, 1757, 1766, 1767, 1776, 1777, 1786,
    1787, 1796, 1797,
];

fn test_span_near_or() -> Result<()> {
    let to1 = span_or_query_terms("field", &["six", "seven"])?;
    let to2 = span_or_query_terms("field", &["seven", "six"])?;
    let query = span_test_util::span_near_ordered_query(10, vec![to1, to2])?;

    check_hits(query, &SIX_SEVEN)
}

fn test_span_complex1() -> Result<()> {
    let tt1 = span_near_ordered_query_terms("field", 0, &["six", "hundred"])?;
    let tt2 = span_near_ordered_query_terms("field", 0, &["seven", "hundred"])?;

    let to1 = span_test_util::span_or_query(vec![tt1, tt2])?;
    let to2 = span_or_query_terms("field", &["seven", "six"])?;
    let query = span_test_util::span_near_ordered_query(100, vec![to1, to2])?;

    check_hits(query, &SIX_SEVEN)
}

/// `assertEquals(String message, Object expected, Object actual)` over
/// booleans and ints, as Java's boxed `assertEquals(String, Object, Object)`
/// and `assertEquals(String, long, long)` compare them.
fn assert_equals_i64(message: &str, expected: i64, actual: i64) {
    Assert::assert_equals_msg(Some(message), expected, actual);
}

// LUCENE-4477 / LUCENE-4401:
fn test_boolean_span_query() -> Result<()> {
    let failed = false;
    let directory = LuceneTestCase::new_directory();
    let indexer_analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));

    let config = IndexWriterConfig::with_analyzer(Some(indexer_analyzer));
    let writer = IndexWriter::new(directory.as_directory(), config)?;
    let field = "content";
    let mut d = Document::new();
    d.add(Box::new(TextField::new(
        field,
        "clockwork orange".to_string(),
        Store::YES,
    )?));
    writer.add_document(&d)?;
    writer.close()?;

    let index_reader = directory_reader::open(directory.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));

    let mut query = BooleanQueryBuilder::new();
    let sq1: Arc<dyn Query> = Arc::new(SpanTermQuery::new(Term::from_text(field, "clockwork")));
    let sq2: Arc<dyn Query> = Arc::new(SpanTermQuery::new(Term::from_text(field, "clckwork")));
    query.add(sq1, Occur::SHOULD)?;
    query.add(sq2, Occur::SHOULD)?;
    let collector_manager = TopScoreDocCollectorManager::with_num_hits(1000, i32::MAX)?;
    let top_docs = searcher.search_with_manager(Arc::new(query.build()), &collector_manager)?;
    let hits = top_docs.score_docs.len() as i64;
    for score_doc in &top_docs.score_docs {
        println!("{}", score_doc.doc);
    }
    index_reader.close()?;
    assert_equals_msg(
        Some("Bug in boolean query composed of span queries"),
        &failed,
        &false,
    );
    assert_equals_i64("Bug in boolean query composed of span queries", hits, 1);
    directory.close()
}

// LUCENE-4477 / LUCENE-4401:
fn test_dismax_span_query() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let indexer_analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));

    let config = IndexWriterConfig::with_analyzer(Some(indexer_analyzer));
    let writer = IndexWriter::new(directory.as_directory(), config)?;
    let field = "content";
    let mut d = Document::new();
    d.add(Box::new(TextField::new(
        field,
        "clockwork orange".to_string(),
        Store::YES,
    )?));
    writer.add_document(&d)?;
    writer.close()?;

    let index_reader = directory_reader::open(directory.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));

    let query: Arc<dyn Query> = Arc::new(DisjunctionMaxQuery::new(
        vec![
            Arc::new(SpanTermQuery::new(Term::from_text(field, "clockwork"))),
            Arc::new(SpanTermQuery::new(Term::from_text(field, "clckwork"))),
        ],
        1.0,
    )?);

    let collector_manager = TopScoreDocCollectorManager::with_num_hits(1000, i32::MAX)?;
    let top_docs = searcher.search_with_manager(query, &collector_manager)?;
    let hits = top_docs.score_docs.len() as i64;
    for score_doc in &top_docs.score_docs {
        println!("{}", score_doc.doc);
    }
    index_reader.close()?;
    Assert::assert_equals(hits, 1i64);
    directory.close()
}
