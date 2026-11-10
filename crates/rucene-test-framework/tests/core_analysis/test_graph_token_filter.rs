//! Port of `org.apache.lucene.analysis.TestGraphTokenFilter`
//! (`extends BaseTokenStreamTestCase`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::analysis::graph_token_filter::{GraphTokenFilter, MAX_TOKEN_CACHE_SIZE};
use rucene::analysis::{StringCharReader, TokenStream};
use rucene::error::Result;
use rucene_test_framework::analysis::attributes::{char_term_id, pos_inc_id, Att};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::{shared, BaseTokenStreamTestCase, CannedTokenStream};
use rucene_test_framework::util::java_exceptions::expect_throws;

use super::test_graph_tokenizers::new_graph_tokenizer;
use crate::lucene_core_shims::t;

/// `new TestFilter(input)`: `TestFilter extends GraphTokenFilter`, whose only
/// override is `incrementToken() { return incrementBaseToken(); }`.
///
/// **Divergence (declared).** Rucene's `GraphTokenFilter` is a concrete
/// type with Java's protected methods public, not a `TokenStream` base
/// class; the tests call those methods on it directly, as Java's do on
/// `TestFilter`, and never call `incrementToken()`.
fn test_filter(input: Box<dyn TokenStream>) -> Result<GraphTokenFilter> {
    GraphTokenFilter::new(input)
}

