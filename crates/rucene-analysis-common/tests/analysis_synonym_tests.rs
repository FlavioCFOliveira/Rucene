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

//! Port of the `org.apache.lucene.analysis.synonym` test classes of Lucene
//! 10.5.0 `lucene/analysis/common/src/test` that rmp task #453 lists, each
//! run as a suite by the ported runner: `BaseSynonymParserTestCase` (the
//! abstract base of `TestSolrSynonymParser`, its helpers here),
//! `TestMultiWordSynonyms`, `TestSolrSynonymParser`, `TestSynonymGraphFilter`,
//! `TestSynonymMapFilter` and `TestWordnetSynonymParser`.
//!
//! `TestSynonymMapFilter` and `TestSolrSynonymParser` are `@Deprecated` and
//! exercise the deprecated `SynonymFilter`, as in Lucene: deprecation
//! warnings are allowed here.

#![allow(deprecated)]

mod support;

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class, ClassBuilder, Constructor};
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute,
};
use rucene::analysis::{
    Analyzer, CharReader, SharedTokenStream, StringCharReader, TokenStream, TokenStreamComponents,
};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::index_writer::MAX_TERM_LENGTH;
use rucene::search::phrase_query::PhraseQuery;
use rucene::store::{ByteArrayDataInput, DataInput};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::automaton::{
    Automaton, AutomatonBuilder, Operations, Transition, TransitionAccessor,
};
use rucene::util::chars_ref::CharsRef;
use rucene::util::extra::Version;
use rucene::util::fst::Util;
use rucene::util::java_collections::JavaHashMap;
use rucene::util::refs::{CharsRefBuilder, IntsRefBuilder};
use rucene_analysis_common::analysis::core::flatten_graph_filter::{
    new_flatten_graph_filter, FlattenGraphFilter,
};
use rucene_analysis_common::analysis::en::english_analyzer::EnglishAnalyzer;
use rucene_analysis_common::analysis::synonym::solr_synonym_parser::SolrSynonymParser;
use rucene_analysis_common::analysis::synonym::synonym_filter::{
    new_synonym_filter, SynonymFilter,
};
use rucene_analysis_common::analysis::synonym::synonym_graph_filter::{
    new_synonym_graph_filter, SynonymGraphFilter,
};
use rucene_analysis_common::analysis::synonym::synonym_map::{
    Builder as SynonymMapBuilder, Parser, SynonymMap,
};
use rucene_analysis_common::analysis::synonym::wordnet_synonym_parser::WordnetSynonymParser;
use rucene_test_framework::analysis::mock_analyzer::BoxedStream;
use rucene_test_framework::analysis::mock_graph_token_filter::MockGraphTokenFilter;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, KEYWORD, SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::{
    components, shared, sink, BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase, MockAnalyzer,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::automaton::automaton_test_util::{
    AutomatonTestUtil, RandomAcceptedStrings,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::VERBOSE;
use rucene_test_framework::util::string_mock_resource_loader::StringMockResourceLoader;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use support::{boxed, btstc_class, factory_class, iae_message, run, t, FnAnalyzer};

type B = BaseTokenStreamTestCase;
type F = BaseTokenStreamFactoryTestCase;

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `s.split(" +")`: splits on runs of spaces; as Java's `String.split`, a
/// leading empty string is kept and trailing empty strings are dropped.
fn split_spaces(s: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = Vec::new();
    let mut rest = s;
    loop {
        match rest.find(' ') {
            Some(i) => {
                parts.push(&rest[..i]);
                rest = rest[i..].trim_start_matches(' ');
            }
            None => {
                parts.push(rest);
                break;
            }
        }
    }
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}

/// `s.split(" ")`: splits on every space, trailing empty strings dropped.
fn split_space(s: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = s.split(' ').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}

/// Java's `String.trim()`: strips leading and trailing characters `<= ' '`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `assertAnalyzesTo(a, input, output, posIncrements)`.
fn analyzes_pos(a: &dyn Analyzer, input: &str, output: &[&str], pos_inc: &[i32]) -> Result<()> {
    B::assert_analyzes_to_pos_increments(a, input, output, pos_inc)
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
    types: Option<&[&str]>,
    pos_inc: &[i32],
    pos_len: &[i32],
) -> Result<()> {
    B::assert_analyzes_to_pos_lengths(
        a,
        input,
        output,
        Some(start),
        Some(end),
        types,
        Some(pos_inc),
        Some(pos_len),
    )
}

/// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets, null,
/// posIncrements, posLengths, graphOffsetsAreCorrect)`.
#[allow(clippy::too_many_arguments)]
fn analyzes_graph(
    a: &dyn Analyzer,
    input: &str,
    output: &[&str],
    start: &[i32],
    end: &[i32],
    pos_inc: &[i32],
    pos_len: &[i32],
    graph_offsets_are_correct: bool,
) -> Result<()> {
    B::assert_analyzes_to_graph(
        a,
        input,
        output,
        Some(start),
        Some(end),
        None,
        Some(pos_inc),
        Some(pos_len),
        graph_offsets_are_correct,
    )
}

/// `new StringReader(text)`.
fn reader(text: &str) -> StringCharReader {
    StringCharReader::new(text)
}

/// An analyzer over `new MockTokenizer(runAutomaton, lowerCase)` and the
/// filter `chain` builds over it.
fn mock_analyzer(
    name: &'static str,
    run_automaton: &LazyLock<Arc<rucene::util::automaton::CharacterRunAutomaton>>,
    lower_case: bool,
    chain: impl Fn(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> + Send + Sync + 'static,
) -> Arc<dyn Analyzer> {
    support::mock_tokenizer_analyzer(name, Arc::clone(run_automaton), lower_case, chain)
}

/// `new SynonymFilter(input, map, ignoreCase)`, boxed.
fn syn_filter(
    input: Box<dyn TokenStream>,
    map: &Arc<SynonymMap>,
    ignore_case: bool,
) -> Result<Box<dyn TokenStream>> {
    Ok(boxed(new_synonym_filter(
        input,
        Arc::clone(map),
        ignore_case,
    )?))
}

/// `add(b, input, output, keepOrig)`: joins the space-separated words of
/// `input` and `output` and adds the rule.
fn add(b: &mut SynonymMapBuilder, input: &str, output: &str, keep_orig: bool) -> Result<()> {
    if *VERBOSE {
        println!("  add input={input} output={output} keepOrig={keep_orig}");
    }
    let mut input_chars_ref = CharsRefBuilder::new();
    SynonymMapBuilder::join(&split_spaces(input), &mut input_chars_ref);
    let mut output_chars_ref = CharsRefBuilder::new();
    SynonymMapBuilder::join(&split_spaces(output), &mut output_chars_ref);
    b.add(&input_chars_ref.get(), &output_chars_ref.get(), keep_orig)
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

/// `parser.parse(new StringReader(text))`.
fn parse(parser: &mut dyn Parser, text: &str) -> Result<()> {
    let mut r = reader(text);
    parser.parse(&mut r as &mut dyn CharReader)
}

// -----------------------------------------------------------------------------
// BaseSynonymParserTestCase
// -----------------------------------------------------------------------------

/// `org.apache.lucene.analysis.synonym.BaseSynonymParserTestCase`, the
/// abstract class.
fn base_synonym_parser_test_case_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.analysis.synonym.BaseSynonymParserTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&BaseTokenStreamTestCase::class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// The builder of a `BaseSynonymParserTestCase` subclass named `name`.
fn base_synonym_parser_class(name: &str) -> ClassBuilder {
    Class::builder(name)
        .extends(&base_synonym_parser_test_case_class())
        .constructor(Constructor::no_args(|| ()))
}

/// The output of `word` (spaces as word separators) in `synonym_map`'s FST.
fn fst_value(synonym_map: &SynonymMap, word: &str) -> Result<Option<rucene::util::BytesRef>> {
    let word = word.replace(' ', "\u{0}");
    let fst = synonym_map.fst.as_ref().expect("an FST");
    Util::get(fst, &Util::to_utf32(&word, &mut IntsRefBuilder::new()))
}

/// Helper method to validate synonym parsing.
///
/// `synonym_map`: the generated synonym map after parsing; `word`: word
/// (phrase) we are validating the synonyms for. Should be the value that
/// comes out of the analyzer. All spaces will be replaced by word
/// separators; `include_orig`: if synonyms should include original;
/// `synonyms`: actual synonyms. All word separators are replaced with a
/// single space.
fn assert_entry_equals(
    synonym_map: &SynonymMap,
    word: &str,
    include_orig: bool,
    synonyms: &[&str],
) -> Result<()> {
    let value = fst_value(synonym_map, word)?;
    Assert::assert_true_msg(
        &format!("No synonyms found for: {}", word.replace(' ', "\u{0}")),
        value.is_some(),
    );
    let value = value.unwrap();
    let mut bytes_reader = ByteArrayDataInput::new(value.slice());
    let code = bytes_reader.read_v_int()?;
    let keep_orig = (code & 0x1) == 0;
    Assert::assert_equals_msg(
        Some(&format!(
            "Include original different than expected. Expected {include_orig} was {keep_orig}"
        )),
        include_orig,
        keep_orig,
    );
    let count = ((code as u32) >> 1) as i32;
    Assert::assert_equals_msg(
        Some(&format!(
            "Invalid synonym count. Expected {} was {count}",
            synonyms.len()
        )),
        synonyms.len() as i32,
        count,
    );
    let synonym_set: HashSet<&str> = synonyms.iter().copied().collect();
    for _ in 0..count {
        let ord = bytes_reader.read_v_int()?;
        let bytes = synonym_map.words.read().unwrap().get(ord).to_vec();
        let synonym = String::from_utf8(bytes)
            .expect("UTF-8")
            .replace('\u{0}', " ");
        Assert::assert_true_msg(
            &format!("Unexpected synonym found: {synonym}"),
            synonym_set.contains(synonym.as_str()),
        );
    }
    Ok(())
}

/// Validates that there are no synonyms for the given word.
fn assert_entry_absent(synonym_map: &SynonymMap, word: &str) -> Result<()> {
    let value = fst_value(synonym_map, word)?;
    Assert::assert_true_msg(
        &format!(
            "There should be no synonyms for: {}",
            word.replace(' ', "\u{0}")
        ),
        value.is_none(),
    );
    Ok(())
}

/// `assertAnalyzesToPositions(a, input, output, types, posIncrements,
/// posLengths)`.
fn assert_analyzes_to_positions(
    a: &dyn Analyzer,
    input: &str,
    output: &[&str],
    types: &[&str],
    pos_increments: &[i32],
    pos_lengths: &[i32],
) -> Result<()> {
    B::assert_analyzes_to_pos_lengths(
        a,
        input,
        output,
        None,
        None,
        Some(types),
        Some(pos_increments),
        Some(pos_lengths),
    )
}

// -----------------------------------------------------------------------------
// TestMultiWordSynonyms
// -----------------------------------------------------------------------------

fn mws_test_multi_word_synonyms() -> Result<()> {
    let stream = B::whitespace_mock_tokenizer(Box::new(reader("a e")))?;
    let mut stream = F::token_filter_factory_with_loader(
        "Synonym",
        Some(Version::LATEST),
        &StringMockResourceLoader::new("a b c,d"),
        &["synonyms", "synonyms.txt"],
    )?
    .create(boxed(stream))?;
    // This fails because ["e","e"] is the value of the token stream
    B::assert_token_stream_contents(&mut *stream, &["a", "e"])
}

fn multi_word_synonyms_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.synonym.TestMultiWordSynonyms")
        .method(t("testMultiWordSynonyms", mws_test_multi_word_synonyms))
        .build()
}

// -----------------------------------------------------------------------------
// TestWordnetSynonymParser
// -----------------------------------------------------------------------------

const WORDNET_SYNONYMS_FILE: &str = "s(100000001,1,'woods',n,1,0).\n\
     s(100000001,2,'wood',n,1,0).\n\
     s(100000001,3,'forest',n,1,0).\n\
     s(100000002,1,'wolfish',n,1,0).\n\
     s(100000002,2,'ravenous',n,1,0).\n\
     s(100000003,1,'king',n,1,1).\n\
     s(100000003,2,'baron',n,1,1).\n\
     s(100000004,1,'king''s evil',n,1,1).\n\
     s(100000004,2,'king''s meany',n,1,1).\n";

fn wn_test_synonyms() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let mut parser = WordnetSynonymParser::new(true, true, Arc::clone(&analyzer));
    parse(&mut parser, WORDNET_SYNONYMS_FILE)?;
    let map = Arc::new(parser.build()?);
    analyzer.close();
    let analyzer = mock_analyzer(
        "org.apache.lucene.analysis.synonym.TestWordnetSynonymParser$1",
        &WHITESPACE,
        false,
        move |t| syn_filter(t, &map, false),
    );
    // all expansions
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "Lost in the woods",
        &["Lost", "in", "the", "woods", "wood", "forest"],
        &[0, 5, 8, 12, 12, 12],
        &[4, 7, 11, 17, 17, 17],
        &[1, 1, 1, 1, 0, 0],
    )?;
    // single quote
    B::assert_analyzes_to(&*analyzer, "king", &["king", "baron"])?;
    // multi words
    B::assert_analyzes_to(
        &*analyzer,
        "king's evil",
        &["king's", "king's", "evil", "meany"],
    )?;
    // all expansions, test types
    B::assert_analyzes_to_types_only(
        &*analyzer,
        "Lost in the forest",
        &["Lost", "in", "the", "forest", "woods", "wood"],
        &["word", "word", "word", "word", "SYNONYM", "SYNONYM"],
    )?;
    analyzer.close();
    Ok(())
}

fn wordnet_synonym_parser_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.synonym.TestWordnetSynonymParser")
        .method(t("testSynonyms", wn_test_synonyms))
        .build()
}

// -----------------------------------------------------------------------------
// TestSolrSynonymParser
// -----------------------------------------------------------------------------

/// `new SolrSynonymParser(true, true, analyzer)` parsed over `text`, built.
fn solr_map(analyzer: Arc<dyn Analyzer>, text: &str) -> Result<Arc<SynonymMap>> {
    let mut parser = SolrSynonymParser::new(true, true, Arc::clone(&analyzer));
    parse(&mut parser, text)?;
    let map = Arc::new(parser.build()?);
    analyzer.close();
    Ok(map)
}

