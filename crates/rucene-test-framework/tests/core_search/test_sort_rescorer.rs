//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSortRescorer.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexReader, Term};
use rucene::search::double_values_source;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::similarities::ClassicSimilarity;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::{Query, Rescorer, SortRescorer, TopDocs};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_equals, random};

/// The instance of `TestSortRescorer`.
#[derive(Default)]
pub struct SortRescorerTest {
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    dir: Option<TestDirectory>,
}

/// `TestSortRescorer`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<SortRescorerTest>(
        "org.apache.lucene.search.TestSortRescorer",
        vec![
            set_up(sort_rescorer_set_up),
            tear_down(sort_rescorer_tear_down),
            ftest("testBasic", test_basic),
            ftest("testDoubleValuesSourceSort", test_double_values_source_sort),
            ftest("testRandom", test_random),
        ],
    )
}

/// `setUp()`.
fn sort_rescorer_set_up(f: &mut SortRescorerTest) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config();
    config.set_similarity(Some(Arc::new(ClassicSimilarity::new())))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "1",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "body",
        "some contents and more contents",
        Store::NO,
    )));
    doc.add(Box::new(NumericDocValuesField::new("popularity", 5)));
    iw.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "2",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "body",
        "another document with different contents",
        Store::NO,
    )));
    doc.add(Box::new(NumericDocValuesField::new("popularity", 20)));
    iw.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "3",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "body",
        "crappy contents",
        Store::NO,
    )));
    doc.add(Box::new(NumericDocValuesField::new("popularity", 2)));
    iw.add_document(&doc)?;

    let reader = iw.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    // TODO: fix this test to not be so flaky and use newSearcher
    searcher.set_similarity(Arc::new(ClassicSimilarity::new()));
    iw.close()?;
    f.reader = Some(reader);
    f.searcher = Some(searcher);
    f.dir = Some(dir);
    Ok(())
}

/// `tearDown()`.
fn sort_rescorer_tear_down(f: &mut SortRescorerTest) -> Result<()> {
    f.searcher = None;
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// `r.storedFields().document(doc).get("id")`.
fn id_of(r: &Arc<dyn IndexReader>, doc: i32) -> Result<Option<String>> {
    Ok(r.stored_fields()?.document(doc)?.get("id"))
}

/// The assertions on the first-pass hits both sorting tests share.
fn first_pass(searcher: &IndexSearcher, query: &Arc<dyn Query>) -> Result<TopDocs> {
    let r = searcher.get_index_reader();

    // Just first pass query
    let hits = searcher.search(Arc::clone(query), 10)?;
    assert_equals(&3, &hits.total_hits.value());
    assert_equals(&Some("3".to_string()), &id_of(r, hits.score_docs[0].doc)?);
    assert_equals(&Some("1".to_string()), &id_of(r, hits.score_docs[1].doc)?);
    assert_equals(&Some("2".to_string()), &id_of(r, hits.score_docs[2].doc)?);
    Ok(hits)
}

/// The rescoring and explanation assertions both sorting tests share;
/// `sort_field_line` is the explanation line of the sort field.
fn rescore_and_explain(
    searcher: &IndexSearcher,
    query: &Arc<dyn Query>,
    hits: TopDocs,
    sort: Sort,
    sort_field_line: &str,
) -> Result<()> {
    let r = searcher.get_index_reader();
    let rescorer = SortRescorer::new(sort);
    let hits = rescorer.rescore(searcher, &hits, 10)?;
    assert_equals(&3, &hits.total_hits.value());
    assert_equals(&Some("2".to_string()), &id_of(r, hits.score_docs[0].doc)?);
    assert_equals(&Some("1".to_string()), &id_of(r, hits.score_docs[1].doc)?);
    assert_equals(&Some("3".to_string()), &id_of(r, hits.score_docs[2].doc)?);

    let expl = rescorer
        .explain(
            searcher,
            &searcher.explain(Arc::clone(query), hits.score_docs[0].doc)?,
            hits.score_docs[0].doc,
        )?
        .to_string();

    // Confirm the explanation breaks out the individual
    // sort fields:
    Assert::assert_true_msg(&expl, expl.contains(sort_field_line));

    // Confirm the explanation includes first pass details:
    Assert::assert_true(expl.contains("= first pass score"));
    Assert::assert_true(expl.contains("body:contents in"));
    Ok(())
}

/// Port of `TestSortRescorer.testBasic`.
fn test_basic(f: &mut SortRescorerTest) -> Result<()> {
    let searcher = f.searcher.as_ref().expect("setUp ran");

    // create a sort field and sort by it (reverse order)
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("body", "contents")));
    let hits = first_pass(searcher, &query)?;

    // Now, rescore:
    let sort = Sort::new_fields(vec![SortField::new_reverse(
        Some("popularity".to_string()),
        SortFieldType::Int,
        true,
    )?])?;
    rescore_and_explain(
        searcher,
        &query,
        hits,
        sort,
        "= sort field <int: \"popularity\">! value=20",
    )
}

