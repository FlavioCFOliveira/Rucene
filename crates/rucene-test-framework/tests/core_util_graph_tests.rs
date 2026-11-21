//! Port of `lucene/core/src/test/org/apache/lucene/util/graph/TestGraphTokenStreamFiniteStrings.java`
//! (sprint 9, task #401): `GraphTokenStreamFiniteStrings` tests, run as a
//! suite under `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops, counters and unit `NO_OUTPUT`
// bindings line by line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, PositionIncrementAttribute, PositionLengthAttribute,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;
use rucene::index::terms::Term;
use rucene::util::graph::GraphTokenStreamFiniteStrings;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::canned_token_stream::CannedTokenStream;
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::util::java_exceptions::expect_throws;

use core_support::{assert_array_equals, assert_equals, assert_not_null, class, run, test};

/// `TestGraphTokenStreamFiniteStrings`.
fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testIllegalState", test_illegal_state),
        test("testEmpty", test_empty),
        test("testSingleGraph", test_single_graph),
        test("testSingleGraphWithGap", test_single_graph_with_gap),
        test("testGraphAndGapSameToken", test_graph_and_gap_same_token),
        test(
            "testGraphAndGapSameTokenTerm",
            test_graph_and_gap_same_token_term,
        ),
        test("testStackedGraph", test_stacked_graph),
        test("testStackedGraphWithGap", test_stacked_graph_with_gap),
        test("testStackedGraphWithRepeat", test_stacked_graph_with_repeat),
        test(
            "testGraphWithRegularSynonym",
            test_graph_with_regular_synonym,
        ),
        test("testMultiGraph", test_multi_graph),
        test("testMultipleSidePaths", test_multiple_side_paths),
        test("testSidePathWithGap", test_side_path_with_gap),
        test(
            "testMultipleSidePathsWithGaps",
            test_multiple_side_paths_with_gaps,
        ),
        test(
            "testLongTokenStreamStackOverflowError",
            test_long_token_stream_stack_overflow_error,
        ),
    ];
    class(
        "org.apache.lucene.util.graph.TestGraphTokenStreamFiniteStrings",
        methods,
    )
}

#[test]
fn test_graph_token_stream_finite_strings() {
    run(class_def);
}

/// `token(term, posInc, posLength)`.
fn token(term: &str, pos_inc: i32, pos_length: i32) -> Token {
    let mut t = Token::with_offsets(term, 0, term.chars().count() as i32);
    t.set_position_increment(pos_inc);
    t.set_position_length(pos_length);
    t
}

/// `new Term("field", text)`.
fn term(text: &str) -> Term {
    Term::new("field", BytesRef::from_char_sequence(text))
}

/// `assertTokenStream(ts, terms, increments)`.
fn assert_token_stream(
    ts: Option<impl TokenStream>,
    terms: &[&str],
    increments: &[i32],
) -> Result<()> {
    // verify no nulls and arrays same length
    assert_not_null(&ts);
    let mut ts = ts.expect("asserted not null");
    assert_equals(&terms.len(), &increments.len());
    let mut offset = 0usize;
    while ts.increment_token()? {
        let source = ts.attribute_source();
        let term_att = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("CharTermAttribute");
        let incr_att = source
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .expect("PositionIncrementAttribute");
        let len_att = source
            .get_attribute_interface::<dyn PositionLengthAttribute>()
            .expect("PositionLengthAttribute");
        // verify term and increment
        assert!(offset < terms.len());
        assert_equals(
            terms[offset],
            term_att.sub_sequence(0, term_att.length()).as_str(),
        );
        assert_equals(&increments[offset], &incr_att.get_position_increment());
        assert_equals(&1, &len_att.get_position_length()); // we always output linear token streams
        offset += 1;
    }

    // make sure we processed all items
    assert_equals(&offset, &terms.len());
    Ok(())
}

/// `testIllegalState()`.
fn test_illegal_state() -> Result<()> {
    expect_throws("java.lang.IllegalStateException", || -> Result<()> {
        let mut ts = CannedTokenStream::new(vec![token("a", 0, 1), token("b", 1, 1)]);
        GraphTokenStreamFiniteStrings::new(&mut ts)?
            .get_finite_strings()
            .has_next()?;
        Ok(())
    });
    Ok(())
}

/// `testEmpty()`.
fn test_empty() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_false(it.has_next()?);
    assert_array_equals::<i32>(&[], &graph.articulation_points()?);
    Ok(())
}