/// `expectThrows(ParseException.class, () -> parser.parse(new
/// StringReader(testFile)))` with a `SolrSynonymParser(true, true,
/// analyzer)`.
fn expect_parse_exception(analyzer: Arc<dyn Analyzer>, test_file: &str) {
    let mut parser = SolrSynonymParser::new(true, true, Arc::clone(&analyzer));
    expect_throws("java.text.ParseException", || parse(&mut parser, test_file));
    analyzer.close();
}

/// Tests some simple examples from the solr wiki
fn ssp_test_simple() -> Result<()> {
    let test_file = "i-pod, ipod, ipoooood\nfoo => foo bar\nfoo => baz\nthis test, that testing";
    let map = solr_map(Arc::new(MockAnalyzer::new(&*random())), test_file)?;
    let analyzer = mock_analyzer(
        "org.apache.lucene.analysis.synonym.TestSolrSynonymParser$1",
        &WHITESPACE,
        true,
        move |t| syn_filter(t, &map, true),
    );
    analyzes_pos(&*analyzer, "ball", &["ball"], &[1])?;
    analyzes_pos(
        &*analyzer,
        "i-pod",
        &["i-pod", "ipod", "ipoooood"],
        &[1, 0, 0],
    )?;
    analyzes_pos(&*analyzer, "foo", &["foo", "baz", "bar"], &[1, 0, 1])?;
    analyzes_pos(
        &*analyzer,
        "this test",
        &["this", "that", "test", "testing"],
        &[1, 0, 1, 0],
    )?;
    analyzer.close();
    Ok(())
}

/// parse a syn file with bad syntax
fn ssp_test_invalid_double_map() -> Result<()> {
    expect_parse_exception(Arc::new(MockAnalyzer::new(&*random())), "a => b => c");
    Ok(())
}

/// parse a syn file with bad syntax
fn ssp_test_invalid_analyzes_to_nothing_output() -> Result<()> {
    expect_parse_exception(
        Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&SIMPLE),
            false,
        )),
        "a => 1",
    );
    Ok(())
}

/// parse a syn file with bad syntax
fn ssp_test_invalid_analyzes_to_nothing_input() -> Result<()> {
    expect_parse_exception(
        Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&SIMPLE),
            false,
        )),
        "1 => a",
    );
    Ok(())
}

/// parse a syn file with bad syntax
fn ssp_test_invalid_positions_input() -> Result<()> {
    expect_parse_exception(Arc::new(EnglishAnalyzer::new()), "testola => the test");
    Ok(())
}

/// parse a syn file with bad syntax
fn ssp_test_invalid_positions_output() -> Result<()> {
    expect_parse_exception(Arc::new(EnglishAnalyzer::new()), "the test => testola");
    Ok(())
}

/// parse a syn file with some escaped syntax chars
fn ssp_test_escaped_stuff() -> Result<()> {
    let test_file = "a\\=>a => b\\=>b\na\\,a => b\\,b";
    let map = solr_map(
        Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&KEYWORD),
            false,
        )),
        test_file,
    )?;
    let analyzer = mock_analyzer(
        "org.apache.lucene.analysis.synonym.TestSolrSynonymParser$2",
        &KEYWORD,
        false,
        move |t| syn_filter(t, &map, false),
    );
    analyzes_pos(&*analyzer, "ball", &["ball"], &[1])?;
    analyzes_pos(&*analyzer, "a=>a", &["b=>b"], &[1])?;
    analyzes_pos(&*analyzer, "a,a", &["b,b"], &[1])?;
    analyzer.close();
    Ok(())
}

/// Verify type of token and positionLength after analyzer.
fn ssp_test_position_length_and_type_simple() -> Result<()> {
    let map = solr_map(
        Arc::new(MockAnalyzer::new(&*random())),
        "spider man, spiderman",
    )?;
    let analyzer = mock_analyzer(
        "org.apache.lucene.analysis.synonym.TestSolrSynonymParser$3",
        &WHITESPACE,
        true,
        move |t| syn_filter(t, &map, true),
    );
    assert_analyzes_to_positions(
        &*analyzer,
        "spider man",
        &["spider", "spiderman", "man"],
        &["word", "SYNONYM", "word"],
        &[1, 0, 1],
        &[1, 2, 1],
    )
}

/// Verify type of original token is "word", others are Synonym.
fn ssp_test_types() -> Result<()> {
    let map = solr_map(
        Arc::new(MockAnalyzer::new(&*random())),
        "woods, wood, forest",
    )?;
    let analyzer = mock_analyzer(
        "org.apache.lucene.analysis.synonym.TestSolrSynonymParser$4",
        &WHITESPACE,
        true,
        move |t| syn_filter(t, &map, true),
    );
    B::assert_analyzes_to_types_only(
        &*analyzer,
        "lost in the forest",
        &["lost", "in", "the", "forest", "woods", "wood"],
        &["word", "word", "word", "word", "SYNONYM", "SYNONYM"],
    )
}

/// Test parsing of simple examples.
fn ssp_test_parse_simple() -> Result<()> {
    let test_file = "spider man, spiderman\n\
                     usa,united states,u s a,united states of america\n\
                     mystyped, mistyped => mistyped\n\
                     foo => foo bar\n\
                     foo => baz";
    let map = solr_map(Arc::new(MockAnalyzer::new(&*random())), test_file)?;
    assert_entry_equals(&map, "spiderman", true, &["spider man"])?;
    assert_entry_equals(&map, "spider man", true, &["spiderman"])?;
    assert_entry_equals(
        &map,
        "usa",
        true,
        &["united states", "u s a", "united states of america"],
    )?;
    assert_entry_equals(
        &map,
        "united states",
        true,
        &["usa", "u s a", "united states of america"],
    )?;
    assert_entry_equals(
        &map,
        "u s a",
        true,
        &["usa", "united states", "united states of america"],
    )?;
    assert_entry_equals(
        &map,
        "united states of america",
        true,
        &["usa", "u s a", "united states"],
    )?;
    assert_entry_equals(&map, "mistyped", false, &["mistyped"])?;
    assert_entry_equals(&map, "mystyped", false, &["mistyped"])?;
    assert_entry_equals(&map, "foo", false, &["foo bar", "baz"])?;
    assert_entry_absent(&map, "baz")?;
    assert_entry_absent(&map, "bar")
}

fn solr_synonym_parser_class() -> Arc<Class> {
    base_synonym_parser_class("org.apache.lucene.analysis.synonym.TestSolrSynonymParser")
        .method(t("testSimple", ssp_test_simple))
        .method(t("testInvalidDoubleMap", ssp_test_invalid_double_map))
        .method(t(
            "testInvalidAnalyzesToNothingOutput",
            ssp_test_invalid_analyzes_to_nothing_output,
        ))
        .method(t(
            "testInvalidAnalyzesToNothingInput",
            ssp_test_invalid_analyzes_to_nothing_input,
        ))
        .method(t(
            "testInvalidPositionsInput",
            ssp_test_invalid_positions_input,
        ))
        .method(t(
            "testInvalidPositionsOutput",
            ssp_test_invalid_positions_output,
        ))
        .method(t("testEscapedStuff", ssp_test_escaped_stuff))
        .method(t(
            "testPositionLengthAndTypeSimple",
            ssp_test_position_length_and_type_simple,
        ))
        .method(t("testTypes", ssp_test_types))
        .method(t("testParseSimple", ssp_test_parse_simple))
        .build()
}

// -----------------------------------------------------------------------------
// TestSynonymMapFilter
// -----------------------------------------------------------------------------

/// The instance fields `tokensIn`, `tokensOut` and the attributes `verify`
/// reads (the attributes are read from `tokens_out` by interface).
struct MapFilterState {
    tokens_in: Arc<AtomicRefCell<MockTokenizer>>,
    tokens_out: SynonymFilter,
}

/// `tokensIn = new MockTokenizer(WHITESPACE, true)`, consumed once over "a",
/// then `tokensOut = new SynonymFilter(tokensIn, b.build(), true)` with the
/// four attributes added.
fn map_filter_state(b: &mut SynonymMapBuilder) -> Result<MapFilterState> {
    let tokens_in = Arc::new(AtomicRefCell::new(MockTokenizer::new(
        Arc::clone(&WHITESPACE),
        true,
    )?));
    {
        let mut t = tokens_in.borrow_mut();
        t.set_reader(Box::new(reader("a")))?;
        t.reset()?;
        Assert::assert_true(t.increment_token()?);
        Assert::assert_false(t.increment_token()?);
        t.end()?;
        t.close()?;
    }
    let mut tokens_out = new_synonym_filter(shared(&tokens_in), Arc::new(b.build()?), true)?;
    {
        let source = tokens_out.attribute_source_mut();
        source.add_attribute_interface::<dyn CharTermAttribute>()?;
        source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
        source.add_attribute_interface::<dyn OffsetAttribute>()?;
    }
    Ok(MapFilterState {
        tokens_in,
        tokens_out,
    })
}

/// The term, position increment, offsets and position length of `ts`.
fn token_state(ts: &dyn TokenStream) -> (String, i32, i32, i32, i32) {
    let src = ts.attribute_source();
    (
        support::term(ts),
        src.get_attribute_interface::<dyn PositionIncrementAttribute>()
            .unwrap()
            .get_position_increment(),
        src.get_attribute_interface::<dyn OffsetAttribute>()
            .unwrap()
            .start_offset(),
        src.get_attribute_interface::<dyn OffsetAttribute>()
            .unwrap()
            .end_offset(),
        src.get_attribute_interface::<dyn PositionLengthAttribute>()
            .unwrap()
            .get_position_length(),
    )
}

// For the output string: separate positions with a space,
// and separate multiple tokens at each position with a
// /.  If a token should have end offset != the input
// token's end offset then add :X to it:
// TODO: we should probably refactor this guy to use/take analyzer,
// the tests are a little messy
fn verify(state: &mut MapFilterState, input: &str, output: &str) -> Result<()> {
    if *VERBOSE {
        println!("TEST: verify input={input} expectedOutput={output}");
    }
    state
        .tokens_in
        .borrow_mut()
        .set_reader(Box::new(reader(input)))?;
    let tokens_out = &mut state.tokens_out;
    tokens_out.reset()?;
    let expected = split_space(output);
    let mut expected_upto = 0;
    while tokens_out.increment_token()? {
        if *VERBOSE {
            let (term, pos_inc, start, end, _) = token_state(tokens_out);
            println!("  incr token={term} posIncr={pos_inc} startOff={start} endOff={end}");
        }
        Assert::assert_true(expected_upto < expected.len());
        let (_, _, start_offset, end_offset, _) = token_state(tokens_out);
        let expected_at_pos: Vec<&str> = expected[expected_upto].split('/').collect();
        expected_upto += 1;
        for (at_pos, exp) in expected_at_pos.iter().enumerate() {
            if at_pos > 0 {
                Assert::assert_true(tokens_out.increment_token()?);
                if *VERBOSE {
                    let (term, pos_inc, start, end, _) = token_state(tokens_out);
                    println!("  incr token={term} posIncr={pos_inc} startOff={start} endOff={end}");
                }
            }
            let colon_index = exp.find(':');
            let underbar_index = exp.find('_');
            let (expected_token, expected_end_offset, expected_pos_len) = match colon_index {
                Some(colon) => {
                    let token = &exp[..colon];
                    match underbar_index {
                        Some(underbar) => (
                            token,
                            exp[1 + colon..underbar].parse::<i32>().unwrap(),
                            exp[1 + underbar..].parse::<i32>().unwrap(),
                        ),
                        None => (token, exp[1 + colon..].parse::<i32>().unwrap(), 1),
                    }
                }
                None => (*exp, end_offset, 1),
            };
            let (term, pos_inc, start, end, pos_len) = token_state(tokens_out);
            Assert::assert_equals(expected_token, term.as_str());
            Assert::assert_equals(if at_pos == 0 { 1 } else { 0 }, pos_inc);
            // start/end offset of all tokens at same pos should
            // be the same:
            Assert::assert_equals(start_offset, start);
            Assert::assert_equals(expected_end_offset, end);
            Assert::assert_equals(expected_pos_len, pos_len);
        }
    }
    tokens_out.end()?;
    tokens_out.close()?;
    if *VERBOSE {
        println!("  incr: END");
    }
    Assert::assert_equals(expected_upto, expected.len());
    Ok(())
}

fn smf_test_dont_keep_orig() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "a b", "foo", false)?;
    let map = Arc::new(b.build()?);
    let analyzer = mock_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$1",
        &SIMPLE,
        true,
        move |t| syn_filter(t, &map, false),
    );
    analyzes_graph(
        &*analyzer,
        "a b c",
        &["foo", "c"],
        &[0, 4],
        &[3, 5],
        &[1, 1],
        &[1, 1],
        true,
    )?;
    B::check_analysis_consistency(&random(), &*analyzer, false, "a b c")?;
    analyzer.close();
    Ok(())
}

fn smf_test_do_keep_orig() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "a b", "foo", true)?;
    let map = Arc::new(b.build()?);
    let analyzer = mock_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$2",
        &SIMPLE,
        true,
        move |t| syn_filter(t, &map, false),
    );
    analyzes_graph(
        &*analyzer,
        "a b c",
        &["a", "foo", "b", "c"],
        &[0, 0, 2, 4],
        &[1, 3, 3, 5],
        &[1, 0, 1, 1],
        &[1, 2, 1, 1],
        true,
    )?;
    B::check_analysis_consistency(&random(), &*analyzer, false, "a b c")?;
    analyzer.close();
    Ok(())
}

