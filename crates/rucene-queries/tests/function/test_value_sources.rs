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

//! Port of `TestValueSources.java`: barebones tests for function queries.
//!
//! The static `searcher` is changed in place (`setSimilarity`), so the
//! tests reach it through the class's static state ([`STATICS`]) and pass
//! it to the helpers.

use std::any::Any;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{
    Document, NumericDocValuesField, SortedDocValuesField, SortedNumericDocValuesField, Store,
    StringField, TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{LeafReaderContext, Term};
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::double_values_source;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::matches::Matches;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorable::Scorable;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::similarities::{ClassicSimilarity, Explanation};
use rucene::search::sort::Sort;
use rucene::search::sorted_numeric_selector::SortedNumericSelectorType;
use rucene::search::term_query::TermQuery;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::search::{ScoreDoc, SegmentCacheable};
use rucene::util::mutable::MutableValueObject;
use rucene::util::{BytesRef, NumericUtils};
use rucene_queries::function::docvalues::float_doc_values::{self, FloatDocValues};
use rucene_queries::function::function_query::FunctionQuery;
use rucene_queries::function::function_range_query::FunctionRangeQuery;
use rucene_queries::function::function_score_query::{
    FunctionScoreQuery, MultiplicativeBoostValuesSource,
};
use rucene_queries::function::function_values::{FunctionValues, ValueFiller};
use rucene_queries::function::value_source::{
    from_double_values_source, new_context, SharedScorable, ValueSource, ValueSourceContext,
    WrappedDoubleValuesSource,
};
use rucene_queries::function::valuesource::bool_function::BoolFunction;
use rucene_queries::function::valuesource::bytes_ref_field_source::BytesRefFieldSource;
use rucene_queries::function::valuesource::const_value_source::ConstValueSource;
use rucene_queries::function::valuesource::div_float_function::DivFloatFunction;
use rucene_queries::function::valuesource::doc_freq_value_source::DocFreqValueSource;
use rucene_queries::function::valuesource::double_const_value_source::DoubleConstValueSource;
use rucene_queries::function::valuesource::double_field_source::DoubleFieldSource;
use rucene_queries::function::valuesource::float_field_source::FloatFieldSource;
use rucene_queries::function::valuesource::idf_value_source::IDFValueSource;
use rucene_queries::function::valuesource::if_function::IfFunction;
use rucene_queries::function::valuesource::int_field_source::IntFieldSource;
use rucene_queries::function::valuesource::join_doc_freq_value_source::JoinDocFreqValueSource;
use rucene_queries::function::valuesource::linear_float_function::LinearFloatFunction;
use rucene_queries::function::valuesource::literal_value_source::LiteralValueSource;
use rucene_queries::function::valuesource::long_field_source::LongFieldSource;
use rucene_queries::function::valuesource::max_doc_value_source::MaxDocValueSource;
use rucene_queries::function::valuesource::max_float_function::MaxFloatFunction;
use rucene_queries::function::valuesource::min_float_function::MinFloatFunction;
use rucene_queries::function::valuesource::multi_bool_function::{self, MultiBoolFunction};
use rucene_queries::function::valuesource::multi_float_function::{self, MultiFloatFunction};
use rucene_queries::function::valuesource::multi_function;
use rucene_queries::function::valuesource::multi_valued_double_field_source::MultiValuedDoubleFieldSource;
use rucene_queries::function::valuesource::multi_valued_float_field_source::MultiValuedFloatFieldSource;
use rucene_queries::function::valuesource::multi_valued_int_field_source::MultiValuedIntFieldSource;
use rucene_queries::function::valuesource::multi_valued_long_field_source::MultiValuedLongFieldSource;
use rucene_queries::function::valuesource::norm_value_source::NormValueSource;
use rucene_queries::function::valuesource::num_docs_value_source::NumDocsValueSource;
use rucene_queries::function::valuesource::pow_float_function::PowFloatFunction;
use rucene_queries::function::valuesource::product_float_function::ProductFloatFunction;
use rucene_queries::function::valuesource::query_value_source::QueryValueSource;
use rucene_queries::function::valuesource::range_map_float_function::RangeMapFloatFunction;
use rucene_queries::function::valuesource::reciprocal_float_function::ReciprocalFloatFunction;
use rucene_queries::function::valuesource::scale_float_function::ScaleFloatFunction;
use rucene_queries::function::valuesource::sum_float_function::SumFloatFunction;
use rucene_queries::function::valuesource::sum_total_term_freq_value_source::SumTotalTermFreqValueSource;
use rucene_queries::function::valuesource::term_freq_value_source::TermFreqValueSource;
use rucene_queries::function::valuesource::tf_value_source::TFValueSource;
use rucene_queries::function::valuesource::total_term_freq_value_source::TotalTermFreqValueSource;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::function::test_index_reader_functions::id_sort;
use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::search_support::static_hook;

/// `documents`: id, double, float, int, long, string, text, double MV (x3),
/// int MV (x3).
const DOCUMENTS: [[&str; 13]; 2] = [
    [
        "0",
        "3.63",
        "5.2",
        "35",
        "4343",
        "test",
        "this is a test test test",
        "2.13",
        "3.69",
        "0.11",
        "1",
        "7",
        "5",
    ],
    [
        "1",
        "5.65",
        "9.3",
        "54",
        "1954",
        "bar",
        "second test",
        "12.79",
        "123.456",
        "0.01",
        "12",
        "900",
        "3",
    ],
];

/// The static fields `dir`, `analyzer`, `reader` and `searcher`.
pub struct Statics {
    dir: TestDirectory,
    analyzer: Arc<dyn Analyzer>,
    reader: Arc<dyn DirectoryReader>,
    searcher: IndexSearcher,
}

/// The static state of the class.
pub static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

struct Guard(MutexGuard<'static, Option<Statics>>);

impl Guard {
    fn s(&mut self) -> &mut IndexSearcher {
        &mut self.0.as_mut().expect("beforeClass ran").searcher
    }
}

fn statics() -> Guard {
    Guard(STATICS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `BOGUS_FLOAT_VS`.
fn bogus_float_vs() -> Arc<dyn ValueSource> {
    Arc::new(FloatFieldSource::new("bogus_field"))
}

/// `BOGUS_DOUBLE_VS`.
fn bogus_double_vs() -> Arc<dyn ValueSource> {
    Arc::new(DoubleFieldSource::new("bogus_field"))
}

/// `BOGUS_INT_VS`.
fn bogus_int_vs() -> Arc<dyn ValueSource> {
    Arc::new(IntFieldSource::new("bogus_field"))
}

/// `BOGUS_LONG_VS`.
fn bogus_long_vs() -> Arc<dyn ValueSource> {
    Arc::new(LongFieldSource::new("bogus_field"))
}

/// `new ConstValueSource(constant)`.
fn cvs(constant: f32) -> Arc<dyn ValueSource> {
    Arc::new(ConstValueSource::new(constant))
}

/// `new FunctionQuery(vs)`.
fn fq(vs: Arc<dyn ValueSource>) -> Arc<dyn Query> {
    Arc::new(FunctionQuery::new(vs))
}

/// `TestValueSources`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.function.TestValueSources",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testConst", test_const),
            test("testDiv", test_div),
            test("testDocFreq", test_doc_freq),
            test("testDoubleConst", test_double_const),
            test("testDouble", test_double),
            test("testDoubleMultiValued", test_double_multi_valued),
            test("testFloat", test_float),
            test("testFloatMultiValued", test_float_multi_valued),
            test("testIDF", test_idf),
            test("testIf", test_if),
            test("testInt", test_int),
            test("testIntMultiValued", test_int_multi_valued),
            test("testJoinDocFreq", test_join_doc_freq),
            test("testLinearFloat", test_linear_float),
            test("testLong", test_long),
            test("testLongMultiValued", test_long_multi_valued),
            test("testMaxDoc", test_max_doc),
            test("testMaxFloat", test_max_float),
            test("testMinFloat", test_min_float),
            test("testNorm", test_norm),
            test("testNumDocs", test_num_docs),
            test("testPow", test_pow),
            test("testProduct", test_product),
            test(
                "testQueryWrapedFuncWrapedQuery",
                test_query_wraped_func_wraped_query,
            ),
            test("testQuery", test_query),
            test("testRangeMap", test_range_map),
            test("testReciprocal", test_reciprocal),
            test("testScale", test_scale),
            test("testSumFloat", test_sum_float),
            test("testSumTotalTermFreq", test_sum_total_term_freq),
            test("testTermFreq", test_term_freq),
            test("testMultiBoolFunction", test_multi_bool_function),
            test("testTF", test_tf),
            test("testTotalTermFreq", test_total_term_freq),
            test(
                "testMultiFunctionHelperEquivilence",
                test_multi_function_helper_equivilence,
            ),
            test(
                "testReWrappingAsDoubleValues",
                test_re_wrapping_as_double_values,
            ),
            test("testUnWrappingDoubleValues", test_un_wrapping_double_values),
            test("testWrappingAsDoubleValues", test_wrapping_as_double_values),
            test("testScorerContext", test_scorer_context),
            test(
                "testBuildingFromDoubleValues",
                test_building_from_double_values,
            ),
            test(
                "testFromDoubleValuesSortWithScore",
                test_from_double_values_sort_with_score,
            ),
        ],
    )
}

fn parse_double(s: &str) -> f64 {
    s.parse::<f64>().expect("a double literal")
}

fn parse_float(s: &str) -> f32 {
    s.parse::<f32>().expect("a float literal")
}

fn before_class() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let iw_config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
    iw_config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_config)?;
    for doc in DOCUMENTS {
        let mut document = Document::new();
        document.add(StringField::new("id", doc[0].to_string(), Store::NO)?);
        document.add(SortedDocValuesField::new(
            "id",
            BytesRef::from_char_sequence(doc[0]),
        ));
        document.add(NumericDocValuesField::new(
            "double",
            parse_double(doc[1]).to_bits() as i64,
        ));
        document.add(NumericDocValuesField::new(
            "float",
            i64::from(parse_float(doc[2]).to_bits() as i32),
        ));
        document.add(NumericDocValuesField::new(
            "int",
            i64::from(doc[3].parse::<i32>().expect("an int")),
        ));
        document.add(NumericDocValuesField::new(
            "long",
            doc[4].parse::<i64>().expect("a long"),
        ));
        document.add(StringField::new("string", doc[5].to_string(), Store::NO)?);
        document.add(SortedDocValuesField::new(
            "string",
            BytesRef::from_char_sequence(doc[5]),
        ));
        document.add(TextField::new("text", doc[6].to_string(), Store::NO)?);
        for v in &doc[7..10] {
            document.add(SortedNumericDocValuesField::new(
                "floatMv",
                i64::from(NumericUtils::float_to_sortable_int(parse_float(v))),
            ));
        }
        for v in &doc[7..10] {
            document.add(SortedNumericDocValuesField::new(
                "doubleMv",
                NumericUtils::double_to_sortable_long(parse_double(v)),
            ));
        }
        for v in &doc[10..13] {
            document.add(SortedNumericDocValuesField::new(
                "intMv",
                v.parse::<i64>().expect("a long"),
            ));
        }
        for v in &doc[10..13] {
            document.add(SortedNumericDocValuesField::new(
                "longMv",
                v.parse::<i64>().expect("a long"),
            ));
        }
        iw.add_document(&document)?;
    }

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        dir,
        analyzer,
        reader,
        searcher,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    if let Some(Statics {
        dir,
        analyzer,
        reader,
        searcher,
    }) = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take()
    {
        drop(searcher);
        reader.close()?;
        dir.close()?;
        analyzer.close();
    }
    Ok(())
}

