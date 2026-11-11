//! Port of `lucene/core/src/test/org/apache/lucene/search/TestRegexpQuery.java`.
//!
//! Some simple regex tests, mostly converted from contrib's TestRegexQuery.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::Term;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::constant_score_blended_rewrite;
use rucene::search::{DefaultProvider, RegexpQuery};
use rucene::util::automaton::automaton_provider::AutomatonProvider;
use rucene::util::automaton::{
    Automata, Automaton, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{
    close_dir, close_reader, fixture_class, ftest, set_up, tear_down, term,
};
use crate::core_support::{assert_equals, random, TOO_COMPLEX_TO_DETERMINIZE};

const FN: &str = "field";

/// The instance fields of `TestRegexpQuery`.
#[derive(Default)]
struct Fixture {
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    directory: Option<TestDirectory>,
}

impl Fixture {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }
}

/// `TestRegexpQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<Fixture>(
        "org.apache.lucene.search.TestRegexpQuery",
        vec![
            set_up(fixture_set_up),
            tear_down(|f: &mut Fixture| {
                drop(f.searcher.take());
                close_reader(f.reader.take())?;
                close_dir(f.directory.take())
            }),
            ftest("testRegex1", test_regex1),
            ftest("testRegex2", test_regex2),
            ftest("testRegex3", test_regex3),
            ftest("testNumericRange", test_numeric_range),
            ftest("testCharacterClasses", test_character_classes),
            ftest("testCaseInsensitive", test_case_insensitive),
            ftest(
                "testRegexNegatedCharacterClass",
                test_regex_negated_character_class,
            ),
            ftest("testCustomProvider", test_custom_provider),
            ftest("testBacktracking", test_backtracking),
            ftest("testSlowCommonSuffix", test_slow_common_suffix),
        ],
    )
}

fn fixture_set_up(f: &mut Fixture) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        FN,
        "the quick brown fox jumps over the lazy ??? dog 493432 49344 [foo] 12.3 \\ ς",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    writer.close()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.directory = Some(directory);
    Ok(())
}

fn new_term(value: &str) -> Term {
    term(FN, value)
}

fn regex_query_nr_hits(f: &Fixture, regex: &str) -> Result<i64> {
    let query = RegexpQuery::new(new_term(regex))?;
    Ok(f.searcher().count(Arc::new(query))? as i64)
}

fn case_insensitive_regex_query_nr_hits(f: &Fixture, regex: &str) -> Result<i64> {
    let query = RegexpQuery::with_everything(
        new_term(regex),
        RegExp::ALL,
        RegExp::ASCII_CASE_INSENSITIVE | RegExp::CASE_INSENSITIVE,
        &DefaultProvider,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        constant_score_blended_rewrite(),
        random().next_boolean(),
    )?;
    Ok(f.searcher().count(Arc::new(query))? as i64)
}

/// Port of `TestRegexpQuery.testRegex1`.
fn test_regex1(f: &mut Fixture) -> Result<()> {
    assert_equals(&1, &regex_query_nr_hits(f, "q.[aeiou]c.*")?);
    Ok(())
}

/// Port of `TestRegexpQuery.testRegex2`.
fn test_regex2(f: &mut Fixture) -> Result<()> {
    assert_equals(&0, &regex_query_nr_hits(f, ".[aeiou]c.*")?);
    Ok(())
}

/// Port of `TestRegexpQuery.testRegex3`.
fn test_regex3(f: &mut Fixture) -> Result<()> {
    assert_equals(&0, &regex_query_nr_hits(f, "q.[aeiou]c")?);
    Ok(())
}

/// Port of `TestRegexpQuery.testNumericRange`.
fn test_numeric_range(f: &mut Fixture) -> Result<()> {
    assert_equals(&1, &regex_query_nr_hits(f, "<420000-600000>")?);
    assert_equals(&0, &regex_query_nr_hits(f, "<493433-600000>")?);
    Ok(())
}

