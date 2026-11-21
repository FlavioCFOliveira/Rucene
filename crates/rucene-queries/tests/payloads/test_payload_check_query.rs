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

//! Port of `TestPayloadCheckQuery.java`: basic test of payload-spans.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::Term;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::BytesRef;
use rucene_queries::payloads::span_payload_check_query::{
    MatchOperation, PayloadType, SpanPayloadCheckQuery,
};
use rucene_queries::spans::span_multi_term_query_wrapper::SpanMultiTermQueryWrapper;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_position_range_query::SpanPositionRangeQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE};
use rucene_test_framework::analysis::simple_payload_filter::SimplePayloadFilter;
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::english::English;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::search_support::{assert_query_equals, static_hook};

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

/// `TestPayloadCheckQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.payloads.TestPayloadCheckQuery",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testSpanPayloadCheck", test_span_payload_check),
            test(
                "testInequalityPayloadChecks",
                test_inequality_payload_checks,
            ),
            test("testUnorderedPayloadChecks", test_unordered_payload_checks),
            test("testComplexSpanChecks", test_complex_span_checks),
            test("testEquality", test_equality),
            test("testRewrite", test_rewrite),
        ],
    )
}

/// The anonymous `simplePayloadAnalyzer` of `beforeClass`.
#[derive(Debug)]
struct SimplePayloadAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for SimplePayloadAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.payloads.TestPayloadCheckQuery$1"
    }

    fn create_components(&self, _field_name: Option<&str>) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::new(
            Arc::clone(&SIMPLE),
            true,
        )?));
        let filter = SimplePayloadFilter::new(shared(&tokenizer))?;
        Ok(components(tokenizer, sink(filter)))
    }
}

fn before_class() -> Result<()> {
    let simple_payload_analyzer: Arc<dyn Analyzer> = Arc::new(SimplePayloadAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });

    let directory = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(simple_payload_analyzer));
    config.set_max_buffered_docs(TestUtil::next_int(&*random(), 100, 1000))?;
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    // writer.infoStream = System.out;
    for i in 0..2000 {
        let mut doc = Document::new();
        doc.add(LuceneTestCase::new_text_field(
            "field",
            &English::int_to_english(i),
            Store::YES,
        ));
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
        drop(searcher);
        // IOUtils.close(reader, directory)
        let r = reader.close();
        let d = directory.close();
        r?;
        d?;
    }
    Ok(())
}

fn check_hits(query: Arc<dyn SpanQuery>, results: &[i32]) -> Result<()> {
    CheckHits::check_hits(
        &*random(),
        &(query as Arc<dyn Query>),
        Some("field"),
        &searcher(),
        results,
    )
}

fn stq(text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(Term::from_text("field", text)))
}

fn pos(n: i32) -> BytesRef {
    BytesRef::from_char_sequence(&format!("pos: {n}"))
}

fn check(
    match_query: &Arc<dyn SpanQuery>,
    payloads: Vec<BytesRef>,
    payload_type: PayloadType,
    operation: MatchOperation,
) -> Arc<dyn SpanQuery> {
    Arc::new(SpanPayloadCheckQuery::with_type_and_operation(
        Arc::clone(match_query),
        payloads.into_iter().map(Some).collect(),
        payload_type,
        operation,
    ))
}

fn near0(clauses: Vec<Arc<dyn SpanQuery>>, in_order: bool) -> Result<Arc<dyn SpanQuery>> {
    Ok(Arc::new(SpanNearQuery::new(clauses, 0, in_order)?))
}

/// The hits of `five` with payload `pos: 5` and the GT/GTE checks.
const FIVE_AT_5: [i32; 72] = [
    1125, 1135, 1145, 1155, 1165, 1175, 1185, 1195, 1225, 1235, 1245, 1255, 1265, 1275, 1285, 1295,
    1325, 1335, 1345, 1355, 1365, 1375, 1385, 1395, 1425, 1435, 1445, 1455, 1465, 1475, 1485, 1495,
    1525, 1535, 1545, 1555, 1565, 1575, 1585, 1595, 1625, 1635, 1645, 1655, 1665, 1675, 1685, 1695,
    1725, 1735, 1745, 1755, 1765, 1775, 1785, 1795, 1825, 1835, 1845, 1855, 1865, 1875, 1885, 1895,
    1925, 1935, 1945, 1955, 1965, 1975, 1985, 1995,
];