/// `testLongTokenStreamStackOverflowError()`.
fn test_long_token_stream_stack_overflow_error() -> Result<()> {
    let mut tokens = vec![
        token("fast", 1, 1),
        token("wi", 1, 1),
        token("wifi", 0, 2),
        token("fi", 1, 1),
    ];

    // Add in too many tokens to get a high depth graph
    for _ in 0..1024 + 1 {
        tokens.push(token("network", 1, 1));
    }

    let mut ts = CannedTokenStream::new(tokens);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;

    // `assertThrows(IllegalArgumentException.class, graph::articulationPoints)`.
    expect_throws("java.lang.IllegalArgumentException", || {
        graph.articulation_points()
    });
    Ok(())
}

/// `testSingleGraph()`.
#[allow(unused_mut, unused_assignments)]
fn test_single_graph() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("fast", 1, 1),
        token("wi", 1, 1),
        token("wifi", 0, 2),
        token("fi", 1, 1),
        token("network", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wi", "fi", "network"], &[1, 1, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wifi", "network"], &[1, 1, 1])?;
    Assert::assert_false(it.has_next()?);
    let mut points = graph.articulation_points()?;
    assert_array_equals(&points, &[1, 3]);
    Assert::assert_false(graph.has_side_path(0));
    it = graph.get_finite_strings_between(0, 1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast"], &[1])?;
    Assert::assert_false(it.has_next()?);
    let mut terms = graph.get_terms_for_field("field", 0);
    assert_array_equals(&terms, &[term("fast")]);
    Assert::assert_true(graph.has_side_path(1));
    it = graph.get_finite_strings_between(1, 3);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wi", "fi"], &[1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wifi"], &[1])?;
    Assert::assert_false(it.has_next()?);
    Assert::assert_false(graph.has_side_path(3));
    it = graph.get_finite_strings_between(3, -1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["network"], &[1])?;
    Assert::assert_false(it.has_next()?);
    terms = graph.get_terms_for_field("field", 3);
    assert_array_equals(&terms, &[term("network")]);
    Ok(())
}

/// `testSingleGraphWithGap()`.
#[allow(unused_mut, unused_assignments)]
fn test_single_graph_with_gap() -> Result<()> {
    // "hey the fast wifi network", where "the" removed
    let mut ts = CannedTokenStream::new(vec![
        token("hey", 1, 1),
        token("fast", 2, 1),
        token("wi", 1, 1),
        token("wifi", 0, 2),
        token("fi", 1, 1),
        token("network", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["hey", "fast", "wi", "fi", "network"],
        &[1, 2, 1, 1, 1],
    )?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["hey", "fast", "wifi", "network"],
        &[1, 2, 1, 1],
    )?;
    Assert::assert_false(it.has_next()?);
    let mut points = graph.articulation_points()?;
    assert_array_equals(&points, &[1, 2, 4]);
    Assert::assert_false(graph.has_side_path(0));
    it = graph.get_finite_strings_between(0, 1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["hey"], &[1])?;
    Assert::assert_false(it.has_next()?);
    let mut terms = graph.get_terms_for_field("field", 0);
    assert_array_equals(&terms, &[term("hey")]);
    Assert::assert_false(graph.has_side_path(1));
    it = graph.get_finite_strings_between(1, 2);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast"], &[2])?;
    Assert::assert_false(it.has_next()?);
    terms = graph.get_terms_for_field("field", 1);
    assert_array_equals(&terms, &[term("fast")]);
    Assert::assert_true(graph.has_side_path(2));
    it = graph.get_finite_strings_between(2, 4);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wi", "fi"], &[1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wifi"], &[1])?;
    Assert::assert_false(it.has_next()?);
    Assert::assert_false(graph.has_side_path(4));
    it = graph.get_finite_strings_between(4, -1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["network"], &[1])?;
    Assert::assert_false(it.has_next()?);
    terms = graph.get_terms_for_field("field", 4);
    assert_array_equals(&terms, &[term("network")]);
    Ok(())
}

/// `testGraphAndGapSameToken()`.
#[allow(unused_mut, unused_assignments)]
fn test_graph_and_gap_same_token() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("fast", 1, 1),
        token("wi", 2, 1),
        token("wifi", 0, 2),
        token("fi", 1, 1),
        token("network", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wi", "fi", "network"], &[1, 2, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wifi", "network"], &[1, 2, 1])?;
    Assert::assert_false(it.has_next()?);
    let mut points = graph.articulation_points()?;
    assert_array_equals(&points, &[1, 3]);
    Assert::assert_false(graph.has_side_path(0));
    it = graph.get_finite_strings_between(0, 1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast"], &[1])?;
    Assert::assert_false(it.has_next()?);
    let mut terms = graph.get_terms_for_field("field", 0);
    assert_array_equals(&terms, &[term("fast")]);
    Assert::assert_true(graph.has_side_path(1));
    it = graph.get_finite_strings_between(1, 3);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wi", "fi"], &[2, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wifi"], &[2])?;
    Assert::assert_false(it.has_next()?);
    Assert::assert_false(graph.has_side_path(3));
    it = graph.get_finite_strings_between(3, -1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["network"], &[1])?;
    Assert::assert_false(it.has_next()?);
    terms = graph.get_terms_for_field("field", 3);
    assert_array_equals(&terms, &[term("network")]);
    Ok(())
}

