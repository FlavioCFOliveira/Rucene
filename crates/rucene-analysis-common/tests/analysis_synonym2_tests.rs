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

//! Port of the remaining test classes of Lucene 10.5.0
//! `lucene/analysis/common/src/test` that rmp task #453 lists, each run as a
//! suite by the ported runner: `core.TestFlattenGraphFilter`,
//! `miscellaneous.TestConcatenateGraphFilter`,
//! `miscellaneous.TestLimitTokenPositionFilter`,
//! `miscellaneous.TestRemoveDuplicatesTokenFilter` (all exercise the synonym
//! filters) and the `synonym.word2vec` classes `TestDl4jModelReader`,
//! `TestWord2VecSynonymFilter`, `TestWord2VecSynonymFilterFactory` and
//! `TestWord2VecSynonymProvider`.
//!
//! Not portable: `TestWord2VecSynonymProvider.getSynonyms_nullToken_shouldThrowException`
//! passes a `null` term; the port's `get_synonyms` takes `&BytesRef`, which
//! cannot be null, so the `IllegalArgumentException` path has no input.

#![allow(deprecated)]

mod support;

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::{Assert, Test};
use rucene::analysis::automaton_conversion::{
    AutomatonToTokenStream, TokenStreamToAutomaton, POS_SEP,
};
use rucene::analysis::stop_filter::{make_stop_set, new_stop_filter};
use rucene::analysis::{Analyzer, CharArraySet, StringCharReader, TokenStream};
use rucene::error::Result;
use rucene::index::KnnVectorValues;
use rucene::util::automaton::{Automata, Automaton, Operations, Transition, TransitionAccessor};
use rucene::util::chars_ref::CharsRef;
use rucene::util::misc::TermAndVector;
use rucene::util::refs::CharsRefBuilder;
use rucene::util::BytesRef;
use rucene_analysis_common::analysis::core::flatten_graph_filter::new_flatten_graph_filter;
use rucene_analysis_common::analysis::core::whitespace_tokenizer::new_whitespace_tokenizer;
use rucene_analysis_common::analysis::miscellaneous::concatenate_graph_filter::{
    BytesRefBuilderTermAttribute, ConcatenateGraphFilter, SEP_LABEL,
};
use rucene_analysis_common::analysis::miscellaneous::limit_token_position_filter::{
    new_limit_token_position_filter, new_limit_token_position_filter_consuming,
};
use rucene_analysis_common::analysis::miscellaneous::remove_duplicates_token_filter::new_remove_duplicates_token_filter;
use rucene_analysis_common::analysis::synonym::synonym_filter::new_synonym_filter;
use rucene_analysis_common::analysis::synonym::synonym_graph_filter::new_synonym_graph_filter;
use rucene_analysis_common::analysis::synonym::synonym_map::Builder as SynonymMapBuilder;
use rucene_analysis_common::analysis::synonym::word2vec::dl4j_model_reader::Dl4jModelReader;
use rucene_analysis_common::analysis::synonym::word2vec::word2vec_model::Word2VecModel;
use rucene_analysis_common::analysis::synonym::word2vec::word2vec_synonym_filter::new_word2vec_synonym_filter;
use rucene_analysis_common::analysis::synonym::word2vec::word2vec_synonym_filter_factory::Word2VecSynonymFilterFactory;
use rucene_analysis_common::analysis::synonym::word2vec::word2vec_synonym_provider::Word2VecSynonymProvider;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::{BaseTokenStreamTestCase, CannedTokenStream};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use support::{
    boxed, btstc_class, concrete_factory, factory_class, iae_message, keyword_tokenizer_analyzer,
    lucene_class, mock_tokenizer_analyzer, run, t, token_filter_factory, tokenizer_analyzer,
};

type B = BaseTokenStreamTestCase;

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// A JUnit 4 `@Test` method (the runner also runs methods not named
/// `test*` when annotated).
fn at_test(name: &str, body: fn() -> Result<()>) -> Method {
    t(name, body).annotate(Test::new())
}

/// An `@Test(expected = IllegalArgumentException.class)` method.
fn t_iae(name: &str, body: fn() -> Result<()>) -> Method {
    t(name, body).annotate(Test::expected("java.lang.IllegalArgumentException"))
}

/// `new BytesRef(s)`.
fn bytes(s: &str) -> BytesRef {
    BytesRef::new(s.as_bytes().to_vec())
}

/// `assertEquals(new BytesRef(expected), actual)`.
fn assert_bytes(expected: &str, actual: &BytesRef) {
    Assert::assert_equals(
        format!("{:?}", expected.as_bytes()),
        format!("{:?}", actual.slice()),
    );
}

/// Java's `String.trim()`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `randomNonEmptyString()`.
fn random_non_empty_string() -> String {
    loop {
        let s = TestUtil::random_unicode_string(&*random());
        let s = java_trim(&s);
        if !s.is_empty() && !s.contains('\u{0000}') {
            return s.to_string();
        }
    }
}

// -----------------------------------------------------------------------------
// TestFlattenGraphFilter
// -----------------------------------------------------------------------------

fn token(term: &str, pos_inc: i32, pos_length: i32, start_offset: i32, end_offset: i32) -> Token {
    Token::with_pos_len(term, pos_inc, start_offset, end_offset, pos_length)
}

/// `new FlattenGraphFilter(new CannedTokenStream(0, finalOffset, tokens))`,
/// asserted with `assertTokenStreamContents(out, output, startOffsets,
/// endOffsets, posIncrements, posLengths, finalOffset)`.
#[allow(clippy::too_many_arguments)]
fn flatten_case(
    final_offset: i32,
    tokens: Vec<Token>,
    output: &[&str],
    start: &[i32],
    end: &[i32],
    pos_inc: &[i32],
    pos_len: &[i32],
) -> Result<()> {
    let input = CannedTokenStream::with_final(0, final_offset, tokens);
    let mut out = new_flatten_graph_filter(boxed(input))?;
    B::assert_token_stream_contents_offsets_positions_final_offset(
        &mut out,
        output,
        start,
        end,
        pos_inc,
        pos_len,
        Some(final_offset),
    )
}

fn fg_test_simple_mock() -> Result<()> {
    let a = mock_tokenizer_analyzer(
        "org.apache.lucene.analysis.core.TestFlattenGraphFilter$1",
        Arc::clone(&SIMPLE),
        true,
        |t| Ok(boxed(new_flatten_graph_filter(t)?)),
    );
    B::assert_analyzes_to_graph(
        &*a,
        "wtf happened",
        &["wtf", "happened"],
        Some(&[0, 4]),
        Some(&[3, 12]),
        None,
        Some(&[1, 1]),
        Some(&[1, 1]),
        true,
    )
}

// Make sure graph is unchanged if it's already flat
fn fg_test_already_flatten() -> Result<()> {
    flatten_case(
        12,
        vec![
            token("wtf", 1, 1, 0, 3),
            token("what", 0, 1, 0, 3),
            token("wow", 0, 1, 0, 3),
            token("the", 1, 1, 0, 3),
            token("that's", 0, 1, 0, 3),
            token("fudge", 1, 1, 0, 3),
            token("funny", 0, 1, 0, 3),
            token("happened", 1, 1, 4, 12),
        ],
        // ... but on output, it's flattened to wtf/what/wow that's/the fudge/funny happened:
        &[
            "wtf", "what", "wow", "the", "that's", "fudge", "funny", "happened",
        ],
        &[0, 0, 0, 0, 0, 0, 0, 4],
        &[3, 3, 3, 3, 3, 3, 3, 12],
        &[1, 0, 0, 1, 0, 1, 0, 1],
        &[1, 1, 1, 1, 1, 1, 1, 1],
    )
}

fn fg_test_wtf1() -> Result<()> {
    // "wow that's funny" and "what the fudge" are separate side paths, in parallel with "wtf", on
    // input:
    flatten_case(
        12,
        vec![
            token("wtf", 1, 5, 0, 3),
            token("what", 0, 1, 0, 3),
            token("wow", 0, 3, 0, 3),
            token("the", 1, 1, 0, 3),
            token("fudge", 1, 3, 0, 3),
            token("that's", 1, 1, 0, 3),
            token("funny", 1, 1, 0, 3),
            token("happened", 1, 1, 4, 12),
        ],
        // ... but on output, it's flattened to wtf/what/wow that's/the fudge/funny happened:
        &[
            "wtf", "what", "wow", "the", "that's", "fudge", "funny", "happened",
        ],
        &[0, 0, 0, 0, 0, 0, 0, 4],
        &[3, 3, 3, 3, 3, 3, 3, 12],
        &[1, 0, 0, 1, 0, 1, 0, 1],
        &[3, 1, 1, 1, 1, 1, 1, 1],
    )
}

