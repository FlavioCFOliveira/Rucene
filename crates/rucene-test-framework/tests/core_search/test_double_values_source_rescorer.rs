//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDoubleValuesSourceRescorer.java`.
//!
//! The anonymous `DoubleValuesSourceRescorer` subclass is a
//! `DoubleValuesSourceRescorerImpl`; its `combiner_name` is the
//! `getClass()` Java interpolates into the explanation, the anonymous
//! class `TestDoubleValuesSourceRescorer$1`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::IndexReader;
use rucene::search::double_values_source::{self as dvs, DoubleValuesSource};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::rescorer::Rescorer;
use rucene::search::term_query::TermQuery;
use rucene::search::{DoubleValuesSourceRescorer, DoubleValuesSourceRescorerImpl, Query};
use rucene::store::Directory;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{fixture_class, ftest, term};
use crate::core_support::{assert_equals, assert_equals_delta, random};

const ID_FIELD: &str = "id";
const DOC_VAL_FIELD: &str = "docVal";
const DOC_VAL_STORED_FIELD: &str = "storedDocVal";

/// The anonymous `DoubleValuesSourceRescorer` subclass.
struct Combine;

impl DoubleValuesSourceRescorerImpl for Combine {
    fn combine(&self, _first_pass_score: f32, value_present: bool, source_value: f64) -> f32 {
        if value_present {
            source_value as f32
        } else {
            0.0
        }
    }

    fn combiner_name(&self) -> String {
        "class org.apache.lucene.search.TestDoubleValuesSourceRescorer$1".to_string()
    }
}

/// The instance of `TestDoubleValuesSourceRescorer`.
pub struct DoubleValuesSourceRescorerTest {
    #[allow(dead_code)]
    double_values_source: Arc<dyn DoubleValuesSource>,
    rescorer: DoubleValuesSourceRescorer<Combine>,
}

impl Default for DoubleValuesSourceRescorerTest {
    fn default() -> Self {
        let double_values_source = dvs::from_int_field(DOC_VAL_FIELD);
        DoubleValuesSourceRescorerTest {
            rescorer: DoubleValuesSourceRescorer::new(Arc::clone(&double_values_source), Combine),
            double_values_source,
        }
    }
}

/// `TestDoubleValuesSourceRescorer`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<DoubleValuesSourceRescorerTest>(
        "org.apache.lucene.search.TestDoubleValuesSourceRescorer",
        vec![
            ftest("testBasic", test_basic),
            ftest("testSubsetAndIdempotency", test_subset_and_idempotency),
            ftest("testMissingValues", test_missing_values),
        ],
    )
}

const DICTIONARY: &[&str] = &[
    "river", "quick", "brown", "fox", "jumped", "lazy", "fence", "wizard", "of", "a", "an", "the",
    "cookie", "golf", "golden", "tennis", "boy", "plays", "likes", "wants",
];

fn random_sentence() -> String {
    let length = random().next_int_range(3, 10);
    let mut sentence = String::new();
    for _ in 0..length {
        sentence
            .push_str(DICTIONARY[random().next_int_bound(DICTIONARY.len() as i32 - 1) as usize]);
        sentence.push(' ');
    }
    sentence
}

fn publish_docs(
    num_docs: i32,
    field_name: &str,
    index_doc_values: bool,
    dir: Arc<dyn Directory>,
) -> Result<()> {
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir,
        LuceneTestCase::new_index_writer_config(),
    )?;
    for i in 0..num_docs {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_string_field(
            ID_FIELD,
            &i.to_string(),
            Store::YES,
        )));
        d.add(Box::new(LuceneTestCase::new_text_field(
            field_name,
            &random_sentence(),
            Store::NO,
        )));
        if index_doc_values {
            let val = i + 100;
            d.add(Box::new(NumericDocValuesField::new(
                DOC_VAL_FIELD,
                i64::from(val),
            )));
            d.add(Box::new(LuceneTestCase::new_string_field(
                DOC_VAL_STORED_FIELD,
                &val.to_string(),
                Store::YES,
            )));
        }
        w.add_document(&d)?;
    }
    w.close()?;
    Ok(())
}

/// The query of the three tests:
/// `new TermQuery(new Term("title", dictionary.get(random().nextInt(dictionary.size() - 1))))`.
fn random_title_query() -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(
        "title",
        DICTIONARY[random().next_int_bound(DICTIONARY.len() as i32 - 1) as usize],
    )))
}

