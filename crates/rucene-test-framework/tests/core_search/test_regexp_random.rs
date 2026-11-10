//! Port of `lucene/core/src/test/org/apache/lucene/search/TestRegexpRandom.java`.
//!
//! Create an index with terms from 000-999. Generates random regexps according
//! to simple patterns, and validates the correct number of hits are returned.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::{Query, RegexpQuery};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{
    close_dir, close_reader, doc_of, fixture_class, ftest, set_up, tear_down, term,
};
use crate::core_support::{assert_equals_msg, random};

/// The instance fields of `TestRegexpRandom`.
#[derive(Default)]
struct Fixture {
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    dir: Option<TestDirectory>,
}

/// `TestRegexpRandom`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<Fixture>(
        "org.apache.lucene.search.TestRegexpRandom",
        vec![
            set_up(fixture_set_up),
            tear_down(|f: &mut Fixture| {
                drop(f.searcher.take());
                close_reader(f.reader.take())?;
                close_dir(f.dir.take())
            }),
            ftest("testRegexps", test_regexps),
        ],
    )
}

fn fixture_set_up(f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    config.set_max_buffered_docs(TestUtil::next_int(&*random(), 50, 1000))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;

    let custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_omit_norms(true)?;
    let mut field = LuceneTestCase::new_field("field", "", &custom_type);

    // `new DecimalFormat("000", new DecimalFormatSymbols(Locale.ROOT))`
    for i in 0..1000 {
        field.set_string_value(format!("{i:03}"))?;
        writer.add_document(&doc_of(&[&field])?)?;
    }

    let reader = writer.get_reader()?;
    writer.close()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.dir = Some(dir);
    Ok(())
}

#[allow(non_snake_case)]
fn N() -> char {
    char::from(0x30 + random().next_int_bound(10) as u8)
}

fn fill_pattern(wildcard_pattern: &str) -> String {
    let mut sb = String::new();
    for c in wildcard_pattern.chars() {
        match c {
            'N' => sb.push(N()),
            _ => sb.push(c),
        }
    }
    sb
}

fn assert_pattern_hits(f: &Fixture, pattern: &str, num_hits: i64) -> Result<()> {
    let wq: Arc<dyn Query> = Arc::new(RegexpQuery::new(term("field", &fill_pattern(pattern)))?);
    let docs = f.searcher.as_ref().expect("setUp ran").search(wq, 25)?;
    assert_equals_msg(
        Some(&format!("Incorrect hits for pattern: {pattern}")),
        &num_hits,
        &docs.total_hits.value(),
    );
    Ok(())
}

/// Port of `TestRegexpRandom.testRegexps`.
fn test_regexps(f: &mut Fixture) -> Result<()> {
    let num = LuceneTestCase::at_least(1);
    for _ in 0..num {
        assert_pattern_hits(f, "NNN", 1)?;
        assert_pattern_hits(f, ".NN", 10)?;
        assert_pattern_hits(f, "N.N", 10)?;
        assert_pattern_hits(f, "NN.", 10)?;
    }

    for _ in 0..num {
        assert_pattern_hits(f, ".{1,2}N", 100)?;
        assert_pattern_hits(f, "N.{1,2}", 100)?;
        assert_pattern_hits(f, ".{1,3}", 1000)?;

        assert_pattern_hits(f, "NN[3-7]", 5)?;
        assert_pattern_hits(f, "N[2-6][3-7]", 25)?;
        assert_pattern_hits(f, "[1-5][2-6][3-7]", 125)?;
        assert_pattern_hits(f, "[0-4][3-7][4-8]", 125)?;
        assert_pattern_hits(f, "[2-6][0-4]N", 25)?;
        assert_pattern_hits(f, "[2-6]NN", 5)?;

        assert_pattern_hits(f, "NN.*", 10)?;
        assert_pattern_hits(f, "N.*", 100)?;
        assert_pattern_hits(f, ".*", 1000)?;

        assert_pattern_hits(f, ".*NN", 10)?;
        assert_pattern_hits(f, ".*N", 100)?;

        assert_pattern_hits(f, "N.*N", 10)?;

        // combo of ? and * operators
        assert_pattern_hits(f, ".N.*", 100)?;
        assert_pattern_hits(f, "N..*", 100)?;

        assert_pattern_hits(f, ".*N.", 100)?;
        assert_pattern_hits(f, ".*..", 1000)?;
        assert_pattern_hits(f, ".*.N", 100)?;
    }
    Ok(())
}
