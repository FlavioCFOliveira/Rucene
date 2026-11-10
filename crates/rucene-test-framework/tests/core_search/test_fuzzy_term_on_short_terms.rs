//! Port of `lucene/core/src/test/org/apache/lucene/search/TestFuzzyTermOnShortTerms.java`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Test;
use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::{directory_reader, IndexReader};
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::{query_to_string, Query};
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::{tokenizer_components, MockTokenizer};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::term;
use crate::core_support::{assert_equals_msg, class, random, test};

const FIELD: &str = "field";

/// `TestFuzzyTermOnShortTerms`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestFuzzyTermOnShortTerms",
        vec![test("test", test_).annotate(Test::new())],
    )
}

fn fq(text: &str, max_edits: i32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(FuzzyQuery::with_max_edits(
        term(FIELD, text),
        max_edits,
    )?))
}

/// Port of `TestFuzzyTermOnShortTerms.test`.
fn test_() -> Result<()> {
    // proves rule that edit distance between the two terms
    // must be > smaller term for there to be a match
    let a: Arc<dyn Analyzer> = get_analyzer();

    // these work
    count_hits(&a, &["abc"], fq("ab", 1)?, 1)?;
    count_hits(&a, &["ab"], fq("abc", 1)?, 1)?;

    count_hits(&a, &["abcde"], fq("abc", 2)?, 1)?;
    count_hits(&a, &["abc"], fq("abcde", 2)?, 1)?;

    // LUCENE-7439: these now work as well:

    count_hits(&a, &["ab"], fq("a", 1)?, 1)?;
    count_hits(&a, &["a"], fq("ab", 1)?, 1)?;

    count_hits(&a, &["abc"], fq("a", 2)?, 1)?;
    count_hits(&a, &["a"], fq("abc", 2)?, 1)?;

    count_hits(&a, &["abcd"], fq("ab", 2)?, 1)?;
    count_hits(&a, &["ab"], fq("abcd", 2)?, 1)?;
    Ok(())
}

fn count_hits(
    analyzer: &Arc<dyn Analyzer>,
    docs: &[&str],
    q: Arc<dyn Query>,
    expected: i32,
) -> Result<()> {
    let mut d = get_directory(analyzer, docs)?;
    let r = directory_reader::open(d.as_directory())?;
    let s = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    let total_hits = s.count(Arc::clone(&q))?;
    assert_equals_msg(Some(&query_to_string(&*q)), &expected, &total_hits);
    drop(s);
    r.close()?;
    d.close()?;
    Ok(())
}

/// The anonymous `Analyzer` of `getAnalyzer()`.
#[derive(Debug)]
struct SimpleLowerCaseAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for SimpleLowerCaseAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestFuzzyTermOnShortTerms$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = MockTokenizer::new(Arc::clone(&SIMPLE), true)?;
        Ok(tokenizer_components(Rc::new(RefCell::new(tokenizer))))
    }
}

/// `public static Analyzer getAnalyzer()`.
pub fn get_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(SimpleLowerCaseAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    })
}

/// `public static Directory getDirectory(Analyzer analyzer, String[] vals)`.
pub fn get_directory(analyzer: &Arc<dyn Analyzer>, vals: &[&str]) -> Result<TestDirectory> {
    let directory = LuceneTestCase::new_directory();
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(Arc::clone(analyzer));
    config.set_max_buffered_docs(TestUtil::next_int(&*random(), 100, 1000))?;
    config.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;

    for s in vals {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_text_field(
            FIELD,
            s,
            Store::YES,
        )));
        writer.add_document(&d)?;
    }
    writer.close()?;
    Ok(directory)
}
