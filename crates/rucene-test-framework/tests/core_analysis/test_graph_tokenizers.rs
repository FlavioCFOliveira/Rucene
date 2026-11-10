//! Port of `org.apache.lucene.analysis.TestGraphTokenizers`
//! (`extends BaseTokenStreamTestCase`), with its `GraphTokenizer`, which
//! `TestGraphTokenFilter` also uses.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::exception::java_assert;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::analysis::automaton_conversion::{TokenStreamToAutomaton, HOLE, POS_SEP};
use rucene::analysis::tokenattributes::{
    OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute,
};
use rucene::analysis::{
    default_token_attribute_factory, Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy,
    TokenFilter, TokenFilterLogic, TokenStream, TokenStreamComponents, Tokenizer, TokenizerLogic,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::operations::{Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};
use rucene::util::automaton::{Automata, Automaton};
use rucene::util::fst::Util;
use rucene::util::java_collections::JavaHashSet;
use rucene::util::{BytesRefBuilder, IntsRef};
use rucene_test_framework::analysis::attributes::{
    char_term_id, offset_id, pos_inc_id, pos_len_id, Att,
};
use rucene_test_framework::analysis::mock_graph_token_filter::MockGraphTokenFilter;
use rucene_test_framework::analysis::mock_hole_injecting_token_filter::MockHoleInjectingTokenFilter;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::token_stream_to_dot::TokenStreamToDot;
use rucene_test_framework::analysis::{
    components, shared, BaseTokenStreamTestCase, CannedTokenStream,
};
use rucene_test_framework::util::automaton::AutomatonTestUtil;
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, VERBOSE};
use rucene_test_framework::util::LuceneTestCase;

use crate::lucene_core_shims::t;

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

// Makes a graph TokenStream from the string; separate
// positions with single space, multiple tokens at the same
// position with /, and add optional position length with
// :.  EG "a b c" is a simple chain, "a/x b c" adds 'x'
// over 'a' at position 0 with posLen=1, "a/x:3 b c" adds
// 'x' over a with posLen=3.  Tokens are in normal-form!
// So, offsets are computed based on the first token at a
// given position.  NOTE: each token must be a single
// character!  We assume this when computing offsets...

// NOTE: all input tokens must be length 1!!!  This means
// you cannot turn on MockCharFilter when random
// testing...

/// `GraphTokenizer` (`public static final class … extends Tokenizer`): a
/// core `Tokenizer` over [`GraphTokenizerLogic`].
pub type GraphTokenizer = Tokenizer<GraphTokenizerLogic>;

/// The fields and overrides of `GraphTokenizer`.
#[derive(Debug)]
pub struct GraphTokenizerLogic {
    tokens: Option<Vec<Token>>,
    upto: usize,
    input_length: i32,

    term_att: Att,
    offset_att: Att,
    pos_incr_att: Att,
    pos_length_att: Att,
}

/// `new GraphTokenizer()`: `Tokenizer()` over
/// `DEFAULT_TOKEN_ATTRIBUTE_FACTORY`, then the attribute fields.
pub fn new_graph_tokenizer() -> Result<GraphTokenizer> {
    let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
    let term_att = Att::add(&mut source, char_term_id())?;
    let offset_att = Att::add(&mut source, offset_id())?;
    let pos_incr_att = Att::add(&mut source, pos_inc_id())?;
    let pos_length_att = Att::add(&mut source, pos_len_id())?;
    Ok(Tokenizer::with_source(
        source,
        GraphTokenizerLogic {
            tokens: None,
            upto: 0,
            input_length: 0,
            term_att,
            offset_att,
            pos_incr_att,
            pos_length_att,
        },
    ))
}

/// `String.split(String regex)` for a one-character literal separator:
/// trailing empty strings are removed, and an input without the separator
/// is returned whole.
fn java_split(s: &str, separator: char) -> Vec<String> {
    if !s.contains(separator) {
        return vec![s.to_string()];
    }
    let mut parts: Vec<String> = s.split(separator).map(str::to_string).collect();
    while parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    parts
}