/// Port of `TestRegexpQuery.testCharacterClasses`.
fn test_character_classes(f: &mut Fixture) -> Result<()> {
    for (expected, regex) in [
        (0, "\\d"),
        (1, "\\d*"),
        (1, "\\d{6}"),
        (1, "[a\\d]{6}"),
        (1, "\\d{2,7}"),
        (0, "\\d{4}"),
        (0, "\\dog"),
        (1, "493\\d32"),
        (1, "\\wox"),
        (1, "493\\w32"),
        (1, "\\?\\?\\?"),
        (1, "\\?\\W\\?"),
        (1, "\\?\\S\\?"),
        (1, "\\[foo\\]"),
        (1, "\\[\\w{3}\\]"),
        (0, "\\s.*"),                    // no matches because all whitespace stripped
        (1, "\\S*ck"),                   // matches quick
        (1, "[\\d\\.]{3,10}"),           // matches 12.3
        (1, "\\d{1,3}(\\.(\\d{1,2}))+"), // matches 12.3
        (1, "\\\\"),
        (1, "\\\\.*"),
    ] {
        assert_equals(&expected, &regex_query_nr_hits(f, regex)?);
    }

    let expected = expect_throws("java.lang.IllegalArgumentException", || {
        regex_query_nr_hits(f, "\\p")
    });
    Assert::assert_true(
        expected
            .message()
            .unwrap_or_default()
            .contains("invalid character class"),
    );
    Ok(())
}

/// Port of `TestRegexpQuery.testCaseInsensitive`.
fn test_case_insensitive(f: &mut Fixture) -> Result<()> {
    assert_equals(&0, &regex_query_nr_hits(f, "Quick")?);
    assert_equals(&1, &case_insensitive_regex_query_nr_hits(f, "Quick")?);
    assert_equals(&1, &case_insensitive_regex_query_nr_hits(f, "Σ")?);
    assert_equals(&1, &case_insensitive_regex_query_nr_hits(f, "σ")?);
    Ok(())
}

/// Port of `TestRegexpQuery.testRegexNegatedCharacterClass`.
fn test_regex_negated_character_class(f: &mut Fixture) -> Result<()> {
    assert_equals(&1, &regex_query_nr_hits(f, "[^a-z]")?);
    assert_equals(&1, &regex_query_nr_hits(f, "[^03ad]")?);
    Ok(())
}

/// The anonymous `AutomatonProvider` of `testCustomProvider`.
///
/// Java's `getAutomaton` answers `null` for any other name, on which
/// `RegExp` throws `IllegalArgumentException("'" + name + "' not found")`;
/// the port's provider answers that error itself.
struct QuickBrownProvider {
    // automaton that matches quick or brown
    quick_brown_automaton: Automaton,
}

impl AutomatonProvider for QuickBrownProvider {
    fn get_automaton(&self, name: &str) -> Result<Automaton> {
        if name == "quickBrown" {
            Ok(self.quick_brown_automaton.clone())
        } else {
            Err(LuceneError::illegal_argument(format!("'{name}' not found")))
        }
    }
}

/// Port of `TestRegexpQuery.testCustomProvider`.
fn test_custom_provider(f: &mut Fixture) -> Result<()> {
    let my_provider = QuickBrownProvider {
        quick_brown_automaton: Operations::union(&[
            Automata::make_string("quick"),
            Automata::make_string("brown"),
            Automata::make_string("bob"),
        ]),
    };
    let query = RegexpQuery::with_provider(
        new_term("<quickBrown>"),
        RegExp::ALL,
        &my_provider,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    )?;
    assert_equals(
        &1,
        &f.searcher().search(Arc::new(query), 5)?.total_hits.value(),
    );
    Ok(())
}

/// Test a corner case for backtracking: In this case the term dictionary has
/// 493432 followed by 49344. When backtracking from 49343... to 4934, it's
/// necessary to test that 4934 itself is ok before trying to append more
/// characters.
fn test_backtracking(f: &mut Fixture) -> Result<()> {
    assert_equals(&1, &regex_query_nr_hits(f, "4934[314]")?);
    Ok(())
}

/// Test worst-case for getCommonSuffix optimization
fn test_slow_common_suffix(_f: &mut Fixture) -> Result<()> {
    expect_throws(TOO_COMPLEX_TO_DETERMINIZE, || {
        RegexpQuery::new(term("stringvalue", "(.*a){2000}"))
    });
    Ok(())
}
