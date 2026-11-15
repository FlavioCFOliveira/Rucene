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

//! Port of the test class of Lucene 10.5.0
//! `lucene/queryparser/src/test/org/apache/lucene/queryparser/simple/` that
//! rmp task #485 lists, run as a suite by the ported runner:
//! `TestSimpleQueryParser`.

mod qp_support;

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::Query;
use rucene::util::automaton::levenshtein_automata::MAXIMUM_SUPPORTED_DISTANCE;
use rucene_queryparser::simple::simple_query_parser::{
    SimpleQueryParser, SimpleQueryParserOps, AND_OPERATOR, ESCAPE_OPERATOR, FUZZY_OPERATOR,
    NEAR_OPERATOR, NOT_OPERATOR, OR_OPERATOR, PHRASE_OPERATOR, PRECEDENCE_OPERATORS,
    PREFIX_OPERATOR, WHITESPACE_OPERATOR,
};
use rucene_test_framework::analysis::mock_tokenizer::KEYWORD;
use rucene_test_framework::util::test_util::TestUtil;

use qp_support::core_support::{class, random, run, test};
use qp_support::query_parser_test_base::{mock_analyzer, mock_analyzer_default};
use qp_support::search_support::{term, tq};
use qp_support::{assert_query_eq, assert_query_eq_msg, jdk_default_hooks};

#[test]
fn test_simple_query_parser() {
    run(class_def);
}

/// `TestSimpleQueryParser`.
fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    let tests: [(&str, qp_support::core_support::TestFn); 46] = [
        ("testTerm", test_term),
        ("testFuzzy", test_fuzzy),
        ("testPhrase", test_phrase),
        ("testPhraseWithSlop", test_phrase_with_slop),
        ("testPrefix", test_prefix),
        ("testAND", test_and),
        ("testANDPhrase", test_and_phrase),
        ("testANDImplicit", test_and_implicit),
        ("testOR", test_or),
        ("testORImplicit", test_or_implicit),
        ("testORPhrase", test_or_phrase),
        ("testNOT", test_not),
        ("testCrazyPrefixes1", test_crazy_prefixes1),
        ("testCrazyPrefixes2", test_crazy_prefixes2),
        ("testTermInDisguise", test_term_in_disguise),
        ("testGarbageTerm", test_garbage_term),
        ("testGarbageEmpty", test_garbage_empty),
        ("testGarbageAND", test_garbage_and),
        ("testGarbageOR", test_garbage_or),
        ("testGarbageNOT", test_garbage_not),
        ("testGarbagePhrase", test_garbage_phrase),
        ("testGarbageSubquery", test_garbage_subquery),
        ("testCompoundAnd", test_compound_and),
        ("testCompoundOr", test_compound_or),
        ("testComplex00", test_complex00),
        ("testComplex01", test_complex01),
        ("testComplex02", test_complex02),
        ("testComplex03", test_complex03),
        ("testComplex04", test_complex04),
        ("testComplex05", test_complex05),
        ("testComplex06", test_complex06),
        ("testWeightedTerm", test_weighted_term),
        ("testWeightedOR", test_weighted_or),
        ("testDisablePhrase", test_disable_phrase),
        ("testDisablePrefix", test_disable_prefix),
        ("testDisableAND", test_disable_and),
        ("testDisableOR", test_disable_or),
        ("testDisableNOT", test_disable_not),
        ("testDisablePrecedence", test_disable_precedence),
        ("testDisableEscape", test_disable_escape),
        ("testDisableWhitespace", test_disable_whitespace),
        ("testDisableFuzziness", test_disable_fuzziness),
        ("testDisableSlop", test_disable_slop),
        ("testRandomQueries", test_random_queries),
        ("testRandomQueries2", test_random_queries2),
        ("testStarBecomesMatchAll", test_star_becomes_match_all),
    ];
    for (name, body) in tests {
        methods.push(test(name, body));
    }
    class(
        "org.apache.lucene.queryparser.simple.TestSimpleQueryParser",
        methods,
    )
}

/// helper to parse a query with whitespace+lowercase analyzer across
/// "field", with default operator of MUST
fn parse(text: &str) -> Result<Arc<dyn Query>> {
    let analyzer = mock_analyzer_default();
    let mut parser = SimpleQueryParser::new(Some(analyzer), "field");
    parser.set_default_operator(Occur::MUST)?;
    parser.parse(text)
}