impl GraphTokenizerLogic {
    /// `fillTokens()` (`private`).
    fn fill_tokens(&mut self, input: &mut dyn CharReader) -> Result<()> {
        let mut sb: Vec<u16> = Vec::new();
        let mut buffer = [0u16; 256];
        loop {
            let count = input.read(&mut buffer)?;
            // `if (count == -1) break;`: a `CharReader` answers 0 at the end.
            if count == 0 {
                break;
            }
            sb.extend_from_slice(&buffer[..count]);
        }

        self.input_length = sb.len() as i32;

        let parts = java_split(&String::from_utf16_lossy(&sb), ' ');

        let mut tokens = Vec::new();
        let mut pos = 0;
        let mut max_pos = -1;
        let mut offset = 0;
        for part in &parts {
            let overlapped = java_split(part, '/');
            let mut first_at_pos = true;
            let mut min_pos_length = i32::MAX;
            for part2 in &overlapped {
                let colon_index = part2.find(':');
                let token;
                let pos_length;
                if let Some(colon_index) = colon_index {
                    token = part2[..colon_index].to_string();
                    pos_length = part2[1 + colon_index..].parse::<i32>().map_err(|_| {
                        rucene::error::LuceneError::unchecked(
                            "java.lang.NumberFormatException",
                            format!("For input string: \"{}\"", &part2[1 + colon_index..]),
                        )
                    })?;
                } else {
                    token = part2.clone();
                    pos_length = 1;
                }
                max_pos = max_pos.max(pos + pos_length);
                min_pos_length = min_pos_length.min(pos_length);
                let mut t = Token::with_offsets(&token, offset, offset + 2 * pos_length - 1);
                t.set_position_length(pos_length);
                t.set_position_increment(if first_at_pos { 1 } else { 0 });
                first_at_pos = false;
                tokens.push(t);
            }
            pos += min_pos_length;
            offset = 2 * pos;
        }
        java_assert(max_pos <= pos, || {
            "input string mal-formed: posLength>1 tokens hang over the end".to_string()
        });
        self.tokens = Some(tokens);
        Ok(())
    }
}

impl TokenizerLogic for GraphTokenizerLogic {
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        // `super.reset()` is the `Tokenizer`'s own.
        self.tokens = None;
        self.upto = 0;
        Ok(())
    }

    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        if self.tokens.is_none() {
            self.fill_tokens(reader)?;
        }
        let tokens = self.tokens.as_ref().expect("fillTokens set the tokens");
        if self.upto == tokens.len() {
            return Ok(false);
        }
        let t = &tokens[self.upto];
        self.upto += 1;
        source.clear_attributes();
        self.term_att.char_term_mut().append_string(Some(&t.term()));
        self.offset_att
            .offset_mut()
            .set_offset(t.start_offset(), t.end_offset());
        self.pos_incr_att
            .pos_inc_mut()
            .set_position_increment(t.get_position_increment());
        self.pos_length_att
            .pos_len_mut()
            .set_position_length(t.get_position_length());
        Ok(true)
    }

    fn end(&mut self, _source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        // `super.end()` (the `Tokenizer`'s `endAttributes()`) ran first.
        // NOTE: somewhat... hackish, but we need this to
        // satisfy BTSTC:
        let last_offset = match &self.tokens {
            Some(tokens) if !tokens.is_empty() => tokens[tokens.len() - 1].end_offset(),
            _ => 0,
        };
        self.offset_att.offset_mut().set_offset(
            Tokenizer::<GraphTokenizerLogic>::correct_offset_of(input, last_offset),
            Tokenizer::<GraphTokenizerLogic>::correct_offset_of(input, self.input_length),
        );
        Ok(())
    }
}

/// An anonymous `Analyzer` of the tests: `createComponents` is `make`.
struct ChainAnalyzer {
    state: AnalyzerState,
    class_name: &'static str,
    make: fn() -> Result<TokenStreamComponents>,
}

impl std::fmt::Debug for ChainAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.class_name)
    }
}

impl Analyzer for ChainAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        self.class_name
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        (self.make)()
    }
}

/// `new Analyzer() { createComponents = make }`.
fn analyzer(
    class_name: &'static str,
    make: fn() -> Result<TokenStreamComponents>,
) -> Arc<dyn Analyzer> {
    Arc::new(ChainAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        class_name,
        make,
    })
}