/// Port of `TestSortRescorer.testDoubleValuesSourceSort`.
fn test_double_values_source_sort(f: &mut SortRescorerTest) -> Result<()> {
    let searcher = f.searcher.as_ref().expect("setUp ran");

    // create a sort field and sort by it (reverse order)
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("body", "contents")));
    let hits = first_pass(searcher, &query)?;

    let source = double_values_source::from_long_field("popularity");

    // Now, rescore:
    let sort = Sort::new_fields(vec![double_values_source::get_sort_field(source, true)?])?;
    rescore_and_explain(
        searcher,
        &query,
        hits,
        sort,
        "= sort field <double(popularity)>! value=20.0",
    )
}

/// Port of `TestSortRescorer.testRandom`.
fn test_random(_f: &mut SortRescorerTest) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let num_docs = LuceneTestCase::at_least(1000);
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let mut id_to_num = vec![0i32; num_docs as usize];
    let max_value = TestUtil::next_int(&*random(), 10, 1000000);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        )));
        let num_tokens = TestUtil::next_int(&*random(), 1, 10);
        let mut b = String::new();
        for _j in 0..num_tokens {
            b.push_str("a ");
        }
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            &b,
            Store::NO,
        )));
        id_to_num[i as usize] = random().next_int_bound(max_value);
        doc.add(Box::new(NumericDocValuesField::new(
            "num",
            i64::from(id_to_num[i as usize]),
        )));
        w.add_document(&doc)?;
    }
    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let num_hits = TestUtil::next_int(&*random(), 1, num_docs);
    let reverse = random().next_boolean();

    let hits = s.search(
        Arc::new(TermQuery::new(Term::from_text("field", "a"))),
        num_hits,
    )?;

    let rescorer = SortRescorer::new(Sort::new_fields(vec![SortField::new_reverse(
        Some("num".to_string()),
        SortFieldType::Int,
        reverse,
    )?])?);
    let hits2 = rescorer.rescore(&s, &hits, num_hits)?;

    let mut expected: Vec<i32> = (0..num_hits as usize)
        .map(|i| hits.score_docs[i].doc)
        .collect();

    let reverse_int = if reverse { -1 } else { 1 };

    let stored_fields = r.stored_fields()?;
    let num_of = |doc: i32| -> i32 {
        let id = stored_fields
            .document(doc)
            .unwrap_or_else(|e| panic!("java.lang.RuntimeException: {e}"))
            .get("id")
            .expect("id");
        id_to_num[id.parse::<usize>().expect("Integer.parseInt")]
    };
    expected.sort_by(|&a, &b| {
        let av = num_of(a);
        let bv = num_of(b);
        let c = if av < bv {
            -reverse_int
        } else if bv < av {
            reverse_int
        } else {
            // Tie break by docID
            a - b
        };
        c.cmp(&0)
    });

    let mut fail = false;
    for i in 0..num_hits as usize {
        fail |= expected[i] != hits2.score_docs[i].doc;
    }
    Assert::assert_false(fail);

    drop(stored_fields);
    drop(s);
    r.close()?;
    dir.close()
}