fn test_const() -> Result<()> {
    let mut g = statics();
    let vs = cvs(0.3);
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[0.3, 0.3])?;
    assert_all_exist(g.s(), vs)
}

fn test_div() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(DivFloatFunction::new(cvs(10.0), cvs(5.0)));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[2.0, 2.0])?;
    assert_all_exist(g.s(), vs)?;
    let vs: Arc<dyn ValueSource> = Arc::new(DivFloatFunction::new(cvs(10.0), bogus_float_vs()));
    assert_none_exist(g.s(), vs)?;
    let vs: Arc<dyn ValueSource> = Arc::new(DivFloatFunction::new(bogus_float_vs(), cvs(10.0)));
    assert_none_exist(g.s(), vs)
}

fn test_doc_freq() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(DocFreqValueSource::new(
        "bogus",
        "bogus",
        "text",
        BytesRef::from_char_sequence("test"),
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[2.0, 2.0])?;
    assert_all_exist(g.s(), vs)
}

fn test_double_const() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(DoubleConstValueSource::new(0.3));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[0.3, 0.3])?;
    assert_all_exist(g.s(), vs)
}

fn test_double() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(DoubleFieldSource::new("double"));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[3.63, 5.65])?;
    assert_all_exist(g.s(), vs)?;
    assert_none_exist(g.s(), bogus_double_vs())
}

fn test_double_multi_valued() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(MultiValuedDoubleFieldSource::new(
        "doubleMv",
        SortedNumericSelectorType::MAX,
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[3.69, 123.456])?;
    assert_all_exist(g.s(), vs)?;

    let vs: Arc<dyn ValueSource> = Arc::new(MultiValuedDoubleFieldSource::new(
        "doubleMv",
        SortedNumericSelectorType::MIN,
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[0.11, 0.01])?;
    assert_all_exist(g.s(), vs)
}

fn test_float() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(FloatFieldSource::new("float"));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[5.2, 9.3])?;
    assert_all_exist(g.s(), vs)?;
    assert_none_exist(g.s(), bogus_float_vs())
}