/// helper to parse a query with whitespace+lowercase analyzer across
/// "field", with default operator of MUST
fn parse_flags(text: &str, flags: i32) -> Result<Arc<dyn Query>> {
    let analyzer = mock_analyzer_default();
    let mut parser =
        SimpleQueryParser::with_flags(Some(analyzer), vec![("field".to_string(), 1.0)], flags);
    parser.set_default_operator(Occur::MUST)?;
    parser.parse(text)
}

fn bq(clauses: Vec<(Arc<dyn Query>, Occur)>) -> Result<Arc<dyn Query>> {
    let mut b = BooleanQueryBuilder::new();
    for (q, o) in clauses {
        b.add(q, o)?;
    }
    Ok(Arc::new(b.build()))
}

fn phrase(slop: i32, terms: &[&str]) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PhraseQuery::with_slop(slop, "field", terms)?))
}

/// test a simple term
fn test_term() -> Result<()> {
    let expected = tq("field", "foobar");

    assert_query_eq(&*expected, &*parse("foobar")?);
    Ok(())
}

/// test a fuzzy query
fn test_fuzzy() -> Result<()> {
    let regular = tq("field", "foobar");
    let expected = FuzzyQuery::with_max_edits(term("field", "foobar"), 2)?;

    assert_query_eq(&expected, &*parse("foobar~2")?);
    assert_query_eq(&expected, &*parse("foobar~")?);
    assert_query_eq(&*regular, &*parse("foobar~a")?);
    assert_query_eq(&*regular, &*parse("foobar~1a")?);

    let fuzzy: Arc<dyn Query> = Arc::new(FuzzyQuery::with_max_edits(
        term("field", "foo"),
        MAXIMUM_SUPPORTED_DISTANCE,
    )?);
    let expected = bq(vec![
        (fuzzy, Occur::MUST),
        (tq("field", "bar"), Occur::MUST),
    ])?;

    assert_query_eq(
        &*expected,
        &*parse(&format!("foo~{} bar", MAXIMUM_SUPPORTED_DISTANCE + 1))?,
    );
    Ok(())
}

/// test a simple phrase
fn test_phrase() -> Result<()> {
    let expected = phrase(0, &["foo", "bar"])?;

    assert_query_eq(&*expected, &*parse("\"foo bar\"")?);
    Ok(())
}

/// test a simple phrase with various slop settings
fn test_phrase_with_slop() -> Result<()> {
    let expected_with_slop = phrase(2, &["foo", "bar"])?;

    assert_query_eq(&*expected_with_slop, &*parse("\"foo bar\"~2")?);

    let expected_with_multi_digit_slop = phrase(10, &["foo", "bar"])?;

    assert_query_eq(&*expected_with_multi_digit_slop, &*parse("\"foo bar\"~10")?);

    let expected_no_slop = phrase(0, &["foo", "bar"])?;

    assert_query_eq_msg(
        "Ignore trailing tilde with no slop",
        &*expected_no_slop,
        &*parse("\"foo bar\"~")?,
    );
    assert_query_eq_msg(
        "Ignore non-numeric trailing slop",
        &*expected_no_slop,
        &*parse("\"foo bar\"~a")?,
    );
    assert_query_eq_msg(
        "Ignore non-numeric trailing slop",
        &*expected_no_slop,
        &*parse("\"foo bar\"~1a")?,
    );
    assert_query_eq_msg(
        "Ignore negative trailing slop",
        &*expected_no_slop,
        &*parse("\"foo bar\"~-1")?,
    );

    let pq = phrase(12, &["foo", "bar"])?;

    let expected_boolean = bq(vec![(pq, Occur::MUST), (tq("field", "baz"), Occur::MUST)])?;

    assert_query_eq(&*expected_boolean, &*parse("\"foo bar\"~12 baz")?);
    Ok(())
}

/// test a simple prefix
fn test_prefix() -> Result<()> {
    let expected = PrefixQuery::new(term("field", "foobar"))?;

    assert_query_eq(&expected, &*parse("foobar*")?);
    Ok(())
}

/// test some AND'd terms using '+' operator
fn test_and() -> Result<()> {
    let expected = bq(vec![
        (tq("field", "foo"), Occur::MUST),
        (tq("field", "bar"), Occur::MUST),
    ])?;

    assert_query_eq(&*expected, &*parse("foo+bar")?);
    Ok(())
}