/// `testGraphAndGapSameTokenTerm()`.
#[allow(unused_mut, unused_assignments)]
fn test_graph_and_gap_same_token_term() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("a", 1, 1),
        token("b", 1, 1),
        token("c", 2, 1),
        token("a", 0, 2),
        token("d", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["a", "b", "c", "d"], &[1, 1, 2, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["a", "b", "a"], &[1, 1, 2])?;
    Assert::assert_false(it.has_next()?);
    let mut points = graph.articulation_points()?;
    assert_array_equals(&points, &[1, 2]);
    Assert::assert_false(graph.has_side_path(0));
    it = graph.get_finite_strings_between(0, 1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["a"], &[1])?;
    Assert::assert_false(it.has_next()?);
    let mut terms = graph.get_terms_for_field("field", 0);
    assert_array_equals(&terms, &[term("a")]);
    Assert::assert_false(graph.has_side_path(1));
    it = graph.get_finite_strings_between(1, 2);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["b"], &[1])?;
    Assert::assert_false(it.has_next()?);
    terms = graph.get_terms_for_field("field", 1);
    assert_array_equals(&terms, &[term("b")]);
    Assert::assert_true(graph.has_side_path(2));
    it = graph.get_finite_strings_between(2, -1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["c", "d"], &[2, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["a"], &[2])?;
    Assert::assert_false(it.has_next()?);
    Ok(())
}

/// `testStackedGraph()`.
#[allow(unused_mut, unused_assignments)]
fn test_stacked_graph() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("fast", 1, 1),
        token("wi", 1, 1),
        token("wifi", 0, 2),
        token("wireless", 0, 2),
        token("fi", 1, 1),
        token("network", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wi", "fi", "network"], &[1, 1, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wifi", "network"], &[1, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wireless", "network"], &[1, 1, 1])?;
    Assert::assert_false(it.has_next()?);
    let mut points = graph.articulation_points()?;
    assert_array_equals(&points, &[1, 3]);
    Assert::assert_false(graph.has_side_path(0));
    it = graph.get_finite_strings_between(0, 1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast"], &[1])?;
    Assert::assert_false(it.has_next()?);
    let mut terms = graph.get_terms_for_field("field", 0);
    assert_array_equals(&terms, &[term("fast")]);
    Assert::assert_true(graph.has_side_path(1));
    it = graph.get_finite_strings_between(1, 3);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wi", "fi"], &[1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wifi"], &[1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wireless"], &[1])?;
    Assert::assert_false(it.has_next()?);
    Assert::assert_false(graph.has_side_path(3));
    it = graph.get_finite_strings_between(3, -1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["network"], &[1])?;
    Assert::assert_false(it.has_next()?);
    terms = graph.get_terms_for_field("field", 3);
    assert_array_equals(&terms, &[term("network")]);
    Ok(())
}

/// `testStackedGraphWithGap()`.
#[allow(unused_mut, unused_assignments)]
fn test_stacked_graph_with_gap() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("fast", 1, 1),
        token("wi", 2, 1),
        token("wifi", 0, 2),
        token("wireless", 0, 2),
        token("fi", 1, 1),
        token("network", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wi", "fi", "network"], &[1, 2, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wifi", "network"], &[1, 2, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wireless", "network"], &[1, 2, 1])?;
    Assert::assert_false(it.has_next()?);
    let mut points = graph.articulation_points()?;
    assert_array_equals(&points, &[1, 3]);
    Assert::assert_false(graph.has_side_path(0));
    it = graph.get_finite_strings_between(0, 1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast"], &[1])?;
    Assert::assert_false(it.has_next()?);
    let mut terms = graph.get_terms_for_field("field", 0);
    assert_array_equals(&terms, &[term("fast")]);
    Assert::assert_true(graph.has_side_path(1));
    it = graph.get_finite_strings_between(1, 3);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wi", "fi"], &[2, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wifi"], &[2])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wireless"], &[2])?;
    Assert::assert_false(it.has_next()?);
    Assert::assert_false(graph.has_side_path(3));
    it = graph.get_finite_strings_between(3, -1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["network"], &[1])?;
    Assert::assert_false(it.has_next()?);
    terms = graph.get_terms_for_field("field", 3);
    assert_array_equals(&terms, &[term("network")]);
    Ok(())
}