/// Same as testWTF1 except the "wtf" token comes out later
fn fg_test_wtf2() -> Result<()> {
    flatten_case(
        12,
        vec![
            token("what", 1, 1, 0, 3),
            token("wow", 0, 3, 0, 3),
            token("wtf", 0, 5, 0, 3),
            token("the", 1, 1, 0, 3),
            token("fudge", 1, 3, 0, 3),
            token("that's", 1, 1, 0, 3),
            token("funny", 1, 1, 0, 3),
            token("happened", 1, 1, 4, 12),
        ],
        &[
            "what", "wow", "wtf", "the", "that's", "fudge", "funny", "happened",
        ],
        &[0, 0, 0, 0, 0, 0, 0, 4],
        &[3, 3, 3, 3, 3, 3, 3, 12],
        &[1, 0, 0, 1, 0, 1, 0, 1],
        &[1, 1, 3, 1, 1, 1, 1, 1],
    )
}

fn fg_test_non_greedy_synonyms() -> Result<()> {
    // This is just "hypothetical" for Lucene today, because SynFilter is
    // greedy: when two syn rules match on overlapping tokens, only one
    // (greedily) wins.  This test pretends all syn matches could match:
    flatten_case(
        20,
        vec![
            token("wizard", 1, 1, 0, 6),
            token("wizard_of_oz", 0, 3, 0, 12),
            token("of", 1, 1, 7, 9),
            token("oz", 1, 1, 10, 12),
            token("oz_screams", 0, 2, 10, 20),
            token("screams", 1, 1, 13, 20),
        ],
        &[
            "wizard",
            "wizard_of_oz",
            "of",
            "oz",
            "oz_screams",
            "screams",
        ],
        &[0, 0, 7, 10, 10, 13],
        &[6, 12, 9, 12, 20, 20],
        &[1, 0, 1, 1, 0, 1],
        &[1, 3, 1, 1, 2, 1],
    )
}

fn fg_test_non_graph() -> Result<()> {
    flatten_case(
        22,
        vec![
            token("hello", 1, 1, 0, 5),
            token("pseudo", 1, 1, 6, 12),
            token("world", 1, 1, 13, 18),
            token("fun", 1, 1, 19, 22),
        ],
        &["hello", "pseudo", "world", "fun"],
        &[0, 6, 13, 19],
        &[5, 12, 18, 22],
        &[1, 1, 1, 1],
        &[1, 1, 1, 1],
    )
}

fn fg_test_simple_hole() -> Result<()> {
    flatten_case(
        13,
        vec![
            token("hello", 1, 1, 0, 5),
            token("hole", 2, 1, 6, 10),
            token("fun", 1, 1, 11, 13),
        ],
        &["hello", "hole", "fun"],
        &[0, 6, 11],
        &[5, 10, 13],
        &[1, 2, 1],
        &[1, 1, 1],
    )
}

fn fg_test_hole_under_syn() -> Result<()> {
    // Tests a StopFilter after SynFilter where a stopword in a syn is removed
    //
    //   wizard of oz -> woz syn, but then "of" becomes a hole
    flatten_case(
        12,
        vec![
            token("wizard", 1, 1, 0, 6),
            token("woz", 0, 3, 0, 12),
            token("oz", 2, 1, 10, 12),
        ],
        &["wizard", "woz", "oz"],
        &[0, 0, 10],
        &[6, 12, 12],
        &[1, 0, 2],
        &[1, 3, 1],
    )
}

fn fg_test_strangely_numbered_nodes() -> Result<()> {
    // Uses only nodes 0, 2, 3, i.e. 1 is just never used (it is not a hole!!)
    flatten_case(
        27,
        vec![
            token("dog", 1, 3, 0, 5),
            token("puppy", 0, 3, 0, 5),
            token("flies", 3, 1, 6, 11),
        ],
        &["dog", "puppy", "flies"],
        &[0, 0, 6],
        &[5, 5, 11],
        &[1, 0, 1],
        &[1, 1, 1],
    )
}