/// The hits of `five hundred` at payloads `pos: 0`, `pos: 1` and of the
/// LT/LTE checks.
const FIVE_HUNDRED: [i32; 100] = [
    500, 501, 502, 503, 504, 505, 506, 507, 508, 509, 510, 511, 512, 513, 514, 515, 516, 517, 518,
    519, 520, 521, 522, 523, 524, 525, 526, 527, 528, 529, 530, 531, 532, 533, 534, 535, 536, 537,
    538, 539, 540, 541, 542, 543, 544, 545, 546, 547, 548, 549, 550, 551, 552, 553, 554, 555, 556,
    557, 558, 559, 560, 561, 562, 563, 564, 565, 566, 567, 568, 569, 570, 571, 572, 573, 574, 575,
    576, 577, 578, 579, 580, 581, 582, 583, 584, 585, 586, 587, 588, 589, 590, 591, 592, 593, 594,
    595, 596, 597, 598, 599,
];

fn test_span_payload_check() -> Result<()> {
    let term1 = stq("five");
    let pay = pos(5);
    let query: Arc<dyn SpanQuery> = Arc::new(SpanPayloadCheckQuery::new(
        Arc::clone(&term1),
        vec![Some(pay)],
    ));
    check_hits(Arc::clone(&query), &FIVE_AT_5)?;
    Assert::assert_true(
        searcher()
            .explain(query as Arc<dyn Query>, 1125)?
            .value()
            .double_value()
            > 0.0,
    );

    let term2 = stq("hundred");
    let clauses: Vec<Arc<dyn SpanQuery>> = vec![Arc::clone(&term1), Arc::clone(&term2)];
    let snq: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses, 0, true)?);
    let pay = pos(0);
    let pay2 = pos(1);
    let list = vec![Some(pay), Some(pay2)];
    let query: Arc<dyn SpanQuery> = Arc::new(SpanPayloadCheckQuery::new(snq, list));
    check_hits(query, &FIVE_HUNDRED)?;

    let clauses: Vec<Arc<dyn SpanQuery>> = vec![term1, term2, stq("five")];
    let snq: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses, 0, true)?);
    let pay = pos(0);
    let pay2 = pos(1);
    let pay3 = pos(2);
    let list = vec![Some(pay), Some(pay2), Some(pay3)];
    let query: Arc<dyn SpanQuery> = Arc::new(SpanPayloadCheckQuery::new(snq, list));
    check_hits(query, &[505])
}

/// `5` and `500` to `599`.
fn five_lt() -> Vec<i32> {
    let mut v = vec![5];
    v.extend_from_slice(&FIVE_HUNDRED);
    v
}

