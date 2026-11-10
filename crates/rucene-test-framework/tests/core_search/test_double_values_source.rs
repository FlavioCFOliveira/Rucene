//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDoubleValuesSource.java`.
//!
//! Notes on the translation:
//!
//! * The collectors of `testExplanations` and `doTestQueryDoubleValuesSources`
//!   build `rewritten.getValues(ctx, DoubleValuesSource.fromScorer(scorer))`
//!   in `setScorer` and keep it. Rucene's `from_scorer` borrows the scorable
//!   for the life of the values and the scorable reaches a leaf collector
//!   only as an argument of each call, so the values are built in `collect`,
//!   from the scorable of that call, then advanced to the doc as Java does.
//! * The collectors keep the context `doSetNextReader` receives; Rucene's
//!   context is borrowed, so they keep its ordinal and read the context back
//!   from the searcher's leaves.
//! * `testRewriteSame`'s `assertSame(doubleField, doubleField.rewrite(searcher))`
//!   is checked by value: Rucene's `SortField::rewrite` returns a clone (a
//!   pre-existing divergence, declared at `src/search/sort.rs`).

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{DoubleDocValuesField, FloatDocValuesField, NumericDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::reader_context::LeafReaderContext;
use rucene::search::double_values_source::{
    self as dvs, from_scorer, get_sort_field, get_sort_field_with_missing, DoubleValuesSource,
};
use rucene::search::field_comparator::SortValue;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::scorable::Scorable;
use rucene::search::term_query::TermQuery;
use rucene::search::{
    BooleanQueryBuilder, CollectionResult, CollectorManager, LeafCollector, MatchAllDocsQuery,
    Occur, PhraseQueryBuilder, Query, ScoreMode, SimpleCollector, SimpleCollectorImpl, Sort,
    SortField, SortFieldType,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::english::English;
use rucene_test_framework::util::lucene_test_case::{SuppressCodecs, TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{score_docs_of, static_hook, term};
use crate::core_support::{
    assert_equals, assert_equals_delta, class_annotated, random, shuffle, test, unchecked,
};

const LEAST_DOUBLE_VALUE: f64 = 45.72;

struct Statics {
    dir: TestDirectory,
    reader: Arc<dyn DirectoryReader>,
    searcher: Arc<IndexSearcher>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// The static `searcher`.
fn searcher() -> Arc<IndexSearcher> {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&guard.as_ref().expect("beforeClass ran").searcher)
}

/// `TestDoubleValuesSource` (`@LuceneTestCase.SuppressCodecs("SimpleText")`).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.search.TestDoubleValuesSource",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testSortMissingZeroDefault", test_sort_missing_zero_default),
            test("testSortMissingExplicit", test_sort_missing_explicit),
            test(
                "testSimpleFieldEquivalences",
                test_simple_field_equivalences,
            ),
            test("testHashCodeAndEquals", test_hash_code_and_equals),
            test("testSimpleFieldSortables", test_simple_field_sortables),
            test("testExplanations", test_explanations),
            test(
                "testQueryDoubleValuesSource",
                test_query_double_values_source,
            ),
            test("testRewriteSame", test_rewrite_same),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

fn before_class() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = if *TEST_NIGHTLY {
        TestUtil::next_int(&*random(), 2049, 4000)
    } else {
        LuceneTestCase::at_least(546)
    };
    for i in 0..num_docs {
        let mut document = rucene::document::Document::new();
        document.add(Box::new(LuceneTestCase::new_text_field(
            "english",
            &English::int_to_english(i),
            Store::NO,
        )));
        document.add(Box::new(LuceneTestCase::new_text_field(
            "oddeven",
            if i % 2 == 0 { "even" } else { "odd" },
            Store::NO,
        )));
        document.add(Box::new(NumericDocValuesField::new(
            "int",
            i64::from(random().next_int()),
        )));
        document.add(Box::new(NumericDocValuesField::new(
            "long",
            random().next_long(),
        )));
        document.add(Box::new(FloatDocValuesField::new(
            "float",
            random().next_float(),
        )));
        document.add(Box::new(DoubleDocValuesField::new(
            "double",
            random().next_double(),
        )));
        if i == 545 {
            document.add(Box::new(DoubleDocValuesField::new(
                "onefield",
                LEAST_DOUBLE_VALUE,
            )));
        }
        iw.add_document(&document)?;
    }
    let reader = iw.get_reader()?;
    iw.close()?;
    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        dir,
        reader,
        searcher,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(Statics {
        mut dir,
        reader,
        searcher,
    }) = statics
    {
        reader.close()?;
        dir.close()?;
        drop(searcher);
    }
    Ok(())
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

/// `new Sort(field)`.
fn sort_of(fields: Vec<SortField>) -> Result<Sort> {
    Sort::new_fields(fields)
}

/// `(FieldDoc) results.scoreDocs[0]` and its `fields[0]`.
fn first_field(results: &rucene::search::TopFieldDocs) -> SortValue {
    results.score_docs[0]
        .fields
        .as_ref()
        .expect("NullPointerException")[0]
        .clone()
}

fn test_sort_missing_zero_default() -> Result<()> {
    let searcher = searcher();
    // docs w/no value get default missing value = 0

    let onefield = dvs::from_double_field("onefield");
    // sort decreasing
    let results = searcher.search_with_sort(
        match_all(),
        1,
        &sort_of(vec![get_sort_field(Arc::clone(&onefield), true)?])?,
    )?;
    assert_equals(
        &SortValue::Double(LEAST_DOUBLE_VALUE),
        &first_field(&results),
    );

    // sort increasing
    let results = searcher.search_with_sort(
        match_all(),
        1,
        &sort_of(vec![get_sort_field(Arc::clone(&onefield), false)?])?,
    )?;
    assert_equals(&SortValue::Double(0.0), &first_field(&results));
    Ok(())
}

fn test_sort_missing_explicit() -> Result<()> {
    let searcher = searcher();
    // docs w/no value get provided missing value

    let onefield = dvs::from_double_field("onefield");

    // sort decreasing, missing last
    // `Double.MIN_VALUE`.
    let one_field_sort =
        get_sort_field_with_missing(Arc::clone(&onefield), true, f64::from_bits(1))?;

    let results = searcher.search_with_sort(match_all(), 1, &sort_of(vec![one_field_sort])?)?;
    assert_equals(
        &SortValue::Double(LEAST_DOUBLE_VALUE),
        &first_field(&results),
    );

    // sort increasing, missing last
    let one_field_sort = get_sort_field_with_missing(Arc::clone(&onefield), false, f64::MAX)?;

    let results = searcher.search_with_sort(match_all(), 1, &sort_of(vec![one_field_sort])?)?;
    assert_equals(
        &SortValue::Double(LEAST_DOUBLE_VALUE),
        &first_field(&results),
    );
    Ok(())
}

/// `new SortField(field, type, reverse)`.
fn sort_field(field: &str, field_type: SortFieldType, reverse: bool) -> Result<SortField> {
    SortField::new_reverse(Some(field.to_string()), field_type, reverse)
}

fn test_simple_field_equivalences() -> Result<()> {
    check_sorts(
        match_all(),
        sort_of(vec![sort_field(
            "int",
            SortFieldType::Int,
            random().next_boolean(),
        )?])?,
    )?;
    check_sorts(
        match_all(),
        sort_of(vec![sort_field(
            "long",
            SortFieldType::Long,
            random().next_boolean(),
        )?])?,
    )?;
    check_sorts(
        match_all(),
        sort_of(vec![sort_field(
            "float",
            SortFieldType::Float,
            random().next_boolean(),
        )?])?,
    )?;
    check_sorts(
        match_all(),
        sort_of(vec![sort_field(
            "double",
            SortFieldType::Double,
            random().next_boolean(),
        )?])?,
    )?;
    Ok(())
}

/// `assertEquals(a, b)` over `DoubleValuesSource.equals`.
fn assert_source_equals(a: &Arc<dyn DoubleValuesSource>, b: &Arc<dyn DoubleValuesSource>) {
    if !a.source_eq(&**b) {
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            a.to_source_string(),
            b.to_source_string()
        ));
    }
}

fn test_hash_code_and_equals() -> Result<()> {
    let vs1 = dvs::from_double_field("double");
    let vs2 = dvs::from_double_field("double");
    assert_source_equals(&vs1, &vs2);
    assert_equals(&vs1.source_hash(), &vs2.source_hash());
    let v3 = dvs::from_long_field("long");
    Assert::assert_false(vs1.source_eq(&*v3));

    assert_source_equals(&dvs::constant(5.0), &dvs::constant(5.0));
    assert_equals(
        &dvs::constant(5.0).source_hash(),
        &dvs::constant(5.0).source_hash(),
    );
    Assert::assert_false(dvs::constant(5.0).source_eq(&*dvs::constant(6.0)));

    assert_source_equals(&dvs::scores(), &dvs::scores());
    Assert::assert_false(dvs::constant(5.0).source_eq(&*dvs::scores()));
    Ok(())
}

fn test_simple_field_sortables() -> Result<()> {
    let n = LuceneTestCase::at_least(4);
    for _ in 0..n {
        let sort = random_sort()?;
        check_sorts(match_all(), sort.clone())?;
        check_sorts(Arc::new(TermQuery::new(term("english", "one"))), sort)?;
    }
    Ok(())
}

/// `Sort randomSort()`.
fn random_sort() -> Result<Sort> {
    let reversed = random().next_boolean();
    let mut fields = vec![
        sort_field("int", SortFieldType::Int, reversed)?,
        sort_field("long", SortFieldType::Long, reversed)?,
        sort_field("float", SortFieldType::Float, reversed)?,
        sort_field("double", SortFieldType::Double, reversed)?,
        SortField::new(Some("score".to_string()), SortFieldType::Score)?,
    ];
    shuffle(&mut fields, &*random());
    let num_sorts = TestUtil::next_int(&*random(), 1, fields.len() as i32);
    fields.truncate(num_sorts as usize);
    Sort::new_fields(fields)
}

/// Take a Sort, and replace any field sorts with Sortables
fn convert_sort_to_sortable(sort: &Sort) -> Result<Sort> {
    let original = sort.fields();
    let mut mutated = Vec::with_capacity(original.len());
    for s in original {
        if random().next_int_bound(3) > 0 {
            let reverse = s.field_type() == SortFieldType::Score || s.reverse();
            let field = s.field().unwrap_or_default();
            mutated.push(match s.field_type() {
                SortFieldType::Int => get_sort_field(dvs::from_int_field(field), reverse)?,
                SortFieldType::Long => get_sort_field(dvs::from_long_field(field), reverse)?,
                SortFieldType::Float => get_sort_field(dvs::from_float_field(field), reverse)?,
                SortFieldType::Double => get_sort_field(dvs::from_double_field(field), reverse)?,
                SortFieldType::Score => get_sort_field(dvs::scores(), reverse)?,
                _ => s.clone(),
            });
        } else {
            mutated.push(s.clone());
        }
    }

    Sort::new_fields(mutated)
}

fn check_sorts(query: Arc<dyn Query>, sort: Sort) -> Result<()> {
    let searcher = searcher();
    let size = TestUtil::next_int(&*random(), 1, searcher.get_index_reader().max_doc() / 5);
    let expected = searcher.search_with_sort_and_scores(
        Arc::clone(&query),
        size,
        &sort,
        random().next_boolean(),
    )?;
    let mutated_sort = convert_sort_to_sortable(&sort)?;
    let actual = searcher.search_with_sort_and_scores(
        Arc::clone(&query),
        size,
        &mutated_sort,
        random().next_boolean(),
    )?;

    CheckHits::check_equal(
        &*query,
        &score_docs_of(&expected.score_docs),
        &score_docs_of(&actual.score_docs),
    );

    if i64::from(size) < actual.total_hits.value() {
        let expected = searcher.search_after_with_sort(
            Some(expected.score_docs[(size - 1) as usize].clone()),
            Arc::clone(&query),
            size,
            &sort,
        )?;
        let actual = searcher.search_after_with_sort(
            Some(actual.score_docs[(size - 1) as usize].clone()),
            Arc::clone(&query),
            size,
            &mutated_sort,
        )?;
        CheckHits::check_equal(
            &*query,
            &score_docs_of(&expected.score_docs),
            &score_docs_of(&actual.score_docs),
        );
    }
    Ok(())
}

/// `static final Query[] testQueries`.
fn test_queries() -> Result<Vec<Arc<dyn Query>>> {
    let mut b = BooleanQueryBuilder::new();
    b.add(
        Arc::new(TermQuery::new(term("english", "one"))),
        Occur::MUST,
    )?;
    b.add(
        Arc::new(TermQuery::new(term("english", "two"))),
        Occur::MUST,
    )?;
    Ok(vec![
        match_all(),
        Arc::new(TermQuery::new(term("oddeven", "odd"))),
        Arc::new(b.build()),
    ])
}

fn test_explanations() -> Result<()> {
    for q in test_queries()? {
        test_explanations_with(
            &q,
            dvs::from_query(Arc::new(TermQuery::new(term("english", "one")))),
        )?;
        test_explanations_with(&q, dvs::from_int_field("int"))?;
        test_explanations_with(&q, dvs::from_long_field("long"))?;
        test_explanations_with(&q, dvs::from_float_field("float"))?;
        test_explanations_with(&q, dvs::from_double_field("double"))?;
        test_explanations_with(&q, dvs::from_double_field("onefield"))?;
        test_explanations_with(&q, dvs::constant(5.45))?;
    }
    Ok(())
}

/// The anonymous `SimpleCollector` of `testExplanations`.
struct ExplanationsCollector {
    searcher: Arc<IndexSearcher>,
    q: Arc<dyn Query>,
    vs_needs_scores: bool,
    rewritten: Arc<dyn DoubleValuesSource>,
    /// `ctx`, by ordinal (see the module documentation).
    ctx: usize,
}

impl ExplanationsCollector {
    fn ctx(&self) -> Arc<LeafReaderContext> {
        Arc::clone(&self.searcher.get_leaf_contexts()[self.ctx])
    }
}

impl LeafCollector for ExplanationsCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let ctx = self.ctx();
        let score_expl = self
            .searcher
            .explain(Arc::clone(&self.q), ctx.doc_base() + doc)?;
        let (exact, value) = {
            let mut v = Arc::clone(&self.rewritten)
                .get_values(&ctx, Some(Box::new(from_scorer(scorer))))?;
            if v.advance_exact(doc)? {
                (true, v.double_value()?)
            } else {
                (false, 0.0)
            }
        };
        if exact {
            CheckHits::verify_explanation(
                "",
                doc,
                value as f32,
                true,
                &Arc::clone(&self.rewritten).explain(&ctx, doc, &score_expl)?,
            );
        } else {
            Assert::assert_false(
                Arc::clone(&self.rewritten)
                    .explain(&ctx, doc, &score_expl)?
                    .is_match(),
            );
        }
        Ok(())
    }
}