/// `new MockTokenizer(MockTokenizer.WHITESPACE, false)`, shared by the
/// components and the chain.
fn whitespace_tokenizer() -> Result<Rc<RefCell<MockTokenizer>>> {
    Ok(Rc::new(RefCell::new(MockTokenizer::new(
        Arc::clone(&WHITESPACE),
        false,
    )?)))
}

/// `new TokenStreamComponents(t, sink)`.
fn sink_of<T: TokenStream + 'static>(stream: T) -> Rc<RefCell<dyn TokenStream>> {
    Rc::new(RefCell::new(stream))
}

fn test_mock_graph_token_filter_basic() -> Result<()> {
    for iter in 0..10 * *RANDOM_MULTIPLIER {
        if *VERBOSE {
            println!("\nTEST: iter={iter}");
        }

        // Make new analyzer each time, because MGTF has fixed
        // seed:
        let a = analyzer("org.apache.lucene.analysis.TestGraphTokenizers$1", || {
            let t = whitespace_tokenizer()?;
            let t2 = MockGraphTokenFilter::new(&*random(), shared(&t))?;
            Ok(components(t, sink_of(t2)))
        });

        BaseTokenStreamTestCase::check_analysis_consistency(
            &random(),
            &*a,
            false,
            "a b c d e f g h i j k",
        )?;
    }
    Ok(())
}

fn test_mock_graph_token_filter_on_graph_input() -> Result<()> {
    for iter in 0..100 * *RANDOM_MULTIPLIER {
        if *VERBOSE {
            println!("\nTEST: iter={iter}");
        }

        // Make new analyzer each time, because MGTF has fixed
        // seed:
        let a = analyzer("org.apache.lucene.analysis.TestGraphTokenizers$2", || {
            let t = Rc::new(RefCell::new(new_graph_tokenizer()?));
            let t2 = MockGraphTokenFilter::new(&*random(), shared(&t))?;
            Ok(components(t, sink_of(t2)))
        });

        BaseTokenStreamTestCase::check_analysis_consistency(
            &random(),
            &*a,
            false,
            "a/x:3 c/y:2 d e f/z:4 g h i j k",
        )?;
    }
    Ok(())
}

// Just deletes (leaving hole) token 'a':
/// `RemoveATokens` (`private static final class … extends TokenFilter`).
#[derive(Debug)]
struct RemoveATokens {
    pending_pos_inc: i32,

    term_att: Att,
    pos_inc_att: Att,
}

/// `new RemoveATokens(in)`.
fn remove_a_tokens(mut r#in: Box<dyn TokenStream>) -> Result<TokenFilter<RemoveATokens>> {
    let source = r#in.attribute_source_mut();
    let term_att = Att::add(source, char_term_id())?;
    let pos_inc_att = Att::add(source, pos_inc_id())?;
    Ok(TokenFilter::new(
        r#in,
        RemoveATokens {
            pending_pos_inc: 0,
            term_att,
            pos_inc_att,
        },
    ))
}

impl TokenFilterLogic for RemoveATokens {
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.pending_pos_inc = 0;
        Ok(())
    }

    fn end(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.end()?;
        let pos_inc = self.pos_inc_att.pos_inc().get_position_increment();
        self.pos_inc_att
            .pos_inc_mut()
            .set_position_increment(self.pending_pos_inc + pos_inc);
        Ok(())
    }

    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        loop {
            let got_one = input.increment_token()?;
            if !got_one {
                return Ok(false);
            } else if self.term_att.term_string() == "a" {
                self.pending_pos_inc += self.pos_inc_att.pos_inc().get_position_increment();
            } else {
                let pos_inc = self.pos_inc_att.pos_inc().get_position_increment();
                self.pos_inc_att
                    .pos_inc_mut()
                    .set_position_increment(self.pending_pos_inc + pos_inc);
                self.pending_pos_inc = 0;
                return Ok(true);
            }
        }
    }
}