fn smf_test_basic() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "a", "foo", true)?;
    add(&mut b, "a b", "bar fee", true)?;
    add(&mut b, "b c", "dog collar", true)?;
    add(&mut b, "c d", "dog harness holder extras", true)?;
    add(&mut b, "m c e", "dog barks loudly", false)?;
    add(&mut b, "i j k", "feep", true)?;
    add(&mut b, "e f", "foo bar", false)?;
    add(&mut b, "e f", "baz bee", false)?;
    add(&mut b, "z", "boo", false)?;
    add(&mut b, "y", "bee", true)?;
    let mut s = map_filter_state(&mut b)?;
    verify(&mut s, "a b c", "a/bar b/fee c")?;
    // syn output extends beyond input tokens
    verify(
        &mut s,
        "x a b c d",
        "x a/bar b/fee c/dog d/harness holder extras",
    )?;
    verify(&mut s, "a b a", "a/bar b/fee a/foo")?;
    // outputs that add to one another:
    verify(
        &mut s,
        "c d c d",
        "c/dog d/harness c/holder/dog d/extras/harness holder extras",
    )?;
    // two outputs for same input
    verify(&mut s, "e f", "foo/baz bar/bee")?;
    // verify multi-word / single-output offsets:
    verify(&mut s, "g i j k g", "g i/feep:7_3 j k g")?;
    // mixed keepOrig true/false:
    verify(&mut s, "a m c e x", "a/foo dog barks loudly x")?;
    verify(
        &mut s,
        "c d m c e x",
        "c/dog d/harness holder/dog extras/barks loudly x",
    )?;
    Assert::assert_true(s.tokens_out.logic().get_capture_count() > 0);
    // no captureStates when no syns matched
    verify(&mut s, "p q r s t", "p q r s t")?;
    Assert::assert_equals(0, s.tokens_out.logic().get_capture_count());
    // no captureStates when only single-input syns, w/ no
    // lookahead needed, matched
    verify(&mut s, "p q z y t", "p q boo y/bee t")?;
    Assert::assert_equals(0, s.tokens_out.logic().get_capture_count());
    Ok(())
}

/// `getRandomString(start, alphabetSize, length)`.
fn get_random_string(start: u8, alphabet_size: i32, length: i32) -> String {
    assert!(alphabet_size <= 26);
    let mut s = String::with_capacity(2 * length as usize);
    for _ in 0..length {
        s.push((start + random().next_int_bound(alphabet_size) as u8) as char);
        s.push(' ');
    }
    s
}

/// `OneSyn` of `TestSynonymMapFilter`.
#[derive(Debug, Clone)]
struct MapOneSyn {
    in_: String,
    out: Vec<String>,
    keep_orig: bool,
}

// The loops and arithmetic are the Java test's, kept as written.
#[allow(
    clippy::manual_div_ceil,
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::manual_is_multiple_of
)]
fn slow_syn_matcher(doc: &str, syns: &[MapOneSyn], max_output_length: usize) -> String {
    Assert::assert_true(doc.len() % 2 == 0);
    let num_inputs = doc.len() / 2;
    let mut keep_origs = vec![false; num_inputs];
    let mut has_match = vec![false; num_inputs];
    let mut outputs: Vec<Option<String>> = vec![None; num_inputs + max_output_length];
    let mut matches: Vec<Option<usize>> = vec![None; num_inputs];
    for (syn_index, syn) in syns.iter().enumerate() {
        let mut idx: isize = -1;
        loop {
            let from = (1 + idx) as usize;
            match doc.get(from..).and_then(|rest| rest.find(syn.in_.as_str())) {
                None => break,
                Some(found) => idx = (from + found) as isize,
            }
            let idx_u = idx as usize;
            Assert::assert_true(idx_u % 2 == 0);
            let match_idx = idx_u / 2;
            Assert::assert_true(syn.in_.len() % 2 == 1);
            match matches[match_idx] {
                None => matches[match_idx] = Some(syn_index),
                Some(m) if syn.in_.len() > syns[m].in_.len() => {
                    // Greedy conflict resolution: longer match wins:
                    matches[match_idx] = Some(syn_index);
                }
                Some(m) => Assert::assert_true(syn.in_.len() < syns[m].in_.len()),
            }
        }
    }
    // Greedy conflict resolution: if syn matches a range of inputs,
    // it prevents other syns from matching that range
    for input_idx in 0..num_inputs {
        if let Some(m) = matches[input_idx] {
            let syn_in_length = (1 + syns[m].in_.len()) / 2;
            let mut next_input_idx = input_idx + 1;
            while next_input_idx < num_inputs && next_input_idx < input_idx + syn_in_length {
                matches[next_input_idx] = None;
                next_input_idx += 1;
            }
        }
    }
    // Fill overlapping outputs:
    for input_idx in 0..num_inputs {
        let Some(m) = matches[input_idx] else {
            continue;
        };
        let syn = &syns[m];
        for idx in 0..(1 + syn.in_.len()) / 2 {
            has_match[input_idx + idx] = true;
            keep_origs[input_idx + idx] |= syn.keep_orig;
        }
        for syn_out in &syn.out {
            let syn_outputs = split_space(syn_out);
            Assert::assert_equals(syn_outputs.len(), (1 + syn_out.len()) / 2);
            let match_end = input_idx + syn_outputs.len();
            let mut syn_upto = 0;
            for match_idx in input_idx..match_end {
                outputs[match_idx] = Some(match &outputs[match_idx] {
                    None => syn_outputs[syn_upto].to_string(),
                    Some(o) => format!("{o}/{}", syn_outputs[syn_upto]),
                });
                syn_upto += 1;
                if match_idx < num_inputs {
                    let (end_offset, pos_len) = if syn_outputs.len() == 1 {
                        // Add full endOffset
                        (
                            (input_idx * 2) + syn.in_.len(),
                            if syn.keep_orig {
                                (1 + syn.in_.len()) / 2
                            } else {
                                1
                            },
                        )
                    } else {
                        // Add endOffset matching input token's
                        ((match_idx * 2) + 1, 1)
                    };
                    let o = outputs[match_idx].take().unwrap();
                    outputs[match_idx] = Some(format!("{o}:{end_offset}_{pos_len}"));
                }
            }
        }
    }
    let mut sb = String::new();
    let input_tokens = split_space(doc);
    let limit = input_tokens.len() + max_output_length;
    for input_idx in 0..limit {
        let mut pos_has_output = false;
        if input_idx >= num_inputs && outputs[input_idx].is_none() {
            break;
        }
        if input_idx < num_inputs && (!has_match[input_idx] || keep_origs[input_idx]) {
            Assert::assert_true(!input_tokens[input_idx].is_empty());
            sb.push_str(input_tokens[input_idx]);
            pos_has_output = true;
        }
        if let Some(o) = &outputs[input_idx] {
            if pos_has_output {
                sb.push('/');
            }
            sb.push_str(o);
        } else if !pos_has_output {
            continue;
        }
        if input_idx < limit - 1 {
            sb.push(' ');
        }
    }
    sb
}

fn smf_test_random() -> Result<()> {
    let alphabet_size = TestUtil::next_int(&*random(), 2, 7);
    let doc_len = LuceneTestCase::at_least(3000);
    // final int docLen = 50;
    let document = get_random_string(b'a', alphabet_size, doc_len);
    if *VERBOSE {
        println!("TEST: doc={document}");
    }
    let num_syn = LuceneTestCase::at_least(5);
    // final int numSyn = 2;
    let mut syn_map: JavaHashMap<usize> = JavaHashMap::new();
    let mut syns: Vec<MapOneSyn> = Vec::new();
    let dedup = random().next_boolean();
    if *VERBOSE {
        println!("  dedup={dedup}");
    }
    let mut b = SynonymMapBuilder::with_dedup(dedup);
    for syn_idx in 0..num_syn {
        let syn_in = get_random_string(b'a', alphabet_size, TestUtil::next_int(&*random(), 1, 5));
        let syn_in = java_trim(&syn_in).to_string();
        let s = match syn_map.get(&syn_in) {
            Some(&s) => s,
            None => {
                syns.push(MapOneSyn {
                    in_: syn_in.clone(),
                    out: Vec::new(),
                    keep_orig: false,
                });
                let s = syns.len() - 1;
                syn_map.insert(syn_in.clone(), s);
                syns[s].keep_orig = random().next_boolean();
                s
            }
        };
        let syn_out = get_random_string(b'0', 10, TestUtil::next_int(&*random(), 1, 5));
        let syn_out = java_trim(&syn_out).to_string();
        syns[s].out.push(syn_out.clone());
        add(&mut b, &syn_in, &syn_out, syns[s].keep_orig)?;
        if *VERBOSE {
            println!(
                "  syns[{syn_idx}] = {} -> {:?} keepOrig={}",
                syns[s].in_, syns[s].out, syns[s].keep_orig
            );
        }
    }
    let mut state = map_filter_state(&mut b)?;
    if dedup {
        prune_dups(&mut syns);
    }
    let expected = slow_syn_matcher(&document, &syns, 5);
    if *VERBOSE {
        println!("TEST: expected={expected}");
    }
    verify(&mut state, &document, &expected)
}

fn prune_dups(syns: &mut [MapOneSyn]) {
    let mut seen: HashSet<String> = HashSet::new();
    for syn in syns.iter_mut() {
        let mut idx = 0;
        while idx < syn.out.len() {
            let out = syn.out[idx].clone();
            if !seen.contains(&out) {
                seen.insert(out);
                idx += 1;
            } else {
                syn.out.remove(idx);
            }
        }
        seen.clear();
    }
}

/// A builder with `atLeast(10)` random rules, `dedup` at random.
fn random_builder() -> Result<SynonymMapBuilder> {
    let mut b = SynonymMapBuilder::with_dedup(random().next_boolean());
    let num_entries = LuceneTestCase::at_least(10);
    for _ in 0..num_entries {
        let input = random_non_empty_string();
        let output = random_non_empty_string();
        add(&mut b, &input, &output, random().next_boolean())?;
    }
    Ok(b)
}

/// simple random test, doesn't verify correctness. does verify it doesnt
/// throw exceptions, or that the stream doesn't misbehave
fn smf_test_random2() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(3);
    for _ in 0..num_iters {
        let map = Arc::new(random_builder()?.build()?);
        let ignore_case = random().next_boolean();
        let analyzer = mock_analyzer(
            "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$3",
            &SIMPLE,
            true,
            move |t| syn_filter(t, &map, ignore_case),
        );
        B::check_random_data(&*random(), &analyzer, 100)?;
        analyzer.close();
    }
    Ok(())
}

// Adds MockGraphTokenFilter after SynFilter:
fn smf_test_random2_graph_after() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(3);
    let random = random();
    for _ in 0..num_iters {
        let mut b = SynonymMapBuilder::with_dedup(random.next_boolean());
        let num_entries = LuceneTestCase::at_least(10);
        for _ in 0..num_entries {
            let input = random_non_empty_string();
            let output = random_non_empty_string();
            add(&mut b, &input, &output, random.next_boolean())?;
        }
        let map = Arc::new(b.build()?);
        let ignore_case = random.next_boolean();
        let analyzer = mock_analyzer(
            "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$4",
            &SIMPLE,
            true,
            move |t| {
                let syns = syn_filter(t, &map, ignore_case)?;
                Ok(boxed(MockGraphTokenFilter::new(
                    &*LuceneTestCase::random(),
                    syns,
                )?))
            },
        );
        B::check_random_data(&*random, &analyzer, 100)?;
        analyzer.close();
    }
    Ok(())
}

fn smf_test_empty_term() -> Result<()> {
    let random = random();
    let num_iters = LuceneTestCase::at_least(10);
    for _ in 0..num_iters {
        let mut b = SynonymMapBuilder::with_dedup(random.next_boolean());
        let num_entries = LuceneTestCase::at_least(10);
        for _ in 0..num_entries {
            let input = random_non_empty_string();
            let output = random_non_empty_string();
            add(&mut b, &input, &output, random.next_boolean())?;
        }
        let map = Arc::new(b.build()?);
        let ignore_case = random.next_boolean();
        let analyzer = support::keyword_tokenizer_analyzer(
            "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$5",
            move |t| syn_filter(t, &map, ignore_case),
        );
        let use_char_filter = random.next_boolean();
        B::check_analysis_consistency(&random, &*analyzer, use_char_filter, "")?;
        analyzer.close();
    }
    Ok(())
}

/// simple random test like testRandom2, but for larger docs
fn smf_test_random_huge() -> Result<()> {
    let random = random();
    let num_iters = LuceneTestCase::at_least(3);
    for i in 0..num_iters {
        let mut b = SynonymMapBuilder::with_dedup(random.next_boolean());
        let num_entries = LuceneTestCase::at_least(10);
        if *VERBOSE {
            println!("TEST: iter={i} numEntries={num_entries}");
        }
        for _ in 0..num_entries {
            let input = random_non_empty_string();
            let output = random_non_empty_string();
            add(&mut b, &input, &output, random.next_boolean())?;
        }
        let map = Arc::new(b.build()?);
        let ignore_case = random.next_boolean();
        let analyzer = FnAnalyzer::new(
            "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$6",
            move |_| {
                let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::with_max_token_length(
                    Arc::clone(&SIMPLE),
                    true,
                    (MAX_TERM_LENGTH / 2) as i32,
                )?));
                let filter = syn_filter(shared(&tokenizer), &map, ignore_case)?;
                Ok(components(tokenizer, sink(BoxedStream(filter))))
            },
        )
        .arc();
        B::check_random_data_max_word_length(&*random, &analyzer, 100, 1024)?;
        analyzer.close();
    }
    Ok(())
}

// LUCENE-3375
fn smf_test_vanishing_terms() -> Result<()> {
    let test_file = "aaa => aaaa1 aaaa2 aaaa3\nbbb => bbbb1 bbbb2\n";
    let map = solr_map(Arc::new(MockAnalyzer::new(&*random())), test_file)?;
    let analyzer = mock_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$7",
        &WHITESPACE,
        true,
        move |t| syn_filter(t, &map, true),
    );
    // where did my pot go?!
    B::assert_analyzes_to(
        &*analyzer,
        "xyzzy bbb pot of gold",
        &["xyzzy", "bbbb1", "pot", "bbbb2", "of", "gold"],
    )?;
    // this one nukes 'pot' and 'of'
    // xyzzy aaa pot of gold -> xyzzy aaaa1 aaaa2 aaaa3 gold
    B::assert_analyzes_to(
        &*analyzer,
        "xyzzy aaa pot of gold",
        &["xyzzy", "aaaa1", "pot", "aaaa2", "of", "aaaa3", "gold"],
    )?;
    analyzer.close();
    Ok(())
}