/// test some AND'd phrases using '+' operator
fn test_and_phrase() -> Result<()> {
    let phrase1 = phrase(0, &["foo", "bar"])?;
    let phrase2 = phrase(0, &["star", "wars"])?;
    let expected = bq(vec![(phrase1, Occur::MUST), (phrase2, Occur::MUST)])?;

    assert_query_eq(&*expected, &*parse("\"foo bar\"+\"star wars\"")?);
    Ok(())
}

/// test some AND'd terms (just using whitespace)
fn test_and_implicit() -> Result<()> {
    let expected = bq(vec![
        (tq("field", "foo"), Occur::MUST),
        (tq("field", "bar"), Occur::MUST),
    ])?;

    assert_query_eq(&*expected, &*parse("foo bar")?);
    Ok(())
}

/// test some OR'd terms
fn test_or() -> Result<()> {
    let expected = bq(vec![
        (tq("field", "foo"), Occur::SHOULD),
        (tq("field", "bar"), Occur::SHOULD),
    ])?;

    assert_query_eq(&*expected, &*parse("foo|bar")?);
    assert_query_eq(&*expected, &*parse("foo||bar")?);
    Ok(())
}

/// test some OR'd terms (just using whitespace)
fn test_or_implicit() -> Result<()> {
    let expected = bq(vec![
        (tq("field", "foo"), Occur::SHOULD),
        (tq("field", "bar"), Occur::SHOULD),
    ])?;

    let mut parser = SimpleQueryParser::new(Some(mock_analyzer_default()), "field");
    assert_query_eq(&*expected, &*parser.parse("foo bar")?);
    Ok(())
}