fn fg_test_two_long_parallel_paths() -> Result<()> {
    // "a a a a a a" in parallel with "b b b b b b"
    flatten_case(
        11,
        vec![
            token("a", 1, 1, 0, 1),
            token("b", 0, 2, 0, 1),
            token("a", 1, 2, 2, 3),
            token("b", 1, 2, 2, 3),
            token("a", 1, 2, 4, 5),
            token("b", 1, 2, 4, 5),
            token("a", 1, 2, 6, 7),
            token("b", 1, 2, 6, 7),
            token("a", 1, 2, 8, 9),
            token("b", 1, 2, 8, 9),
            token("a", 1, 2, 10, 11),
            token("b", 1, 2, 10, 11),
        ],
        // ... becomes flattened to a single path with overlapping a/b token between each node:
        &["a", "b", "a", "b", "a", "b", "a", "b", "a", "b", "a", "b"],
        &[0, 0, 2, 2, 4, 4, 6, 6, 8, 8, 10, 10],
        &[1, 1, 3, 3, 5, 5, 7, 7, 9, 9, 11, 11],
        &[1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0],
        &[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    )
}

// b has a posInc of 1, which is correct, but no edge ever visited that node.
// After hole recovery 'b' and 'c' should still be under 'abc'
// assert disabled = pos length of abc = 4
// assert enabled = AssertionError: outputEndNode=3 vs inputTo=2
fn fg_test_alt_path_first_step_hole() -> Result<()> {
    flatten_case(
        3,
        vec![
            token("abc", 1, 3, 0, 3),
            token("b", 1, 1, 1, 2),
            token("c", 1, 1, 2, 3),
        ],
        &["abc", "b", "c"],
        &[0, 1, 2],
        &[3, 2, 3],
        &[1, 1, 1],
        &[3, 1, 1],
    )
}

// Last node in an alt path fixes output node of long path. In this graph the follow-up node fixes
// that.
// incorrect pos length of abc = 1
fn fg_test_alt_path_last_step_hole() -> Result<()> {
    flatten_case(
        4,
        vec![
            token("abc", 1, 3, 0, 3),
            token("a", 0, 1, 0, 1),
            token("b", 1, 1, 1, 2),
            token("d", 2, 1, 3, 4),
        ],
        &["abc", "a", "b", "d"],
        &[0, 0, 1, 3],
        &[1, 1, 2, 4],
        &[1, 0, 1, 2],
        &[3, 1, 1, 1],
    )
}

// Check to see how multiple holes in a row are preserved.
fn fg_test_long_hole() -> Result<()> {
    flatten_case(
        28,
        vec![
            token("hello", 1, 1, 0, 5),
            token("hole", 5, 1, 20, 24),
            token("fun", 1, 1, 25, 28),
        ],
        &["hello", "hole", "fun"],
        &[0, 20, 25],
        &[5, 24, 28],
        &[1, 2, 1],
        &[1, 1, 1],
    )
}

// multiple nodes missing in the alt path.
// assert disabled = nothing
// assert enabled = AssertionError
fn fg_test_alt_path_last_step_long_hole() -> Result<()> {
    flatten_case(
        4,
        vec![
            token("abc", 1, 3, 0, 3),
            token("a", 0, 1, 0, 1),
            token("d", 3, 1, 3, 4),
        ],
        &["abc", "a", "d"],
        &[0, 0, 3],
        &[1, 1, 4],
        &[1, 0, 2],
        &[2, 1, 1],
    )
}

// LUCENE-8723
// Token stream ends without any edge to fix the long edge's output node
// assert disabled = dropped token
// assert enabled = AssertionError: 2
fn fg_test_alt_path_last_step_hole_without_end_token() -> Result<()> {
    flatten_case(
        2,
        vec![
            token("abc", 1, 3, 0, 3),
            token("a", 0, 1, 0, 1),
            token("b", 1, 1, 1, 2),
        ],
        &["abc", "a", "b"],
        &[0, 0, 1],
        &[1, 1, 2],
        &[1, 0, 1],
        &[1, 1, 1],
    )
}

// similar to AltPathLastStepHoleWithoutEndToken, but instead of no token to trigger long path
// resolution,
// the next token has no way to reference to the long path so we have to resolve as if that last
// token wasn't present.
fn fg_test_alt_path_last_step_hole_followed_by_hole() -> Result<()> {
    flatten_case(
        5,
        vec![
            token("abc", 1, 3, 0, 3),
            token("b", 1, 1, 1, 2),
            token("e", 3, 1, 4, 5),
        ],
        &["abc", "b", "e"],
        &[0, 1, 4],
        &[3, 2, 5],
        &[1, 1, 2],
        &[1, 1, 1],
    )
}

// Two Shingled long paths pass each other which gives a flattened graph with tokens backing up a
// lot.
fn fg_test_shingled_gap() -> Result<()> {
    flatten_case(
        5,
        vec![
            token("abc", 1, 3, 0, 3),
            token("a", 0, 1, 0, 1),
            token("b", 1, 1, 1, 2),
            token("cde", 1, 3, 2, 5),
            token("d", 1, 1, 3, 4),
            token("e", 1, 1, 4, 5),
        ],
        &["abc", "a", "d", "b", "cde", "e"],
        &[0, 0, 3, 3, 4, 4],
        &[1, 1, 3, 3, 5, 5],
        &[1, 0, 1, 0, 1, 0],
        &[1, 1, 1, 1, 1, 1],
    )
}

// With shingles, token order may change during flattening.
// We need to be careful not to free input nodes if they still have unreleased edges.
// with/without exceptions ArrayIndexOutOfBoundsException
fn fg_test_shingled_gap_with_holes() -> Result<()> {
    flatten_case(
        5,
        vec![
            token("abc", 1, 3, 0, 3),
            token("b", 1, 1, 1, 2),
            token("cde", 1, 3, 2, 5),
            token("d", 1, 1, 3, 4),
            token("e", 1, 1, 4, 5),
        ],
        &["abc", "d", "b", "cde", "e"],
        &[0, 3, 3, 4, 4],
        &[3, 3, 3, 5, 5],
        &[1, 1, 0, 1, 0],
        &[1, 1, 1, 1, 1],
    )
}

// When the first token is a hole there is no original token to offset from.
fn fg_test_first_token_hole() -> Result<()> {
    flatten_case(
        9,
        vec![token("start", 2, 1, 0, 5)],
        &["start"],
        &[0],
        &[5],
        &[2],
        &[1],
    )
}

// The singled token starts from a hole.
// Hole recovery will cause the shingled token to start later in the output than its alternate
// paths.
// This will result in it being released too early.
fn fg_test_shingle_from_gap() -> Result<()> {
    flatten_case(
        9,
        vec![
            token("a", 1, 1, 4, 8),
            token("abc", 0, 3, 4, 7),
            token("cd", 2, 2, 6, 8),
            token("d", 1, 1, 7, 8),
            token("e", 1, 1, 8, 9),
        ],
        &["a", "abc", "d", "cd", "e"],
        &[4, 4, 7, 7, 8],
        &[7, 7, 8, 8, 9],
        &[1, 0, 1, 1, 1],
        &[1, 1, 2, 1, 1],
    )
}

fn fg_test_shingled_gap_alt_path() -> Result<()> {
    flatten_case(
        4,
        vec![
            token("abc", 1, 3, 0, 3),
            token("abcd", 0, 4, 0, 4),
            token("cd", 2, 2, 2, 4),
        ],
        &["abc", "abcd", "cd"],
        &[0, 0, 2],
        &[3, 4, 4],
        &[1, 0, 1],
        &[1, 2, 1],
    )
}

// Lots of shingles and alternate paths connecting to each other. One edge 'c' missing between
// 'ab' and 'def'
fn fg_test_heavily_connected_graph_with_gap() -> Result<()> {
    flatten_case(
        7,
        vec![
            token("a", 1, 1, 0, 1),
            token("ab", 0, 2, 0, 2),
            token("abcdef", 0, 6, 0, 6),
            token("abcd", 0, 4, 0, 4),
            token("bcdef", 1, 5, 1, 7),
            token("def", 2, 3, 4, 7),
            token("e", 1, 1, 5, 6),
            token("f", 1, 1, 6, 7),
        ],
        &["a", "ab", "abcdef", "abcd", "bcdef", "e", "def", "f"],
        &[0, 0, 0, 0, 5, 5, 6, 6],
        &[1, 1, 7, 1, 7, 6, 7, 7],
        &[1, 0, 0, 0, 1, 0, 1, 0],
        &[1, 1, 3, 1, 2, 1, 1, 1],
    )
}

// This graph can create a disconnected input node that is farther ahead in the output than its
// subsequent input node.
// Exceptions: Free too early or dropped tokens.
fn fg_test_shingle_with_large_leading_gap() -> Result<()> {
    flatten_case(
        6,
        vec![
            token("abcde", 1, 5, 0, 5),
            token("ef", 4, 2, 4, 6),
            token("f", 1, 1, 5, 6),
        ],
        &["abcde", "f", "ef"],
        &[0, 5, 5],
        &[5, 6, 6],
        &[1, 1, 0],
        &[1, 1, 1],
    )
}

/// build CharsRef containing 2-4 tokens
fn build_multi_token_chars_ref(
    tokens: &[&str],
    chars_ref_builder: &mut CharsRefBuilder,
    random: &dyn Random,
) -> CharsRef {
    let src_len = random.next_int_bound(2) + 2;
    let mut src_tokens: Vec<&str> = Vec::with_capacity(src_len as usize);
    for _ in 0..src_len {
        src_tokens
            .push(tokens[LuceneTestCase::random().next_int_bound(tokens.len() as i32) as usize]);
    }
    SynonymMapBuilder::join(&src_tokens, chars_ref_builder);
    chars_ref_builder.to_chars_ref()
}

/// `charRefBuilder.append(s).toCharsRef()`.
fn append_to_chars_ref(builder: &mut CharsRefBuilder, s: &str) -> CharsRef {
    let units: Vec<u16> = s.encode_utf16().collect();
    builder.append_str_range(Some(&units), 0, units.len());
    builder.to_chars_ref()
}

// Create a random graph then delete some edges to see if we can trip up FlattenGraphFilter
fn fg_test_random_graphs() -> Result<()> {
    let base_tokens = ["t1", "t2", "t3", "t4"];
    let syn_tokens = ["s1", "s2", "s3", "s4"];
    let mut map_builder = SynonymMapBuilder::new();
    let mut char_ref_builder = CharsRefBuilder::new();
    let random = random();
    // between 10 and 20 synonym entries
    let syn_count = random.next_int_bound(10) + 10;
    for _ in 0..syn_count {
        let type_ = random.next_int_bound(4);
        let (src, dest);
        match type_ {
            0 => {
                // 1:1
                src = append_to_chars_ref(
                    &mut char_ref_builder,
                    base_tokens[random.next_int_bound(base_tokens.len() as i32) as usize],
                );
                char_ref_builder.clear();
                dest = append_to_chars_ref(
                    &mut char_ref_builder,
                    syn_tokens[random.next_int_bound(syn_tokens.len() as i32) as usize],
                );
                char_ref_builder.clear();
            }
            1 => {
                // many:1
                src = build_multi_token_chars_ref(&base_tokens, &mut char_ref_builder, &*random);
                char_ref_builder.clear();
                dest = append_to_chars_ref(
                    &mut char_ref_builder,
                    syn_tokens[random.next_int_bound(syn_tokens.len() as i32) as usize],
                );
                char_ref_builder.clear();
            }
            2 => {
                // 1:many
                src = append_to_chars_ref(
                    &mut char_ref_builder,
                    base_tokens[random.next_int_bound(base_tokens.len() as i32) as usize],
                );
                char_ref_builder.clear();
                dest = build_multi_token_chars_ref(&syn_tokens, &mut char_ref_builder, &*random);
                char_ref_builder.clear();
            }
            _ => {
                // many:many
                src = build_multi_token_chars_ref(&base_tokens, &mut char_ref_builder, &*random);
                char_ref_builder.clear();
                dest = build_multi_token_chars_ref(&syn_tokens, &mut char_ref_builder, &*random);
                char_ref_builder.clear();
            }
        }
        map_builder.add(&src, &dest, true)?;
    }
    let syn_map = Arc::new(map_builder.build()?);
    let stop_word_count = random.next_int_bound(4) + 1;
    let mut stop_words = CharArraySet::new(stop_word_count as usize, true);
    while (stop_words.size() as i32) < stop_word_count {
        let mut index =
            random.next_int_bound((base_tokens.len() + syn_tokens.len()) as i32) as usize;
        let mut token_array: &[&str] = &base_tokens;
        if index >= base_tokens.len() {
            index -= base_tokens.len();
            token_array = &syn_tokens;
        }
        stop_words.add(token_array[index]);
    }
    let with_flatten_graph = tokenizer_analyzer(
        "org.apache.lucene.analysis.core.TestFlattenGraphFilter$2",
        || Ok(new_whitespace_tokenizer()),
        move |input| {
            let result = boxed(new_synonym_graph_filter(input, Arc::clone(&syn_map), true)?);
            let result = boxed(new_stop_filter(result, stop_words.clone())?);
            Ok(boxed(new_flatten_graph_filter(result)?))
        },
    );
    let token_count = random.next_int_bound(20) + 20;
    let mut string_tokens: Vec<&str> = Vec::new();
    while (string_tokens.len() as i32) < token_count {
        string_tokens.push(base_tokens[random.next_int_bound(base_tokens.len() as i32) as usize]);
    }
    let text = string_tokens.join(" ");
    // FlattenGraphFilter can create inconsistent offsets.
    // If that is resolved we can check offsets
    // Until then converting to automaton will pull text through and check if we hit asserts.
    // checkAnalysisConsistency(random, withFlattenGraph, false, text);
    let tsta = TokenStreamToAutomaton::new();
    let flattened_token_stream =
        with_flatten_graph.token_stream("field", Box::new(StringCharReader::new(text)))?;
    let automaton = tsta.to_automaton(&mut *flattened_token_stream.borrow_mut())?;
    Assert::assert_false(Operations::has_dead_states(&automaton));
    flattened_token_stream.borrow_mut().close()?;
    Ok(())
}

/// Make some strings, make an automaton that accepts those strings, convert
/// that automaton into a TokenStream, flatten it, back to an automaton, and
/// see if the original strings are still accepted.
fn fg_test_paths_not_lost() -> Result<()> {
    let word_count = random().next_int_bound(5) + 5;
    let mut accept_strings: Vec<Vec<u8>> = Vec::new();
    for _ in 0..word_count {
        let word_len = random().next_int_bound(5) + 5;
        let mut word = Vec::with_capacity(word_len as usize);
        for _ in 0..word_len {
            word.push((random().next_int_bound(5) + 65) as u8);
        }
        accept_strings.push(word);
    }
    // `BytesRef` natural order: unsigned bytes, as `Vec<u8>`'s.
    accept_strings.sort();
    accept_strings.truncate(word_count as usize);
    let non_flattened_automaton =
        Automata::make_string_union(accept_strings.iter().map(|w| BytesRef::new(w.clone())))?;
    let ts = AutomatonToTokenStream::to_token_stream(&non_flattened_automaton)?;
    let mut flattened_token_stream = new_flatten_graph_filter(boxed(ts))?;
    let tsta = TokenStreamToAutomaton::new();
    let flattened_automaton = tsta.to_automaton(&mut flattened_token_stream)?;
    // TokenStreamToAutomaton adds position increment transitions into the automaton.
    let accept_strings_with_pos_sep = create_accept_strings_with_pos_sep(&accept_strings);
    for accept_string in &accept_strings_with_pos_sep {
        Assert::assert_true_msg(
            &format!(
                "string not accepted {}",
                String::from_utf8_lossy(accept_string)
            ),
            recursively_validate(accept_string, 0, 0, &flattened_automaton),
        );
    }
    Ok(())
}

/// adds POS_SEP bytes between bytes to match TokenStreamToAutomaton format.
fn create_accept_strings_with_pos_sep(accept_strings: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for accept_string in accept_strings {
        let len = accept_string.len() * 2 - 1;
        let mut with_pos_sep = vec![0u8; len];
        for (i, &b) in accept_string.iter().enumerate() {
            with_pos_sep[i * 2] = b;
            if i * 2 + 1 < len {
                with_pos_sep[i * 2 + 1] = POS_SEP as u8;
            }
        }
        out.push(with_pos_sep);
    }
    out
}

/// Checks if acceptString is accepted by the automaton. Automaton may be an
/// NFA.
fn recursively_validate(
    accept_string: &[u8],
    accept_string_index: usize,
    state: i32,
    automaton: &Automaton,
) -> bool {
    if accept_string_index == accept_string.len() {
        return automaton.is_accept(state);
    }
    let mut transition = Transition::default();
    automaton.init_transition(state, &mut transition);
    let num_transitions = TransitionAccessor::get_num_transitions(automaton, state);
    let mut accept = false;
    // Automaton can be NFA, so we need to check all matching transitions
    for i in 0..num_transitions {
        automaton.get_transition(state, i, &mut transition);
        // Java compares with the signed `byte`.
        let b = i32::from(accept_string[accept_string_index] as i8);
        if transition.min <= b && transition.max >= b {
            accept = recursively_validate(
                accept_string,
                accept_string_index + 1,
                transition.dest,
                automaton,
            );
        }
        if accept {
            break;
        }
    }
    accept
}

fn flatten_graph_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestFlattenGraphFilter")
        .method(t("testSimpleMock", fg_test_simple_mock))
        .method(t("testAlreadyFlatten", fg_test_already_flatten))
        .method(t("testWTF1", fg_test_wtf1))
        .method(t("testWTF2", fg_test_wtf2))
        .method(t("testNonGreedySynonyms", fg_test_non_greedy_synonyms))
        .method(t("testNonGraph", fg_test_non_graph))
        .method(t("testSimpleHole", fg_test_simple_hole))
        .method(t("testHoleUnderSyn", fg_test_hole_under_syn))
        .method(t(
            "testStrangelyNumberedNodes",
            fg_test_strangely_numbered_nodes,
        ))
        .method(t(
            "testTwoLongParallelPaths",
            fg_test_two_long_parallel_paths,
        ))
        .method(t(
            "testAltPathFirstStepHole",
            fg_test_alt_path_first_step_hole,
        ))
        .method(t(
            "testAltPathLastStepHole",
            fg_test_alt_path_last_step_hole,
        ))
        .method(t("testLongHole", fg_test_long_hole))
        .method(t(
            "testAltPathLastStepLongHole",
            fg_test_alt_path_last_step_long_hole,
        ))
        .method(t(
            "testAltPathLastStepHoleWithoutEndToken",
            fg_test_alt_path_last_step_hole_without_end_token,
        ))
        .method(t(
            "testAltPathLastStepHoleFollowedByHole",
            fg_test_alt_path_last_step_hole_followed_by_hole,
        ))
        .method(t("testShingledGap", fg_test_shingled_gap))
        .method(t(
            "testShingledGapWithHoles",
            fg_test_shingled_gap_with_holes,
        ))
        .method(t("testFirstTokenHole", fg_test_first_token_hole))
        .method(t("testShingleFromGap", fg_test_shingle_from_gap))
        .method(t("testShingledGapAltPath", fg_test_shingled_gap_alt_path))
        .method(t(
            "testHeavilyConnectedGraphWithGap",
            fg_test_heavily_connected_graph_with_gap,
        ))
        .method(t(
            "testShingleWithLargeLeadingGap",
            fg_test_shingle_with_large_leading_gap,
        ))
        .method(t("testRandomGraphs", fg_test_random_graphs))
        .method(t("testPathsNotLost", fg_test_paths_not_lost))
        .build()
}