fn test_mock_graph_token_filter_before_holes() -> Result<()> {
    for iter in 0..100 * *RANDOM_MULTIPLIER {
        if *VERBOSE {
            println!("\nTEST: iter={iter}");
        }

        // Make new analyzer each time, because MGTF has fixed
        // seed:
        let a = analyzer("org.apache.lucene.analysis.TestGraphTokenizers$3", || {
            let t = whitespace_tokenizer()?;
            let t2 = MockGraphTokenFilter::new(&*random(), shared(&t))?;
            let t3 = remove_a_tokens(Box::new(t2))?;
            Ok(components(t, sink_of(t3)))
        });

        let random = random();
        BaseTokenStreamTestCase::check_analysis_consistency(
            &random,
            &*a,
            false,
            "a b c d e f g h i j k",
        )?;
        BaseTokenStreamTestCase::check_analysis_consistency(
            &random,
            &*a,
            false,
            "x y a b c d e f g h i j k",
        )?;
        BaseTokenStreamTestCase::check_analysis_consistency(
            &random,
            &*a,
            false,
            "a b c d e f g h i j k a",
        )?;
        BaseTokenStreamTestCase::check_analysis_consistency(
            &random,
            &*a,
            false,
            "a b c d e f g h i j k a x y",
        )?;
    }
    Ok(())
}

fn test_mock_graph_token_filter_after_holes() -> Result<()> {
    for iter in 0..100 * *RANDOM_MULTIPLIER {
        if *VERBOSE {
            println!("\nTEST: iter={iter}");
        }

        // Make new analyzer each time, because MGTF has fixed
        // seed:
        let a = analyzer("org.apache.lucene.analysis.TestGraphTokenizers$4", || {
            let t = whitespace_tokenizer()?;
            let t2 = remove_a_tokens(shared(&t))?;
            let t3 = MockGraphTokenFilter::new(&*random(), Box::new(t2))?;
            Ok(components(t, sink_of(t3)))
        });

        let random = random();
        BaseTokenStreamTestCase::check_analysis_consistency(
            &random,
            &*a,
            false,
            "a b c d e f g h i j k",
        )?;
        BaseTokenStreamTestCase::check_analysis_consistency(
            &random,
            &*a,
            false,
            "x y a b c d e f g h i j k",
        )?;
        BaseTokenStreamTestCase::check_analysis_consistency(
            &random,
            &*a,
            false,
            "a b c d e f g h i j k a",
        )?;
        BaseTokenStreamTestCase::check_analysis_consistency(
            &random,
            &*a,
            false,
            "a b c d e f g h i j k a x y",
        )?;
    }
    Ok(())
}

fn test_mock_graph_token_filter_random() -> Result<()> {
    for iter in 0..3 * *RANDOM_MULTIPLIER {
        if *VERBOSE {
            println!("\nTEST: iter={iter}");
        }

        // Make new analyzer each time, because MGTF has fixed
        // seed:
        let a = analyzer("org.apache.lucene.analysis.TestGraphTokenizers$5", || {
            let t = whitespace_tokenizer()?;
            let t2 = MockGraphTokenFilter::new(&*random(), shared(&t))?;
            Ok(components(t, sink_of(t2)))
        });

        let random = random();
        BaseTokenStreamTestCase::check_random_data_max_word_length(
            &*random,
            &a,
            5,
            LuceneTestCase::at_least(100),
        )?;
    }
    Ok(())
}

// Two MockGraphTokenFilters
fn test_double_mock_graph_token_filter_random() -> Result<()> {
    for iter in 0..3 * *RANDOM_MULTIPLIER {
        if *VERBOSE {
            println!("\nTEST: iter={iter}");
        }

        // Make new analyzer each time, because MGTF has fixed
        // seed:
        let a = analyzer("org.apache.lucene.analysis.TestGraphTokenizers$6", || {
            let t = whitespace_tokenizer()?;
            let t1 = MockGraphTokenFilter::new(&*random(), shared(&t))?;
            let t2 = MockGraphTokenFilter::new(&*random(), Box::new(t1))?;
            Ok(components(t, sink_of(t2)))
        });

        let random = random();
        BaseTokenStreamTestCase::check_random_data_max_word_length(
            &*random,
            &a,
            5,
            LuceneTestCase::at_least(100),
        )?;
    }
    Ok(())
}