fn test_inequality_payload_checks() -> Result<()> {
    // searching for the term five with a payload of either "pos: 0" or a payload of "pos: 1"
    let term_five = stq("five");
    let term_fifty = stq("fifty");
    let payload_zero = pos(0);
    let payload_one = pos(1);
    let payload_two = pos(2);
    let payload_three = pos(3);
    let payload_four = pos(4);
    let payload_five = pos(5);
    // Terms that equal five with a payload of "pos: 1"
    let string_eq1 = check(
        &term_five,
        vec![payload_one.clone()],
        PayloadType::String,
        MatchOperation::Eq,
    );
    check_hits(string_eq1, &[25, 35, 45, 55, 65, 75, 85, 95])?;

    // These queries return the same thing
    let string_lt = check(
        &term_five,
        vec![payload_one.clone()],
        PayloadType::String,
        MatchOperation::Lt,
    );
    let string_lte = check(
        &term_five,
        vec![payload_zero.clone()],
        PayloadType::String,
        MatchOperation::Lte,
    );
    // string less than and string less than or equal
    check_hits(string_lt, &five_lt())?;
    check_hits(string_lte, &five_lt())?;
    // greater than and greater than or equal tests.
    let string_gt = check(
        &term_five,
        vec![payload_four],
        PayloadType::String,
        MatchOperation::Gt,
    );
    let string_gte = check(
        &term_five,
        vec![payload_five],
        PayloadType::String,
        MatchOperation::Gte,
    );
    check_hits(string_gt, &FIVE_AT_5)?;
    check_hits(string_gte, &FIVE_AT_5)?;
    // now a not so happy path...
    let string_eq2many = check(
        &term_five,
        vec![payload_one.clone(), payload_zero.clone()],
        PayloadType::String,
        MatchOperation::Eq,
    );
    // fewer terms than payloads should not match anything, one wonders if this should be an error,
    // but it's been explicitly ignored previously, so changing it is a possible back compat issue.
    check_hits(string_eq2many, &[])?;

    // now some straight forward two term cases...
    let string_eq2 = check(
        &near0(vec![Arc::clone(&term_fifty), Arc::clone(&term_five)], true)?,
        vec![payload_zero.clone(), payload_one.clone()],
        PayloadType::String,
        MatchOperation::Eq,
    );
    check_hits(string_eq2, &[55])?;

    let string_gt2 = check(
        &near0(vec![Arc::clone(&term_fifty), Arc::clone(&term_five)], true)?,
        vec![payload_zero.clone(), payload_one.clone()],
        PayloadType::String,
        MatchOperation::Gt,
    );
    check_hits(
        string_gt2,
        &aligned_int_array(
            "       155,  255,  355,  455,  555,  655,  755,  855,  955,\n\
             1055, 1155, 1255, 1355, 1455, 1555, 1655, 1755, 1855, 1955\n",
        ),
    )?;

    let string_gte2 = check(
        &near0(vec![Arc::clone(&term_fifty), Arc::clone(&term_five)], true)?,
        vec![payload_zero, payload_one],
        PayloadType::String,
        MatchOperation::Gte,
    );
    check_hits(
        string_gte2,
        &aligned_int_array(
            "  55,  155,  255,  355,  455,  555,  655,  755,  855,  955,\n\
             1055, 1155, 1255, 1355, 1455, 1555, 1655, 1755, 1855, 1955\n",
        ),
    )?;

    let string_lt2 = check(
        &near0(vec![Arc::clone(&term_fifty), Arc::clone(&term_five)], true)?,
        vec![payload_two.clone(), payload_three.clone()],
        PayloadType::String,
        MatchOperation::Lt,
    );
    check_hits(string_lt2, &[55])?;

    let string_lte2 = check(
        &near0(vec![term_fifty, term_five], true)?,
        vec![payload_two, payload_three],
        PayloadType::String,
        MatchOperation::Lte,
    );
    check_hits(
        string_lte2,
        &[55, 155, 255, 355, 455, 555, 655, 755, 855, 955, 1055],
    )

    // note: I can imagine support for SpanOrQuery might be interesting but that's for some other
    // time, currently such support is  made intractable by the fact that reset() gets called and
    // sets "upto" back to zero between SpanOrQuery subclauses.
}

/// Parses a comma-separated array of integers, ignoring white space around
/// them. This allows for arbitrary alignment of integers in the source
/// string to convey additional information about their mutual relations.
///
/// `alignedIntArray(String)`: `Arrays.stream(ints.split(",")).map(String::trim)
/// .mapToInt(Integer::parseInt).toArray()`.
fn aligned_int_array(ints: &str) -> Vec<i32> {
    ints.split(',')
        .map(|s| s.trim().parse::<i32>().expect("Integer.parseInt"))
        .collect()
}