fn smf_test_basic2() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    let keep_orig = false;
    add(&mut b, "aaa", "aaaa1 aaaa2 aaaa3", keep_orig)?;
    add(&mut b, "bbb", "bbbb1 bbbb2", keep_orig)?;
    let mut s = map_filter_state(&mut b)?;
    #[allow(clippy::if_same_then_else)]
    if keep_orig {
        verify(
            &mut s,
            "xyzzy bbb pot of gold",
            "xyzzy bbb/bbbb1 pot/bbbb2 of gold",
        )?;
        verify(
            &mut s,
            "xyzzy aaa pot of gold",
            "xyzzy aaa/aaaa1 pot/aaaa2 of/aaaa3 gold",
        )?;
    } else {
        verify(
            &mut s,
            "xyzzy bbb pot of gold",
            "xyzzy bbbb1 pot/bbbb2 of gold",
        )?;
        verify(
            &mut s,
            "xyzzy aaa pot of gold",
            "xyzzy aaaa1 pot/aaaa2 of/aaaa3 gold",
        )?;
    }
    Ok(())
}

/// The whitespace analyzer over `new SynonymFilter(tokenizer, map, true)`
/// most `TestSynonymMapFilter` tests build.
fn ws_syn_analyzer(name: &'static str, b: &mut SynonymMapBuilder) -> Result<Arc<dyn Analyzer>> {
    let map = Arc::new(b.build()?);
    Ok(mock_analyzer(name, &WHITESPACE, false, move |t| {
        syn_filter(t, &map, true)
    }))
}

/// The rules `testMatching` and `testIncludeOrig` add.
fn add_matching_rules(b: &mut SynonymMapBuilder, keep_orig: bool) -> Result<()> {
    add(b, "a b", "ab", keep_orig)?;
    add(b, "a c", "ac", keep_orig)?;
    add(b, "a", "aa", keep_orig)?;
    add(b, "b", "bb", keep_orig)?;
    add(b, "z x c v", "zxcv", keep_orig)?;
    add(b, "x c", "xc", keep_orig)
}

/// The assertions of `testMatching` (both filters).
fn assert_matching(a: &dyn Analyzer) -> Result<()> {
    B::check_one_term(a, "$", "$")?;
    B::check_one_term(a, "a", "aa")?;
    B::check_one_term(a, "b", "bb")?;
    analyzes_pos(a, "a $", &["aa", "$"], &[1, 1])?;
    analyzes_pos(a, "$ a", &["$", "aa"], &[1, 1])?;
    analyzes_pos(a, "a a", &["aa", "aa"], &[1, 1])?;
    analyzes_pos(a, "z x c v", &["zxcv"], &[1])?;
    analyzes_pos(a, "z x c $", &["z", "xc", "$"], &[1, 1, 1])
}

fn smf_test_matching() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add_matching_rules(&mut b, false)?;
    let a = ws_syn_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$8",
        &mut b,
    )?;
    assert_matching(&*a)?;
    a.close();
    Ok(())
}

fn smf_test_repeats_off() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    for _ in 0..3 {
        add(&mut b, "a b", "ab", false)?;
    }
    let a = ws_syn_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$9",
        &mut b,
    )?;
    analyzes_pos(&*a, "a b", &["ab"], &[1])?;
    a.close();
    Ok(())
}

fn smf_test_repeats_on() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(false);
    for _ in 0..3 {
        add(&mut b, "a b", "ab", false)?;
    }
    let a = ws_syn_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$10",
        &mut b,
    )?;
    analyzes_pos(&*a, "a b", &["ab", "ab", "ab"], &[1, 0, 0])?;
    a.close();
    Ok(())
}

fn smf_test_recursion() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "zoo", "zoo", false)?;
    let a = ws_syn_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$11",
        &mut b,
    )?;
    analyzes_pos(
        &*a,
        "zoo zoo $ zoo",
        &["zoo", "zoo", "$", "zoo"],
        &[1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn smf_test_recursion2() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "zoo", "zoo", false)?;
    add(&mut b, "zoo", "zoo zoo", false)?;
    let a = ws_syn_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$12",
        &mut b,
    )?;
    // verify("zoo zoo $ zoo", "zoo/zoo zoo/zoo/zoo $/zoo zoo/zoo zoo");
    analyzes_pos(
        &*a,
        "zoo zoo $ zoo",
        &[
            "zoo", "zoo", "zoo", "zoo", "zoo", "$", "zoo", "zoo", "zoo", "zoo",
        ],
        &[1, 0, 1, 0, 0, 1, 0, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

fn smf_test_output_hangs_off_end() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    // b hangs off the end (no input token under it):
    add(&mut b, "a", "a b", false)?;
    let mut s = map_filter_state(&mut b)?;
    // Make sure endOffset inherits from previous input token:
    verify(&mut s, "a", "a b:1")
}

fn smf_test_include_orig() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add_matching_rules(&mut b, true)?;
    let a = ws_syn_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$13",
        &mut b,
    )?;
    analyzes_pos(&*a, "$", &["$"], &[1])?;
    analyzes_pos(&*a, "a", &["a", "aa"], &[1, 0])?;
    analyzes_pos(&*a, "a", &["a", "aa"], &[1, 0])?;
    analyzes_pos(&*a, "$ a", &["$", "a", "aa"], &[1, 1, 0])?;
    analyzes_pos(&*a, "a $", &["a", "aa", "$"], &[1, 0, 1])?;
    analyzes_pos(&*a, "$ a !", &["$", "a", "aa", "!"], &[1, 1, 0, 1])?;
    analyzes_pos(&*a, "a a", &["a", "aa", "a", "aa"], &[1, 0, 1, 0])?;
    analyzes_pos(&*a, "b", &["b", "bb"], &[1, 0])?;
    analyzes_pos(
        &*a,
        "z x c v",
        &["z", "zxcv", "x", "c", "v"],
        &[1, 0, 1, 1, 1],
    )?;
    analyzes_pos(
        &*a,
        "z x c $",
        &["z", "x", "xc", "c", "$"],
        &[1, 1, 0, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn smf_test_recursion3() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "zoo zoo", "zoo", true)?;
    let a = ws_syn_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$14",
        &mut b,
    )?;
    analyzes_pos(
        &*a,
        "zoo zoo $ zoo",
        &["zoo", "zoo", "zoo", "$", "zoo"],
        &[1, 0, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn smf_test_recursion4() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "zoo zoo", "zoo", true)?;
    add(&mut b, "zoo", "zoo zoo", true)?;
    let a = ws_syn_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$15",
        &mut b,
    )?;
    analyzes_pos(
        &*a,
        "zoo zoo $ zoo",
        &["zoo", "zoo", "zoo", "$", "zoo", "zoo", "zoo"],
        &[1, 0, 1, 1, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

fn smf_test_multiword_offsets() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "national hockey league", "nhl", true)?;
    let a = ws_syn_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymMapFilter$16",
        &mut b,
    )?;
    B::assert_analyzes_to_offsets_pos_increments(
        &*a,
        "national hockey league",
        &["national", "nhl", "hockey", "league"],
        &[0, 0, 9, 16],
        &[8, 22, 15, 22],
        &[1, 0, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn smf_test_empty() -> Result<()> {
    // `new MockTokenizer()`: `MockTokenizer(WHITESPACE, true)`.
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), true)?;
    tokenizer.set_reader(Box::new(reader("aa bb")))?;
    let tokenizer = Mutex::new(Some(tokenizer));
    let expected = iae_message(|| {
        new_synonym_filter(
            boxed(tokenizer.lock().unwrap().take().unwrap()),
            Arc::new(SynonymMapBuilder::with_dedup(true).build()?),
            true,
        )
    });
    Assert::assert_equals("fst must be non-null", expected.as_str());
    Ok(())
}

fn synonym_map_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.synonym.TestSynonymMapFilter")
        .method(t("testDontKeepOrig", smf_test_dont_keep_orig))
        .method(t("testDoKeepOrig", smf_test_do_keep_orig))
        .method(t("testBasic", smf_test_basic))
        .method(t("testRandom", smf_test_random))
        .method(t("testRandom2", smf_test_random2))
        .method(t("testRandom2GraphAfter", smf_test_random2_graph_after))
        .method(t("testEmptyTerm", smf_test_empty_term))
        .method(t("testRandomHuge", smf_test_random_huge))
        .method(t("testVanishingTerms", smf_test_vanishing_terms))
        .method(t("testBasic2", smf_test_basic2))
        .method(t("testMatching", smf_test_matching))
        .method(t("testRepeatsOff", smf_test_repeats_off))
        .method(t("testRepeatsOn", smf_test_repeats_on))
        .method(t("testRecursion", smf_test_recursion))
        .method(t("testRecursion2", smf_test_recursion2))
        .method(t("testOutputHangsOffEnd", smf_test_output_hangs_off_end))
        .method(t("testIncludeOrig", smf_test_include_orig))
        .method(t("testRecursion3", smf_test_recursion3))
        .method(t("testRecursion4", smf_test_recursion4))
        .method(t("testMultiwordOffsets", smf_test_multiword_offsets))
        .method(t("testEmpty", smf_test_empty))
        .build()
}

// -----------------------------------------------------------------------------
// TestSynonymGraphFilter
// -----------------------------------------------------------------------------

/// The instance fields `synFilter` and `flattenFilter` that `getAnalyzer`
/// and `getFlattenAnalyzer` set as a side effect (the filters themselves,
/// shared with the components that use them).
type SharedSyn = Arc<AtomicRefCell<SynonymGraphFilter>>;
type SharedFlatten = Arc<AtomicRefCell<FlattenGraphFilter>>;

static SYN_FILTER: Mutex<Option<SharedSyn>> = Mutex::new(None);
static FLATTEN_FILTER: Mutex<Option<SharedFlatten>> = Mutex::new(None);

fn syn_filter_field() -> SharedSyn {
    Arc::clone(SYN_FILTER.lock().unwrap().as_ref().expect("synFilter"))
}

fn flatten_filter_field() -> SharedFlatten {
    Arc::clone(
        FLATTEN_FILTER
            .lock()
            .unwrap()
            .as_ref()
            .expect("flattenFilter"),
    )
}

/// `getAnalyzer(b, ignoreCase)`: `map` is `b.build()`.
fn get_analyzer(map: SynonymMap, ignore_case: bool) -> Arc<dyn Analyzer> {
    let map = Arc::new(map);
    FnAnalyzer::new(
        "org.apache.lucene.analysis.synonym.TestSynonymGraphFilter$1",
        move |_| {
            let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::with_max_token_length(
                Arc::clone(&WHITESPACE),
                false,
                (MAX_TERM_LENGTH / 2) as i32,
            )?));
            // Make a local variable so testRandomHuge doesn't share it across threads!
            let syn_filter: SharedSyn = Arc::new(AtomicRefCell::new(new_synonym_graph_filter(
                shared(&tokenizer),
                Arc::clone(&map),
                ignore_case,
            )?));
            *FLATTEN_FILTER.lock().unwrap() = None;
            *SYN_FILTER.lock().unwrap() = Some(Arc::clone(&syn_filter));
            Ok(TokenStreamComponents::new(
                Box::new(move |reader| tokenizer.borrow_mut().set_reader(reader)),
                syn_filter,
            ))
        },
    )
    .arc()
}

/// Appends FlattenGraphFilter too (`getFlattenAnalyzer(b, ignoreCase)`;
/// `map` is `b.build()`).
fn get_flatten_analyzer(map: SynonymMap, ignore_case: bool) -> Arc<dyn Analyzer> {
    let map = Arc::new(map);
    FnAnalyzer::new(
        "org.apache.lucene.analysis.synonym.TestSynonymGraphFilter$2",
        move |_| {
            let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::with_max_token_length(
                Arc::clone(&WHITESPACE),
                true,
                (MAX_TERM_LENGTH / 2) as i32,
            )?));
            // Make a local variable so testRandomHuge doesn't share it across threads!
            let syn_filter: SharedSyn = Arc::new(AtomicRefCell::new(new_synonym_graph_filter(
                shared(&tokenizer),
                Arc::clone(&map),
                ignore_case,
            )?));
            let syn_input: Arc<AtomicRefCell<dyn TokenStream>> = syn_filter.clone();
            let flatten_filter: SharedFlatten = Arc::new(AtomicRefCell::new(
                new_flatten_graph_filter(Box::new(SharedTokenStream::new(syn_input)))?,
            ));
            *SYN_FILTER.lock().unwrap() = Some(syn_filter);
            *FLATTEN_FILTER.lock().unwrap() = Some(Arc::clone(&flatten_filter));
            Ok(TokenStreamComponents::new(
                Box::new(move |reader| tokenizer.borrow_mut().set_reader(reader)),
                flatten_filter,
            ))
        },
    )
    .arc()
}

/// `new SynonymMap.Builder()` (dedup).
fn new_builder() -> SynonymMapBuilder {
    SynonymMapBuilder::new()
}