fn test_float_multi_valued() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(MultiValuedFloatFieldSource::new(
        "floatMv",
        SortedNumericSelectorType::MAX,
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[3.69, 123.456])?;
    assert_all_exist(g.s(), vs)?;

    let vs: Arc<dyn ValueSource> = Arc::new(MultiValuedFloatFieldSource::new(
        "floatMv",
        SortedNumericSelectorType::MIN,
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[0.11, 0.01])?;
    assert_all_exist(g.s(), vs)
}

/// `Similarity saved = searcher.getSimilarity(); try { body } finally {
/// searcher.setSimilarity(saved); }`.
fn with_similarity(
    s: &mut IndexSearcher,
    sim: Option<Arc<dyn rucene::search::similarities::Similarity>>,
    body: impl FnOnce(&IndexSearcher) -> Result<()>,
) -> Result<()> {
    let saved = Arc::clone(s.get_similarity());
    if let Some(sim) = sim {
        s.set_similarity(sim);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(s)));
    s.set_similarity(saved);
    match result {
        Ok(r) => r,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn test_idf() -> Result<()> {
    let mut g = statics();
    with_similarity(g.s(), Some(Arc::new(ClassicSimilarity::new())), |s| {
        let vs: Arc<dyn ValueSource> = Arc::new(IDFValueSource::new(
            "bogus",
            "bogus",
            "text",
            BytesRef::from_char_sequence("test"),
        ));
        assert_hits(s, fq(Arc::clone(&vs)), &[1.0, 1.0])?;
        assert_all_exist(s, vs)
    })
}

fn test_if() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(IfFunction::new(
        Arc::new(BytesRefFieldSource::new("id")),
        cvs(1.0), // match
        cvs(2.0),
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[1.0, 1.0])?;
    assert_all_exist(g.s(), vs)?;

    // true just if a test value exists...
    let vs: Arc<dyn ValueSource> = Arc::new(IfFunction::new(
        Arc::new(LiteralValueSource::new("false")),
        cvs(1.0), // match
        cvs(2.0),
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[1.0, 1.0])?;
    assert_all_exist(g.s(), vs)?;

    // false value if tests value does not exist
    let vs: Arc<dyn ValueSource> = Arc::new(IfFunction::new(bogus_float_vs(), cvs(1.0), cvs(2.0))); // match
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[2.0, 2.0])?;
    assert_all_exist(g.s(), vs)?;

    // final value may still not exist
    let vs: Arc<dyn ValueSource> = Arc::new(IfFunction::new(
        Arc::new(BytesRefFieldSource::new("id")),
        bogus_float_vs(), // match
        cvs(1.0),
    ));
    assert_none_exist(g.s(), vs)
}

fn test_int() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(IntFieldSource::new("int"));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[35.0, 54.0])?;
    assert_all_exist(g.s(), vs)?;
    assert_none_exist(g.s(), bogus_int_vs())
}

fn test_int_multi_valued() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(MultiValuedIntFieldSource::new(
        "intMv",
        SortedNumericSelectorType::MAX,
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[7.0, 900.0])?;
    assert_all_exist(g.s(), vs)?;

    let vs: Arc<dyn ValueSource> = Arc::new(MultiValuedIntFieldSource::new(
        "intMv",
        SortedNumericSelectorType::MIN,
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[1.0, 3.0])?;
    assert_all_exist(g.s(), vs)
}

fn test_join_doc_freq() -> Result<()> {
    let mut g = statics();
    assert_hits(
        g.s(),
        fq(Arc::new(JoinDocFreqValueSource::new("string", "text"))),
        &[2.0, 0.0],
    )
    // TODO: what *should* the rules be for exist() ?
}

fn test_linear_float() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(LinearFloatFunction::new(cvs(2.0), 3.0, 1.0));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[7.0, 7.0])?;
    assert_all_exist(g.s(), vs)?;
    let vs: Arc<dyn ValueSource> = Arc::new(LinearFloatFunction::new(bogus_float_vs(), 3.0, 1.0));
    assert_none_exist(g.s(), vs)
}

fn test_long() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(LongFieldSource::new("long"));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[4343.0, 1954.0])?;
    assert_all_exist(g.s(), vs)?;
    assert_none_exist(g.s(), bogus_long_vs())
}

fn test_long_multi_valued() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(MultiValuedLongFieldSource::new(
        "longMv",
        SortedNumericSelectorType::MAX,
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[7.0, 900.0])?;
    assert_all_exist(g.s(), vs)?;

    let vs: Arc<dyn ValueSource> = Arc::new(MultiValuedLongFieldSource::new(
        "longMv",
        SortedNumericSelectorType::MIN,
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[1.0, 3.0])?;
    assert_all_exist(g.s(), vs)
}

fn test_max_doc() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(MaxDocValueSource::new());
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[2.0, 2.0])?;
    assert_all_exist(g.s(), vs)
}

fn test_max_float() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(MaxFloatFunction::new(vec![cvs(1.0), cvs(2.0)]));

    assert_all_exist(g.s(), Arc::clone(&vs))?;
    assert_hits(g.s(), fq(vs), &[2.0, 2.0])?;

    // as long as one value exists, then max exists
    let vs: Arc<dyn ValueSource> =
        Arc::new(MaxFloatFunction::new(vec![bogus_float_vs(), cvs(2.0)]));
    assert_all_exist(g.s(), Arc::clone(&vs))?;
    assert_hits(g.s(), fq(vs), &[2.0, 2.0])?;
    let vs: Arc<dyn ValueSource> = Arc::new(MaxFloatFunction::new(vec![
        bogus_float_vs(),
        cvs(2.0),
        bogus_double_vs(),
    ]));
    assert_all_exist(g.s(), Arc::clone(&vs))?;
    assert_hits(g.s(), fq(vs), &[2.0, 2.0])?;

    // if none exist, then max doesn't exist
    let vs: Arc<dyn ValueSource> = Arc::new(MaxFloatFunction::new(vec![
        bogus_float_vs(),
        bogus_int_vs(),
        bogus_double_vs(),
    ]));
    assert_none_exist(g.s(), Arc::clone(&vs))?;
    assert_hits(g.s(), fq(vs), &[0.0, 0.0])?;

    // if no values exist should return 0f even if value returned is non zero
    let vs: Arc<dyn ValueSource> = Arc::new(MaxFloatFunction::new(vec![Arc::new(
        SumFloatFunction::new(vec![bogus_float_vs(), cvs(42.0)]),
    )]));
    assert_none_exist(g.s(), Arc::clone(&vs))?;
    assert_hits(g.s(), fq(vs), &[0.0, 0.0])
}

