//! Port of `lucene/core/src/test/org/apache/lucene/search/TestWildcardRandom.java`.
//!
//! Create an index with terms from 000-999. Generates random wildcards
//! according to patterns, and validates the correct number of hits are
//! returned.
//!
//! Java changes the field's value after `doc.add` and adds the document
//! again; each `addDocument` here gets a document rebuilt from a copy of the
//! field (`copy_field`), with no extra random draw.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::Analyzer;
use rucene::document::Store;
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::Term;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::Query;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{doc_of, fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_equals_msg, random, verbose};

/// The instance of `TestWildcardRandom`.
#[derive(Default)]
pub struct WildcardRandomTest {
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    dir: Option<TestDirectory>,
}

/// `TestWildcardRandom`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<WildcardRandomTest>(
        "org.apache.lucene.search.TestWildcardRandom",
        vec![
            set_up(wildcard_random_set_up),
            tear_down(wildcard_random_tear_down),
            ftest("testWildcards", test_wildcards),
        ],
    )
}

/// `setUp()`.
fn wildcard_random_set_up(f: &mut WildcardRandomTest) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_max_buffered_docs(TestUtil::next_int(&*random(), 50, 1000))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;

    let mut field = LuceneTestCase::new_string_field("field", "", Store::NO);

    // `new DecimalFormat("000", new DecimalFormatSymbols(Locale.ROOT))`.
    for i in 0..1000 {
        field.set_string_value(format!("{i:03}"))?;
        writer.add_document(&doc_of(&[&field])?)?;
    }

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    writer.close()?;
    if verbose() {
        println!("TEST: setUp searcher={searcher:?}");
    }
    f.searcher = Some(searcher);
    f.reader = Some(reader);
    f.dir = Some(dir);
    Ok(())
}

/// `private char N()`.
fn n() -> char {
    char::from(0x30 + random().next_int_bound(10) as u8)
}

/// `private String fillPattern(String wildcardPattern)`.
fn fill_pattern(wildcard_pattern: &str) -> String {
    let mut sb = String::new();
    for c in wildcard_pattern.chars() {
        match c {
            'N' => sb.push(n()),
            _ => sb.push(c),
        }
    }
    sb
}

/// `private void assertPatternHits(String pattern, int numHits)`.
fn assert_pattern_hits(f: &WildcardRandomTest, pattern: &str, num_hits: i64) -> Result<()> {
    // TODO: run with different rewrites
    let filled_pattern = fill_pattern(pattern);
    if verbose() {
        println!("TEST: run wildcard pattern={pattern} filled={filled_pattern}");
    }
    let wq: Arc<dyn Query> = Arc::new(WildcardQuery::new(Term::from_text(
        "field",
        &filled_pattern,
    ))?);
    let docs = f.searcher.as_ref().expect("setUp ran").search(wq, 25)?;
    assert_equals_msg(
        Some(&format!("Incorrect hits for pattern: {pattern}")),
        &num_hits,
        &docs.total_hits.value(),
    );
    Ok(())
}

/// `tearDown()`.
fn wildcard_random_tear_down(f: &mut WildcardRandomTest) -> Result<()> {
    f.searcher = None;
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(mut dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestWildcardRandom.testWildcards`.
fn test_wildcards(f: &mut WildcardRandomTest) -> Result<()> {
    let num = LuceneTestCase::at_least(1);
    for _i in 0..num {
        assert_pattern_hits(f, "NNN", 1)?;
        assert_pattern_hits(f, "?NN", 10)?;
        assert_pattern_hits(f, "N?N", 10)?;
        assert_pattern_hits(f, "NN?", 10)?;
    }

    for _i in 0..num {
        assert_pattern_hits(f, "??N", 100)?;
        assert_pattern_hits(f, "N??", 100)?;
        assert_pattern_hits(f, "???", 1000)?;

        assert_pattern_hits(f, "NN*", 10)?;
        assert_pattern_hits(f, "N*", 100)?;
        assert_pattern_hits(f, "*", 1000)?;

        assert_pattern_hits(f, "*NN", 10)?;
        assert_pattern_hits(f, "*N", 100)?;

        assert_pattern_hits(f, "N*N", 10)?;

        // combo of ? and * operators
        assert_pattern_hits(f, "?N*", 100)?;
        assert_pattern_hits(f, "N?*", 100)?;

        assert_pattern_hits(f, "*N?", 100)?;
        assert_pattern_hits(f, "*??", 1000)?;
        assert_pattern_hits(f, "*?N", 100)?;
    }
    Ok(())
}