fn test_graph_token_stream() -> Result<()> {
    // `TestGraphTokenizers.GraphTokenizer tok = new GraphTokenizer()`: shared
    // by the filter and the test, which drives it.
    let tok = Rc::new(RefCell::new(new_graph_tokenizer()?));
    let mut graph = test_filter(shared(&tok))?;

    let term_att = Att::add(graph.attribute_source_mut(), char_term_id())?;
    let pos_inc_att = Att::add(graph.attribute_source_mut(), pos_inc_id())?;

    tok.borrow_mut()
        .set_reader(Box::new(StringCharReader::new("a b/c d e/f:3 g/h i j k")))?;
    tok.borrow_mut().reset()?;

    Assert::assert_false(graph.increment_graph()?);
    Assert::assert_equals(0, graph.cached_token_count());

    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_equals_str(None, Some("a"), Some(&term_att.term_string()));
    Assert::assert_equals(1, pos_inc_att.pos_inc().get_position_increment());
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("b"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("d"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph()?);
    Assert::assert_equals_str(None, Some("a"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("c"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("d"), Some(&term_att.term_string()));
    Assert::assert_false(graph.increment_graph()?);
    Assert::assert_equals(5, graph.cached_token_count());

    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_equals_str(None, Some("b"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("d"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("e"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph()?);
    Assert::assert_equals_str(None, Some("b"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("d"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("f"), Some(&term_att.term_string()));
    Assert::assert_false(graph.increment_graph()?);
    Assert::assert_equals(6, graph.cached_token_count());

    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_equals_str(None, Some("c"), Some(&term_att.term_string()));
    Assert::assert_equals(0, pos_inc_att.pos_inc().get_position_increment());
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("d"), Some(&term_att.term_string()));
    Assert::assert_false(graph.increment_graph()?);
    Assert::assert_equals(6, graph.cached_token_count());

    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_equals_str(None, Some("d"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("e"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("g"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph()?);
    Assert::assert_equals_str(None, Some("d"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("e"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("h"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph()?);
    Assert::assert_equals_str(None, Some("d"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("f"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("j"), Some(&term_att.term_string()));
    Assert::assert_false(graph.increment_graph()?);
    Assert::assert_equals(8, graph.cached_token_count());

    // tok.setReader(new StringReader("a b/c d e/f:3 g/h i j k"));

    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_equals_str(None, Some("e"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("g"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("i"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("j"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph()?);
    Assert::assert_equals_str(None, Some("e"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("h"), Some(&term_att.term_string()));
    Assert::assert_false(graph.increment_graph()?);
    Assert::assert_equals(8, graph.cached_token_count());

    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_equals_str(None, Some("f"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("j"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("k"), Some(&term_att.term_string()));
    Assert::assert_false(graph.increment_graph_token()?);
    Assert::assert_false(graph.increment_graph()?);
    Assert::assert_equals(8, graph.cached_token_count());

    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_equals_str(None, Some("g"), Some(&term_att.term_string()));
    Assert::assert_true(graph.increment_graph_token()?);
    Assert::assert_equals_str(None, Some("i"), Some(&term_att.term_string()));
    Assert::assert_false(graph.increment_graph()?);
    Assert::assert_equals(8, graph.cached_token_count());

    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_equals_str(None, Some("h"), Some(&term_att.term_string()));
    Assert::assert_false(graph.increment_graph()?);
    Assert::assert_equals(8, graph.cached_token_count());

    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_true(graph.increment_base_token()?);
    Assert::assert_equals_str(None, Some("k"), Some(&term_att.term_string()));
    Assert::assert_false(graph.increment_graph_token()?);
    Assert::assert_equals(0, graph.get_trailing_positions());
    Assert::assert_false(graph.increment_graph()?);
    Assert::assert_false(graph.increment_base_token()?);
    Assert::assert_equals(8, graph.cached_token_count());
    Ok(())
}

fn test_trailing_positions() -> Result<()> {
    // a/b:2 c _
    let cts = CannedTokenStream::with_final(
        1,
        5,
        vec![
            Token::with_offsets("a", 0, 1),
            Token::with_pos_len("b", 0, 0, 1, 2),
            Token::with_pos_inc("c", 1, 2, 3),
        ],
    );

    let mut gts = test_filter(Box::new(cts))?;
    Assert::assert_false(gts.increment_graph()?);
    Assert::assert_true(gts.increment_base_token()?);
    Assert::assert_true(gts.increment_graph_token()?);
    Assert::assert_false(gts.increment_graph_token()?);
    Assert::assert_equals(1, gts.get_trailing_positions());
    Assert::assert_false(gts.increment_graph()?);
    Assert::assert_true(gts.increment_base_token()?);
    Assert::assert_false(gts.increment_graph_token()?);
    Assert::assert_equals(1, gts.get_trailing_positions());
    Assert::assert_false(gts.increment_graph()?);
    Ok(())
}

fn test_maximum_graph_cache_size() -> Result<()> {
    let mut tokens = Vec::with_capacity(MAX_TOKEN_CACHE_SIZE + 5);
    for i in 0..(MAX_TOKEN_CACHE_SIZE + 5) as i32 {
        tokens.push(Token::with_pos_inc("a", 1, i * 2, i * 2 + 1));
    }

    let mut gts = test_filter(Box::new(CannedTokenStream::new(tokens)))?;
    let e = expect_throws("java.lang.IllegalStateException", || -> Result<()> {
        gts.reset()?;
        gts.increment_base_token()?;
        loop {
            gts.increment_graph_token()?;
        }
    });
    Assert::assert_equals_str(None, Some("Too many cached tokens (> 100)"), e.message());

    gts.reset()?;
    // after reset, the cache should be cleared and so we can read ahead once more
    gts.increment_base_token()?;
    gts.increment_graph_token()?;
    Ok(())
}

fn test_graph_path_count_limits() -> Result<()> {
    let mut tokens = Vec::with_capacity(50);
    tokens.push(Token::with_pos_inc("term", 1, 0, 1));
    tokens.push(Token::with_pos_inc("term1", 1, 2, 3));
    for i in 2..50 {
        tokens.push(Token::with_pos_inc(&format!("term{i}"), i % 2, 2, 3));
    }

    let e = expect_throws("java.lang.IllegalStateException", || -> Result<()> {
        let mut graph = test_filter(Box::new(CannedTokenStream::new(tokens)))?;
        graph.reset()?;
        graph.increment_base_token()?;
        for _ in 0..10 {
            graph.increment_graph_token()?;
        }
        while graph.increment_graph()? {
            for _ in 0..10 {
                graph.increment_graph_token()?;
            }
        }
        Ok(())
    });
    Assert::assert_equals_str(None, Some("Too many graph paths (> 1000)"), e.message());
    Ok(())
}

/// The class `org.apache.lucene.analysis.TestGraphTokenFilter`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.analysis.TestGraphTokenFilter")
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(t("testGraphTokenStream", test_graph_token_stream))
        .method(t("testTrailingPositions", test_trailing_positions))
        .method(t(
            "testMaximumGraphCacheSize",
            test_maximum_graph_cache_size,
        ))
        .method(t("testGraphPathCountLimits", test_graph_path_count_limits))
        .build()
}