// -----------------------------------------------------------------------------
// TestConcatenateGraphFilter
// -----------------------------------------------------------------------------

/// `(char) ConcatenateGraphFilter.SEP_LABEL`.
fn sep_label() -> char {
    char::from_u32(SEP_LABEL as u32).expect("a char")
}

/// A `MockTokenizer(runAutomaton, lowerCase)` over `input`.
fn mock_tokenizer(
    run_automaton: &Arc<rucene::util::automaton::CharacterRunAutomaton>,
    lower_case: bool,
    input: &str,
) -> Result<MockTokenizer> {
    let mut t = MockTokenizer::new(Arc::clone(run_automaton), lower_case)?;
    t.set_reader(Box::new(StringCharReader::new(input)))?;
    Ok(t)
}

/// `assertTokenStreamContents(stream, output, null, null, posIncrements)`.
fn assert_pos(stream: &mut dyn TokenStream, output: &[&str], pos_inc: &[i32]) -> Result<()> {
    B::assert_token_stream_contents_types(stream, output, None, None, None, Some(pos_inc))
}

fn cg_test_basic() -> Result<()> {
    let input = "mykeyword";
    let token_stream = mock_tokenizer(&WHITESPACE, true, input)?;
    let mut stream = ConcatenateGraphFilter::new(boxed(token_stream))?;
    assert_pos(&mut stream, &[input], &[1])
}

fn cg_test_with_no_preserve_sep() -> Result<()> {
    let token_stream = mock_tokenizer(&WHITESPACE, true, "mykeyword another keyword")?;
    let mut stream = ConcatenateGraphFilter::with_separator(boxed(token_stream), None, false, 100)?;
    assert_pos(&mut stream, &["mykeywordanotherkeyword"], &[1])
}

fn cg_test_with_multiple_tokens() -> Result<()> {
    let token_stream = mock_tokenizer(&WHITESPACE, true, "mykeyword another keyword")?;
    let mut stream = ConcatenateGraphFilter::new(boxed(token_stream))?;
    let expected = format!("mykeyword{0}another{0}keyword", sep_label());
    assert_pos(&mut stream, &[expected.as_str()], &[1])
}

fn cg_test_with_synonym() -> Result<()> {
    let mut builder = SynonymMapBuilder::with_dedup(true);
    builder.add(
        &CharsRef::from("mykeyword"),
        &CharsRef::from("mysynonym"),
        true,
    )?;
    let tokenizer = mock_tokenizer(&WHITESPACE, true, "mykeyword")?;
    let filter = new_synonym_filter(boxed(tokenizer), Arc::new(builder.build()?), true)?;
    let mut stream = ConcatenateGraphFilter::new(boxed(filter))?;
    assert_pos(&mut stream, &["mykeyword", "mysynonym"], &[1, 0])
}

fn cg_test_with_synonyms() -> Result<()> {
    let mut builder = SynonymMapBuilder::with_dedup(true);
    builder.add(
        &CharsRef::from("mykeyword"),
        &CharsRef::from("mysynonym"),
        true,
    )?;
    let token_stream = mock_tokenizer(&WHITESPACE, true, "mykeyword another keyword")?;
    let filter = new_synonym_filter(boxed(token_stream), Arc::new(builder.build()?), true)?;
    let mut stream =
        ConcatenateGraphFilter::with_separator(boxed(filter), Some(sep_label()), false, 100)?;
    let expected0 = format!("mykeyword{0}another{0}keyword", sep_label());
    let expected1 = format!("mysynonym{0}another{0}keyword", sep_label());
    assert_pos(
        &mut stream,
        &[expected0.as_str(), expected1.as_str()],
        &[1, 0],
    )
}