fn test_unordered_payload_checks() -> Result<()> {
    let term5 = stq("five");
    let term100 = stq("hundred");
    let term4 = stq("four");
    let near_query = near0(vec![term5, term100, term4], false)?;

    let payloads = vec![Some(pos(2)), Some(pos(1)), Some(pos(0))];

    let payload_query: Arc<dyn SpanQuery> = Arc::new(SpanPayloadCheckQuery::new(
        Arc::clone(&near_query),
        payloads,
    ));
    check_hits(payload_query, &[405])?;

    let payloads = vec![Some(pos(0)), Some(pos(1)), Some(pos(2))];

    let payload_query: Arc<dyn SpanQuery> =
        Arc::new(SpanPayloadCheckQuery::new(near_query, payloads));
    check_hits(payload_query, &[504])
}

fn test_complex_span_checks() -> Result<()> {
    let one = stq("one");
    let thous = stq("thousand");
    // should be one position in between
    let hundred = stq("hundred");
    let three = stq("three");

    let one_thous = near0(vec![one, thous], true)?;
    let hundred_three = near0(vec![hundred, three], true)?;
    let one_thous_hun_three: Arc<dyn SpanQuery> =
        Arc::new(SpanNearQuery::new(vec![one_thous, hundred_three], 1, true)?);
    // this one's too small
    let query: Arc<dyn SpanQuery> = Arc::new(SpanPositionRangeQuery::new(
        Arc::clone(&one_thous_hun_three),
        1,
        2,
    ));
    check_hits(query, &[])?;
    // this one's just right
    let query: Arc<dyn SpanQuery> = Arc::new(SpanPositionRangeQuery::new(
        Arc::clone(&one_thous_hun_three),
        0,
        6,
    ));
    check_hits(
        query,
        &[1103, 1203, 1303, 1403, 1503, 1603, 1703, 1803, 1903],
    )?;

    let pay = BytesRef::new("pos: 0".as_bytes().to_vec());
    let pay2 = BytesRef::new("pos: 1".as_bytes().to_vec());
    let pay3 = BytesRef::new("pos: 3".as_bytes().to_vec());
    let pay4 = BytesRef::new("pos: 4".as_bytes().to_vec());
    let payloads = vec![Some(pay), Some(pay2), Some(pay3), Some(pay4)];
    let query: Arc<dyn SpanQuery> =
        Arc::new(SpanPayloadCheckQuery::new(one_thous_hun_three, payloads));
    check_hits(
        query,
        &[1103, 1203, 1303, 1403, 1503, 1603, 1703, 1803, 1903],
    )
}

/// `assertFalse(a.equals(b))`.
fn assert_unequal(a: &Arc<dyn SpanQuery>, b: &Arc<dyn SpanQuery>) {
    Assert::assert_false(a.query_eq(&**b as &dyn Query));
}