fn test_mock_graph_token_filter_before_holes_random() -> Result<()> {
    for iter in 0..3 * *RANDOM_MULTIPLIER {
        if *VERBOSE {
            println!("\nTEST: iter={iter}");
        }

        // Make new analyzer each time, because MGTF has fixed
        // seed:
        let a = analyzer("org.apache.lucene.analysis.TestGraphTokenizers$7", || {
            let t = whitespace_tokenizer()?;
            let t1 = MockGraphTokenFilter::new(&*random(), shared(&t))?;
            let t2 = MockHoleInjectingTokenFilter::new(&*random(), Box::new(t1))?;
            Ok(components(t, sink_of(t2)))
        });

        let random = random();
        BaseTokenStreamTestCase::check_random_data_max_word_length(
            &*random,
            &a,
            5,
            LuceneTestCase::at_least(100),
        )?;
    }
    Ok(())
}

fn test_mock_graph_token_filter_after_holes_random() -> Result<()> {
    for iter in 0..3 * *RANDOM_MULTIPLIER {
        if *VERBOSE {
            println!("\nTEST: iter={iter}");
        }

        // Make new analyzer each time, because MGTF has fixed
        // seed:
        let a = analyzer("org.apache.lucene.analysis.TestGraphTokenizers$8", || {
            let t = whitespace_tokenizer()?;
            let t1 = MockHoleInjectingTokenFilter::new(&*random(), shared(&t))?;
            let t2 = MockGraphTokenFilter::new(&*random(), Box::new(t1))?;
            Ok(components(t, sink_of(t2)))
        });

        let random = random();
        BaseTokenStreamTestCase::check_random_data_max_word_length(
            &*random,
            &a,
            5,
            LuceneTestCase::at_least(100),
        )?;
    }
    Ok(())
}

/// `token(String term, int posInc, int posLength)` (`private static`).
fn token(term: &str, pos_inc: i32, pos_length: i32) -> Token {
    let mut t = Token::with_offsets(term, 0, 0);
    t.set_position_increment(pos_inc);
    t.set_position_length(pos_length);
    t
}

/// `token(String term, int posInc, int posLength, int startOffset, int
/// endOffset)` (`private static`).
fn token_offsets(
    term: &str,
    pos_inc: i32,
    pos_length: i32,
    start_offset: i32,
    end_offset: i32,
) -> Token {
    let mut t = Token::with_offsets(term, start_offset, end_offset);
    t.set_position_increment(pos_inc);
    t.set_position_length(pos_length);
    t
}

fn test_single_token() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![token("abc", 1, 1)]);
    assert_same_language_ts(&s2a("abc"), &mut ts)
}

fn test_multiple_holes() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![token("a", 1, 1), token("b", 3, 1)]);
    assert_same_language_ts(
        &join_a(&[
            s2a("a"),
            sep_a(),
            hole_a(),
            sep_a(),
            hole_a(),
            sep_a(),
            s2a("b"),
        ]),
        &mut ts,
    )
}

fn test_syn_over_multiple_holes() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![token("a", 1, 1), token("x", 0, 3), token("b", 3, 1)]);
    let a1 = join_a(&[
        s2a("a"),
        sep_a(),
        hole_a(),
        sep_a(),
        hole_a(),
        sep_a(),
        s2a("b"),
    ]);
    let a2 = join_a(&[s2a("x"), sep_a(), s2a("b")]);
    assert_same_language_ts(&Operations::union(&[a1, a2]), &mut ts)
}

// for debugging!
/*
private static void toDot(Automaton a) throws IOException {
  final String s = a.toDot();
  Writer w = new OutputStreamWriter(new FileOutputStream("/x/tmp/out.dot"));
  w.write(s);
  w.close();
  System.out.println("TEST: saved to /x/tmp/out.dot");
}
*/

/// `SEP_A` (`private static final Automaton`).
fn sep_a() -> Automaton {
    Automata::make_char(POS_SEP)
}

/// `HOLE_A` (`private static final Automaton`).
fn hole_a() -> Automaton {
    Automata::make_char(HOLE)
}

/// `join(String... strings)` (`private`).
fn join(strings: &[&str]) -> Automaton {
    let mut r#as = Vec::new();
    for s in strings {
        r#as.push(s2a(s));
        r#as.push(sep_a());
    }
    r#as.remove(r#as.len() - 1);
    Operations::concatenate(&r#as)
}

/// `join(Automaton... as)` (`private`).
fn join_a(r#as: &[Automaton]) -> Automaton {
    Operations::concatenate(r#as)
}

/// `s2a(String s)` (`private`).
fn s2a(s: &str) -> Automaton {
    Automata::make_string(s)
}

fn test_two_tokens() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![token("abc", 1, 1), token("def", 1, 1)]);
    assert_same_language_ts(&join(&["abc", "def"]), &mut ts)
}