fn test_min_float() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(MinFloatFunction::new(vec![cvs(1.0), cvs(2.0)]));

    assert_all_exist(g.s(), Arc::clone(&vs))?;
    assert_hits(g.s(), fq(vs), &[1.0, 1.0])?;

    // as long as one value exists, then min exists
    let vs: Arc<dyn ValueSource> =
        Arc::new(MinFloatFunction::new(vec![bogus_float_vs(), cvs(2.0)]));
    assert_all_exist(g.s(), Arc::clone(&vs))?;
    assert_hits(g.s(), fq(vs), &[2.0, 2.0])?;
    let vs: Arc<dyn ValueSource> = Arc::new(MinFloatFunction::new(vec![
        bogus_float_vs(),
        cvs(2.0),
        bogus_double_vs(),
    ]));
    assert_all_exist(g.s(), Arc::clone(&vs))?;
    assert_hits(g.s(), fq(vs), &[2.0, 2.0])?;

    // if none exist, then min doesn't exist
    let vs: Arc<dyn ValueSource> = Arc::new(MinFloatFunction::new(vec![
        bogus_float_vs(),
        bogus_int_vs(),
        bogus_double_vs(),
    ]));
    assert_none_exist(g.s(), Arc::clone(&vs))?;
    assert_hits(g.s(), fq(vs), &[0.0, 0.0])?;

    // if no values exist should return 0f even if value returned is non zero
    let vs: Arc<dyn ValueSource> = Arc::new(MinFloatFunction::new(vec![Arc::new(
        SumFloatFunction::new(vec![bogus_float_vs(), cvs(42.0)]),
    )]));
    assert_none_exist(g.s(), Arc::clone(&vs))?;
    assert_hits(g.s(), fq(vs), &[0.0, 0.0])
}

fn test_norm() -> Result<()> {
    let mut g = statics();
    // no norm field (so agnostic to indexed similarity)
    with_similarity(g.s(), Some(Arc::new(ClassicSimilarity::new())), |s| {
        let vs: Arc<dyn ValueSource> = Arc::new(NormValueSource::new("byte"));
        assert_hits(s, fq(Arc::clone(&vs)), &[1.0, 1.0])?;

        // regardless of whether norms exist, value source exists == 0
        assert_all_exist(s, vs)?;

        let vs: Arc<dyn ValueSource> = Arc::new(NormValueSource::new("text"));
        assert_all_exist(s, vs)
    })
}

fn test_num_docs() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(NumDocsValueSource::new());
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[2.0, 2.0])?;
    assert_all_exist(g.s(), vs)
}

fn test_pow() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(PowFloatFunction::new(cvs(2.0), cvs(3.0)));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[8.0, 8.0])?;
    assert_all_exist(g.s(), vs)?;
    let vs: Arc<dyn ValueSource> = Arc::new(PowFloatFunction::new(bogus_float_vs(), cvs(3.0)));
    assert_none_exist(g.s(), vs)?;
    let vs: Arc<dyn ValueSource> = Arc::new(PowFloatFunction::new(cvs(3.0), bogus_float_vs()));
    assert_none_exist(g.s(), vs)
}

fn test_product() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(ProductFloatFunction::new(vec![cvs(2.0), cvs(3.0)]));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[6.0, 6.0])?;
    assert_all_exist(g.s(), vs)?;

    let vs: Arc<dyn ValueSource> =
        Arc::new(ProductFloatFunction::new(vec![bogus_float_vs(), cvs(3.0)]));
    assert_none_exist(g.s(), vs)
}

fn test_query_wraped_func_wraped_query() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(QueryValueSource::new(fq(cvs(2.0)), 0.0));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[2.0, 2.0])?;
    assert_all_exist(g.s(), vs)?;

    let vs: Arc<dyn ValueSource> = Arc::new(QueryValueSource::new(
        Arc::new(FunctionRangeQuery::from_numbers(
            Arc::new(IntFieldSource::new("int")),
            Some(rucene::search::similarities::ExplanationValue::Int(
                i32::MIN,
            )),
            Some(rucene::search::similarities::ExplanationValue::Int(
                i32::MAX,
            )),
            true,
            true,
        )),
        0.0,
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[35.0, 54.0])?;
    assert_all_exist(g.s(), vs)
}

/// The anonymous `MultiFloatFunction` of `testQuery`: a quick & dirty
/// wrapper around tf.
#[derive(Debug)]
struct TfBasedExists {
    sources: Vec<Arc<dyn ValueSource>>,
}

impl MultiFloatFunction for TfBasedExists {
    fn sources(&self) -> &[Arc<dyn ValueSource>] {
        &self.sources
    }

    fn name(&self) -> String {
        "tf_based_exists".to_string()
    }