fn cg_test_with_stopword() -> Result<()> {
    for preserve_pos_inc in [true, false] {
        let input = "a mykeyword a keyword"; // LUCENE-8344 add "a"
        let token_stream = mock_tokenizer(&WHITESPACE, true, input)?;
        let token_filter = new_stop_filter(boxed(token_stream), make_stop_set(["a"], false))?;
        let mut concat_stream = ConcatenateGraphFilter::with_separator(
            boxed(token_filter),
            Some(sep_label()),
            preserve_pos_inc,
            10,
        )?;
        let mut builder = String::new();
        if preserve_pos_inc {
            builder.push(sep_label());
        }
        builder.push_str("mykeyword");
        builder.push(sep_label());
        if preserve_pos_inc {
            builder.push(sep_label());
        }
        builder.push_str("keyword");
        //      if (preservePosInc) { LUCENE-8344 uncomment
        //        builder.append(SEP_LABEL);
        //      }
        B::assert_token_stream_contents(&mut concat_stream, &[builder.as_str()])?;
    }
    Ok(())
}

fn cg_test_valid_number_of_expansions() -> Result<()> {
    let mut builder = SynonymMapBuilder::with_dedup(true);
    for i in 0..256 {
        builder.add(
            &CharsRef::from(format!("{}", i + 1)),
            &CharsRef::from(format!("{}", 1000 + (i + 1))),
            true,
        )?;
    }
    let mut value_builder = String::new();
    for i in 0..8 {
        value_builder.push_str(&format!("{} ", i + 1));
    }
    let tokenizer = mock_tokenizer(&WHITESPACE, true, &value_builder)?;
    let filter = new_synonym_filter(boxed(tokenizer), Arc::new(builder.build()?), true)?;
    let mut count = 0;
    {
        let mut stream = ConcatenateGraphFilter::new(boxed(filter))?;
        stream.reset()?;
        while stream.increment_token()? {
            count += 1;
            let attr = stream
                .attribute_source()
                .get_attribute_interface::<dyn BytesRefBuilderTermAttribute>()
                .expect("addAttribute(BytesRefBuilderTermAttribute.class)");
            let bytes = attr.get_bytes_ref();
            Assert::assert_true(!bytes.slice().is_empty());
        }
        stream.close()?;
    }
    Assert::assert_equals(count, 256);
    Ok(())
}

fn cg_test_empty() -> Result<()> {
    let tokenizer = B::whitespace_mock_tokenizer_str("")?;
    let mut filter = ConcatenateGraphFilter::new(boxed(tokenizer))?;
    B::assert_token_stream_contents(&mut filter, &[])
}

fn cg_test_separator() -> Result<()> {
    let token_stream = mock_tokenizer(&SIMPLE, true, "...mykeyword.another.keyword.")?;
    let mut stream =
        ConcatenateGraphFilter::with_separator(boxed(token_stream), Some(' '), false, 100)?; // not \u001F
    assert_pos(&mut stream, &["mykeyword another keyword"], &[1])
}

fn cg_test_separator_with_stop_words() -> Result<()> {
    let token_stream = mock_tokenizer(&WHITESPACE, false, "A B C D E F J H")?;
    let token_filter = new_stop_filter(
        boxed(token_stream),
        make_stop_set(["A", "D", "E", "J"], false),
    )?;
    let mut stream =
        ConcatenateGraphFilter::with_separator(boxed(token_filter), Some('-'), false, 100)?;
    assert_pos(&mut stream, &["B-C-F-H"], &[1])
}

fn cg_test_separator_with_stop_words_and_preserve_position_increments() -> Result<()> {
    let token_stream = mock_tokenizer(&WHITESPACE, false, "A B C D E F J H")?;
    let token_filter = new_stop_filter(
        boxed(token_stream),
        make_stop_set(["A", "D", "E", "J"], false),
    )?;
    let mut stream =
        ConcatenateGraphFilter::with_separator(boxed(token_filter), Some('-'), true, 100)?;
    assert_pos(&mut stream, &["-B-C---F--H"], &[1])
}

fn cg_test_separator_with_synonyms() -> Result<()> {
    let mut builder = SynonymMapBuilder::with_dedup(true);
    builder.add(
        &CharsRef::from("mykeyword"),
        &CharsRef::from("mysynonym"),
        true,
    )?;
    builder.add(
        &CharsRef::from("mykeyword"),
        &CharsRef::from("three words synonym"),
        true,
    )?;
    let tokenizer = mock_tokenizer(&WHITESPACE, true, " mykeyword another keyword   ")?;
    let filter = new_synonym_graph_filter(boxed(tokenizer), Arc::new(builder.build()?), true)?;
    let mut stream = ConcatenateGraphFilter::with_separator(boxed(filter), Some('-'), false, 100)?;
    assert_pos(
        &mut stream,
        &[
            "mykeyword-another-keyword",
            "mysynonym-another-keyword",
            "three words synonym-another-keyword",
        ],
        &[1, 0, 0],
    )
}