/// `testStackedGraphWithRepeat()`.
#[allow(unused_mut, unused_assignments)]
fn test_stacked_graph_with_repeat() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("ny", 1, 4),
        token("new", 0, 1),
        token("new", 0, 3),
        token("york", 1, 1),
        token("city", 1, 2),
        token("york", 1, 1),
        token("is", 1, 1),
        token("great", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["ny", "is", "great"], &[1, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["new", "york", "city", "is", "great"],
        &[1, 1, 1, 1, 1],
    )?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["new", "york", "is", "great"], &[1, 1, 1, 1])?;
    Assert::assert_false(it.has_next()?);
    let mut points = graph.articulation_points()?;
    assert_array_equals(&points, &[4, 5]);
    Assert::assert_true(graph.has_side_path(0));
    it = graph.get_finite_strings_between(0, 4);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["ny"], &[1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["new", "york", "city"], &[1, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["new", "york"], &[1, 1])?;
    Assert::assert_false(it.has_next()?);
    Assert::assert_false(graph.has_side_path(4));
    it = graph.get_finite_strings_between(4, 5);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["is"], &[1])?;
    Assert::assert_false(it.has_next()?);
    let mut terms = graph.get_terms_for_field("field", 4);
    assert_array_equals(&terms, &[term("is")]);
    Assert::assert_false(graph.has_side_path(5));
    it = graph.get_finite_strings_between(5, -1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["great"], &[1])?;
    Assert::assert_false(it.has_next()?);
    terms = graph.get_terms_for_field("field", 5);
    assert_array_equals(&terms, &[term("great")]);
    Ok(())
}

/// `testGraphWithRegularSynonym()`.
#[allow(unused_mut, unused_assignments)]
fn test_graph_with_regular_synonym() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("fast", 1, 1),
        token("speedy", 0, 1),
        token("wi", 1, 1),
        token("wifi", 0, 2),
        token("fi", 1, 1),
        token("network", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wi", "fi", "network"], &[1, 1, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wifi", "network"], &[1, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["speedy", "wi", "fi", "network"],
        &[1, 1, 1, 1],
    )?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["speedy", "wifi", "network"], &[1, 1, 1])?;
    Assert::assert_false(it.has_next()?);
    let mut points = graph.articulation_points()?;
    assert_array_equals(&points, &[1, 3]);
    Assert::assert_false(graph.has_side_path(0));
    it = graph.get_finite_strings_between(0, 1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast"], &[1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["speedy"], &[1])?;
    Assert::assert_false(it.has_next()?);
    let mut terms = graph.get_terms_for_field("field", 0);
    assert_array_equals(&terms, &[term("fast"), term("speedy")]);
    Assert::assert_true(graph.has_side_path(1));
    it = graph.get_finite_strings_between(1, 3);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wi", "fi"], &[1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wifi"], &[1])?;
    Assert::assert_false(it.has_next()?);
    Assert::assert_false(graph.has_side_path(3));
    it = graph.get_finite_strings_between(3, -1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["network"], &[1])?;
    Assert::assert_false(it.has_next()?);
    terms = graph.get_terms_for_field("field", 3);
    assert_array_equals(&terms, &[term("network")]);
    Ok(())
}

/// `testMultiGraph()`.
#[allow(unused_mut, unused_assignments)]
fn test_multi_graph() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("turbo", 1, 1),
        token("fast", 0, 2),
        token("charged", 1, 1),
        token("wi", 1, 1),
        token("wifi", 0, 2),
        token("fi", 1, 1),
        token("network", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["turbo", "charged", "wi", "fi", "network"],
        &[1, 1, 1, 1, 1],
    )?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["turbo", "charged", "wifi", "network"],
        &[1, 1, 1, 1],
    )?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wi", "fi", "network"], &[1, 1, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast", "wifi", "network"], &[1, 1, 1])?;
    Assert::assert_false(it.has_next()?);
    let mut points = graph.articulation_points()?;
    assert_array_equals(&points, &[2, 4]);
    Assert::assert_true(graph.has_side_path(0));
    it = graph.get_finite_strings_between(0, 2);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["turbo", "charged"], &[1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["fast"], &[1])?;
    Assert::assert_false(it.has_next()?);
    Assert::assert_true(graph.has_side_path(2));
    it = graph.get_finite_strings_between(2, 4);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wi", "fi"], &[1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["wifi"], &[1])?;
    Assert::assert_false(it.has_next()?);
    Assert::assert_false(graph.has_side_path(4));
    it = graph.get_finite_strings_between(4, -1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["network"], &[1])?;
    Assert::assert_false(it.has_next()?);
    let mut terms = graph.get_terms_for_field("field", 4);
    assert_array_equals(&terms, &[term("network")]);
    Ok(())
}