/// test some OR'd phrases using '|' operator
fn test_or_phrase() -> Result<()> {
    let phrase1 = phrase(0, &["foo", "bar"])?;
    let phrase2 = phrase(0, &["star", "wars"])?;
    let expected = bq(vec![(phrase1, Occur::SHOULD), (phrase2, Occur::SHOULD)])?;

    assert_query_eq(&*expected, &*parse("\"foo bar\"|\"star wars\"")?);
    Ok(())
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

/// test negated term
fn test_not() -> Result<()> {
    let expected = bq(vec![
        (tq("field", "foo"), Occur::MUST_NOT),
        (match_all(), Occur::SHOULD),
    ])?;

    assert_query_eq(&*expected, &*parse("-foo")?);
    assert_query_eq(&*expected, &*parse("-(foo)")?);
    assert_query_eq(&*expected, &*parse("---foo")?);
    Ok(())
}

/// test crazy prefixes with multiple asterisks
fn test_crazy_prefixes1() -> Result<()> {
    let expected = PrefixQuery::new(term("field", "st*ar"))?;

    assert_query_eq(&expected, &*parse("st*ar*")?);
    Ok(())
}

/// test prefixes with some escaping
fn test_crazy_prefixes2() -> Result<()> {
    let expected = PrefixQuery::new(term("field", "st*ar\\*"))?;

    assert_query_eq(&expected, &*parse("st*ar\\\\**")?);
    Ok(())
}

/// not a prefix query! the prefix operator is escaped
fn test_term_in_disguise() -> Result<()> {
    let expected = tq("field", "st*ar\\*");

    assert_query_eq(&*expected, &*parse("sT*Ar\\\\\\*")?);
    Ok(())
}

// a number of test cases here have garbage/errors in
// the syntax passed in to test that the query can
// still be interpreted as a guess to what the human
// input was trying to be

fn test_garbage_term() -> Result<()> {
    let expected = tq("field", "star");

    for q in [
        "star",
        "star\n",
        "star\r",
        "star\t",
        "star(",
        "star)",
        "star\"",
        "\t \r\n\nstar   \n \r \t ",
        "- + \"\" - star \\",
    ] {
        assert_query_eq(&*expected, &*parse(q)?);
    }
    Ok(())
}

fn test_garbage_empty() -> Result<()> {
    let expected = MatchNoDocsQuery::instance();

    for q in [
        "",
        "  ",
        "  ",
        "\\ ",
        "\\ \\ ",
        "\"\"",
        "\" \"",
        "\" \"|\" \"",
        "(\" \"|\" \")",
        "\" \" \" \"",
        "(\" \" \" \")",
    ] {
        assert_query_eq(&expected, &*parse(q)?);
    }
    Ok(())
}

fn test_garbage_and() -> Result<()> {
    let expected = bq(vec![
        (tq("field", "star"), Occur::MUST),
        (tq("field", "wars"), Occur::MUST),
    ])?;

    for q in [
        "star wars",
        "star+wars",
        "     star     wars   ",
        "     star +    wars   ",
        "  |     star + + |   wars   ",
        "  |     star + + |   wars   \\",
    ] {
        assert_query_eq(&*expected, &*parse(q)?);
    }
    Ok(())
}

fn test_garbage_or() -> Result<()> {
    let expected = bq(vec![
        (tq("field", "star"), Occur::SHOULD),
        (tq("field", "wars"), Occur::SHOULD),
    ])?;

    for q in [
        "star|wars",
        "     star |    wars   ",
        "  |     star | + |   wars   ",
        "  +     star | + +   wars   \\",
    ] {
        assert_query_eq(&*expected, &*parse(q)?);
    }
    Ok(())
}

fn test_garbage_not() -> Result<()> {
    let expected = bq(vec![
        (tq("field", "star"), Occur::MUST_NOT),
        (match_all(), Occur::SHOULD),
    ])?;

    assert_query_eq(&*expected, &*parse("-star")?);
    assert_query_eq(&*expected, &*parse("---star")?);
    assert_query_eq(&*expected, &*parse("- -star -")?);
    Ok(())
}

fn test_garbage_phrase() -> Result<()> {
    let expected = phrase(0, &["star", "wars"])?;

    assert_query_eq(&*expected, &*parse("\"star wars\"")?);
    assert_query_eq(&*expected, &*parse("\"star wars\\ \"")?);
    assert_query_eq(&*expected, &*parse("\"\" | \"star wars\"")?);
    assert_query_eq(
        &*expected,
        &*parse("          \"star wars\"        \"\"\\")?,
    );
    Ok(())
}

fn test_garbage_subquery() -> Result<()> {
    let expected = tq("field", "star");

    assert_query_eq(&*expected, &*parse("(star)")?);
    assert_query_eq(&*expected, &*parse("(star))")?);
    assert_query_eq(&*expected, &*parse("((star)")?);
    assert_query_eq(&*expected, &*parse("     -()(star)        \n\n\r     ")?);
    assert_query_eq(
        &*expected,
        &*parse("| + - ( + - |      star    \n      ) \n")?,
    );
    Ok(())
}

fn test_compound_and() -> Result<()> {
    let expected = bq(vec![
        (tq("field", "star"), Occur::MUST),
        (tq("field", "wars"), Occur::MUST),
        (tq("field", "empire"), Occur::MUST),
    ])?;

    assert_query_eq(&*expected, &*parse("star wars empire")?);
    assert_query_eq(&*expected, &*parse("star+wars + empire")?);
    assert_query_eq(&*expected, &*parse(" | --star wars empire \n\\")?);
    Ok(())
}

fn test_compound_or() -> Result<()> {
    let expected = bq(vec![
        (tq("field", "star"), Occur::SHOULD),
        (tq("field", "wars"), Occur::SHOULD),
        (tq("field", "empire"), Occur::SHOULD),
    ])?;

    assert_query_eq(&*expected, &*parse("star|wars|empire")?);
    assert_query_eq(&*expected, &*parse("star|wars | empire")?);
    assert_query_eq(&*expected, &*parse(" | --star|wars|empire \n\\")?);
    Ok(())
}

fn test_complex00() -> Result<()> {
    let inner = bq(vec![
        (tq("field", "star"), Occur::SHOULD),
        (tq("field", "wars"), Occur::SHOULD),
    ])?;
    let expected = bq(vec![
        (inner, Occur::MUST),
        (tq("field", "empire"), Occur::MUST),
    ])?;

    assert_query_eq(&*expected, &*parse("star|wars empire")?);
    assert_query_eq(&*expected, &*parse("star|wars + empire")?);
    assert_query_eq(&*expected, &*parse("star| + wars + ----empire |")?);
    Ok(())
}

fn test_complex01() -> Result<()> {
    let inner = bq(vec![
        (tq("field", "star"), Occur::MUST),
        (tq("field", "wars"), Occur::MUST),
    ])?;
    let expected = bq(vec![
        (inner, Occur::SHOULD),
        (tq("field", "empire"), Occur::SHOULD),
    ])?;

    assert_query_eq(&*expected, &*parse("star wars | empire")?);
    assert_query_eq(&*expected, &*parse("star + wars|empire")?);
    assert_query_eq(&*expected, &*parse("star + | wars | ----empire +")?);
    Ok(())
}

fn test_complex02() -> Result<()> {
    let inner = bq(vec![
        (tq("field", "star"), Occur::MUST),
        (tq("field", "wars"), Occur::MUST),
    ])?;
    let expected = bq(vec![
        (inner, Occur::SHOULD),
        (tq("field", "empire"), Occur::SHOULD),
        (tq("field", "strikes"), Occur::SHOULD),
    ])?;

    assert_query_eq(&*expected, &*parse("star wars | empire | strikes")?);
    assert_query_eq(&*expected, &*parse("star + wars|empire | strikes")?);
    assert_query_eq(
        &*expected,
        &*parse("star + | wars | ----empire | + --strikes \\")?,
    );
    Ok(())
}

fn test_complex03() -> Result<()> {
    let inner2 = bq(vec![
        (tq("field", "star"), Occur::MUST),
        (tq("field", "wars"), Occur::MUST),
    ])?;
    let inner = bq(vec![
        (inner2, Occur::SHOULD),
        (tq("field", "empire"), Occur::SHOULD),
        (tq("field", "strikes"), Occur::SHOULD),
    ])?;
    let expected = bq(vec![
        (inner, Occur::MUST),
        (tq("field", "back"), Occur::MUST),
    ])?;

    assert_query_eq(&*expected, &*parse("star wars | empire | strikes back")?);
    assert_query_eq(&*expected, &*parse("star + wars|empire | strikes + back")?);
    assert_query_eq(
        &*expected,
        &*parse("star + | wars | ----empire | + --strikes + | --back \\")?,
    );
    Ok(())
}

fn test_complex04() -> Result<()> {
    let inner = bq(vec![
        (tq("field", "star"), Occur::MUST),
        (tq("field", "wars"), Occur::MUST),
    ])?;
    let inner2 = bq(vec![
        (tq("field", "strikes"), Occur::MUST),
        (tq("field", "back"), Occur::MUST),
    ])?;
    let expected = bq(vec![
        (inner, Occur::SHOULD),
        (tq("field", "empire"), Occur::SHOULD),
        (inner2, Occur::SHOULD),
    ])?;

    assert_query_eq(
        &*expected,
        &*parse("(star wars) | empire | (strikes back)")?,
    );
    assert_query_eq(
        &*expected,
        &*parse("(star + wars) |empire | (strikes + back)")?,
    );
    assert_query_eq(
        &*expected,
        &*parse("(star + | wars |) | ----empire | + --(strikes + | --back) \\")?,
    );
    Ok(())
}

fn test_complex05() -> Result<()> {
    let inner1 = bq(vec![
        (tq("field", "star"), Occur::MUST),
        (tq("field", "wars"), Occur::MUST),
    ])?;

    let inner4 = bq(vec![
        (tq("field", "jarjar"), Occur::MUST_NOT),
        (match_all(), Occur::SHOULD),
    ])?;

    let inner3 = bq(vec![
        (tq("field", "strikes"), Occur::MUST),
        (tq("field", "back"), Occur::MUST),
        (inner4, Occur::MUST),
    ])?;

    let inner2 = bq(vec![
        (tq("field", "empire"), Occur::SHOULD),
        (inner3, Occur::SHOULD),
    ])?;

    let expected = bq(vec![(inner1, Occur::SHOULD), (inner2, Occur::SHOULD)])?;

    assert_query_eq(
        &*expected,
        &*parse("(star wars) | (empire | (strikes back -jarjar))")?,
    );
    assert_query_eq(
        &*expected,
        &*parse("(star + wars) |(empire | (strikes + back -jarjar) () )")?,
    );
    assert_query_eq(
        &*expected,
        &*parse("(star + | wars |) | --(--empire | + --(strikes + | --back + -jarjar) \"\" ) \"")?,
    );
    Ok(())
}

fn test_complex06() -> Result<()> {
    let inner3 = bq(vec![
        (tq("field", "empire"), Occur::SHOULD),
        (tq("field", "strikes"), Occur::SHOULD),
    ])?;
    let inner2 = bq(vec![
        (inner3, Occur::MUST),
        (tq("field", "back"), Occur::MUST),
        (tq("field", "jar+|jar"), Occur::MUST),
    ])?;
    let inner1 = bq(vec![
        (tq("field", "wars"), Occur::SHOULD),
        (inner2, Occur::SHOULD),
    ])?;

    let expected = bq(vec![
        (tq("field", "star"), Occur::MUST),
        (inner1, Occur::MUST),
    ])?;

    assert_query_eq(
        &*expected,
        &*parse("star (wars | (empire | strikes back jar\\+\\|jar))")?,
    );
    assert_query_eq(
        &*expected,
        &*parse("star + (wars |(empire | strikes + back jar\\+\\|jar) () )")?,
    );
    assert_query_eq(
        &*expected,
        &*parse(
            "star + (| wars | | --(--empire | + --strikes + | --back + jar\\+\\|jar) \"\" ) \"",
        )?,
    );
    Ok(())
}

fn boosted_tq(field: &str, text: &str, boost: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BoostQuery::new(tq(field, text), boost)?))
}