fn sgf_test_basic_keep_orig_one_output() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "x", true)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c a b",
        &["c", "x", "a", "b"],
        &[0, 2, 2, 4],
        &[1, 5, 3, 5],
        Some(&["word", "SYNONYM", "word", "word"]),
        &[1, 1, 0, 1],
        &[1, 2, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_mixed_keep_orig() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "x", true)?;
    add(&mut b, "e f", "y", false)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c a b c e f g",
        &["c", "x", "a", "b", "c", "y", "g"],
        &[0, 2, 2, 4, 6, 8, 12],
        &[1, 5, 3, 5, 7, 11, 13],
        Some(&["word", "SYNONYM", "word", "word", "word", "SYNONYM", "word"]),
        &[1, 1, 0, 1, 1, 1, 1],
        &[1, 2, 1, 1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_no_parse_after_buffer() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "b a", "x", true)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "b b b",
        &["b", "b", "b"],
        &[0, 2, 4],
        &[1, 3, 5],
        Some(&["word", "word", "word"]),
        &[1, 1, 1],
        &[1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_one_input_multiple_output_keep_orig() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "x", true)?;
    add(&mut b, "a b", "y", true)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c a b c",
        &["c", "x", "y", "a", "b", "c"],
        &[0, 2, 2, 2, 4, 6],
        &[1, 5, 5, 3, 5, 7],
        Some(&["word", "SYNONYM", "SYNONYM", "word", "word", "word"]),
        &[1, 1, 0, 0, 1, 1, 1, 1],
        &[1, 2, 2, 1, 1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

/// Verify type of token and positionLength after analyzer.
fn sgf_test_position_length_and_type_simple() -> Result<()> {
    let analyzer = solr_syns_to_analyzer("spider man, spiderman")?;
    assert_analyzes_to_positions(
        &*analyzer,
        "spider man",
        &["spiderman", "spider", "man"],
        &["SYNONYM", "word", "word"],
        &[1, 0, 1],
        &[2, 1, 1],
    )
}

/// parse a syn file with some escaped syntax chars
fn sgf_test_escaped_stuff() -> Result<()> {
    let analyzer = solr_syns_to_analyzer("a\\=>a => b\\=>b\na\\,a => b\\,b")?;
    analyzes_pos(&*analyzer, "ball", &["ball"], &[1])?;
    analyzes_pos(&*analyzer, "a=>a", &["b=>b"], &[1])?;
    analyzes_pos(&*analyzer, "a,a", &["b,b"], &[1])?;
    analyzer.close();
    Ok(())
}

/// parse a syn file with bad syntax
fn sgf_test_invalid_analyzes_to_nothing_output() -> Result<()> {
    expect_parse_exception(
        Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&SIMPLE),
            false,
        )),
        "a => 1",
    );
    Ok(())
}

/// parse a syn file with bad syntax
fn sgf_test_invalid_double_map() -> Result<()> {
    expect_parse_exception(Arc::new(MockAnalyzer::new(&*random())), "a => b => c");
    Ok(())
}

/// Tests some simple examples from the solr wiki
fn sgf_test_simple() -> Result<()> {
    let test_file = "i-pod, ipod, ipoooood\nfoo => foo bar\nfoo => baz\nthis test, that testing";
    let analyzer = solr_syns_to_analyzer(test_file)?;
    analyzes_pos(&*analyzer, "ball", &["ball"], &[1])?;
    analyzes_pos(
        &*analyzer,
        "i-pod",
        &["ipod", "ipoooood", "i-pod"],
        &[1, 0, 0],
    )?;
    analyzes_pos(&*analyzer, "foo", &["foo", "baz", "bar"], &[1, 0, 1])?;
    analyzes_pos(
        &*analyzer,
        "this test",
        &["that", "this", "testing", "test"],
        &[1, 0, 1, 0],
    )?;
    analyzer.close();
    Ok(())
}

fn sgf_test_buffer_length() -> Result<()> {
    let test_file = "c => 8 2 5 6 7\n\
                     f c e d f, 1\n\
                     c g a f d, 6 5 5\n\
                     e c => 4\n\
                     g => 5\n\
                     a g b f e => 5 0 7 7\n\
                     b => 1";
    let analyzer = solr_syns_to_analyzer(test_file)?;
    let doc = "b c g a f b d";
    let expected = ["1", "8", "2", "5", "6", "7", "5", "a", "f", "1", "d"];
    B::assert_analyzes_to(&*analyzer, doc, &expected)
}

/// `solrSynsToAnalyzer(syns)`.
fn solr_syns_to_analyzer(syns: &str) -> Result<Arc<dyn Analyzer>> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let mut parser = SolrSynonymParser::new(true, true, Arc::clone(&analyzer));
    parse(&mut parser, syns)?;
    analyzer.close();
    Ok(get_flatten_analyzer(parser.build()?, true))
}

fn sgf_test_more_than_one_look_ahead() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b c d", "x", true)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "a b c e",
        &["a", "b", "c", "e"],
        &[0, 2, 4, 6],
        &[1, 3, 5, 7],
        Some(&["word", "word", "word", "word"]),
        &[1, 1, 1, 1],
        &[1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_lookahead_after_parse() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "b b", "x", true)?;
    add(&mut b, "b", "y", true)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_graph(
        &*a,
        "b a b b",
        &["y", "b", "a", "x", "b", "b"],
        &[0, 0, 2, 4, 4, 6],
        &[1, 1, 3, 7, 5, 7],
        &[1, 0, 1, 1, 0, 1],
        &[1, 1, 1, 2, 1, 1],
        true,
    )
}

fn sgf_test_lookahead_second_parse() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "b b b", "x", true)?;
    add(&mut b, "b", "y", true)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_graph(
        &*a,
        "b b",
        &["y", "b", "y", "b"],
        &[0, 0, 2, 2],
        &[1, 1, 3, 3],
        &[1, 0, 1, 0],
        &[1, 1, 1, 1],
        true,
    )
}

fn sgf_test_one_input_multiple_output_no_keep_orig() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "x", false)?;
    add(&mut b, "a b", "y", false)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c a b c",
        &["c", "x", "y", "c"],
        &[0, 2, 2, 6],
        &[1, 5, 5, 7],
        Some(&["word", "SYNONYM", "SYNONYM", "word"]),
        &[1, 1, 0, 1],
        &[1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_one_input_multiple_output_mixed_keep_orig() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "x", true)?;
    add(&mut b, "a b", "y", false)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c a b c",
        &["c", "x", "y", "a", "b", "c"],
        &[0, 2, 2, 2, 4, 6],
        &[1, 5, 5, 3, 5, 7],
        Some(&["word", "SYNONYM", "SYNONYM", "word", "word", "word"]),
        &[1, 1, 0, 0, 1, 1, 1, 1],
        &[1, 2, 2, 1, 1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_syn_at_end() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "x", true)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c d e a b",
        &["c", "d", "e", "x", "a", "b"],
        &[0, 2, 4, 6, 6, 8],
        &[1, 3, 5, 9, 7, 9],
        Some(&["word", "word", "word", "SYNONYM", "word", "word"]),
        &[1, 1, 1, 1, 0, 1],
        &[1, 1, 1, 2, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_two_syns_in_a_row() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a", "x", false)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c a a b",
        &["c", "x", "x", "b"],
        &[0, 2, 4, 6],
        &[1, 3, 5, 7],
        Some(&["word", "SYNONYM", "SYNONYM", "word"]),
        &[1, 1, 1, 1],
        &[1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_basic_keep_orig_two_outputs() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "x y", true)?;
    add(&mut b, "a b", "m n o", true)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c a b d",
        &["c", "x", "m", "a", "y", "n", "o", "b", "d"],
        &[0, 2, 2, 2, 2, 2, 2, 4, 6],
        &[1, 5, 5, 3, 5, 5, 5, 5, 7],
        Some(&[
            "word", "SYNONYM", "SYNONYM", "word", "SYNONYM", "SYNONYM", "SYNONYM", "word", "word",
        ]),
        &[1, 1, 0, 0, 1, 1, 1, 1, 1],
        &[1, 1, 2, 4, 4, 1, 2, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_no_capture_if_no_match() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "x y", true)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c d d",
        &["c", "d", "d"],
        &[0, 2, 4],
        &[1, 3, 5],
        Some(&["word", "word", "word"]),
        &[1, 1, 1],
        &[1, 1, 1],
    )?;
    Assert::assert_equals(0, syn_filter_field().borrow().logic().get_capture_count());
    a.close();
    Ok(())
}

fn sgf_test_basic_not_keep_orig_one_output() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "x", false)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c a b",
        &["c", "x"],
        &[0, 2],
        &[1, 5],
        Some(&["word", "SYNONYM"]),
        &[1, 1],
        &[1, 1],
    )?;
    a.close();
    Ok(())
}

/// The rules of `testBasicNoKeepOrigTwoOutputs`, `testIgnoreCase` and
/// `testDoNotIgnoreCase`.
fn two_outputs_builder() -> Result<SynonymMapBuilder> {
    let mut b = new_builder();
    add(&mut b, "a b", "x y", false)?;
    add(&mut b, "a b", "m n o", false)?;
    Ok(b)
}