fn concatenate_graph_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestConcatenateGraphFilter")
        .method(at_test("testBasic", cg_test_basic))
        .method(at_test(
            "testWithNoPreserveSep",
            cg_test_with_no_preserve_sep,
        ))
        .method(at_test(
            "testWithMultipleTokens",
            cg_test_with_multiple_tokens,
        ))
        .method(at_test("testWithSynonym", cg_test_with_synonym))
        .method(at_test("testWithSynonyms", cg_test_with_synonyms))
        .method(at_test("testWithStopword", cg_test_with_stopword))
        .method(at_test(
            "testValidNumberOfExpansions",
            cg_test_valid_number_of_expansions,
        ))
        .method(t("testEmpty", cg_test_empty))
        .method(at_test("testSeparator", cg_test_separator))
        .method(at_test(
            "testSeparatorWithStopWords",
            cg_test_separator_with_stop_words,
        ))
        .method(at_test(
            "testSeparatorWithStopWordsAndPreservePositionIncrements",
            cg_test_separator_with_stop_words_and_preserve_position_increments,
        ))
        .method(at_test(
            "testSeparatorWithSynonyms",
            cg_test_separator_with_synonyms,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestLimitTokenPositionFilter
// -----------------------------------------------------------------------------

fn ltp_test_max_position2() -> Result<()> {
    for consume_all in [true, false] {
        let a = support::FnAnalyzer::new(
            "org.apache.lucene.analysis.miscellaneous.TestLimitTokenPositionFilter$1",
            move |_| {
                let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
                // if we are consuming all tokens, we can use the checks, otherwise we can't
                tokenizer.set_enable_checks(consume_all);
                let tokenizer =
                    Arc::new(rucene::util::atomic_ref_cell::AtomicRefCell::new(tokenizer));
                let filter = new_limit_token_position_filter_consuming(
                    rucene_test_framework::analysis::shared(&tokenizer),
                    2,
                    consume_all,
                )?;
                Ok(rucene_test_framework::analysis::components(
                    tokenizer,
                    rucene_test_framework::analysis::sink(filter),
                ))
            },
        )
        .arc();
        // don't use assertAnalyzesTo here, as the end offset is not the end of the string (unless
        // consumeAll is true, in which case it's correct)!
        let check =
            |text: &str, output: &[&str], start: &[i32], end: &[i32], fin: i32| -> Result<()> {
                let ts = a.token_stream("dummy", Box::new(StringCharReader::new(text)))?;
                let result = B::assert_token_stream_contents_offsets_final_offset(
                    &mut *ts.borrow_mut(),
                    output,
                    start,
                    end,
                    if consume_all { Some(fin) } else { None },
                );
                result
            };
        check("1  2     3  4  5", &["1", "2"], &[0, 3], &[1, 4], 16)?;
        check("1 2 3 4 5", &["1", "2"], &[0, 2], &[1, 3], 9)?;
        // less than the limit, ensure we behave correctly
        check("1  ", &["1"], &[0], &[1], 3)?;
        // equal to limit
        check("1  2  ", &["1", "2"], &[0, 3], &[1, 4], 6)?;
        a.close();
    }
    Ok(())
}

fn ltp_test_max_position3_with_synomyms() -> Result<()> {
    for consume_all in [true, false] {
        let mut tokenizer = B::whitespace_mock_tokenizer_str("one two three four five")?;
        // if we are consuming all tokens, we can use the checks, otherwise we can't
        tokenizer.set_enable_checks(consume_all);
        let mut builder = SynonymMapBuilder::with_dedup(true);
        builder.add(&CharsRef::from("one"), &CharsRef::from("first"), true)?;
        builder.add(&CharsRef::from("one"), &CharsRef::from("alpha"), true)?;
        builder.add(&CharsRef::from("one"), &CharsRef::from("beguine"), true)?;
        let mut multi_word_chars_ref = CharsRefBuilder::new();
        SynonymMapBuilder::join(
            &["and", "indubitably", "single", "only"],
            &mut multi_word_chars_ref,
        );
        builder.add(&CharsRef::from("one"), &multi_word_chars_ref.get(), true)?;
        SynonymMapBuilder::join(&["dopple", "ganger"], &mut multi_word_chars_ref);
        builder.add(&CharsRef::from("two"), &multi_word_chars_ref.get(), true)?;
        let synonym_map = Arc::new(builder.build()?);
        let stream = new_synonym_filter(boxed(tokenizer), synonym_map, true)?;
        let mut stream = new_limit_token_position_filter_consuming(boxed(stream), 3, consume_all)?;
        // "only", the 4th word of multi-word synonym "and indubitably single only" is not emitted,
        // since its position is greater than 3.
        B::assert_token_stream_contents_pos_increments(
            &mut stream,
            &[
                "one",
                "first",
                "alpha",
                "beguine",
                "and",
                "two",
                "indubitably",
                "dopple",
                "three",
                "single",
                "ganger",
            ],
            &[1, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0],
        )?;
    }
    Ok(())
}

fn ltp_test_illegal_arguments() -> Result<()> {
    new_limit_token_position_filter(
        boxed(B::whitespace_mock_tokenizer_str("one two three four five")?),
        0,
    )?;
    Ok(())
}

fn limit_token_position_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestLimitTokenPositionFilter")
        .method(t("testMaxPosition2", ltp_test_max_position2))
        .method(t(
            "testMaxPosition3WithSynomyms",
            ltp_test_max_position3_with_synomyms,
        ))
        .method(t_iae("testIllegalArguments", ltp_test_illegal_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestRemoveDuplicatesTokenFilter
// -----------------------------------------------------------------------------

fn tok(pos: i32, t: &str, start: i32, end: i32) -> Token {
    Token::with_pos_inc(t, pos, start, end)
}

fn tok0(pos: i32, t: &str) -> Token {
    tok(pos, t, 0, 0)
}

/// `testDups(String expected, Token... tokens)`: a helper with parameters
/// (not run as a test). Java's anonymous stream sets the term, offsets and
/// position increment of each token; `CannedTokenStream` sets the same
/// attributes from the tokens (and their default type and flags, which
/// `RemoveDuplicatesTokenFilter` does not read).
fn rd_test_dups(expected: &str, tokens: Vec<Token>) -> Result<()> {
    let mut ts = new_remove_duplicates_token_filter(boxed(CannedTokenStream::new(tokens)))?;
    // `expected.split("\\s")`
    let expected: Vec<&str> = expected
        .split([' ', '\t', '\n', '\u{0B}', '\u{0C}', '\r'])
        .collect();
    B::assert_token_stream_contents(&mut ts, &expected)
}

fn rd_test_no_dups() -> Result<()> {
    rd_test_dups(
        "A B B C D E",
        vec![
            tok(1, "A", 0, 4),
            tok(1, "B", 5, 10),
            tok(1, "B", 11, 15),
            tok(1, "C", 16, 20),
            tok(0, "D", 16, 20),
            tok(1, "E", 21, 25),
        ],
    )
}

fn rd_test_simple_dups() -> Result<()> {
    rd_test_dups(
        "A B C D E",
        vec![
            tok(1, "A", 0, 4),
            tok(1, "B", 5, 10),
            tok(0, "B", 11, 15),
            tok(1, "C", 16, 20),
            tok(0, "D", 16, 20),
            tok(1, "E", 21, 25),
        ],
    )
}

fn rd_test_complex_dups() -> Result<()> {
    rd_test_dups(
        "A B C D E F G H I J K",
        vec![
            tok0(1, "A"),
            tok0(1, "B"),
            tok0(0, "B"),
            tok0(1, "C"),
            tok0(1, "D"),
            tok0(0, "D"),
            tok0(0, "D"),
            tok0(1, "E"),
            tok0(1, "F"),
            tok0(0, "F"),
            tok0(1, "G"),
            tok0(0, "H"),
            tok0(0, "H"),
            tok0(1, "I"),
            tok0(1, "J"),
            tok0(0, "K"),
            tok0(0, "J"),
        ],
    )
}

/// `add(b, input, output, keepOrig)`: runs of spaces become word separators.
fn rd_add(b: &mut SynonymMapBuilder, input: &str, output: &str, keep_orig: bool) -> Result<()> {
    let sep = |s: &str| -> String {
        let mut out = String::new();
        let mut in_spaces = false;
        for c in s.chars() {
            if c == ' ' {
                if !in_spaces {
                    out.push('\u{0}');
                }
                in_spaces = true;
            } else {
                out.push(c);
                in_spaces = false;
            }
        }
        out
    };
    b.add(
        &CharsRef::from(sep(input)),
        &CharsRef::from(sep(output)),
        keep_orig,
    )
}

/// blast some random strings through the analyzer
fn rd_test_random_strings() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(3);
    for _ in 0..num_iters {
        let mut b = SynonymMapBuilder::with_dedup(random().next_boolean());
        let num_entries = LuceneTestCase::at_least(10);
        for _ in 0..num_entries {
            let input = random_non_empty_string();
            let output = random_non_empty_string();
            rd_add(&mut b, &input, &output, random().next_boolean())?;
        }
        let map = Arc::new(b.build()?);
        let ignore_case = random().next_boolean();
        let analyzer = mock_tokenizer_analyzer(
            "org.apache.lucene.analysis.miscellaneous.TestRemoveDuplicatesTokenFilter$2",
            Arc::clone(&SIMPLE),
            true,
            move |t| {
                let stream = new_synonym_graph_filter(t, Arc::clone(&map), ignore_case)?;
                Ok(boxed(new_remove_duplicates_token_filter(boxed(stream))?))
            },
        );
        B::check_random_data(&*random(), &analyzer, 200)?;
        analyzer.close();
    }
    Ok(())
}

fn rd_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestRemoveDuplicatesTokenFilter$3",
        |t| Ok(boxed(new_remove_duplicates_token_filter(t)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn remove_duplicates_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestRemoveDuplicatesTokenFilter")
        .method(t("testNoDups", rd_test_no_dups))
        .method(t("testSimpleDups", rd_test_simple_dups))
        .method(t("testComplexDups", rd_test_complex_dups))
        .method(t("testRandomStrings", rd_test_random_strings))
        .method(t("testEmptyTerm", rd_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestDl4jModelReader
// -----------------------------------------------------------------------------

const WORD2VEC: &str = "org.apache.lucene.analysis.synonym.word2vec";
const MODEL_FILE: &str = "word2vec-model.zip";
const MODEL_EMPTY_FILE: &str = "word2vec-empty-model.zip";
const CORRUPTED_VECTOR_DIMENSION_MODEL_FILE: &str = "word2vec-corrupted-vector-dimension-model.zip";

/// `TestDl4jModelReader.class.getResourceAsStream(name)`.
fn resource_stream(name: &str) -> Box<dyn std::io::Read + Send> {
    let path = format!("{}/{name}", WORD2VEC.replace('.', "/"));
    let bytes = support::TEST_RESOURCES
        .iter()
        .find(|(n, _)| *n == path)
        .map(|(_, b)| b.to_vec())
        .expect("a test resource");
    Box::new(std::io::Cursor::new(bytes))
}

/// The instance field `unit`: a reader over `MODEL_FILE`.
fn unit() -> Dl4jModelReader {
    Dl4jModelReader::new(resource_stream(MODEL_FILE))
}

fn dl4j_read_zip_file_with_metadata_should_return_dictionary_size() -> Result<()> {
    let model = unit().read()?;
    let expected_dictionary_size: i64 = 235;
    Assert::assert_equals(expected_dictionary_size, i64::from(model.size()));
    Ok(())
}

fn dl4j_read_zip_file_with_metadata_should_return_vector_length() -> Result<()> {
    let model = unit().read()?;
    let expected_vector_dimension = 100;
    Assert::assert_equals(expected_vector_dimension, model.dimension());
    Ok(())
}

fn dl4j_read_zip_file_should_return_decoded_term() -> Result<()> {
    let model = unit().read()?;
    assert_bytes("it", &model.term_value(0)?);
    Ok(())
}

fn dl4j_decode_term_encoded_term_should_return_decoded_term() -> Result<()> {
    // `Base64.getEncoder().encodeToString("lucene".getBytes(UTF_8))`
    let b64_encoded_lucene_term = "bHVjZW5l";
    let word2vec_encoded_lucene_term = format!("B64:{b64_encoded_lucene_term}");
    let units: Vec<u16> = word2vec_encoded_lucene_term.encode_utf16().collect();
    assert_bytes("lucene", &Dl4jModelReader::decode_b64_term(&units)?);
    Ok(())
}

fn dl4j_read_empty_zip_file_should_throw_exception() -> Result<()> {
    let mut unit = Dl4jModelReader::new(resource_stream(MODEL_EMPTY_FILE));
    expect_throws("java.lang.IllegalArgumentException", || unit.read());
    unit.close()
}

fn dl4j_read_corrupted_vector_dimension_model_file_should_throw_exception() -> Result<()> {
    let mut unit = Dl4jModelReader::new(resource_stream(CORRUPTED_VECTOR_DIMENSION_MODEL_FILE));
    expect_throws("java.lang.RuntimeException", || unit.read());
    unit.close()
}

fn dl4j_model_reader_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.synonym.word2vec.TestDl4jModelReader")
        .method(at_test(
            "read_zipFileWithMetadata_shouldReturnDictionarySize",
            dl4j_read_zip_file_with_metadata_should_return_dictionary_size,
        ))
        .method(at_test(
            "read_zipFileWithMetadata_shouldReturnVectorLength",
            dl4j_read_zip_file_with_metadata_should_return_vector_length,
        ))
        .method(at_test(
            "read_zipFile_shouldReturnDecodedTerm",
            dl4j_read_zip_file_should_return_decoded_term,
        ))
        .method(at_test(
            "decodeTerm_encodedTerm_shouldReturnDecodedTerm",
            dl4j_decode_term_encoded_term_should_return_decoded_term,
        ))
        .method(at_test(
            "read_EmptyZipFile_shouldThrowException",
            dl4j_read_empty_zip_file_should_throw_exception,
        ))
        .method(at_test(
            "read_corruptedVectorDimensionModelFile_shouldThrowException",
            dl4j_read_corrupted_vector_dimension_model_file_should_throw_exception,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestWord2VecSynonymFilter
// -----------------------------------------------------------------------------

/// `new Word2VecModel(dictionarySize, dimension)` with `terms` added.
fn model(dictionary_size: i32, dimension: i32, terms: &[(&str, &[f32])]) -> Result<Word2VecModel> {
    let mut model = Word2VecModel::new(dictionary_size, dimension)?;
    for (term, vector) in terms {
        model.add_term_and_vector(TermAndVector::new(bytes(term), vector.to_vec()))?;
    }
    Ok(model)
}

/// `getAnalyzer(synonymProvider, maxSynonymsPerTerm, minAcceptedSimilarity)`.
fn w2v_analyzer(
    synonym_provider: Arc<Word2VecSynonymProvider>,
    max_synonyms_per_term: i32,
    min_accepted_similarity: f32,
) -> Arc<dyn Analyzer> {
    mock_tokenizer_analyzer(
        "org.apache.lucene.analysis.synonym.word2vec.TestWord2VecSynonymFilter$1",
        Arc::clone(&WHITESPACE),
        false,
        move |t| {
            // Make a local variable so testRandomHuge doesn't share it across threads!
            Ok(boxed(new_word2vec_synonym_filter(
                t,
                Arc::clone(&synonym_provider),
                max_synonyms_per_term,
                min_accepted_similarity,
            )?))
        },
    )
}

/// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets, types,
/// posIncrements, posLengths)`.
#[allow(clippy::too_many_arguments)]
fn analyzes_full(
    a: &dyn Analyzer,
    input: &str,
    output: &[&str],
    start: &[i32],
    end: &[i32],
    types: &[&str],
    pos_inc: &[i32],
    pos_len: &[i32],
) -> Result<()> {
    B::assert_analyzes_to_pos_lengths(
        a,
        input,
        output,
        Some(start),
        Some(end),
        Some(types),
        Some(pos_inc),
        Some(pos_len),
    )
}

fn w2vf_one_candidate_within_threshold() -> Result<()> {
    let max_synonym_per_term = 10;
    let min_accepted_similarity = 0.9f32;
    let model = model(
        6,
        2,
        &[
            ("a", &[10.0, 10.0]),
            ("b", &[10.0, 8.0]),
            ("c", &[9.0, 10.0]),
            ("d", &[1.0, 1.0]),
            ("e", &[99.0, 101.0]),
            ("f", &[-1.0, 10.0]),
        ],
    )?;
    let synonym_provider = Arc::new(Word2VecSynonymProvider::new(model)?);
    let a = w2v_analyzer(
        synonym_provider,
        max_synonym_per_term,
        min_accepted_similarity,
    );
    analyzes_full(
        &*a,
        "pre a post",
        &["pre", "a", "d", "e", "c", "b", "post"],
        &[0, 4, 4, 4, 4, 4, 6],
        &[3, 5, 5, 5, 5, 5, 10],
        &[
            "word", "word", "SYNONYM", "SYNONYM", "SYNONYM", "SYNONYM", "word",
        ],
        &[1, 1, 0, 0, 0, 0, 1],
        &[1, 1, 1, 1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn w2vf_one_candidate_top_k() -> Result<()> {
    let max_synonym_per_term = 2;
    let min_accepted_similarity = 0.9f32;
    let model = model(
        5,
        2,
        &[
            ("a", &[10.0, 10.0]),
            ("b", &[10.0, 8.0]),
            ("c", &[9.0, 10.0]),
            ("d", &[1.0, 1.0]),
            ("e", &[99.0, 101.0]),
        ],
    )?;
    let synonym_provider = Arc::new(Word2VecSynonymProvider::new(model)?);
    let a = w2v_analyzer(
        synonym_provider,
        max_synonym_per_term,
        min_accepted_similarity,
    );
    analyzes_full(
        &*a,
        "pre a post",
        &["pre", "a", "d", "e", "post"],
        &[0, 4, 4, 4, 6],
        &[3, 5, 5, 5, 10],
        &["word", "word", "SYNONYM", "SYNONYM", "word"],
        &[1, 1, 0, 0, 1],
        &[1, 1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn w2vf_two_candidates() -> Result<()> {
    let model = model(
        8,
        2,
        &[
            ("a", &[10.0, 10.0]),
            ("b", &[10.0, 8.0]),
            ("c", &[9.0, 10.0]),
            ("d", &[1.0, 1.0]),
            ("e", &[99.0, 101.0]),
            ("f", &[1.0, 10.0]),
            ("post", &[-10.0, -11.0]),
            ("after", &[-8.0, -10.0]),
        ],
    )?;
    let synonym_provider = Arc::new(Word2VecSynonymProvider::new(model)?);
    let a = w2v_analyzer(synonym_provider, 10, 0.9);
    analyzes_full(
        &*a,
        "pre a post",
        &["pre", "a", "d", "e", "c", "b", "post", "after"],
        &[0, 4, 4, 4, 4, 4, 6, 6],
        &[3, 5, 5, 5, 5, 5, 10, 10],
        &[
            "word", "word", "SYNONYM", "SYNONYM", "SYNONYM", "SYNONYM", "word", "SYNONYM",
        ],
        &[1, 1, 0, 0, 0, 0, 1, 0],
        &[1, 1, 1, 1, 1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn w2vf_min_accepted_similarity_none() -> Result<()> {
    let model = model(
        4,
        2,
        &[
            ("a", &[10.0, 10.0]),
            ("b", &[-10.0, -8.0]),
            ("c", &[-9.0, -10.0]),
            ("f", &[-1.0, -10.0]),
        ],
    )?;
    let synonym_provider = Arc::new(Word2VecSynonymProvider::new(model)?);
    let a = w2v_analyzer(synonym_provider, 10, 0.8);
    analyzes_full(
        &*a,
        "pre a post",
        &["pre", "a", "post"],
        &[0, 4, 6],
        &[3, 5, 10],
        &["word", "word", "word"],
        &[1, 1, 1],
        &[1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn word2vec_synonym_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.synonym.word2vec.TestWord2VecSynonymFilter")
        .method(at_test(
            "synonymExpansion_oneCandidate_shouldBeExpandedWithinThreshold",
            w2vf_one_candidate_within_threshold,
        ))
        .method(at_test(
            "synonymExpansion_oneCandidate_shouldBeExpandedWithTopKSynonyms",
            w2vf_one_candidate_top_k,
        ))
        .method(at_test(
            "synonymExpansion_twoCandidates_shouldBothBeExpanded",
            w2vf_two_candidates,
        ))
        .method(at_test(
            "synonymExpansion_forMinAcceptedSimilarity_shouldExpandToNoneSynonyms",
            w2vf_min_accepted_similarity_none,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestWord2VecSynonymFilterFactory
// -----------------------------------------------------------------------------

const FACTORY_NAME: &str = "Word2VecSynonym";
const WORD2VEC_MODEL_FILE: &str = "word2vec-model.zip";

fn w2vff_test_inform() -> Result<()> {
    // `new ClasspathResourceLoader(getClass())` is a value, never null.
    let factory = concrete_factory(
        WORD2VEC,
        Word2VecSynonymFilterFactory::new,
        &["model", WORD2VEC_MODEL_FILE, "minAcceptedSimilarity", "0.7"],
    )?;
    let synonym_provider = factory.get_synonym_provider();
    Assert::assert_true(synonym_provider.is_some());
    Ok(())
}

/// The message of the `IllegalArgumentException` the factory throws for
/// `keys_and_values`.
fn w2v_iae(keys_and_values: &[&str]) -> String {
    iae_message(|| token_filter_factory(WORD2VEC, FACTORY_NAME, keys_and_values))
}

fn w2vff_missing_required_argument() -> Result<()> {
    let expected = w2v_iae(&[
        "format",
        "dl4j",
        "minAcceptedSimilarity",
        "0.7",
        "maxSynonymsPerTerm",
        "10",
    ]);
    Assert::assert_true(expected.contains("Configuration Error: missing parameter 'model'"));
    Ok(())
}

fn w2vff_unsupported_model_format() -> Result<()> {
    let expected = w2v_iae(&["model", WORD2VEC_MODEL_FILE, "format", "bogusValue"]);
    Assert::assert_true(expected.contains("Model format 'BOGUSVALUE' not supported"));
    Ok(())
}

fn w2vff_bogus_argument() -> Result<()> {
    let expected = w2v_iae(&["model", WORD2VEC_MODEL_FILE, "bogusArg", "bogusValue"]);
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn w2vff_illegal_arguments() -> Result<()> {
    let expected = w2v_iae(&[
        "model",
        WORD2VEC_MODEL_FILE,
        "minAcceptedSimilarity",
        "2",
        "maxSynonymsPerTerm",
        "10",
    ]);
    Assert::assert_true(
        expected.contains("minAcceptedSimilarity must be in the range (0, 1]. Found: 2"),
    );
    let expected = w2v_iae(&[
        "model",
        WORD2VEC_MODEL_FILE,
        "minAcceptedSimilarity",
        "0",
        "maxSynonymsPerTerm",
        "10",
    ]);
    Assert::assert_true(
        expected.contains("minAcceptedSimilarity must be in the range (0, 1]. Found: 0"),
    );
    let expected = w2v_iae(&[
        "model",
        WORD2VEC_MODEL_FILE,
        "minAcceptedSimilarity",
        "0.7",
        "maxSynonymsPerTerm",
        "-1",
    ]);
    Assert::assert_true(
        expected
            .contains("maxSynonymsPerTerm must be a positive integer greater than 0. Found: -1"),
    );
    let expected = w2v_iae(&[
        "model",
        WORD2VEC_MODEL_FILE,
        "minAcceptedSimilarity",
        "0.7",
        "maxSynonymsPerTerm",
        "0",
    ]);
    Assert::assert_true(
        expected.contains("maxSynonymsPerTerm must be a positive integer greater than 0. Found: 0"),
    );
    Ok(())
}

fn word2vec_synonym_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.synonym.word2vec.TestWord2VecSynonymFilterFactory")
        .method(at_test("testInform", w2vff_test_inform))
        .method(at_test(
            "missingRequiredArgument_shouldThrowException",
            w2vff_missing_required_argument,
        ))
        .method(at_test(
            "unsupportedModelFormat_shouldThrowException",
            w2vff_unsupported_model_format,
        ))
        .method(at_test(
            "bogusArgument_shouldThrowException",
            w2vff_bogus_argument,
        ))
        .method(at_test(
            "illegalArguments_shouldThrowException",
            w2vff_illegal_arguments,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestWord2VecSynonymProvider
// -----------------------------------------------------------------------------

const MAX_SYNONYMS_PER_TERM: i32 = 10;
const MIN_ACCEPTED_SIMILARITY: f32 = 0.85;

/// The constructor of the test class: builds the `unit` field (unused by the
/// ported methods; kept for the constructor's side effects and errors).
static CTOR_UNIT: Mutex<Option<Arc<Word2VecSynonymProvider>>> = Mutex::new(None);

fn w2vp_constructor() -> Result<()> {
    let model = model(
        2,
        3,
        &[("a", &[0.24, 0.78, 0.28]), ("b", &[0.44, 0.01, 0.81])],
    )?;
    *CTOR_UNIT.lock().unwrap() = Some(Arc::new(Word2VecSynonymProvider::new(model)?));
    Ok(())
}

fn w2vp_min_accepted_similarity() -> Result<()> {
    w2vp_constructor()?;
    let model = model(
        6,
        2,
        &[
            ("a", &[10.0, 10.0]),
            ("b", &[10.0, 8.0]),
            ("c", &[9.0, 10.0]),
            ("d", &[1.0, 1.0]),
            ("e", &[99.0, 101.0]),
            ("f", &[-1.0, 10.0]),
        ],
    )?;
    let unit = Word2VecSynonymProvider::new(model)?;
    let input_term = bytes("a");
    let expected_synonyms = ["d", "e", "c", "b"];
    let actual = unit.get_synonyms(&input_term, MAX_SYNONYMS_PER_TERM, MIN_ACCEPTED_SIMILARITY)?;
    Assert::assert_equals(4, actual.len());
    for (i, expected) in expected_synonyms.iter().enumerate() {
        assert_bytes(expected, actual[i].term());
    }
    Ok(())
}

fn w2vp_boost() -> Result<()> {
    w2vp_constructor()?;
    let model = model(
        3,
        2,
        &[
            ("a", &[10.0, 10.0]),
            ("b", &[1.0, 1.0]),
            ("c", &[99.0, 101.0]),
        ],
    )?;
    let unit = Word2VecSynonymProvider::new(model)?;
    let input_term = bytes("a");
    let actual = unit.get_synonyms(&input_term, MAX_SYNONYMS_PER_TERM, MIN_ACCEPTED_SIMILARITY)?;
    assert_bytes("b", actual[0].term());
    let expected_first_synonym_boost = 1.0f64;
    Assert::assert_equals_double_msg(
        None,
        expected_first_synonym_boost,
        f64::from(actual[0].boost()),
        f64::from(0.001f32),
    );
    Ok(())
}

fn w2vp_no_synonyms() -> Result<()> {
    w2vp_constructor()?;
    let model = model(
        4,
        2,
        &[
            ("a", &[10.0, 10.0]),
            ("b", &[-10.0, -8.0]),
            ("c", &[-9.0, -10.0]),
            ("d", &[6.0, -6.0]),
        ],
    )?;
    let unit = Word2VecSynonymProvider::new(model)?;
    let input_term = LuceneTestCase::new_bytes_ref_str("a");
    let actual = unit.get_synonyms(&input_term, MAX_SYNONYMS_PER_TERM, MIN_ACCEPTED_SIMILARITY)?;
    Assert::assert_equals(0, actual.len());
    Ok(())
}

// `0.70710` is the Java test's literal.
#[allow(clippy::approx_constant)]
fn w2vp_normalized_vectors() -> Result<()> {
    w2vp_constructor()?;
    let model = model(
        4,
        2,
        &[
            ("a", &[10.0, 10.0]),
            ("b", &[10.0, 8.0]),
            ("c", &[9.0, 10.0]),
            ("f", &[-1.0, 10.0]),
        ],
    )?;
    let vector_id_a = model.vector_value_of(&bytes("a")).expect("a vector");
    let vector_id_f = model.vector_value_of(&bytes("f")).expect("a vector");
    assert_array_equals(&[0.70710, 0.70710], &vector_id_a, 0.001);
    assert_array_equals(&[-0.0995, 0.99503], &vector_id_f, 0.001);
    Ok(())
}

/// `assertArrayEquals(float[] expected, float[] actual, float delta)`.
fn assert_array_equals(expected: &[f32], actual: &[f32], delta: f32) {
    Assert::assert_equals(expected.len(), actual.len());
    for (e, a) in expected.iter().zip(actual) {
        Assert::assert_equals_float_msg(None, *e, *a, delta);
    }
}

fn w2vp_normalized_vector_module1() -> Result<()> {
    w2vp_constructor()?;
    let synonym_term = TermAndVector::new(bytes("a"), vec![10.0, 10.0]);
    let synonym_term = synonym_term.normalize_vector()?;
    let vector = synonym_term.vector();
    let mut len = 0f32;
    for v in vector {
        len += v * v;
    }
    Assert::assert_equals_double_msg(None, 1.0, f64::from(len).sqrt(), f64::from(0.0001f32));
    Ok(())
}

fn word2vec_synonym_provider_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.synonym.word2vec.TestWord2VecSynonymProvider")
        .method(at_test(
            "getSynonyms_shouldReturnSynonymsBasedOnMinAcceptedSimilarity",
            w2vp_min_accepted_similarity,
        ))
        .method(at_test("getSynonyms_shouldReturnSynonymsBoost", w2vp_boost))
        .method(at_test(
            "noSynonymsWithinAcceptedSimilarity_shouldReturnNoSynonyms",
            w2vp_no_synonyms,
        ))
        .method(at_test(
            "testModel_shouldReturnNormalizedVectors",
            w2vp_normalized_vectors,
        ))
        .method(at_test(
            "normalizedVector_shouldReturnModule1",
            w2vp_normalized_vector_module1,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// Suites
// -----------------------------------------------------------------------------

#[test]
fn test_flatten_graph_filter() {
    run(flatten_graph_filter_class);
}

#[test]
fn test_concatenate_graph_filter() {
    run(concatenate_graph_filter_class);
}

#[test]
fn test_limit_token_position_filter() {
    run(limit_token_position_filter_class);
}

#[test]
fn test_remove_duplicates_token_filter() {
    run(remove_duplicates_token_filter_class);
}

#[test]
fn test_dl4j_model_reader() {
    run(dl4j_model_reader_class);
}

#[test]
fn test_word2vec_synonym_filter() {
    run(word2vec_synonym_filter_class);
}

#[test]
fn test_word2vec_synonym_filter_factory() {
    run(word2vec_synonym_filter_factory_class);
}

#[test]
fn test_word2vec_synonym_provider() {
    run(word2vec_synonym_provider_class);
}