fn test_basic(f: &mut DoubleValuesSourceRescorerTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    publish_docs(
        random().next_int_range(50, 100),
        "title",
        true,
        dir.as_directory(),
    )?;
    {
        let r = directory_reader::open(dir.as_directory())?;
        let s = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
        let query = random_title_query();
        let query_hits = s.search(Arc::clone(&query), 50)?;
        let rescored_hits = f.rescorer.rescore(&s, &query_hits, 15)?;
        Assert::assert_true(rescored_hits.score_docs.len() <= 15);
        assert_equals(&query_hits.total_hits, &rescored_hits.total_hits);
        for i in 1..rescored_hits.score_docs.len() {
            Assert::assert_true(
                rescored_hits.score_docs[i - 1].score > rescored_hits.score_docs[i].score,
            );
        }
        for hit in &rescored_hits.score_docs {
            assert_equals(
                &s.stored_fields()?
                    .document(hit.doc)?
                    .get(DOC_VAL_STORED_FIELD),
                &Some((hit.score as i32).to_string()),
            );
        }
        let doc = rescored_hits.score_docs[0].doc;
        let e = f
            .rescorer
            .explain(&s, &s.explain(Arc::clone(&query), doc)?, doc)?;
        let msg = e.to_string();
        Assert::assert_true(msg.contains("combined score from firstPass and DoubleValuesSource"));
        Assert::assert_true(
            msg.contains("class org.apache.lucene.search.TestDoubleValuesSourceRescorer"),
        );
        Assert::assert_true(msg.contains("first pass score"));
        Assert::assert_true(msg.contains("value from DoubleValuesSource"));
        drop(s);
        r.close()?;
    }
    dir.close()?;
    Ok(())
}

fn test_subset_and_idempotency(f: &mut DoubleValuesSourceRescorerTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    publish_docs(
        random().next_int_range(60, 200),
        "title",
        true,
        dir.as_directory(),
    )?;
    {
        let r = directory_reader::open(dir.as_directory())?;
        let s = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
        let query = random_title_query();
        let query_hits = s.search(query, 50)?;
        let rescored_hits1 = f.rescorer.rescore(&s, &query_hits, 15)?;

        let hits1_len = rescored_hits1.score_docs.len() as i32;
        let hit2_n = (hits1_len / 2).max(1);
        let rescored_hits2 = f.rescorer.rescore(&s, &query_hits, hit2_n)?;
        assert_equals(&(hit2_n as usize), &rescored_hits2.score_docs.len());
        for i in 0..hit2_n as usize {
            assert_equals(
                &rescored_hits1.score_docs[i].doc,
                &rescored_hits2.score_docs[i].doc,
            );
            assert_equals_delta(
                f64::from(rescored_hits1.score_docs[i].score),
                f64::from(rescored_hits2.score_docs[i].score),
                1e-5,
            );
        }
        drop(s);
        r.close()?;
    }
    dir.close()?;
    Ok(())
}

fn test_missing_values(f: &mut DoubleValuesSourceRescorerTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    publish_docs(
        random().next_int_range(60, 200),
        "title",
        false,
        dir.as_directory(),
    )?;
    {
        let r = directory_reader::open(dir.as_directory())?;
        let s = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
        let query = random_title_query();
        let query_hits = s.search(Arc::clone(&query), 50)?;
        let rescored_hits = f.rescorer.rescore(&s, &query_hits, 15)?;
        Assert::assert_true(rescored_hits.score_docs.len() <= 15);
        assert_equals(&query_hits.total_hits, &rescored_hits.total_hits);
        for i in 0..rescored_hits.score_docs.len() {
            assert_equals_delta(f64::from(rescored_hits.score_docs[i].score), 0.0, 1e-5);
            if i > 0 {
                Assert::assert_true(
                    rescored_hits.score_docs[i - 1].doc < rescored_hits.score_docs[i].doc,
                );
            }
        }
        let doc = rescored_hits.score_docs[0].doc;
        let e = f
            .rescorer
            .explain(&s, &s.explain(Arc::clone(&query), doc)?, doc)?;
        let msg = e.to_string();
        Assert::assert_true(msg.contains("combined score from firstPass and DoubleValuesSource"));
        Assert::assert_true(
            msg.contains("class org.apache.lucene.search.TestDoubleValuesSourceRescorer"),
        );
        Assert::assert_true(msg.contains("first pass score"));
        Assert::assert_true(msg.contains("no value in DoubleValuesSource"));
        drop(s);
        r.close()?;
    }
    dir.close()?;
    Ok(())
}