    fn func(&self, doc: i32, vals_arr: &mut [Box<dyn FunctionValues + '_>]) -> Result<f32> {
        vals_arr[0].float_val(doc)
    }

    fn exists(&self, doc: i32, vals_arr: &mut [Box<dyn FunctionValues + '_>]) -> Result<bool> {
        // if tf > 0, then it should exist
        Ok(0.0 < self.func(doc, vals_arr)?)
    }
}

impl ValueSource for TfBasedExists {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        multi_float_function::get_values(self, context, reader_context)
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        multi_float_function::value_source_eq(self, other)
    }

    fn value_source_hash(&self) -> i32 {
        multi_float_function::value_source_hash(self)
    }

    fn description(&self) -> String {
        multi_float_function::description(self)
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        multi_float_function::create_weight(self, context, searcher)
    }
}

fn test_query() -> Result<()> {
    let mut g = statics();
    with_similarity(g.s(), Some(Arc::new(ClassicSimilarity::new())), |s| {
        let vs: Arc<dyn ValueSource> = Arc::new(QueryValueSource::new(
            Arc::new(TermQuery::new(Term::from_text("string", "bar"))),
            42.0,
        ));
        assert_hits(s, fq(Arc::clone(&vs)), &[42.0, 1.4054651])?;

        // valuesource should exist only for things matching the term query
        // sanity check via quick & dirty wrapper arround tf
        let expected: Arc<dyn ValueSource> = Arc::new(TfBasedExists {
            sources: vec![Arc::new(TFValueSource::new(
                "bogus",
                "bogus",
                "string",
                BytesRef::from_char_sequence("bar"),
            ))],
        });

        assert_exists(s, expected, vs)?;

        // Query matches all docs, func exists for all docs
        let vs: Arc<dyn ValueSource> = Arc::new(QueryValueSource::new(
            Arc::new(TermQuery::new(Term::from_text("text", "test"))),
            0.0,
        ));
        assert_all_exist(s, vs)?;

        // Query matches no docs, func exists for no docs
        let vs: Arc<dyn ValueSource> = Arc::new(QueryValueSource::new(
            Arc::new(TermQuery::new(Term::from_text("bogus", "does not exist"))),
            0.0,
        ));
        assert_none_exist(s, vs)?;

        // doc doesn't match the query, so default value should be returned
        let vs: Arc<dyn ValueSource> = Arc::new(QueryValueSource::new(
            Arc::new(MatchNoDocsQuery::instance()),
            5.0,
        ));
        let leaf = Arc::clone(&s.get_leaf_contexts()[0]);
        let mut fv = vs.get_values(&new_context(Some(s)), &leaf)?;
        Assert::assert_true_msg(
            "expected:<5.0>",
            fv.object_val(1)? == Some(MutableValueObject::Float(5.0)),
        );

        // test with def value but doc matches the query, so def value shouldn't be returned
        let vs: Arc<dyn ValueSource> = Arc::new(QueryValueSource::new(
            Arc::new(TermQuery::new(Term::from_text("text", "test"))),
            2.0,
        ));
        let mut fv = vs.get_values(&new_context(Some(s)), &leaf)?;
        Assert::assert_true_msg(
            "Values should be different. Actual: 2.0",
            fv.object_val(1)? != Some(MutableValueObject::Float(2.0)),
        );
        Ok(())
    })
}

fn test_range_map() -> Result<()> {
    let mut g = statics();
    assert_hits(
        g.s(),
        fq(Arc::new(RangeMapFloatFunction::with_floats(
            Arc::new(FloatFieldSource::new("float")),
            5.0,
            6.0,
            1.0,
            Some(0.0),
        ))),
        &[1.0, 0.0],
    )?;
    assert_hits(
        g.s(),
        fq(Arc::new(RangeMapFloatFunction::new(
            Arc::new(FloatFieldSource::new("float")),
            5.0,
            6.0,
            Arc::new(SumFloatFunction::new(vec![cvs(1.0), cvs(2.0)])),
            Some(cvs(11.0)),
        ))),
        &[3.0, 11.0],
    )
    // TODO: what *should* the rules be for exist() ?
    // ((source exists && source in range && target exists) OR (source not in range && default
    // exists)) ?
}

fn test_reciprocal() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(ReciprocalFloatFunction::new(cvs(2.0), 3.0, 1.0, 4.0));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[0.1, 0.1])?;
    assert_all_exist(g.s(), vs)?;

    let vs: Arc<dyn ValueSource> = Arc::new(ReciprocalFloatFunction::new(
        bogus_float_vs(),
        3.0,
        1.0,
        4.0,
    ));
    assert_none_exist(g.s(), vs)
}

fn test_scale() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(ScaleFloatFunction::new(
        Arc::new(IntFieldSource::new("int")),
        0.0,
        1.0,
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[0.0, 1.0])?;
    assert_all_exist(g.s(), vs)?;

    let vs: Arc<dyn ValueSource> = Arc::new(ScaleFloatFunction::new(bogus_int_vs(), 0.0, 1.0));
    assert_none_exist(g.s(), vs)
}

fn test_sum_float() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(SumFloatFunction::new(vec![cvs(1.0), cvs(2.0)]));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[3.0, 3.0])?;
    assert_all_exist(g.s(), vs)?;

    let vs: Arc<dyn ValueSource> =
        Arc::new(SumFloatFunction::new(vec![bogus_float_vs(), cvs(2.0)]));
    assert_none_exist(g.s(), vs)
}

fn test_sum_total_term_freq() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(SumTotalTermFreqValueSource::new("text"));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[8.0, 8.0])?;
    assert_all_exist(g.s(), vs)
}

