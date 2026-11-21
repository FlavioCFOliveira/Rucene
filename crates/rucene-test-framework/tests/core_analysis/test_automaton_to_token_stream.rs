//! Port of `org.apache.lucene.analysis.TestAutomatonToTokenStream`
//! (`extends BaseTokenStreamTestCase`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::analysis::automaton_conversion::AutomatonToTokenStream;
use rucene::error::Result;
use rucene::util::automaton::automaton::AutomatonBuilder;
use rucene::util::automaton::Automata;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::BaseTokenStreamTestCase;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::lucene_core_shims::t;

/// `new BytesRef(String)`.
fn br(s: &str) -> BytesRef {
    BytesRef::new(s.as_bytes().to_vec())
}

/// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
/// posIncrements, posLengths, finalOffset)`.
fn assert_contents(
    ts: &mut dyn rucene::analysis::TokenStream,
    output: &[&str],
    start_offsets: &[i32],
    end_offsets: &[i32],
    pos_increments: &[i32],
    pos_lengths: &[i32],
    final_offset: i32,
) -> Result<()> {
    BaseTokenStreamTestCase::assert_token_stream_contents_full(
        ts,
        output,
        Some(start_offsets),
        Some(end_offsets),
        None,
        Some(pos_increments),
        Some(pos_lengths),
        Some(final_offset),
        None,
        None,
        true,
        None,
        None,
        None,
    )
}

fn test_single_path() -> Result<()> {
    let accept_strings = vec![br("abc")];

    let flat_path_automaton = Automata::make_string_union(accept_strings)?;
    let mut ts = AutomatonToTokenStream::to_token_stream(&flat_path_automaton)?;
    assert_contents(
        &mut ts,
        &["a", "b", "c"],
        &[0, 1, 2],
        &[1, 2, 3],
        &[1, 1, 1],
        &[1, 1, 1],
        3,
    )
}

fn test_parallel_paths() -> Result<()> {
    let accept_strings = vec![br("123"), br("abc")];

    let flat_path_automaton = Automata::make_string_union(accept_strings)?;
    let mut ts = AutomatonToTokenStream::to_token_stream(&flat_path_automaton)?;
    assert_contents(
        &mut ts,
        &["1", "a", "2", "b", "3", "c"],
        &[0, 0, 1, 1, 2, 2],
        &[1, 1, 2, 2, 3, 3],
        &[1, 0, 1, 0, 1, 0],
        &[1, 1, 1, 1, 1, 1],
        3,
    )
}

fn test_forked_path() -> Result<()> {
    let accept_strings = vec![br("ab3"), br("abc")];

    let flat_path_automaton = Automata::make_string_union(accept_strings)?;
    let mut ts = AutomatonToTokenStream::to_token_stream(&flat_path_automaton)?;
    assert_contents(
        &mut ts,
        &["a", "b", "3", "c"],
        &[0, 1, 2, 2],
        &[1, 2, 3, 3],
        &[1, 1, 1, 0],
        &[1, 1, 1, 1],
        3,
    )
}

fn test_non_deterministic_graph() -> Result<()> {
    let mut builder = AutomatonBuilder::new();
    let start = builder.create_state();
    let middle1 = builder.create_state();
    let middle2 = builder.create_state();
    let accept = builder.create_state();

    builder.add_transition(start, middle1, 'a' as i32);
    builder.add_transition(start, middle2, 'a' as i32);
    builder.add_transition(middle1, accept, 'b' as i32);
    builder.add_transition(middle2, accept, 'c' as i32);
    builder.set_accept(accept, true);

    let nfa = builder.finish();
    let mut ts = AutomatonToTokenStream::to_token_stream(&nfa)?;
    assert_contents(
        &mut ts,
        &["a", "a", "b", "c"],
        &[0, 0, 1, 1],
        &[1, 1, 2, 2],
        &[1, 0, 1, 0],
        &[1, 1, 1, 1],
        2,
    )
}

fn test_graph_with_start_node_cycle() -> Result<()> {
    let mut builder = AutomatonBuilder::new();
    let start = builder.create_state();
    let middle = builder.create_state();
    let accept = builder.create_state();

    builder.add_transition(start, middle, 'a' as i32);
    builder.add_transition(middle, accept, 'b' as i32);
    builder.add_transition(middle, start, '1' as i32);

    builder.set_accept(accept, true);

    let cycle_graph = builder.finish();
    expect_throws("java.lang.IllegalArgumentException", || {
        AutomatonToTokenStream::to_token_stream(&cycle_graph)
    });
    Ok(())
}

fn test_graph_with_non_start_cycle() -> Result<()> {
    let mut builder = AutomatonBuilder::new();
    let start = builder.create_state();
    let middle = builder.create_state();
    let accept = builder.create_state();

    builder.add_transition(start, middle, 'a' as i32);
    builder.add_transition(middle, accept, 'b' as i32);
    builder.add_transition(accept, middle, 'c' as i32);
    builder.set_accept(accept, true);

    let cycle_graph = builder.finish();
    expect_throws("java.lang.IllegalArgumentException", || {
        AutomatonToTokenStream::to_token_stream(&cycle_graph)
    });
    Ok(())
}

/// The class `org.apache.lucene.analysis.TestAutomatonToTokenStream`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.analysis.TestAutomatonToTokenStream")
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(t("testSinglePath", test_single_path))
        .method(t("testParallelPaths", test_parallel_paths))
        .method(t("testForkedPath", test_forked_path))
        .method(t("testNonDeterministicGraph", test_non_deterministic_graph))
        .method(t(
            "testGraphWithStartNodeCycle",
            test_graph_with_start_node_cycle,
        ))
        .method(t(
            "testGraphWithNonStartCycle",
            test_graph_with_non_start_cycle,
        ))
        .build()
}