fn sgf_test_basic_no_keep_orig_two_outputs() -> Result<()> {
    let a = get_analyzer(two_outputs_builder()?.build()?, true);
    analyzes_full(
        &*a,
        "c a b d",
        &["c", "x", "m", "y", "n", "o", "d"],
        &[0, 2, 2, 2, 2, 2, 6],
        &[1, 5, 5, 5, 5, 5, 7],
        Some(&[
            "word", "SYNONYM", "SYNONYM", "SYNONYM", "SYNONYM", "SYNONYM", "word",
        ]),
        &[1, 1, 0, 1, 1, 1, 1],
        &[1, 1, 2, 3, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_ignore_case() -> Result<()> {
    let a = get_analyzer(two_outputs_builder()?.build()?, true);
    analyzes_full(
        &*a,
        "c A B D",
        &["c", "x", "m", "y", "n", "o", "D"],
        &[0, 2, 2, 2, 2, 2, 6],
        &[1, 5, 5, 5, 5, 5, 7],
        Some(&[
            "word", "SYNONYM", "SYNONYM", "SYNONYM", "SYNONYM", "SYNONYM", "word",
        ]),
        &[1, 1, 0, 1, 1, 1, 1],
        &[1, 1, 2, 3, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_do_not_ignore_case() -> Result<()> {
    let a = get_analyzer(two_outputs_builder()?.build()?, false);
    analyzes_full(
        &*a,
        "c A B D",
        &["c", "A", "B", "D"],
        &[0, 2, 4, 6],
        &[1, 3, 5, 7],
        Some(&["word", "word", "word", "word"]),
        &[1, 1, 1, 1],
        &[1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_buffered_finish1() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b c", "m n o", false)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c a b",
        &["c", "a", "b"],
        &[0, 2, 4],
        &[1, 3, 5],
        Some(&["word", "word", "word"]),
        &[1, 1, 1],
        &[1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_buffered_finish2() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "m n o", false)?;
    add(&mut b, "d e", "m n o", false)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "c a d",
        &["c", "a", "d"],
        &[0, 2, 4],
        &[1, 3, 5],
        Some(&["word", "word", "word"]),
        &[1, 1, 1],
        &[1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_can_reuse() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b", "x", true)?;
    let a = get_analyzer(b.build()?, true);
    for _ in 0..10 {
        analyzes_full(
            &*a,
            "c a b",
            &["c", "x", "a", "b"],
            &[0, 2, 2, 4],
            &[1, 5, 3, 5],
            Some(&["word", "SYNONYM", "word", "word"]),
            &[1, 1, 0, 1],
            &[1, 2, 1, 1],
        )?;
    }
    a.close();
    Ok(())
}

/// Multiple input tokens map to a single output token
fn sgf_test_many_to_one() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b c", "z", true)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "a b c d",
        &["z", "a", "b", "c", "d"],
        &[0, 0, 2, 4, 6],
        &[5, 1, 3, 5, 7],
        Some(&["SYNONYM", "word", "word", "word", "word"]),
        &[1, 0, 1, 1, 1],
        &[3, 1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_buffer_after_match() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "a b c d", "x", true)?;
    add(&mut b, "a b", "y", false)?;
    // The 'c' token has to be buffered because SynGraphFilter
    // needs to know whether a b c d -> x matches:
    let a = get_analyzer(b.build()?, true);
    analyzes_full(
        &*a,
        "f a b c e",
        &["f", "y", "c", "e"],
        &[0, 2, 6, 8],
        &[1, 5, 7, 9],
        Some(&["word", "SYNONYM", "word", "word"]),
        &[1, 1, 1, 1],
        &[1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_zero_syns() -> Result<()> {
    // `new MockTokenizer()`: `MockTokenizer(WHITESPACE, true)`.
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), true)?;
    tokenizer.set_reader(Box::new(reader("aa bb")))?;
    let tokenizer = Mutex::new(Some(tokenizer));
    let ex = iae_message(|| {
        new_synonym_graph_filter(
            boxed(tokenizer.lock().unwrap().take().unwrap()),
            Arc::new(SynonymMapBuilder::with_dedup(true).build()?),
            true,
        )
    });
    Assert::assert_equals("fst must be non-null", ex.as_str());
    Ok(())
}

fn sgf_test_output_hangs_off_end() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    // b hangs off the end (no input token under it):
    add(&mut b, "a", "a b", false)?;
    let a = get_flatten_analyzer(b.build()?, true);
    analyzes_graph(
        &*a,
        "a",
        &["a", "b"],
        &[0, 0],
        &[1, 1],
        &[1, 1],
        &[1, 1],
        true,
    )?;
    a.close();
    Ok(())
}

fn sgf_test_dedup() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    for _ in 0..3 {
        add(&mut b, "a b", "ab", false)?;
    }
    let a = get_flatten_analyzer(b.build()?, true);
    analyzes_pos(&*a, "a b", &["ab"], &[1])?;
    a.close();
    Ok(())
}

fn sgf_test_no_dedup() -> Result<()> {
    // dedup is false:
    let mut b = SynonymMapBuilder::with_dedup(false);
    for _ in 0..3 {
        add(&mut b, "a b", "ab", false)?;
    }
    let a = get_flatten_analyzer(b.build()?, true);
    analyzes_pos(&*a, "a b", &["ab", "ab", "ab"], &[1, 0, 0])?;
    a.close();
    Ok(())
}

fn sgf_test_matching() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add_matching_rules(&mut b, false)?;
    let a = get_flatten_analyzer(b.build()?, true);
    assert_matching(&*a)?;
    a.close();
    Ok(())
}

fn sgf_test_basic1() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "a", "foo", true)?;
    add(&mut b, "a b", "bar fee", true)?;
    add(&mut b, "b c", "dog collar", true)?;
    add(&mut b, "c d", "dog harness holder extras", true)?;
    add(&mut b, "m c e", "dog barks loudly", false)?;
    add(&mut b, "i j k", "feep", true)?;
    add(&mut b, "e f", "foo bar", false)?;
    add(&mut b, "e f", "baz bee", false)?;
    add(&mut b, "z", "boo", false)?;
    add(&mut b, "y", "bee", true)?;
    let a = get_flatten_analyzer(b.build()?, true);
    analyzes_pos(
        &*a,
        "a b c",
        &["bar", "a", "fee", "b", "c"],
        &[1, 0, 1, 0, 1],
    )?;
    analyzes_pos(
        &*a,
        "x a b c d",
        &[
            "x", "bar", "a", "fee", "b", "dog", "c", "harness", "d", "holder", "extras",
        ],
        &[1, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1],
    )?;
    analyzes_pos(
        &*a,
        "a b a",
        &["bar", "a", "fee", "b", "foo", "a"],
        &[1, 0, 1, 0, 1, 0],
    )?;
    // outputs no longer add to one another:
    analyzes_pos(
        &*a,
        "c d c d",
        &[
            "dog", "c", "harness", "d", "holder", "extras", "dog", "c", "harness", "d", "holder",
            "extras",
        ],
        &[1, 0, 1, 0, 1, 1, 1, 0, 1, 0, 1, 1],
    )?;
    // two outputs for same input
    analyzes_pos(&*a, "e f", &["foo", "baz", "bar", "bee"], &[1, 0, 1, 0])?;
    // verify multi-word / single-output offsets:
    analyzes_pos(
        &*a,
        "g i j k g",
        &["g", "feep", "i", "j", "k", "g"],
        &[1, 1, 0, 1, 1, 1],
    )?;
    // mixed keepOrig true/false:
    analyzes_pos(
        &*a,
        "a m c e x",
        &["foo", "a", "dog", "barks", "loudly", "x"],
        &[1, 0, 1, 1, 1, 1],
    )?;
    analyzes_pos(
        &*a,
        "c d m c e x",
        &[
            "dog", "c", "harness", "d", "holder", "extras", "dog", "barks", "loudly", "x",
        ],
        &[1, 0, 1, 0, 1, 1, 1, 1, 1, 1],
    )?;
    Assert::assert_true(syn_filter_field().borrow().logic().get_capture_count() > 0);
    // no captureStates when no syns matched
    analyzes_pos(
        &*a,
        "p q r s t",
        &["p", "q", "r", "s", "t"],
        &[1, 1, 1, 1, 1],
    )?;
    Assert::assert_equals(0, syn_filter_field().borrow().logic().get_capture_count());
    // captureStates are necessary for the single-token syn case:
    analyzes_pos(
        &*a,
        "p q z y t",
        &["p", "q", "boo", "bee", "y", "t"],
        &[1, 1, 1, 1, 0, 1],
    )?;
    Assert::assert_true(syn_filter_field().borrow().logic().get_capture_count() > 0);
    Ok(())
}

fn sgf_test_basic2() -> Result<()> {
    let mut keep_orig = true;
    loop {
        keep_orig = !keep_orig;
        let mut b = SynonymMapBuilder::with_dedup(true);
        add(&mut b, "aaa", "aaaa1 aaaa2 aaaa3", keep_orig)?;
        add(&mut b, "bbb", "bbbb1 bbbb2", keep_orig)?;
        let a = get_flatten_analyzer(b.build()?, true);
        if keep_orig {
            analyzes_pos(
                &*a,
                "xyzzy bbb pot of gold",
                &["xyzzy", "bbbb1", "bbb", "bbbb2", "pot", "of", "gold"],
                &[1, 1, 0, 1, 1, 1, 1],
            )?;
            analyzes_pos(
                &*a,
                "xyzzy aaa pot of gold",
                &[
                    "xyzzy", "aaaa1", "aaa", "aaaa2", "aaaa2", "pot", "of", "gold",
                ],
                &[1, 1, 0, 1, 1, 1, 1, 1],
            )?;
        } else {
            analyzes_pos(
                &*a,
                "xyzzy bbb pot of gold",
                &["xyzzy", "bbbb1", "bbbb2", "pot", "of", "gold"],
                &[1, 1, 1, 1, 1, 1],
            )?;
            analyzes_pos(
                &*a,
                "xyzzy aaa pot of gold",
                &["xyzzy", "aaaa1", "aaaa2", "aaaa3", "pot", "of", "gold"],
                &[1, 1, 1, 1, 1, 1, 1],
            )?;
        }
        if !keep_orig {
            break;
        }
    }
    Ok(())
}

/// If we expand synonyms during indexing, it's a bit better than
/// SynonymFilter is today, but still necessarily has false positive and
/// negative PhraseQuery matches because we do not index posLength, so we
/// lose information.
fn sgf_test_flattened_graph() -> Result<()> {
    let mut b = new_builder();
    add(&mut b, "wtf", "what the fudge", true)?;
    let a = get_flatten_analyzer(b.build()?, true);
    analyzes_graph(
        &*a,
        "wtf happened",
        &["what", "wtf", "the", "fudge", "happened"],
        &[0, 0, 0, 0, 4],
        &[3, 3, 3, 3, 12],
        &[1, 0, 1, 1, 1],
        &[1, 3, 1, 1, 1],
        true,
    )?;
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new(&*random(), dir.as_directory(), Arc::clone(&a))?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "wtf happened",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let r = w.get_reader()?;
    w.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let phrase = |terms: &[&str]| -> Result<i32> {
        s.count(Arc::new(PhraseQuery::with_slop(0, "field", terms)?))
    };
    // Good (this should not match, and doesn't):
    Assert::assert_equals(0, phrase(&["what", "happened"])?);
    // Bad (this should match, but doesn't):
    Assert::assert_equals(0, phrase(&["wtf", "happened"])?);
    // Good (this should match, and does):
    Assert::assert_equals(1, phrase(&["what", "the", "fudge", "happened"])?);
    // Bad (this should not match, but does):
    Assert::assert_equals(1, phrase(&["wtf", "the"])?);
    drop(s);
    r.close()?;
    drop(r);
    drop(w);
    dir.close()
}

// Adds MockGraphTokenFilter after SynFilter:
fn sgf_test_random_graph_after() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(3);
    for _ in 0..num_iters {
        let map = Arc::new(random_builder()?.build()?);
        let ignore_case = random().next_boolean();
        let do_flatten = random().next_boolean();
        let analyzer = mock_analyzer(
            "org.apache.lucene.analysis.synonym.TestSynonymGraphFilter$3",
            &SIMPLE,
            true,
            move |t| {
                let syns = boxed(new_synonym_graph_filter(t, Arc::clone(&map), ignore_case)?);
                let mut graph: Box<dyn TokenStream> =
                    boxed(MockGraphTokenFilter::new(&*LuceneTestCase::random(), syns)?);
                if do_flatten {
                    graph = boxed(new_flatten_graph_filter(graph)?);
                }
                Ok(graph)
            },
        );
        B::check_random_data(&*random(), &analyzer, 100)?;
        analyzer.close();
    }
    Ok(())
}

fn sgf_test_empty_string_input() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(10);
    for _ in 0..num_iters {
        let b = random_builder()?;
        let ignore_case = random().next_boolean();
        let mut b = b;
        let analyzer = get_analyzer(b.build()?, ignore_case);
        let use_char_filter = random().next_boolean();
        B::check_analysis_consistency(&random(), &*analyzer, use_char_filter, "")?;
        analyzer.close();
    }
    Ok(())
}

/// simple random test, doesn't verify correctness. does verify it doesnt
/// throw exceptions, or that the stream doesn't misbehave
fn sgf_test_random2() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(3);
    for _ in 0..num_iters {
        let mut b = random_builder()?;
        let ignore_case = random().next_boolean();
        let do_flatten = random().next_boolean();
        let analyzer = if do_flatten {
            get_flatten_analyzer(b.build()?, ignore_case)
        } else {
            get_analyzer(b.build()?, ignore_case)
        };
        B::check_random_data(&*random(), &analyzer, 100)?;
        analyzer.close();
    }
    Ok(())
}

/// simple random test like testRandom2, but for larger docs
fn sgf_test_random_huge() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(1);
    for i in 0..num_iters {
        let mut b = SynonymMapBuilder::with_dedup(random().next_boolean());
        let num_entries = LuceneTestCase::at_least(10);
        if *VERBOSE {
            println!("TEST: iter={i} numEntries={num_entries}");
        }
        for _ in 0..num_entries {
            let input = random_non_empty_string();
            let output = random_non_empty_string();
            add(&mut b, &input, &output, random().next_boolean())?;
        }
        let ignore_case = random().next_boolean();
        let do_flatten = random().next_boolean();
        let analyzer = if do_flatten {
            get_flatten_analyzer(b.build()?, ignore_case)
        } else {
            get_analyzer(b.build()?, ignore_case)
        };
        B::check_random_data_max_word_length(&*random(), &analyzer, 100, 1024)?;
        analyzer.close();
    }
    Ok(())
}

fn sgf_test_empty_term() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(10);
    for _ in 0..num_iters {
        let mut b = random_builder()?;
        let ignore_case = random().next_boolean();
        let analyzer = get_analyzer(b.build()?, ignore_case);
        let use_char_filter = random().next_boolean();
        B::check_analysis_consistency(&random(), &*analyzer, use_char_filter, "")?;
        analyzer.close();
    }
    Ok(())
}

/// The assertions of `testVanishingTermsNoFlatten` and
/// `testVanishingTermsWithFlatten`.
fn vanishing_terms() -> Result<()> {
    let test_file = "aaa => aaaa1 aaaa2 aaaa3\nbbb => bbbb1 bbbb2\n";
    let analyzer = solr_syns_to_analyzer(test_file)?;
    B::assert_analyzes_to(
        &*analyzer,
        "xyzzy bbb pot of gold",
        &["xyzzy", "bbbb1", "bbbb2", "pot", "of", "gold"],
    )?;
    // xyzzy aaa pot of gold -> xyzzy aaaa1 aaaa2 aaaa3 gold
    B::assert_analyzes_to(
        &*analyzer,
        "xyzzy aaa pot of gold",
        &["xyzzy", "aaaa1", "aaaa2", "aaaa3", "pot", "of", "gold"],
    )?;
    analyzer.close();
    Ok(())
}

// LUCENE-3375
fn sgf_test_vanishing_terms_no_flatten() -> Result<()> {
    vanishing_terms()
}

// LUCENE-3375
fn sgf_test_vanishing_terms_with_flatten() -> Result<()> {
    vanishing_terms()
}

fn sgf_test_builder_dedup() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    for _ in 0..3 {
        add(&mut b, "a b", "ab", false)?;
    }
    let a = get_analyzer(b.build()?, true);
    analyzes_pos(&*a, "a b", &["ab"], &[1])?;
    a.close();
    Ok(())
}

fn sgf_test_builder_no_dedup() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(false);
    for _ in 0..3 {
        add(&mut b, "a b", "ab", false)?;
    }
    let a = get_analyzer(b.build()?, true);
    analyzes_pos(&*a, "a b", &["ab", "ab", "ab"], &[1, 0, 0])?;
    a.close();
    Ok(())
}

fn sgf_test_recursion1() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "zoo", "zoo", false)?;
    let a = get_analyzer(b.build()?, true);
    analyzes_pos(
        &*a,
        "zoo zoo $ zoo",
        &["zoo", "zoo", "$", "zoo"],
        &[1, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_recursion2() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "zoo", "zoo", false)?;
    add(&mut b, "zoo", "zoo zoo", false)?;
    let a = get_analyzer(b.build()?, true);
    // verify("zoo zoo $ zoo", "zoo/zoo zoo/zoo/zoo $/zoo zoo/zoo zoo");
    analyzes_pos(
        &*a,
        "zoo zoo $ zoo",
        &[
            "zoo", "zoo", "zoo", "zoo", "zoo", "zoo", "$", "zoo", "zoo", "zoo",
        ],
        &[1, 0, 1, 1, 0, 1, 1, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_recursion3() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "zoo zoo", "zoo", true)?;
    let a = get_flatten_analyzer(b.build()?, true);
    analyzes_pos(
        &*a,
        "zoo zoo $ zoo",
        &["zoo", "zoo", "zoo", "$", "zoo"],
        &[1, 0, 1, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_recursion4() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "zoo zoo", "zoo", true)?;
    add(&mut b, "zoo", "zoo zoo", true)?;
    let a = get_flatten_analyzer(b.build()?, true);
    analyzes_pos(
        &*a,
        "zoo zoo $ zoo",
        &["zoo", "zoo", "zoo", "$", "zoo", "zoo", "zoo"],
        &[1, 0, 1, 1, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_keep_orig() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add_matching_rules(&mut b, true)?;
    let a = get_analyzer(b.build()?, true);
    assert_keep_orig(&*a)?;
    a.close();
    Ok(())
}

/// The assertions of `testKeepOrig` and `testIncludeOrig`.
fn assert_keep_orig(a: &dyn Analyzer) -> Result<()> {
    analyzes_pos(a, "$", &["$"], &[1])?;
    analyzes_pos(a, "a", &["aa", "a"], &[1, 0])?;
    analyzes_pos(a, "a", &["aa", "a"], &[1, 0])?;
    analyzes_pos(a, "$ a", &["$", "aa", "a"], &[1, 1, 0])?;
    analyzes_pos(a, "a $", &["aa", "a", "$"], &[1, 0, 1])?;
    analyzes_pos(a, "$ a !", &["$", "aa", "a", "!"], &[1, 1, 0, 1])?;
    analyzes_pos(a, "a a", &["aa", "a", "aa", "a"], &[1, 0, 1, 0])?;
    analyzes_pos(a, "b", &["bb", "b"], &[1, 0])?;
    analyzes_pos(
        a,
        "z x c v",
        &["zxcv", "z", "x", "c", "v"],
        &[1, 0, 1, 1, 1],
    )?;
    analyzes_pos(a, "z x c $", &["z", "xc", "x", "c", "$"], &[1, 1, 0, 1, 1])
}

/// verify type of token and positionLengths on synonyms of different word
/// counts, with non preserving, explicit rules.
fn sgf_test_non_preserving_multiword_synonyms() -> Result<()> {
    let test_file = "aaa => two words\n\
                     bbb => one two, very many multiple words\n\
                     ee ff, gg, h i j k, h i => one\n\
                     cc dd => usa,united states,u s a,united states of america";
    let analyzer = solr_syns_to_analyzer(test_file)?;
    analyzes_full(
        &*analyzer,
        "aaa",
        &["two", "words"],
        &[0, 0],
        &[3, 3],
        Some(&["SYNONYM", "SYNONYM"]),
        &[1, 1],
        &[1, 1],
    )?;
    assert_analyzes_to_positions(
        &*analyzer,
        "amazing aaa",
        &["amazing", "two", "words"],
        &["word", "SYNONYM", "SYNONYM"],
        &[1, 1, 1],
        &[1, 1, 1],
    )?;
    analyzes_full(
        &*analyzer,
        "p bbb s",
        &["p", "one", "very", "two", "many", "multiple", "words", "s"],
        &[0, 2, 2, 2, 2, 2, 2, 6],
        &[1, 5, 5, 5, 5, 5, 5, 7],
        Some(&[
            "word", "SYNONYM", "SYNONYM", "SYNONYM", "SYNONYM", "SYNONYM", "SYNONYM", "word",
        ]),
        &[1, 1, 0, 1, 0, 1, 1, 1],
        &[1, 1, 1, 3, 1, 1, 1, 1],
    )?;
    analyzes_full(
        &*analyzer,
        "p ee ff s",
        &["p", "one", "s"],
        &[0, 2, 8],
        &[1, 7, 9],
        Some(&["word", "SYNONYM", "word"]),
        &[1, 1, 1],
        &[1, 1, 1],
    )?;
    analyzes_full(
        &*analyzer,
        "p h i j s",
        &["p", "one", "j", "s"],
        &[0, 2, 6, 8],
        &[1, 5, 7, 9],
        Some(&["word", "SYNONYM", "word", "word"]),
        &[1, 1, 1, 1],
        &[1, 1, 1, 1],
    )?;
    analyzer.close();
    Ok(())
}

/// `randomBinaryChars(minLen, maxLen, bias, base)`.
fn random_binary_chars(min_len: i32, max_len: i32, bias: f64, base: u8) -> Vec<u8> {
    let len = TestUtil::next_int(&*random(), min_len, max_len);
    let mut chars = Vec::with_capacity(len as usize);
    for _ in 0..len {
        let ch = if random().next_double() < bias {
            base
        } else {
            base + 1
        };
        chars.push(ch);
    }
    chars
}

/// `toTokenString(chars)`.
fn to_token_string(chars: &[u8]) -> String {
    let mut b = String::new();
    for &c in chars {
        if !b.is_empty() {
            b.push(' ');
        }
        b.push(c as char);
    }
    b
}

/// `OneSyn` of `TestSynonymGraphFilter`.
#[derive(Clone)]
struct GraphOneSyn {
    in_: Vec<u8>,
    out: Vec<u8>,
    keep_orig: bool,
}

impl std::fmt::Display for GraphOneSyn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} --> {} (keepOrig={})",
            to_token_string(&self.in_),
            to_token_string(&self.out),
            self.keep_orig
        )
    }
}