/// test a term with field weights
fn test_weighted_term() -> Result<()> {
    // `new LinkedHashMap<>()`: insertion order.
    let weights = vec![
        ("field0".to_string(), 5.0f32),
        ("field1".to_string(), 10.0f32),
    ];

    let expected = bq(vec![
        (boosted_tq("field0", "foo", 5.0)?, Occur::SHOULD),
        (boosted_tq("field1", "foo", 10.0)?, Occur::SHOULD),
    ])?;

    let analyzer = mock_analyzer_default();
    let mut parser = SimpleQueryParser::with_weights(Some(analyzer), weights);
    assert_query_eq(&*expected, &*parser.parse("foo")?);
    Ok(())
}

/// test a more complex query with field weights
// The Java locals are named `foo` and `bar`.
#[allow(clippy::disallowed_names)]
fn test_weighted_or() -> Result<()> {
    // `new LinkedHashMap<>()`: insertion order.
    let weights = vec![
        ("field0".to_string(), 5.0f32),
        ("field1".to_string(), 10.0f32),
    ];

    let foo = bq(vec![
        (boosted_tq("field0", "foo", 5.0)?, Occur::SHOULD),
        (boosted_tq("field1", "foo", 10.0)?, Occur::SHOULD),
    ])?;
    let bar = bq(vec![
        (boosted_tq("field0", "bar", 5.0)?, Occur::SHOULD),
        (boosted_tq("field1", "bar", 10.0)?, Occur::SHOULD),
    ])?;
    let expected = bq(vec![(foo, Occur::SHOULD), (bar, Occur::SHOULD)])?;

    let analyzer = mock_analyzer_default();
    let mut parser = SimpleQueryParser::with_weights(Some(analyzer), weights);
    assert_query_eq(&*expected, &*parser.parse("foo|bar")?);
    Ok(())
}