fn test_term_freq() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(TermFreqValueSource::new(
        "bogus",
        "bogus",
        "text",
        BytesRef::from_char_sequence("test"),
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[3.0, 1.0])?;
    assert_all_exist(g.s(), vs)?;

    let vs: Arc<dyn ValueSource> = Arc::new(TermFreqValueSource::new(
        "bogus",
        "bogus",
        "string",
        BytesRef::from_char_sequence("bar"),
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[0.0, 1.0])?;
    assert_all_exist(g.s(), vs)?;

    // regardless of whether norms exist, value source exists == 0
    let vs: Arc<dyn ValueSource> = Arc::new(TermFreqValueSource::new(
        "bogus",
        "bogus",
        "bogus",
        BytesRef::from_char_sequence("bogus"),
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[0.0, 0.0])?;
    assert_all_exist(g.s(), vs)
}

/// The anonymous `MultiBoolFunction` of `testMultiBoolFunction`.
#[derive(Debug)]
struct TestMultiBool {
    sources: Vec<Arc<dyn ValueSource>>,
}

impl BoolFunction for TestMultiBool {}

impl MultiBoolFunction for TestMultiBool {
    fn sources(&self) -> &[Arc<dyn ValueSource>] {
        &self.sources
    }

    fn name(&self) -> String {
        "test".to_string()
    }

    fn func(&self, _doc: i32, _vals: &mut [Box<dyn FunctionValues + '_>]) -> Result<bool> {
        Ok(false)
    }
}

impl ValueSource for TestMultiBool {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        multi_bool_function::get_values(self, context, reader_context)
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        multi_bool_function::value_source_eq(self, other)
    }

    fn value_source_hash(&self) -> i32 {
        multi_bool_function::value_source_hash(self)
    }

    fn description(&self) -> String {
        multi_bool_function::description(self)
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        multi_bool_function::create_weight(self, context, searcher)
    }
}

fn test_multi_bool_function() -> Result<()> {
    let mut g = statics();
    let s = g.s();
    // verify toString and description
    let value_sources: Vec<Arc<dyn ValueSource>> = vec![
        cvs(4.1),
        cvs(1.2),
        Arc::new(DoubleFieldSource::new("some_double")),
    ];
    let vs: Arc<dyn ValueSource> = Arc::new(TestMultiBool {
        sources: value_sources,
    });
    Assert::assert_equals_str(
        None,
        Some("test(const(4.1),const(1.2),double(some_double))"),
        Some(&vs.description()),
    );

    let leaf = Arc::clone(&s.get_leaf_contexts()[0]);
    let mut fv = vs.get_values(&new_context(Some(s)), &leaf)?;
    // doesn't matter what is the docId, verify toString
    Assert::assert_equals_str(
        None,
        Some("test(const(4.1),const(1.2),double(some_double)=0.0)"),
        Some(&fv.to_doc_string(1)?),
    );
    Ok(())
}

fn test_tf() -> Result<()> {
    let mut g = statics();
    // no norm field (so agnostic to indexed similarity)
    with_similarity(g.s(), Some(Arc::new(ClassicSimilarity::new())), |s| {
        let vs: Arc<dyn ValueSource> = Arc::new(TFValueSource::new(
            "bogus",
            "bogus",
            "text",
            BytesRef::from_char_sequence("test"),
        ));
        assert_hits(
            s,
            fq(Arc::clone(&vs)),
            &[3f64.sqrt() as f32, 1f64.sqrt() as f32],
        )?;
        assert_all_exist(s, vs)?;

        let vs: Arc<dyn ValueSource> = Arc::new(TFValueSource::new(
            "bogus",
            "bogus",
            "string",
            BytesRef::from_char_sequence("bar"),
        ));
        assert_hits(s, fq(Arc::clone(&vs)), &[0.0, 1.0])?;
        assert_all_exist(s, vs)?;

        // regardless of whether norms exist, value source exists == 0
        let vs: Arc<dyn ValueSource> = Arc::new(TFValueSource::new(
            "bogus",
            "bogus",
            "bogus",
            BytesRef::from_char_sequence("bogus"),
        ));
        assert_hits(s, fq(Arc::clone(&vs)), &[0.0, 0.0])?;
        assert_all_exist(s, vs)
    })
}

fn test_total_term_freq() -> Result<()> {
    let mut g = statics();
    let vs: Arc<dyn ValueSource> = Arc::new(TotalTermFreqValueSource::new(
        "bogus",
        "bogus",
        "text",
        BytesRef::from_char_sequence("test"),
    ));
    assert_hits(g.s(), fq(Arc::clone(&vs)), &[4.0, 4.0])?;
    assert_all_exist(g.s(), vs)
}

fn test_multi_function_helper_equivilence() -> Result<()> {
    let mut g = statics();
    let s = g.s();
    // the 2 arg versions of these methods should return the exact same results as
    // the multi arg versions with a 2 element array

    // actual doc / index is not relevant for this test
    let leaf = Arc::clone(&s.get_leaf_contexts()[0]);
    let context = new_context(Some(s));

    ALL_EXIST_VS.create_weight(&context, s)?;
    NONE_EXIST_VS.create_weight(&context, s)?;

    // `ALL` and `NONE` (`ALL_EXIST_VS.getValues(context, leaf)`, ...). Java
    // reuses each object across the arrays; their values are constant, so the
    // port opens a fresh one per slot, as a Rust slice cannot hold one box
    // twice.
    let fv = |all: bool| -> Result<Box<dyn FunctionValues + 'static>> {
        if all {
            Arc::clone(&ALL_EXIST_VS).get_values(&context, &leaf)
        } else {
            Arc::clone(&NONE_EXIST_VS).get_values(&context, &leaf)
        }
    };
    const ALL: bool = true;
    const NONE: bool = false;

    // quick sanity checks of explicit results
    Assert::assert_true(multi_function::all_exists2(
        1,
        &mut *fv(ALL)?,
        &mut *fv(ALL)?,
    )?);
    Assert::assert_true(multi_function::all_exists(1, &mut [fv(ALL)?, fv(ALL)?])?);
    Assert::assert_true(multi_function::any_exists2(
        1,
        &mut *fv(ALL)?,
        &mut *fv(NONE)?,
    )?);
    Assert::assert_true(multi_function::any_exists(1, &mut [fv(ALL)?, fv(NONE)?])?);
    //
    Assert::assert_false(multi_function::all_exists2(
        1,
        &mut *fv(ALL)?,
        &mut *fv(NONE)?,
    )?);
    Assert::assert_false(multi_function::all_exists(1, &mut [fv(ALL)?, fv(NONE)?])?);
    Assert::assert_false(multi_function::any_exists2(
        1,
        &mut *fv(NONE)?,
        &mut *fv(NONE)?,
    )?);
    Assert::assert_false(multi_function::any_exists(1, &mut [fv(NONE)?, fv(NONE)?])?);

    let name = |all: bool| if all { "ALL" } else { "NONE" };
    // iterate all permutations and verify equivilence
    for first_arg in [ALL, NONE] {
        for second_arg in [ALL, NONE] {
            Assert::assert_equals_msg(
                Some(&format!(
                    "allExists({},{})",
                    name(first_arg),
                    name(second_arg)
                )),
                multi_function::all_exists2(1, &mut *fv(first_arg)?, &mut *fv(second_arg)?)?,
                multi_function::all_exists(1, &mut [fv(first_arg)?, fv(second_arg)?])?,
            );
            Assert::assert_equals_msg(
                Some(&format!(
                    "anyExists({},{})",
                    name(first_arg),
                    name(second_arg)
                )),
                multi_function::any_exists2(1, &mut *fv(first_arg)?, &mut *fv(second_arg)?)?,
                multi_function::any_exists(1, &mut [fv(first_arg)?, fv(second_arg)?])?,
            );

            // future proof against posibility of someone "optimizing" the array method
            // if .length==2 ... redundent third arg should give same results as well...
            Assert::assert_equals_msg(
                Some(&format!(
                    "allExists({},{},{})",
                    name(first_arg),
                    name(second_arg),
                    name(second_arg)
                )),
                multi_function::all_exists2(1, &mut *fv(first_arg)?, &mut *fv(second_arg)?)?,
                multi_function::all_exists(
                    1,
                    &mut [fv(first_arg)?, fv(second_arg)?, fv(second_arg)?],
                )?,
            );
            Assert::assert_equals_msg(
                Some(&format!(
                    "anyExists({},{},{})",
                    name(first_arg),
                    name(second_arg),
                    name(second_arg)
                )),
                multi_function::any_exists2(1, &mut *fv(first_arg)?, &mut *fv(second_arg)?)?,
                multi_function::any_exists(
                    1,
                    &mut [fv(first_arg)?, fv(second_arg)?, fv(second_arg)?],
                )?,
            );
        }
    }
    Ok(())
}

/// `assertSame(expected, actual)` over two shared objects.
fn assert_same<T: ?Sized>(expected: &Arc<T>, actual: &Arc<T>) {
    if !std::ptr::addr_eq(Arc::as_ptr(expected), Arc::as_ptr(actual)) {
        Assert::fail("expected same");
    }
}

fn test_re_wrapping_as_double_values() -> Result<()> {
    let original: Arc<dyn ValueSource> = Arc::new(DoubleFieldSource::new("double"));
    assert_same(
        &original,
        &from_double_values_source(Arc::clone(&original).as_double_values_source()),
    );
    Ok(())
}

fn test_un_wrapping_double_values() -> Result<()> {
    let original = double_values_source::from_double_field("double");
    assert_same(
        &original,
        &from_double_values_source(Arc::clone(&original)).as_double_values_source(),
    );
    Ok(())
}

/// The local class `AssertScoreComputedOnceQuery` of
/// `testWrappingAsDoubleValues`.
#[derive(Debug)]
struct AssertScoreComputedOnceQuery {
    in_: Arc<dyn Query>,
}

/// The anonymous `FilterWeight` of `AssertScoreComputedOnceQuery`: every
/// member but `scorerSupplier` delegates to the wrapped weight.
#[derive(Debug)]
struct AssertScoreComputedOnceWeight {
    in_: Arc<dyn Weight>,
}

/// The anonymous `FilterScorer` of `AssertScoreComputedOnceWeight`.
struct AssertScoreComputedOnceScorer {
    in_: Box<dyn Scorer>,
    last_doc_id: i32,
}

impl Scorable for AssertScoreComputedOnceScorer {
    fn score(&mut self) -> Result<f32> {
        Assert::assert_true_msg(
            "shouldn't re-compute score",
            self.last_doc_id != self.doc_id(),
        );
        self.last_doc_id = self.doc_id();
        self.in_.score()
    }
}

impl Scorer for AssertScoreComputedOnceScorer {
    fn doc_id(&self) -> i32 {
        self.in_.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.in_.iterator()
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        self.in_.two_phase_iterator()
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.in_.get_max_score(up_to)
    }
}

impl SegmentCacheable for AssertScoreComputedOnceWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.in_.is_cacheable(ctx)
    }
}

impl Weight for AssertScoreComputedOnceWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.in_.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.in_.explain(context, doc)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.in_.matches(context, doc)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.in_.count(context)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(sub_scorer) = self.in_.scorer(context)? else {
            return Ok(None);
        };
        let scorer = AssertScoreComputedOnceScorer {
            in_: sub_scorer,
            last_doc_id: -1,
        };
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }
}

impl Query for AssertScoreComputedOnceQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.in_.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.in_.visit(visitor)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(AssertScoreComputedOnceWeight {
            in_: self.in_.create_weight(searcher, score_mode, boost)?,
        }))
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        // `rewrite == in ? this : new AssertScoreComputedOnceQuery(rewrite)`
        Ok(self
            .in_
            .rewrite(index_searcher)?
            .filter(|rewrite| !std::ptr::addr_eq(Arc::as_ptr(rewrite), Arc::as_ptr(&self.in_)))
            .map(|rewrite| {
                Arc::new(AssertScoreComputedOnceQuery { in_: rewrite }) as Arc<dyn Query>
            }))
    }

    fn query_eq(&self, _other: &dyn Query) -> bool {
        Throwable::new("java.lang.UnsupportedOperationException", String::new()).throw()
    }

    fn query_hash(&self) -> i32 {
        self.in_.query_hash()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.function.TestValueSources$1AssertScoreComputedOnceQuery"
    }
}

fn test_wrapping_as_double_values() -> Result<()> {
    let mut g = statics();
    let s = g.s();
    let q = FunctionScoreQuery::boost_by_value(
        Arc::new(AssertScoreComputedOnceQuery {
            in_: Arc::new(TermQuery::new(Term::from_text("text", "test"))),
        }),
        (Arc::new(DoubleFieldSource::new("double")) as Arc<dyn ValueSource>)
            .as_double_values_source(),
    );
    let q = Arc::new(q);
    let top_field_docs = s.search(Arc::clone(&q) as Arc<dyn Query>, 1)?;
    Assert::assert_true(!top_field_docs.score_docs.is_empty());

    // assert that the query has not cached a reference to the IndexSearcher
    let class_cast = |what: &str| -> ! {
        Throwable::new("java.lang.ClassCastException", format!("not a {what}")).throw()
    };
    let source1 = q
        .get_source()
        .as_any()
        .downcast_ref::<MultiplicativeBoostValuesSource>()
        .unwrap_or_else(|| class_cast("MultiplicativeBoostValuesSource"));
    let source2 = source1
        .boost
        .as_any()
        .downcast_ref::<WrappedDoubleValuesSource>()
        .unwrap_or_else(|| class_cast("WrappedDoubleValuesSource"));
    Assert::assert_true_msg(
        "expected null, but was:<IndexSearcher>",
        source2.searcher.is_none(),
    );
    Ok(())
}

/// The local class `ScoreValueSource` of `testScorerContext`: a VS that
/// yields the score.
#[derive(Debug)]
struct ScoreValueSource;

/// Its anonymous `FloatDocValues`.
struct ScoreValues<'a> {
    vs: Arc<ScoreValueSource>,
    scorer: SharedScorable<'a>,
}

impl ValueSource for ScoreValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let scorer = context.scorer().cloned();
        Assert::assert_true_msg("", scorer.is_some());
        Ok(Box::new(ScoreValues {
            vs: self,
            scorer: scorer.expect("assertNotNull(scorer)"),
        }))
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        std::ptr::addr_eq(self as *const Self, other as *const dyn ValueSource) // just for a test
    }

    fn value_source_hash(&self) -> i32 {
        0 // just for a test
    }

    fn description(&self) -> String {
        "score".to_string()
    }
}

/// The members a `FloatDocValues` subclass inherits, delegated to the
/// `float_doc_values` module functions (the crate's
/// `float_doc_values_members!` is private to the crate).
macro_rules! float_doc_values_inherited {
    () => {
        fn byte_val(&mut self, doc: i32) -> Result<i8> {
            float_doc_values::byte_val(self, doc)
        }
        fn short_val(&mut self, doc: i32) -> Result<i16> {
            float_doc_values::short_val(self, doc)
        }
        fn int_val(&mut self, doc: i32) -> Result<i32> {
            float_doc_values::int_val(self, doc)
        }
        fn long_val(&mut self, doc: i32) -> Result<i64> {
            float_doc_values::long_val(self, doc)
        }
        fn bool_val(&mut self, doc: i32) -> Result<bool> {
            float_doc_values::bool_val(self, doc)
        }
        fn double_val(&mut self, doc: i32) -> Result<f64> {
            float_doc_values::double_val(self, doc)
        }
        fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
            float_doc_values::str_val(self, doc)
        }
        fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
            float_doc_values::object_val(self, doc)
        }
        fn to_doc_string(&mut self, doc: i32) -> Result<String> {
            float_doc_values::to_doc_string(self, doc)
        }
        fn get_value_filler(&self) -> Box<dyn ValueFiller> {
            float_doc_values::get_value_filler(self)
        }
    };
}

impl FloatDocValues for ScoreValues<'_> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.vs
    }
}

impl FunctionValues for ScoreValues<'_> {
    fn float_val(&mut self, _doc: i32) -> Result<f32> {
        self.scorer.borrow_mut().score()
    }

    float_doc_values_inherited!();
}

/// Tests "scorer" key-value inside the Map context to ValueSource.
fn test_scorer_context() -> Result<()> {
    let mut g = statics();
    let s = g.s();
    let plain_q: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("text", "test")));
    let orig_score = s.search(Arc::clone(&plain_q), 1)?.score_docs[0].score;

    // boosts the score by the value source (which is the score), thus score^2
    let score_squared_q: Arc<dyn Query> = Arc::new(FunctionScoreQuery::boost_by_value(
        plain_q,
        (Arc::new(ScoreValueSource) as Arc<dyn ValueSource>).as_double_values_source(),
    ));
    let top_field_docs = s.search(score_squared_q, 1)?;
    Assert::assert_true(!top_field_docs.score_docs.is_empty());
    Assert::assert_equals_double_msg(
        None,
        f64::from(orig_score * orig_score),
        f64::from(top_field_docs.score_docs[0].score),
        0.00001,
    );
    Ok(())
}