/// `testMultipleSidePaths()`.
#[allow(unused_mut, unused_assignments)]
fn test_multiple_side_paths() -> Result<()> {
    // 0   1        2    3         4    5  6         7  8
    // the ny:4/new york wifi:5/wi fi:4 [] wifi:2/wi fi network
    let mut ts = CannedTokenStream::new(vec![
        token("the", 1, 1),
        token("ny", 1, 4),
        token("new", 0, 1),
        token("york", 1, 1),
        token("wifi", 1, 5),
        token("wi", 0, 1),
        token("fi", 1, 4),
        token("wifi", 2, 2),
        token("wi", 0, 1),
        token("fi", 1, 1),
        token("network", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["the", "ny", "wifi", "network"], &[1, 1, 2, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["the", "ny", "wi", "fi", "network"],
        &[1, 1, 2, 1, 1],
    )?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["the", "new", "york", "wifi", "network"],
        &[1, 1, 1, 1, 1],
    )?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["the", "new", "york", "wi", "fi", "network"],
        &[1, 1, 1, 1, 1, 1],
    )?;
    Assert::assert_false(it.has_next()?);
    let mut points = graph.articulation_points()?;
    assert_array_equals(&points, &[1, 7]);
    Assert::assert_false(graph.has_side_path(0));
    it = graph.get_finite_strings_between(0, 1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["the"], &[1])?;
    Assert::assert_false(it.has_next()?);
    let mut terms = graph.get_terms_for_field("field", 0);
    assert_array_equals(&terms, &[term("the")]);
    Assert::assert_true(graph.has_side_path(1));
    it = graph.get_finite_strings_between(1, 7);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["ny", "wifi"], &[1, 2])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["ny", "wi", "fi"], &[1, 2, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["new", "york", "wifi"], &[1, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["new", "york", "wi", "fi"], &[1, 1, 1, 1])?;
    Assert::assert_false(it.has_next()?);
    Assert::assert_false(graph.has_side_path(7));
    it = graph.get_finite_strings_between(7, -1);
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["network"], &[1])?;
    Assert::assert_false(it.has_next()?);
    terms = graph.get_terms_for_field("field", 7);
    assert_array_equals(&terms, &[term("network")]);
    Ok(())
}

/// `testSidePathWithGap()`.
#[allow(unused_mut, unused_assignments)]
fn test_side_path_with_gap() -> Result<()> {
    // 0    1               2  3  4             5
    // king alfred:3/alfred [] [] great/awesome ruled
    let mut cts = CannedTokenStream::new(vec![
        token("king", 1, 1),
        token("alfred", 1, 4),
        token("alfred", 0, 1),
        token("great", 3, 1),
        token("awesome", 0, 1),
        token("ruled", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut cts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(it.next()?, &["king", "alfred", "ruled"], &[1, 1, 1])?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["king", "alfred", "great", "ruled"],
        &[1, 1, 3, 1],
    )?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["king", "alfred", "awesome", "ruled"],
        &[1, 1, 3, 1],
    )?;
    Assert::assert_false(it.has_next()?);
    Ok(())
}

/// `testMultipleSidePathsWithGaps()`.
#[allow(unused_mut, unused_assignments)]
fn test_multiple_side_paths_with_gaps() -> Result<()> {
    // king alfred:4/alfred [] [] saxons:3 [] wessex ruled
    let mut cts = CannedTokenStream::new(vec![
        token("king", 1, 1),
        token("alfred", 1, 4),
        token("alfred", 0, 1),
        token("saxons", 3, 3),
        token("wessex", 2, 1),
        token("ruled", 1, 1),
    ]);
    let graph = GraphTokenStreamFiniteStrings::new(&mut cts)?;
    let mut it = graph.get_finite_strings();
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["king", "alfred", "wessex", "ruled"],
        &[1, 1, 2, 1],
    )?;
    Assert::assert_true(it.has_next()?);
    assert_token_stream(
        it.next()?,
        &["king", "alfred", "saxons", "ruled"],
        &[1, 1, 3, 1],
    )?;
    Assert::assert_false(it.has_next()?);
    Ok(())
}