/// helper to parse a query with keyword analyzer across "field"
fn parse_keyword(text: &str, flags: i32) -> Result<Arc<dyn Query>> {
    let analyzer = mock_analyzer(&KEYWORD, false);
    let mut parser =
        SimpleQueryParser::with_flags(Some(analyzer), vec![("field".to_string(), 1.0)], flags);
    parser.parse(text)
}

/// test the ability to enable/disable phrase operator
fn test_disable_phrase() -> Result<()> {
    let expected = tq("field", "\"test\"");
    assert_query_eq(&*expected, &*parse_keyword("\"test\"", !PHRASE_OPERATOR)?);
    Ok(())
}

/// test the ability to enable/disable prefix operator
fn test_disable_prefix() -> Result<()> {
    let expected = tq("field", "test*");
    assert_query_eq(&*expected, &*parse_keyword("test*", !PREFIX_OPERATOR)?);
    Ok(())
}

/// test the ability to enable/disable AND operator
fn test_disable_and() -> Result<()> {
    let expected = tq("field", "foo+bar");
    assert_query_eq(&*expected, &*parse_keyword("foo+bar", !AND_OPERATOR)?);
    let expected = tq("field", "+foo+bar");
    assert_query_eq(&*expected, &*parse_keyword("+foo+bar", !AND_OPERATOR)?);
    Ok(())
}

