//! Port of `lucene/core/src/test/org/apache/lucene/search/TestAutomatonQueryUnicode.java`.
//!
//! Test the automaton query for several unicode corner cases, specifically
//! enumerating strings/indexes containing supplementary characters, and the
//! differences between UTF-8/UTF-32 and UTF-16 binary sort order.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::Query;
use rucene::util::automaton::reg_exp::RegExp;

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down};
use crate::core_search::test_automaton_query::{
    assert_automaton_hits, index_values, tear_down_fixture, AutomatonFixture,
};

/// `TestAutomatonQueryUnicode`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<AutomatonFixture>(
        "org.apache.lucene.search.TestAutomatonQueryUnicode",
        vec![
            set_up(unicode_set_up),
            tear_down(tear_down_fixture),
            ftest("testSortOrder", test_sort_order),
        ],
    )
}

fn unicode_set_up(f: &mut AutomatonFixture) -> Result<()> {
    index_values(
        f,
        "",
        &[
            "\u{29B05}abcdef",
            "\u{29B06}ghijkl",
            // this sorts before the previous two in UTF-8/UTF-32, but after in UTF-16!!!
            "\u{FB94}mnopqr",
            "\u{FB95}stuvwx", // this one too.
            "a\u{FFFC}bc",
            "a\u{FFFD}bc",
            "a\u{FFFE}bc",
            "a\u{FB94}bc",
            "bacadaba",
            "\u{FFFD}",
            "\u{FFFD}\u{29B05}",
            "\u{FFFD}\u{FFFD}",
        ],
    )
}

fn automaton_query_nr_hits(f: &AutomatonFixture, query: Arc<dyn Query>) -> Result<i64> {
    Ok(f.searcher().search(query, 5)?.total_hits.value())
}

/// Test that AutomatonQuery interacts with lucene's sort order correctly.
///
/// This expression matches something either starting with the arabic
/// presentation forms block, or a supplementary character.
fn test_sort_order(f: &mut AutomatonFixture) -> Result<()> {
    let a = RegExp::new("((\u{29B05})|\u{FB94}).*")?.to_automaton()?;
    assert_automaton_hits(f, 2, a, automaton_query_nr_hits)
}