fn sgf_test_random_syns() -> Result<()> {
    let syn_count = LuceneTestCase::at_least(10);
    let bias = random().next_double();
    let dedup = random().next_boolean();
    let flatten = random().next_boolean();
    let mut b = SynonymMapBuilder::with_dedup(dedup);
    let mut syns: Vec<GraphOneSyn> = Vec::new();
    // Makes random syns from random a / b tokens, mapping to random x / y tokens
    if *VERBOSE {
        println!("TEST: make {syn_count} syns");
        println!("  bias for a over b={bias}");
        println!("  dedup={dedup}");
        println!("  flatten={flatten}");
    }
    let mut max_syn_length = 0;
    for _ in 0..syn_count {
        let in_ = random_binary_chars(1, 5, bias, b'a');
        let out = random_binary_chars(1, 5, 0.5, b'x');
        let keep_orig = random().next_boolean();
        let syn = GraphOneSyn {
            in_,
            out,
            keep_orig,
        };
        max_syn_length = max_syn_length.max(syn.in_.len());
        if *VERBOSE {
            println!("  {syn}");
        }
        add(
            &mut b,
            &to_token_string(&syn.in_),
            &to_token_string(&syn.out),
            syn.keep_orig,
        )?;
        syns.push(syn);
    }
    // Compute max allowed lookahead for flatten filter:
    let mut max_flatten_lookahead = 0;
    if flatten {
        for i in 0..syns.len() {
            let syn1 = &syns[i];
            let mut count = syn1.out.len();
            let mut keep_orig = syn1.keep_orig;
            for _ in 0..syns.len() {
                // (sic) Java reads `syns.get(i)` here, not `syns.get(j)`.
                let syn2 = &syns[i];
                keep_orig |= syn2.keep_orig;
                if syn1.in_ == syn2.in_ {
                    count += syn2.out.len();
                }
            }
            if keep_orig {
                count += syn1.in_.len();
            }
            max_flatten_lookahead = max_flatten_lookahead.max(count);
        }
    }
    // Only used w/ VERBOSE:
    let a_no_flattened = if *VERBOSE {
        Some(get_analyzer(b.build()?, true))
    } else {
        None
    };
    let a = if flatten {
        get_flatten_analyzer(b.build()?, true)
    } else {
        get_analyzer(b.build()?, true)
    };
    let iters = LuceneTestCase::at_least(1);
    for iter in 0..iters {
        let doc = to_token_string(&random_binary_chars(50, 100, bias, b'a'));
        // String doc = toTokenString(randomBinaryChars(10, 50, bias, 'a'));
        if *VERBOSE {
            println!("TEST: iter={iter} doc={doc}");
        }
        let expected = slow_syn_filter(&doc, &syns, flatten)?;
        if *VERBOSE {
            println!("  expected:\n{}", expected.to_dot());
            if flatten {
                let unflattened = to_automaton(
                    &mut *a_no_flattened
                        .as_ref()
                        .unwrap()
                        .token_stream("field", Box::new(reader(&doc)))?
                        .borrow_mut(),
                )?;
                println!("  actual unflattened:\n{}", unflattened.to_dot());
            }
        }
        let actual = to_automaton(
            &mut *a
                .token_stream("field", Box::new(reader(&doc)))?
                .borrow_mut(),
        )?;
        if *VERBOSE {
            println!("  actual:\n{}", actual.to_dot());
        }
        let max_lookahead_used = syn_filter_field().borrow().logic().get_max_lookahead_used();
        Assert::assert_true_msg(
            &format!("maxLookaheadUsed={max_lookahead_used} maxSynLength={max_syn_length}"),
            max_lookahead_used <= max_syn_length,
        );
        if flatten {
            let used = flatten_filter_field()
                .borrow()
                .logic()
                .get_max_lookahead_used();
            Assert::assert_true_msg(
                &format!(
                    "flatten maxLookaheadUsed={used} maxFlattenLookahead={max_flatten_lookahead}"
                ),
                used <= max_flatten_lookahead,
            );
        }
        let use_char_filter = random().next_boolean();
        B::check_analysis_consistency(&random(), &*a, use_char_filter, &doc)?;
        // We can easily have a non-deterministic automaton at this point, e.g. if
        // more than one syn matched at given point, or if the syn mapped to an
        // output token that also happens to be in the input:
        let actual = match Operations::determinize(&actual, 50000) {
            Ok(d) => d,
            Err(_) => {
                // Unfortunately the syns can easily create difficult-to-determinize graphs:
                Assert::assert_true(approx_equals(&actual, &expected));
                continue;
            }
        };
        let expected = match Operations::determinize(&expected, 50000) {
            Ok(d) => d,
            Err(_) => {
                // Unfortunately the syns can easily create difficult-to-determinize graphs:
                Assert::assert_true(approx_equals(&actual, &expected));
                continue;
            }
        };
        Assert::assert_true(approx_equals(&actual, &expected));
        Assert::assert_true(AutomatonTestUtil::same_language(&actual, &expected));
    }
    a.close();
    Ok(())
}

/// Only used when true equality is too costly to check!
fn approx_equals(actual: &Automaton, expected: &Automaton) -> bool {
    // Don't collapse these into one line else the thread stack won't say which direction failed!:
    let b1 = approx_subset_of(actual, expected);
    let b2 = approx_subset_of(expected, actual);
    b1 && b2
}

fn approx_subset_of(a1: &Automaton, a2: &Automaton) -> bool {
    let ras = RandomAcceptedStrings::new(a1);
    for _ in 0..2000 {
        let ints = ras.get_random_accepted_string(&*random());
        if !accepts(a2, &ints) {
            panic!("a2 does not accept {ints:?}");
        }
    }
    // Presumed true
    true
}

/// Like `Operations#run` except the incoming automaton is allowed to be
/// non-deterministic.
fn accepts(a: &Automaton, path: &[i32]) -> bool {
    let mut states: HashSet<i32> = HashSet::new();
    states.insert(0);
    let mut t = Transition::default();
    for &digit in path {
        let mut next_states: HashSet<i32> = HashSet::new();
        for &state in &states {
            let count = a.init_transition(state, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                if digit >= t.min && digit <= t.max {
                    next_states.insert(t.dest);
                }
            }
        }
        states = next_states;
        if states.is_empty() {
            return false;
        }
    }
    states.iter().any(|&state| a.is_accept(state))
}

/// Stupid, slow brute-force, yet hopefully bug-free, synonym filter.
fn slow_syn_filter(doc: &str, syns: &[GraphOneSyn], flatten: bool) -> Result<Automaton> {
    let tokens = split_spaces(doc);
    if *VERBOSE {
        println!("  doc has {} tokens", tokens.len());
    }
    let mut i = 0;
    let mut a = AutomatonBuilder::new();
    let mut last_state = a.create_state();
    while i < tokens.len() {
        // Consider all possible syn matches starting at this point:
        assert_eq!(1, tokens[i].len());
        if *VERBOSE {
            println!("    i={i}");
        }
        let mut matches: Vec<&GraphOneSyn> = Vec::new();
        for syn in syns {
            if i + syn.in_.len() <= tokens.len() {
                let mut is_match = true;
                for j in 0..syn.in_.len() {
                    if tokens[i + j].as_bytes()[0] != syn.in_[j] {
                        is_match = false;
                        break;
                    }
                }
                if is_match {
                    if !matches.is_empty() {
                        if syn.in_.len() < matches[0].in_.len() {
                            // Greedy matching: we already found longer syns matching here
                            continue;
                        } else if syn.in_.len() > matches[0].in_.len() {
                            // Greedy matching: all previous matches were shorter, so we drop them
                            matches.clear();
                        } else {
                            // Keep the current matches: we allow multiple synonyms matching the same input
                            // string
                        }
                    }
                    matches.push(syn);
                }
            }
        }
        let next_state = a.create_state();
        if !matches.is_empty() {
            // We have match(es) starting at this token
            if *VERBOSE {
                let m: Vec<String> = matches.iter().map(|s| s.to_string()).collect();
                println!("  matches @ i={i}: {m:?}");
            }
            // We keepOrig if any of the matches said to:
            let keep_orig = matches.iter().any(|s| s.keep_orig);
            let mut flat_states: Option<Vec<i32>> = if flatten { Some(Vec::new()) } else { None };
            if keep_orig {
                // Add path for the original tokens
                add_side_path(
                    &mut a,
                    last_state,
                    next_state,
                    &matches[0].in_,
                    &mut flat_states,
                );
            }
            for syn in &matches {
                add_side_path(&mut a, last_state, next_state, &syn.out, &mut flat_states);
            }
            i += matches[0].in_.len();
        } else {
            a.add_transition(last_state, next_state, i32::from(tokens[i].as_bytes()[0]));
            i += 1;
        }
        last_state = next_state;
    }
    a.set_accept(last_state, true);
    topo_sort(&a.finish())
}

/// Just creates a side path from startState to endState with the provided
/// tokens.
fn add_side_path(
    a: &mut AutomatonBuilder,
    start_state: i32,
    end_state: i32,
    tokens: &[u8],
    flat_states: &mut Option<Vec<i32>>,
) {
    let mut last_state = start_state;
    for i in 0..tokens.len() {
        let next_state;
        if i == tokens.len() - 1 {
            next_state = end_state;
        } else if flat_states.as_ref().is_none_or(|f| i >= f.len()) {
            next_state = a.create_state();
            if let Some(f) = flat_states.as_mut() {
                assert_eq!(i, f.len());
                f.push(next_state);
            }
        } else {
            next_state = flat_states.as_ref().unwrap()[i];
        }
        a.add_transition(last_state, next_state, i32::from(tokens[i]));
        last_state = next_state;
    }
}

fn to_automaton(ts: &mut dyn TokenStream) -> Result<Automaton> {
    {
        let src = ts.attribute_source_mut();
        src.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        src.add_attribute_interface::<dyn PositionLengthAttribute>()?;
        src.add_attribute_interface::<dyn CharTermAttribute>()?;
    }
    ts.reset()?;
    let mut a = Automaton::new();
    let mut src_node: i32 = -1;
    let mut dest_node: i32 = -1;
    let mut state = a.create_state();
    while ts.increment_token()? {
        let src = ts.attribute_source();
        let term_att = src
            .get_attribute_interface::<dyn CharTermAttribute>()
            .unwrap();
        assert_eq!(1, term_att.length());
        let c = term_att.buffer()[0];
        let pos_inc = src
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .unwrap()
            .get_position_increment();
        if pos_inc != 0 {
            src_node += pos_inc;
            while state < src_node {
                state = a.create_state();
            }
        }
        dest_node = src_node
            + src
                .get_attribute_interface::<dyn PositionLengthAttribute>()
                .unwrap()
                .get_position_length();
        while state < dest_node {
            state = a.create_state();
        }
        a.add_transition(src_node, dest_node, c as i32);
    }
    ts.end()?;
    ts.close()?;
    a.finish_state();
    a.set_accept(dest_node, true);
    Ok(a)
}

/// Renumbers nodes according to their topo sort
fn topo_sort(in_: &Automaton) -> Result<Automaton> {
    let new_to_old = Operations::topo_sort_states(in_)?;
    let mut old_to_new = vec![0i32; new_to_old.len()];
    let mut a = AutomatonBuilder::new();
    for (i, &old) in new_to_old.iter().enumerate() {
        a.create_state();
        old_to_new[old as usize] = i as i32;
        if in_.is_accept(old) {
            a.set_accept(i as i32, true);
        }
    }
    let mut t = Transition::default();
    for (i, &old) in new_to_old.iter().enumerate() {
        let count = in_.init_transition(old, &mut t);
        for _ in 0..count {
            in_.get_next_transition(&mut t);
            a.add_transition_range(i as i32, old_to_new[t.dest as usize], t.min, t.max);
        }
    }
    Ok(a.finish())
}