/// test the ability to enable/disable OR operator
fn test_disable_or() -> Result<()> {
    let expected = tq("field", "foo|bar");
    assert_query_eq(&*expected, &*parse_keyword("foo|bar", !OR_OPERATOR)?);
    let expected = tq("field", "|foo|bar");
    assert_query_eq(&*expected, &*parse_keyword("|foo|bar", !OR_OPERATOR)?);
    Ok(())
}

/// test the ability to enable/disable NOT operator
fn test_disable_not() -> Result<()> {
    let expected = tq("field", "-foo");
    assert_query_eq(&*expected, &*parse_keyword("-foo", !NOT_OPERATOR)?);
    Ok(())
}

/// test the ability to enable/disable precedence operators
fn test_disable_precedence() -> Result<()> {
    let expected = tq("field", "(foo)");
    assert_query_eq(&*expected, &*parse_keyword("(foo)", !PRECEDENCE_OPERATORS)?);
    let expected = tq("field", ")foo(");
    assert_query_eq(&*expected, &*parse_keyword(")foo(", !PRECEDENCE_OPERATORS)?);
    Ok(())
}

/// test the ability to enable/disable escape operators
fn test_disable_escape() -> Result<()> {
    let expected = tq("field", "foo\\bar");
    assert_query_eq(&*expected, &*parse_keyword("foo\\bar", !ESCAPE_OPERATOR)?);
    assert_query_eq(&*expected, &*parse_keyword("(foo\\bar)", !ESCAPE_OPERATOR)?);
    assert_query_eq(
        &*expected,
        &*parse_keyword("\"foo\\bar\"", !ESCAPE_OPERATOR)?,
    );
    Ok(())
}

fn test_disable_whitespace() -> Result<()> {
    let expected = tq("field", "foo foo");
    assert_query_eq(
        &*expected,
        &*parse_keyword("foo foo", !WHITESPACE_OPERATOR)?,
    );
    let expected = tq("field", " foo foo\n ");
    assert_query_eq(
        &*expected,
        &*parse_keyword(" foo foo\n ", !WHITESPACE_OPERATOR)?,
    );
    let expected = tq("field", "\t\tfoo foo foo");
    assert_query_eq(
        &*expected,
        &*parse_keyword("\t\tfoo foo foo", !WHITESPACE_OPERATOR)?,
    );
    Ok(())
}

fn test_disable_fuzziness() -> Result<()> {
    let expected = tq("field", "foo~1");
    assert_query_eq(&*expected, &*parse_keyword("foo~1", !FUZZY_OPERATOR)?);
    Ok(())
}

fn test_disable_slop() -> Result<()> {
    let expected_phrase = phrase(0, &["foo", "bar"])?;

    let expected = bq(vec![
        (expected_phrase, Occur::MUST),
        (tq("field", "~2"), Occur::MUST),
    ])?;
    assert_query_eq(&*expected, &*parse_flags("\"foo bar\"~2", !NEAR_OPERATOR)?);
    Ok(())
}

// we aren't supposed to barf on any input...
fn test_random_queries() -> Result<()> {
    for _ in 0..1000 {
        let query = TestUtil::random_unicode_string(&*random());
        parse(&query)?; // no exception
        parse_keyword(&query, TestUtil::next_int(&*random(), 0, 1024))?; // no exception
    }
    Ok(())
}

fn test_random_queries2() -> Result<()> {
    let chars = ['a', '1', '|', '&', ' ', '(', ')', '"', '-', '~'];
    let mut sb = String::new();
    for _ in 0..1000 {
        sb.clear();
        let query_length = random().next_int_bound(20);
        for _ in 0..query_length {
            sb.push(chars[random().next_int_bound(chars.len() as i32) as usize]);
        }
        parse(&sb)?; // no exception
        parse_keyword(&sb, TestUtil::next_int(&*random(), 0, 1024))?; // no exception
    }
    Ok(())
}

fn test_star_becomes_match_all() -> Result<()> {
    let q = parse("*")?;
    assert_query_eq(&*q, &MatchAllDocsQuery::instance());
    let q = parse(" *   ")?;
    assert_query_eq(&*q, &MatchAllDocsQuery::instance());
    Ok(())
}