fn test_hole() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![token("abc", 1, 1), token("def", 2, 1)]);
    assert_same_language_ts(
        &join_a(&[s2a("abc"), sep_a(), hole_a(), sep_a(), s2a("def")]),
        &mut ts,
    )
}

fn test_overlapped_tokens_sausage() -> Result<()> {
    // Two tokens on top of each other (sausage):
    let mut ts = CannedTokenStream::new(vec![token("abc", 1, 1), token("xyz", 0, 1)]);
    let a1 = s2a("abc");
    let a2 = s2a("xyz");
    assert_same_language_ts(&Operations::union(&[a1, a2]), &mut ts)
}

fn test_overlapped_tokens_lattice() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("abc", 1, 1),
        token("xyz", 0, 2),
        token("def", 1, 1),
    ]);
    let a1 = s2a("xyz");
    let a2 = join(&["abc", "def"]);
    assert_same_language_ts(&Operations::union(&[a1, a2]), &mut ts)
}

fn test_syn_over_hole() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![token("a", 1, 1), token("X", 0, 2), token("b", 2, 1)]);
    let a1 = Operations::union(&[join_a(&[s2a("a"), sep_a(), hole_a()]), s2a("X")]);
    let expected = Operations::concatenate(&[a1, join_a(&[sep_a(), s2a("b")])]);
    assert_same_language_ts(&expected, &mut ts)
}

fn test_syn_over_hole2() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("xyz", 1, 1),
        token("abc", 0, 3),
        token("def", 2, 1),
    ]);
    let expected = Operations::union(&[
        join_a(&[s2a("xyz"), sep_a(), hole_a(), sep_a(), s2a("def")]),
        s2a("abc"),
    ]);
    assert_same_language_ts(&expected, &mut ts)
}

fn test_overlapped_tokens_lattice2() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("abc", 1, 1),
        token("xyz", 0, 3),
        token("def", 1, 1),
        token("ghi", 1, 1),
    ]);
    let a1 = s2a("xyz");
    let a2 = join(&["abc", "def", "ghi"]);
    assert_same_language_ts(&Operations::union(&[a1, a2]), &mut ts)
}

fn test_to_dot() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![token_offsets("abc", 1, 1, 0, 4)]);
    // `new TokenStreamToDot("abcd", ts, new PrintWriter(w)).toDot()`: the
    // writer is the `out` buffer.
    let mut to_dot = TokenStreamToDot::new(Some("abcd"), &mut ts)?;
    to_dot.to_dot()?;
    Assert::assert_true(to_dot.out.contains("abc / abcd"));
    Ok(())
}

fn test_starts_with_hole() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![token("abc", 2, 1)]);
    assert_same_language_ts(&join_a(&[hole_a(), sep_a(), s2a("abc")]), &mut ts)
}

fn test_ends_with_hole() -> Result<()> {
    let mut ts = CannedTokenStream::with_final(1, 0, vec![token("abc", 2, 1)]);
    assert_same_language_ts(
        &join_a(&[hole_a(), sep_a(), s2a("abc"), sep_a(), hole_a()]),
        &mut ts,
    )
}

fn test_syn_hanging_over_end() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![token("a", 1, 1), token("X", 0, 10)]);
    assert_same_language_ts(&Operations::union(&[s2a("a"), s2a("X")]), &mut ts)
}

/// Returns all paths
fn to_path_strings(a: &Automaton) -> Result<JavaHashSet> {
    let mut scratch_bytes_ref_builder = BytesRefBuilder::new();
    let mut paths = JavaHashSet::new();
    for ir in AutomatonTestUtil::get_finite_strings_recursive(a, -1) {
        let ir = IntsRef::new(ir);
        let path = Util::to_bytes_ref(&ir, &mut scratch_bytes_ref_builder)?
            .utf8_to_string()?
            .replace(char::from(POS_SEP as u8), " ");
        paths.insert(path);
    }
    Ok(paths)
}

/// `assertSameLanguage(Automaton expected, TokenStream ts)` (`private`).
fn assert_same_language_ts(expected: &Automaton, ts: &mut dyn TokenStream) -> Result<()> {
    let actual = TokenStreamToAutomaton::new().to_automaton(ts)?;
    assert_same_language(expected, &actual)
}