// Java's literal `2.71828f`, kept as written.
#[allow(clippy::approx_constant)]
fn test_equality() -> Result<()> {
    let sq1 = stq("one");
    let sq2 = stq("two");
    let payload1 = BytesRef::from_char_sequence("pay1");
    let payload2 = BytesRef::from_char_sequence("pay2");
    let new_check = |sq: &Arc<dyn SpanQuery>, p: &BytesRef| -> Arc<dyn SpanQuery> {
        Arc::new(SpanPayloadCheckQuery::new(
            Arc::clone(sq),
            vec![Some(p.clone())],
        ))
    };
    let query1 = new_check(&sq1, &payload1);
    let query2 = new_check(&sq2, &payload1);
    let query3 = new_check(&sq1, &payload2);
    let query4 = new_check(&sq2, &payload2);
    let query5 = new_check(&sq1, &payload1);

    assert_query_equals(&*query1, &*query5);
    assert_unequal(&query1, &query2);
    assert_unequal(&query1, &query3);
    assert_unequal(&query1, &query4);
    assert_unequal(&query2, &query3);
    assert_unequal(&query2, &query4);
    assert_unequal(&query3, &query4);

    // Create an integer and a float encoded payload
    let i: i32 = 451;
    let int_payload = BytesRef::new(i.to_be_bytes().to_vec());
    let e: f32 = 2.71828;
    let float_payload = BytesRef::new(e.to_be_bytes().to_vec());
    let float_lt_query = check(
        &sq1,
        vec![float_payload.clone()],
        PayloadType::Float,
        MatchOperation::Lt,
    );
    let float_lte_query = check(
        &sq1,
        vec![float_payload.clone()],
        PayloadType::Float,
        MatchOperation::Lte,
    );
    let float_gt_query = check(
        &sq1,
        vec![float_payload.clone()],
        PayloadType::Float,
        MatchOperation::Gt,
    );
    let float_gte_query = check(
        &sq1,
        vec![float_payload],
        PayloadType::Float,
        MatchOperation::Gte,
    );

    let int_lt_query = check(
        &sq1,
        vec![int_payload.clone()],
        PayloadType::Int,
        MatchOperation::Lt,
    );
    let int_lte_query = check(
        &sq1,
        vec![int_payload.clone()],
        PayloadType::Int,
        MatchOperation::Lte,
    );
    let int_gt_query = check(
        &sq1,
        vec![int_payload.clone()],
        PayloadType::Int,
        MatchOperation::Gt,
    );
    let int_gte_query = check(
        &sq1,
        vec![int_payload.clone()],
        PayloadType::Int,
        MatchOperation::Gt,
    );

    // string inequality checks
    let string_lt_query = check(
        &sq1,
        vec![int_payload.clone()],
        PayloadType::String,
        MatchOperation::Lt,
    );
    let string_lte_query = check(
        &sq1,
        vec![int_payload.clone()],
        PayloadType::String,
        MatchOperation::Lte,
    );
    let string_gt_query = check(
        &sq1,
        vec![int_payload.clone()],
        PayloadType::String,
        MatchOperation::Gt,
    );
    let string_gte_query = check(
        &sq1,
        vec![int_payload.clone()],
        PayloadType::String,
        MatchOperation::Gt,
    );
    let string_eq_query = check(
        &sq1,
        vec![int_payload.clone()],
        PayloadType::String,
        MatchOperation::Eq,
    );
    let string_default_query: Arc<dyn SpanQuery> = Arc::new(SpanPayloadCheckQuery::new(
        Arc::clone(&sq1),
        vec![Some(int_payload)],
    ));
    Assert::assert_true(string_default_query.query_eq(&*string_eq_query as &dyn Query));
    assert_unequal(&string_default_query, &string_gt_query);
    assert_unequal(&string_default_query, &string_gte_query);
    assert_unequal(&string_default_query, &string_lt_query);
    assert_unequal(&string_default_query, &string_lte_query);

    assert_unequal(&float_lt_query, &float_lte_query);
    assert_unequal(&float_lt_query, &float_gt_query);
    assert_unequal(&float_lt_query, &float_gte_query);

    assert_unequal(&float_lt_query, &int_lt_query);
    assert_unequal(&float_lt_query, &int_lte_query);
    assert_unequal(&float_lt_query, &int_gt_query);
    assert_unequal(&float_lt_query, &int_gte_query);
    Ok(())
}

fn test_rewrite() -> Result<()> {
    let wrap = |p: &str| -> Result<Arc<dyn SpanQuery>> {
        Ok(Arc::new(SpanMultiTermQueryWrapper::new(Arc::new(
            WildcardQuery::new(Term::from_text("field", p))?,
        ))))
    };
    let fiv = wrap("fiv*")?;
    let hund = wrap("hund*")?;
    let twent = wrap("twent*")?;
    let nin = wrap("nin*")?;

    let sq = near0(vec![fiv, hund, twent, nin], true)?;

    let payloads = vec![
        Some(BytesRef::from_char_sequence("pos: 0")),
        Some(BytesRef::from_char_sequence("pos: 1")),
        Some(BytesRef::from_char_sequence("pos: 2")),
        Some(BytesRef::from_char_sequence("pos: 3")),
    ];

    let query: Arc<dyn SpanQuery> = Arc::new(SpanPayloadCheckQuery::new(sq, payloads));

    // if query wasn't rewritten properly, the query would have failed with "Rewrite first!"
    check_hits(query, &[529])
}