impl SimpleCollectorImpl for ExplanationsCollector {
    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<()> {
        self.ctx = context.ord() as usize;
        Ok(())
    }

    fn score_mode(&self) -> ScoreMode {
        if self.vs_needs_scores {
            ScoreMode::COMPLETE
        } else {
            ScoreMode::COMPLETE_NO_SCORES
        }
    }
}

/// The anonymous `CollectorManager<SimpleCollector, Void>` of
/// `testExplanations`.
struct ExplanationsManager {
    searcher: Arc<IndexSearcher>,
    q: Arc<dyn Query>,
    vs: Arc<dyn DoubleValuesSource>,
    rewritten: Arc<dyn DoubleValuesSource>,
}

impl CollectorManager for ExplanationsManager {
    type Collector = SimpleCollector<ExplanationsCollector>;
    type Output = ();

    fn new_collector(&self) -> Result<Self::Collector> {
        Ok(SimpleCollector::new(ExplanationsCollector {
            searcher: Arc::clone(&self.searcher),
            q: Arc::clone(&self.q),
            vs_needs_scores: self.vs.needs_scores(),
            rewritten: Arc::clone(&self.rewritten),
            ctx: 0,
        }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<()> {
        Ok(())
    }
}

/// `private void testExplanations(Query q, DoubleValuesSource vs)`.
fn test_explanations_with(q: &Arc<dyn Query>, vs: Arc<dyn DoubleValuesSource>) -> Result<()> {
    let searcher = searcher();
    let rewritten = Arc::clone(&vs).rewrite(&searcher)?;
    searcher.search_with_manager(
        Arc::clone(q),
        &ExplanationsManager {
            searcher: Arc::clone(&searcher),
            q: Arc::clone(q),
            vs,
            rewritten,
        },
    )
}

fn test_query_double_values_source() -> Result<()> {
    let iterating_query: Arc<dyn Query> = Arc::new(TermQuery::new(term("english", "two")));
    let mut b = PhraseQueryBuilder::new();
    b.add_at(term("english", "hundred"), 0)?;
    b.add_at(term("english", "one"), 1)?;
    let approximating_query: Arc<dyn Query> = Arc::new(b.build()?);

    do_test_query_double_values_sources(iterating_query)?;
    do_test_query_double_values_sources(approximating_query)?;
    Ok(())
}

fn test_rewrite_same() -> Result<()> {
    let searcher = searcher();
    let double_field = get_sort_field(dvs::constant(1.0), false)?;
    assert_equals(&double_field, &double_field.rewrite(&searcher)?);
    Ok(())
}

/// The anonymous `SimpleCollector` of `doTestQueryDoubleValuesSources`.
struct QueryValuesCollector {
    searcher: Arc<IndexSearcher>,
    vs: Arc<dyn DoubleValuesSource>,
    ctx: usize,
}

impl LeafCollector for QueryValuesCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let ctx = Arc::clone(&self.searcher.get_leaf_contexts()[self.ctx]);
        let value = {
            let mut v =
                Arc::clone(&self.vs).get_values(&ctx, Some(Box::new(from_scorer(scorer))))?;
            Assert::assert_true(v.advance_exact(doc)?);
            v.double_value()?
        };
        assert_equals_delta(f64::from(unchecked(scorer.score())), value, 0.00001);
        Ok(())
    }
}

impl SimpleCollectorImpl for QueryValuesCollector {
    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<()> {
        self.ctx = context.ord() as usize;
        Ok(())
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

struct QueryValuesManager {
    searcher: Arc<IndexSearcher>,
    vs: Arc<dyn DoubleValuesSource>,
}

impl CollectorManager for QueryValuesManager {
    type Collector = SimpleCollector<QueryValuesCollector>;
    type Output = ();

    fn new_collector(&self) -> Result<Self::Collector> {
        Ok(SimpleCollector::new(QueryValuesCollector {
            searcher: Arc::clone(&self.searcher),
            vs: Arc::clone(&self.vs),
            ctx: 0,
        }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<()> {
        Ok(())
    }
}

fn do_test_query_double_values_sources(q: Arc<dyn Query>) -> Result<()> {
    let searcher = searcher();
    let vs = dvs::from_query(Arc::clone(&q)).rewrite(&searcher)?;
    searcher.search_with_manager(
        q,
        &QueryValuesManager {
            searcher: Arc::clone(&searcher),
            vs,
        },
    )
}