/// `assertSameLanguage(Automaton expected, Automaton actual)` (`private`).
fn assert_same_language(expected: &Automaton, actual: &Automaton) -> Result<()> {
    let expected_det = Operations::determinize(
        &Operations::remove_dead_states(expected),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    )?;
    let actual_det = Operations::determinize(
        &Operations::remove_dead_states(actual),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    )?;
    if !AutomatonTestUtil::same_language(&expected_det, &actual_det) {
        let expected_paths = to_path_strings(&expected_det)?;
        let actual_paths = to_path_strings(&actual_det)?;
        let mut b = String::new();
        b.push_str("expected:\n");
        for path in expected_paths.iter() {
            b.push_str("  ");
            b.push_str(path);
            if !actual_paths.contains(path) {
                b.push_str(" [missing!]");
            }
            b.push('\n');
        }
        b.push_str("actual:\n");
        for path in actual_paths.iter() {
            b.push_str("  ");
            b.push_str(path);
            if !expected_paths.contains(path) {
                b.push_str(" [unexpected!]");
            }
            b.push('\n');
        }
        Assert::fail(&format!("accepted language is different:\n\n{b}"));
    }
    Ok(())
}

fn test_token_stream_graph_with_holes() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        token("abc", 1, 1),
        token("xyz", 1, 8),
        token("def", 1, 1),
        token("ghi", 1, 1),
    ]);
    assert_same_language_ts(
        &Operations::union(&[
            join_a(&[s2a("abc"), sep_a(), s2a("xyz")]),
            join_a(&[
                s2a("abc"),
                sep_a(),
                hole_a(),
                sep_a(),
                s2a("def"),
                sep_a(),
                s2a("ghi"),
            ]),
        ]),
        &mut ts,
    )
}

/// The class `org.apache.lucene.analysis.TestGraphTokenizers`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.analysis.TestGraphTokenizers")
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(t(
            "testMockGraphTokenFilterBasic",
            test_mock_graph_token_filter_basic,
        ))
        .method(t(
            "testMockGraphTokenFilterOnGraphInput",
            test_mock_graph_token_filter_on_graph_input,
        ))
        .method(t(
            "testMockGraphTokenFilterBeforeHoles",
            test_mock_graph_token_filter_before_holes,
        ))
        .method(t(
            "testMockGraphTokenFilterAfterHoles",
            test_mock_graph_token_filter_after_holes,
        ))
        .method(t(
            "testMockGraphTokenFilterRandom",
            test_mock_graph_token_filter_random,
        ))
        .method(t(
            "testDoubleMockGraphTokenFilterRandom",
            test_double_mock_graph_token_filter_random,
        ))
        .method(t(
            "testMockGraphTokenFilterBeforeHolesRandom",
            test_mock_graph_token_filter_before_holes_random,
        ))
        .method(t(
            "testMockGraphTokenFilterAfterHolesRandom",
            test_mock_graph_token_filter_after_holes_random,
        ))
        .method(t("testSingleToken", test_single_token))
        .method(t("testMultipleHoles", test_multiple_holes))
        .method(t("testSynOverMultipleHoles", test_syn_over_multiple_holes))
        .method(t("testTwoTokens", test_two_tokens))
        .method(t("testHole", test_hole))
        .method(t(
            "testOverlappedTokensSausage",
            test_overlapped_tokens_sausage,
        ))
        .method(t(
            "testOverlappedTokensLattice",
            test_overlapped_tokens_lattice,
        ))
        .method(t("testSynOverHole", test_syn_over_hole))
        .method(t("testSynOverHole2", test_syn_over_hole2))
        .method(t(
            "testOverlappedTokensLattice2",
            test_overlapped_tokens_lattice2,
        ))
        .method(t("testToDot", test_to_dot))
        .method(t("testStartsWithHole", test_starts_with_hole))
        .method(t("testEndsWithHole", test_ends_with_hole))
        .method(t("testSynHangingOverEnd", test_syn_hanging_over_end))
        .method(t(
            "testTokenStreamGraphWithHoles",
            test_token_stream_graph_with_holes,
        ))
        .build()
}
