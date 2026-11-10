//! Port of `lucene/core/src/test/org/apache/lucene/search/TestLongValuesSource.java`.
//!
//! `testRewriteSame`'s `assertSame(longField, longField.rewrite(searcher))`
//! is checked by value: Rucene's `SortField::rewrite` returns a clone (a
//! pre-existing divergence, declared at `src/search/sort.rs`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::search::field_comparator::SortValue;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::long_values_source::{
    self as lvs, get_sort_field, get_sort_field_with_missing, LongValuesSource,
};
use rucene::search::term_query::TermQuery;
use rucene::search::{MatchAllDocsQuery, Query, Sort, SortField, SortFieldType};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::english::English;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{fixture_class, ftest, score_docs_of, set_up, tear_down, term};
use crate::core_support::{assert_equals, random, shuffle};

const LEAST_LONG_VALUE: i64 = 45;

/// The instance of `TestLongValuesSource`.
#[derive(Default)]
pub struct LongValuesSourceTest {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
}

impl LongValuesSourceTest {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }
}

/// `TestLongValuesSource`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<LongValuesSourceTest>(
        "org.apache.lucene.search.TestLongValuesSource",
        vec![
            set_up(long_values_set_up),
            tear_down(long_values_tear_down),
            ftest("testSortMissingZeroDefault", test_sort_missing_zero_default),
            ftest("testSortMissingExplicit", test_sort_missing_explicit),
            ftest(
                "testSimpleFieldEquivalences",
                test_simple_field_equivalences,
            ),
            ftest("testHashCodeAndEquals", test_hash_code_and_equals),
            ftest("testSimpleFieldSortables", test_simple_field_sortables),
            ftest("testRewriteSame", test_rewrite_same),
        ],
    )
}

fn long_values_set_up(f: &mut LongValuesSourceTest) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = TestUtil::next_int(&*random(), 2049, 4000);
    for i in 0..num_docs {
        let mut document = Document::new();
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
        if i == 545 {
            document.add(Box::new(NumericDocValuesField::new(
                "onefield",
                LEAST_LONG_VALUE,
            )));
        }
        iw.add_document(&document)?;
    }
    let reader = iw.get_reader()?;
    iw.close()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.dir = Some(dir);
    Ok(())
}

fn long_values_tear_down(f: &mut LongValuesSourceTest) -> Result<()> {
    drop(f.searcher.take());
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(mut dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

/// `(FieldDoc) results.scoreDocs[0]` and its `fields[0]`.
fn first_field(results: &rucene::search::TopFieldDocs) -> SortValue {
    results.score_docs[0]
        .fields
        .as_ref()
        .expect("NullPointerException")[0]
        .clone()
}

fn test_sort_missing_zero_default(f: &mut LongValuesSourceTest) -> Result<()> {
    // docs w/no value get default missing value = 0

    let onefield = lvs::from_long_field("onefield");
    // sort decreasing
    let results = f.searcher().search_with_sort(
        match_all(),
        1,
        &Sort::new_fields(vec![get_sort_field(Arc::clone(&onefield), true)?])?,
    )?;
    assert_equals(&SortValue::Long(LEAST_LONG_VALUE), &first_field(&results));

    // sort increasing
    let results = f.searcher().search_with_sort(
        match_all(),
        1,
        &Sort::new_fields(vec![get_sort_field(Arc::clone(&onefield), false)?])?,
    )?;
    assert_equals(&SortValue::Long(0), &first_field(&results));
    Ok(())
}

fn test_sort_missing_explicit(f: &mut LongValuesSourceTest) -> Result<()> {
    // docs w/no value get provided missing value

    let onefield = lvs::from_long_field("onefield");

    // sort decreasing, missing last
    let one_field_sort = get_sort_field_with_missing(Arc::clone(&onefield), true, i64::MIN)?;

    let results =
        f.searcher()
            .search_with_sort(match_all(), 1, &Sort::new_fields(vec![one_field_sort])?)?;
    assert_equals(&SortValue::Long(LEAST_LONG_VALUE), &first_field(&results));

    // sort increasing, missing last
    let one_field_sort = get_sort_field_with_missing(Arc::clone(&onefield), false, i64::MAX)?;

    let results =
        f.searcher()
            .search_with_sort(match_all(), 1, &Sort::new_fields(vec![one_field_sort])?)?;
    assert_equals(&SortValue::Long(LEAST_LONG_VALUE), &first_field(&results));
    Ok(())
}

/// `new SortField(field, type, reverse)`.
fn sort_field(field: &str, field_type: SortFieldType, reverse: bool) -> Result<SortField> {
    SortField::new_reverse(Some(field.to_string()), field_type, reverse)
}

fn test_simple_field_equivalences(f: &mut LongValuesSourceTest) -> Result<()> {
    check_sorts(
        f,
        match_all(),
        Sort::new_fields(vec![sort_field(
            "int",
            SortFieldType::Int,
            random().next_boolean(),
        )?])?,
    )?;
    check_sorts(
        f,
        match_all(),
        Sort::new_fields(vec![sort_field(
            "long",
            SortFieldType::Long,
            random().next_boolean(),
        )?])?,
    )?;
    Ok(())
}

fn test_hash_code_and_equals(_f: &mut LongValuesSourceTest) -> Result<()> {
    let vs1 = lvs::from_long_field("long");
    let vs2 = lvs::from_long_field("long");
    if !vs1.source_eq(&*vs2) {
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            vs1.to_source_string(),
            vs2.to_source_string()
        ));
    }
    assert_equals(&vs1.source_hash(), &vs2.source_hash());
    let v3 = lvs::from_long_field("int");
    Assert::assert_false(vs1.source_eq(&*v3));
    Ok(())
}

fn test_simple_field_sortables(f: &mut LongValuesSourceTest) -> Result<()> {
    let n = LuceneTestCase::at_least(4);
    for _i in 0..n {
        let sort = random_sort()?;
        check_sorts(f, match_all(), sort.clone())?;
        check_sorts(f, Arc::new(TermQuery::new(term("english", "one"))), sort)?;
    }
    Ok(())
}

fn test_rewrite_same(f: &mut LongValuesSourceTest) -> Result<()> {
    let long_field = get_sort_field(lvs::constant(1) as Arc<dyn LongValuesSource>, false)?;
    assert_equals(&long_field, &long_field.rewrite(f.searcher())?);
    Ok(())
}

/// `Sort randomSort()`.
fn random_sort() -> Result<Sort> {
    let reversed = random().next_boolean();
    let mut fields = vec![
        sort_field("int", SortFieldType::Int, reversed)?,
        sort_field("long", SortFieldType::Long, reversed)?,
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
                SortFieldType::Int => get_sort_field(lvs::from_int_field(field), reverse)?,
                SortFieldType::Long => get_sort_field(lvs::from_long_field(field), reverse)?,
                _ => s.clone(),
            });
        } else {
            mutated.push(s.clone());
        }
    }

    Sort::new_fields(mutated)
}

fn check_sorts(f: &LongValuesSourceTest, query: Arc<dyn Query>, sort: Sort) -> Result<()> {
    let searcher = f.searcher();
    let size = TestUtil::next_int(&*random(), 1, searcher.get_index_reader().max_doc() / 5);
    let mutated_sort = convert_sort_to_sortable(&sort)?;
    let actual = searcher.search_with_sort_and_scores(
        Arc::clone(&query),
        size,
        &mutated_sort,
        random().next_boolean(),
    )?;
    let expected = searcher.search_with_sort_and_scores(
        Arc::clone(&query),
        size,
        &sort,
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