fn test_building_from_double_values() -> Result<()> {
    let mut g = statics();
    let dvs = from_double_values_source(double_values_source::from_double_field("double"))
        .as_double_values_source();
    assert_hits(g.s(), fq(from_double_values_source(dvs)), &[3.63, 5.65])
}

fn test_from_double_values_sort_with_score() -> Result<()> {
    let mut g = statics();
    let s = g.s();
    // Create a query that matches all documents
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("text", "test")));
    // Get the original scores
    let original_docs = s.search(Arc::clone(&query), DOCUMENTS.len() as i32)?;
    let original_score_docs = original_docs.score_docs;
    // Use a DoubleValuesSource that references the score and convert it to a ValueSource
    let score_source = from_double_values_source(double_values_source::scores());
    let sort_field = score_source.get_sort_field(true)?; // true for reverse (descending)
    let sort = Sort::new_fields(vec![sort_field])?;
    // Search with the sort.
    //   note: used to fail with an assertion before fromDoubleValuesSource's impl was improved
    let sorted_docs = s.search_with_sort(query, DOCUMENTS.len() as i32, &sort)?;
    let sorted_score_docs = sorted_docs.score_docs;
    // Verify we got the same docs in-order but don't check the scores
    Assert::assert_equals(original_score_docs.len(), sorted_score_docs.len());
    for i in 0..original_score_docs.len() {
        Assert::assert_equals(
            original_score_docs[i].doc,
            sorted_score_docs[i].score_doc.doc,
        );
    }
    Ok(())
}

/// Asserts that for every doc, the `FunctionValues#exists` value from the
/// `ValueSource` is **true**.
fn assert_all_exist(s: &IndexSearcher, vs: Arc<dyn ValueSource>) -> Result<()> {
    assert_exists(s, Arc::clone(&ALL_EXIST_VS), vs)
}

/// Asserts that for every doc, the `FunctionValues#exists` value from the
/// `ValueSource` is **false**.
fn assert_none_exist(s: &IndexSearcher, vs: Arc<dyn ValueSource>) -> Result<()> {
    assert_exists(s, Arc::clone(&NONE_EXIST_VS), vs)
}

/// Asserts that for every doc, the `FunctionValues#exists` value from the
/// `actual` `ValueSource` matches the `FunctionValues#exists` value from the
/// `expected` `ValueSource`.
fn assert_exists(
    s: &IndexSearcher,
    expected: Arc<dyn ValueSource>,
    actual: Arc<dyn ValueSource>,
) -> Result<()> {
    let context = new_context(Some(s));
    let result = (|| -> Result<()> {
        expected.create_weight(&context, s)?;
        actual.create_weight(&context, s)?;

        for leaf in s.get_leaf_contexts() {
            let mut expected_vals = Arc::clone(&expected).get_values(&context, leaf)?;
            let mut actual_vals = Arc::clone(&actual).get_values(&context, leaf)?;

            let msg = format!("{} ?= {} -> ", expected.description(), actual.description());
            for i in 0..leaf.leaf_reader().max_doc() {
                Assert::assert_equals_msg(
                    Some(&format!("{msg}{i}")),
                    expected_vals.exists(i)?,
                    actual_vals.exists(i)?,
                );
            }
        }
        Ok(())
    })();
    match result {
        // catch (IOException e) { throw new RuntimeException(actual.toString(), e); }
        Err(e) if e.is_io_exception() => Err(LuceneError::from(
            rucene::error::LuceneErrorKind::Unchecked {
                class: "java.lang.RuntimeException",
                message: Some(actual.description()),
                // `new RuntimeException(actual.toString(), e)`
                cause: Some(Arc::new(e)),
            },
        )),
        other => other,
    }
}

/// `assertHits(Query, float[])`.
fn assert_hits(s: &IndexSearcher, q: Arc<dyn Query>, scores: &[f32]) -> Result<()> {
    let mut expected: Vec<ScoreDoc> = Vec::with_capacity(scores.len());
    let mut expected_docs = vec![0i32; scores.len()];
    for i in 0..scores.len() {
        expected_docs[i] = i as i32;
        expected.push(ScoreDoc::new(i as i32, scores[i]));
    }
    let docs =
        s.search_with_sort_and_scores(Arc::clone(&q), DOCUMENTS.len() as i32, &id_sort()?, true)?;
    let score_docs: Vec<ScoreDoc> = docs.score_docs.iter().map(|fd| fd.score_doc).collect();
    CheckHits::check_hits(&*random(), &q, Some(""), s, &expected_docs)?;
    CheckHits::check_hits_query(&*q, &expected, &score_docs, &expected_docs);
    CheckHits::check_explanations(&q, Some(""), s)
}

/// Simple test value source that implements `FunctionValues#exists` as a
/// constant.
#[derive(Debug)]
struct ExistsValueSource {
    expected: bool,
    value: i32,
}

impl ExistsValueSource {
    fn new(expected: bool) -> Self {
        Self {
            expected,
            value: if expected { 1 } else { 0 },
        }
    }
}

/// The anonymous `FloatDocValues` of `ExistsValueSource.getValues`.
struct ExistsValues {
    vs: Arc<ExistsValueSource>,
}

impl FloatDocValues for ExistsValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.vs
    }
}

impl FunctionValues for ExistsValues {
    fn float_val(&mut self, _doc: i32) -> Result<f32> {
        Ok(self.vs.value as f32)
    }

    fn exists(&mut self, _doc: i32) -> Result<bool> {
        Ok(self.vs.expected)
    }

    float_doc_values_inherited!();
}

impl ValueSource for ExistsValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        Ok(Box::new(ExistsValues { vs: self }))
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        std::ptr::addr_eq(other as *const dyn ValueSource, self as *const Self)
    }

    fn value_source_hash(&self) -> i32 {
        self.value
    }

    fn description(&self) -> String {
        if self.expected {
            "ALL_EXIST"
        } else {
            "NONE_EXIST"
        }
        .to_string()
    }
}

/// `ALL_EXIST_VS`.
static ALL_EXIST_VS: LazyLock<Arc<dyn ValueSource>> =
    LazyLock::new(|| Arc::new(ExistsValueSource::new(true)) as Arc<dyn ValueSource>);

/// `NONE_EXIST_VS`.
static NONE_EXIST_VS: LazyLock<Arc<dyn ValueSource>> =
    LazyLock::new(|| Arc::new(ExistsValueSource::new(false)) as Arc<dyn ValueSource>);