/// verify type of token and positionLengths on synonyms of different word
/// counts.
fn sgf_test_position_length_and_type() -> Result<()> {
    let test_file = "spider man, spiderman\nusa,united states,u s a,united states of america";
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let mut parser = SolrSynonymParser::new(true, true, Arc::clone(&analyzer));
    parse(&mut parser, test_file)?;
    analyzer.close();
    let map = parser.build()?;
    let analyzer = get_flatten_analyzer(parser.build()?, true);
    let value = Util::get(
        map.fst.as_ref().expect("an FST"),
        &Util::to_utf32("usa", &mut IntsRefBuilder::new()),
    )?
    .expect("an output");
    let mut bytes_reader = ByteArrayDataInput::new(value.slice());
    let code = bytes_reader.read_v_int()?;
    let count = ((code as u32) >> 1) as i32;
    let mut synonyms_idxs = vec![0i32; count as usize];
    for idx in synonyms_idxs.iter_mut() {
        *idx = bytes_reader.read_v_int()?;
    }
    let scratch_bytes = map.words.read().unwrap().get(synonyms_idxs[2]).to_vec();
    let mut synonym_length = 1;
    for &byte in &scratch_bytes {
        if u16::from(byte) == SynonymMap::WORD_SEPARATOR {
            synonym_length += 1;
        }
    }
    Assert::assert_equals(count, 3);
    Assert::assert_equals(synonym_length, 4);
    analyzes_full(
        &*analyzer,
        "spider man",
        &["spiderman", "spider", "man"],
        &[0, 0, 7],
        &[10, 6, 10],
        Some(&["SYNONYM", "word", "word"]),
        &[1, 0, 1],
        &[2, 1, 1],
    )?;
    assert_analyzes_to_positions(
        &*analyzer,
        "amazing spider man",
        &["amazing", "spiderman", "spider", "man"],
        &["word", "SYNONYM", "word", "word"],
        &[1, 1, 0, 1],
        &[1, 2, 1, 1],
    )?;
    analyzes_full(
        &*analyzer,
        "the united states of america is wealthy",
        &[
            "the", "usa", "united", "u", "united", "states", "s", "states", "a", "of", "america",
            "is", "wealthy",
        ],
        &[0, 4, 4, 4, 4, 11, 11, 11, 18, 18, 21, 29, 32],
        &[3, 28, 10, 10, 10, 28, 17, 17, 28, 20, 28, 31, 39],
        Some(&[
            "word", "SYNONYM", "SYNONYM", "SYNONYM", "word", "SYNONYM", "SYNONYM", "word",
            "SYNONYM", "word", "word", "word", "word",
        ]),
        &[1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 1, 1, 1],
        &[1, 4, 1, 1, 1, 3, 1, 1, 2, 1, 1, 1, 1],
    )?;
    assert_analyzes_to_positions(
        &*analyzer,
        "spiderman",
        &["spider", "spiderman", "man"],
        &["SYNONYM", "word", "SYNONYM"],
        &[1, 0, 1],
        &[1, 2, 1],
    )?;
    analyzes_full(
        &*analyzer,
        "spiderman enemies",
        &["spider", "spiderman", "man", "enemies"],
        &[0, 0, 0, 10],
        &[9, 9, 9, 17],
        Some(&["SYNONYM", "word", "SYNONYM", "word"]),
        &[1, 0, 1, 1],
        &[1, 2, 1, 1],
    )?;
    analyzes_full(
        &*analyzer,
        "the usa is wealthy",
        &[
            "the", "united", "u", "united", "usa", "states", "s", "states", "a", "of", "america",
            "is", "wealthy",
        ],
        &[0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 8, 11],
        &[3, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 10, 18],
        Some(&[
            "word", "SYNONYM", "SYNONYM", "SYNONYM", "word", "SYNONYM", "SYNONYM", "SYNONYM",
            "SYNONYM", "SYNONYM", "SYNONYM", "word", "word",
        ]),
        &[1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 1, 1, 1],
        &[1, 1, 1, 1, 4, 3, 1, 1, 2, 1, 1, 1, 1],
    )?;
    B::assert_graph_strings(
        &*analyzer,
        "the usa is wealthy",
        &[
            "the usa is wealthy",
            "the united states is wealthy",
            "the u s a is wealthy",
            "the united states of america is wealthy",
            // Wrong. Here only due to "sausagization" of the multi word synonyms.
            "the u states is wealthy",
            "the u states a is wealthy",
            "the u s of america is wealthy",
            "the u states of america is wealthy",
            "the united s a is wealthy",
            "the united states a is wealthy",
            "the united s of america is wealthy",
        ],
    )?;
    B::assert_analyzes_to_graph(
        &*analyzer,
        "the united states is wealthy",
        &[
            "the", "usa", "u", "united", "united", "s", "states", "states", "a", "of", "america",
            "is", "wealthy",
        ],
        Some(&[0, 4, 4, 4, 4, 11, 11, 11, 11, 11, 11, 18, 21]),
        Some(&[3, 17, 10, 10, 10, 17, 17, 17, 17, 17, 17, 20, 28]),
        Some(&[
            "word", "SYNONYM", "SYNONYM", "SYNONYM", "word", "SYNONYM", "SYNONYM", "word",
            "SYNONYM", "SYNONYM", "SYNONYM", "word", "word",
        ]),
        Some(&[1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 1, 1, 1]),
        Some(&[1, 4, 1, 1, 1, 1, 1, 3, 2, 1, 1, 1, 1]),
        false,
    )?;
    analyzes_full(
        &*analyzer,
        "the united states of balance",
        &[
            "the", "usa", "u", "united", "united", "s", "states", "states", "a", "of", "america",
            "of", "balance",
        ],
        &[0, 4, 4, 4, 4, 11, 11, 11, 11, 11, 11, 18, 21],
        &[3, 17, 10, 10, 10, 17, 17, 17, 17, 17, 17, 20, 28],
        Some(&[
            "word", "SYNONYM", "SYNONYM", "SYNONYM", "word", "SYNONYM", "SYNONYM", "word",
            "SYNONYM", "SYNONYM", "SYNONYM", "word", "word",
        ]),
        &[1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 1, 1, 1],
        &[1, 4, 1, 1, 1, 1, 1, 3, 2, 1, 1, 1, 1],
    )?;
    analyzer.close();
    Ok(())
}

fn sgf_test_multiword_offsets() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add(&mut b, "national hockey league", "nhl", true)?;
    let a = get_flatten_analyzer(b.build()?, true);
    B::assert_analyzes_to_offsets_pos_increments(
        &*a,
        "national hockey league",
        &["nhl", "national", "hockey", "league"],
        &[0, 0, 9, 16],
        &[22, 8, 15, 22],
        &[1, 0, 1, 1],
    )?;
    a.close();
    Ok(())
}

fn sgf_test_include_orig() -> Result<()> {
    let mut b = SynonymMapBuilder::with_dedup(true);
    add_matching_rules(&mut b, true)?;
    let a = get_flatten_analyzer(b.build()?, true);
    assert_keep_orig(&*a)?;
    a.close();
    Ok(())
}

fn sgf_test_upper_case() -> Result<()> {
    assert_mapping("word", "synonym")?;
    assert_mapping(&"word".to_uppercase(), "synonym")
}

fn assert_mapping(input_string: &str, output_string: &str) -> Result<()> {
    let mut builder = SynonymMapBuilder::with_dedup(false);
    // the rules must be lowercased up front, but the incoming tokens will be case insensitive:
    let lower = input_string.to_lowercase();
    let input: CharsRef =
        SynonymMapBuilder::join(&split_space(&lower), &mut CharsRefBuilder::new());
    let output: CharsRef =
        SynonymMapBuilder::join(&split_space(output_string), &mut CharsRefBuilder::new());
    builder.add(&input, &output, true)?;
    let analyzer = custom_analyzer(builder.build()?);
    let token_stream = analyzer.token_stream("field", Box::new(reader(input_string)))?;
    let result = B::assert_token_stream_contents(
        &mut *token_stream.borrow_mut(),
        &[output_string, input_string],
    );
    result
}

/// `CustomAnalyzer(synonymMap)`: whitespace tokens, then
/// `new SynonymGraphFilter(tokenizer, synonymMap, true)` (Ignore case True).
fn custom_analyzer(synonym_map: SynonymMap) -> Arc<dyn Analyzer> {
    let synonym_map = Arc::new(synonym_map);
    mock_analyzer(
        "org.apache.lucene.analysis.synonym.TestSynonymGraphFilter$CustomAnalyzer",
        &WHITESPACE,
        false,
        move |t| {
            Ok(boxed(new_synonym_graph_filter(
                t,
                Arc::clone(&synonym_map),
                true,
            )?))
        },
    )
}

fn synonym_graph_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.synonym.TestSynonymGraphFilter")
        .method(t(
            "testBasicKeepOrigOneOutput",
            sgf_test_basic_keep_orig_one_output,
        ))
        .method(t("testMixedKeepOrig", sgf_test_mixed_keep_orig))
        .method(t("testNoParseAfterBuffer", sgf_test_no_parse_after_buffer))
        .method(t(
            "testOneInputMultipleOutputKeepOrig",
            sgf_test_one_input_multiple_output_keep_orig,
        ))
        .method(t(
            "testPositionLengthAndTypeSimple",
            sgf_test_position_length_and_type_simple,
        ))
        .method(t("testEscapedStuff", sgf_test_escaped_stuff))
        .method(t(
            "testInvalidAnalyzesToNothingOutput",
            sgf_test_invalid_analyzes_to_nothing_output,
        ))
        .method(t("testInvalidDoubleMap", sgf_test_invalid_double_map))
        .method(t("testSimple", sgf_test_simple))
        .method(t("testBufferLength", sgf_test_buffer_length))
        .method(t(
            "testMoreThanOneLookAhead",
            sgf_test_more_than_one_look_ahead,
        ))
        .method(t("testLookaheadAfterParse", sgf_test_lookahead_after_parse))
        .method(t(
            "testLookaheadSecondParse",
            sgf_test_lookahead_second_parse,
        ))
        .method(t(
            "testOneInputMultipleOutputNoKeepOrig",
            sgf_test_one_input_multiple_output_no_keep_orig,
        ))
        .method(t(
            "testOneInputMultipleOutputMixedKeepOrig",
            sgf_test_one_input_multiple_output_mixed_keep_orig,
        ))
        .method(t("testSynAtEnd", sgf_test_syn_at_end))
        .method(t("testTwoSynsInARow", sgf_test_two_syns_in_a_row))
        .method(t(
            "testBasicKeepOrigTwoOutputs",
            sgf_test_basic_keep_orig_two_outputs,
        ))
        .method(t("testNoCaptureIfNoMatch", sgf_test_no_capture_if_no_match))
        .method(t(
            "testBasicNotKeepOrigOneOutput",
            sgf_test_basic_not_keep_orig_one_output,
        ))
        .method(t(
            "testBasicNoKeepOrigTwoOutputs",
            sgf_test_basic_no_keep_orig_two_outputs,
        ))
        .method(t("testIgnoreCase", sgf_test_ignore_case))
        .method(t("testDoNotIgnoreCase", sgf_test_do_not_ignore_case))
        .method(t("testBufferedFinish1", sgf_test_buffered_finish1))
        .method(t("testBufferedFinish2", sgf_test_buffered_finish2))
        .method(t("testCanReuse", sgf_test_can_reuse))
        .method(t("testManyToOne", sgf_test_many_to_one))
        .method(t("testBufferAfterMatch", sgf_test_buffer_after_match))
        .method(t("testZeroSyns", sgf_test_zero_syns))
        .method(t("testOutputHangsOffEnd", sgf_test_output_hangs_off_end))
        .method(t("testDedup", sgf_test_dedup))
        .method(t("testNoDedup", sgf_test_no_dedup))
        .method(t("testMatching", sgf_test_matching))
        .method(t("testBasic1", sgf_test_basic1))
        .method(t("testBasic2", sgf_test_basic2))
        .method(t("testFlattenedGraph", sgf_test_flattened_graph))
        .method(t("testRandomGraphAfter", sgf_test_random_graph_after))
        .method(t("testEmptyStringInput", sgf_test_empty_string_input))
        .method(t("testRandom2", sgf_test_random2))
        .method(t("testRandomHuge", sgf_test_random_huge))
        .method(t("testEmptyTerm", sgf_test_empty_term))
        .method(t(
            "testVanishingTermsNoFlatten",
            sgf_test_vanishing_terms_no_flatten,
        ))
        .method(t(
            "testVanishingTermsWithFlatten",
            sgf_test_vanishing_terms_with_flatten,
        ))
        .method(t("testBuilderDedup", sgf_test_builder_dedup))
        .method(t("testBuilderNoDedup", sgf_test_builder_no_dedup))
        .method(t("testRecursion1", sgf_test_recursion1))
        .method(t("testRecursion2", sgf_test_recursion2))
        .method(t("testRecursion3", sgf_test_recursion3))
        .method(t("testRecursion4", sgf_test_recursion4))
        .method(t("testKeepOrig", sgf_test_keep_orig))
        .method(t(
            "testNonPreservingMultiwordSynonyms",
            sgf_test_non_preserving_multiword_synonyms,
        ))
        .method(t("testRandomSyns", sgf_test_random_syns))
        .method(t(
            "testPositionLengthAndType",
            sgf_test_position_length_and_type,
        ))
        .method(t("testMultiwordOffsets", sgf_test_multiword_offsets))
        .method(t("testIncludeOrig", sgf_test_include_orig))
        .method(t("testUpperCase", sgf_test_upper_case))
        .build()
}

// -----------------------------------------------------------------------------
// Suites
// -----------------------------------------------------------------------------

#[test]
fn test_multi_word_synonyms() {
    run(multi_word_synonyms_class);
}

#[test]
fn test_wordnet_synonym_parser() {
    run(wordnet_synonym_parser_class);
}

#[test]
fn test_solr_synonym_parser() {
    run(solr_synonym_parser_class);
}

#[test]
fn test_synonym_map_filter() {
    run(synonym_map_filter_class);
}

#[test]
fn test_synonym_graph_filter() {
    run(synonym_graph_filter_class);
}
