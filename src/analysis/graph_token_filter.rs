//! `GraphTokenFilter` ported from `org.apache.lucene.analysis`.
//!
//! Gives a filter a view of the token *graph* rather than the token line: the
//! paths a synonym expansion opens up, walked one at a time.

use std::collections::VecDeque;

use crate::analysis::tokenattributes::{
    OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute,
};
use crate::analysis::TokenStream;
use crate::error::{LuceneError, Result};
use crate::util::attribute::CapturedState;

/// Largest number of graph paths one base token may open.
///
/// Equivalent to `GraphTokenFilter.MAX_GRAPH_STACK_SIZE`.
pub const MAX_GRAPH_STACK_SIZE: usize = 1000;
/// Largest number of tokens held in the lookahead cache.
///
/// Equivalent to `GraphTokenFilter.MAX_TOKEN_CACHE_SIZE`.
pub const MAX_TOKEN_CACHE_SIZE: usize = 100;

/// One token held in the lookahead cache.
///
/// Equivalent to `GraphTokenFilter.Token`.
#[derive(Clone, Debug)]
struct Token {
    state: CapturedState,
    pos_inc: i32,
    pos_length: i32,
    /// Index of the token that follows in the stream, once it has been read.
    next_token: Option<usize>,
}

/// Walks the paths through a token graph.
///
/// Equivalent to `org.apache.lucene.analysis.GraphTokenFilter`.
///
/// **Divergence from Lucene 10.5.0.** Java holds the cached tokens as a linked
/// structure of `Token` objects recycled through an `ArrayDeque` pool, each
/// carrying its own cloned `AttributeSource`. Rust cannot hold that graph of
/// mutable back-references, so this port keeps the tokens in an arena `Vec` and
/// links them by index, recycling the same way through a free list. The paths
/// walked and their order are unchanged.
pub struct GraphTokenFilter {
    input: Box<dyn TokenStream>,
    /// Every token read so far, linked by index.
    tokens: Vec<Token>,
    /// Indices free for reuse.
    token_pool: VecDeque<usize>,
    /// Indices of the tokens on the current path, deepest last.
    current_graph: Vec<usize>,
    /// The token every path starts from.
    base_token: Option<usize>,
    graph_depth: usize,
    graph_pos: usize,
    /// Position increment the stream reported at its end, or `-1`.
    trailing_positions: i32,
    /// End offset the stream reported at its end, or `-1`.
    final_offsets: i32,
    stack_size: usize,
    cache_size: usize,
}

/// The position increment of the stream's current token.
fn pos_inc_of(source: &crate::util::attribute::AttributeSource) -> i32 {
    source
        .get_attribute_interface::<dyn PositionIncrementAttribute>()
        .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
        .get_position_increment()
}

impl std::fmt::Debug for GraphTokenFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphTokenFilter")
            .field("graph_depth", &self.graph_depth)
            .field("graph_pos", &self.graph_pos)
            .finish_non_exhaustive()
    }
}

impl GraphTokenFilter {
    /// Wraps `input`, adding the `PositionIncrementAttribute` and the
    /// `OffsetAttribute` to it (`GraphTokenFilter.java:64-68`).
    ///
    /// # Errors
    ///
    /// What the input's attribute factory raises for an interface it cannot
    /// implement, as Java's `addAttribute` throws.
    pub fn new(mut input: Box<dyn TokenStream>) -> Result<Self> {
        let source = input.attribute_source_mut();
        source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        source.add_attribute_interface::<dyn OffsetAttribute>()?;
        Ok(Self {
            input,
            tokens: Vec::new(),
            token_pool: VecDeque::new(),
            current_graph: Vec::new(),
            base_token: None,
            graph_depth: 0,
            graph_pos: 0,
            trailing_positions: -1,
            final_offsets: -1,
            stack_size: 0,
            cache_size: 0,
        })
    }

    /// The attributes of the stream, which are its input's: Java's
    /// `GraphTokenFilter` is a `TokenFilter`, whose `AttributeSource` shares
    /// the input's attributes.
    pub fn attribute_source(&self) -> &crate::util::attribute::AttributeSource {
        self.input.attribute_source()
    }

    /// The attributes of the stream, mutably; see
    /// [`attribute_source`](Self::attribute_source).
    pub fn attribute_source_mut(&mut self) -> &mut crate::util::attribute::AttributeSource {
        self.input.attribute_source_mut()
    }

    /// Closes the input.
    ///
    /// Equivalent to the inherited `TokenFilter.close()`, which closes the
    /// input.
    ///
    /// # Errors
    ///
    /// What the input's `close` raises.
    pub fn close(&mut self) -> Result<()> {
        self.input.close()
    }

    /// Returns the position increment the stream reported at its end.
    ///
    /// Equivalent to `getTrailingPositions()`.
    pub fn get_trailing_positions(&self) -> i32 {
        self.trailing_positions
    }

    /// Returns how many tokens the lookahead cache has allocated since the
    /// last [`reset`](Self::reset); recycled tokens are not counted again.
    ///
    /// Equivalent to the package-private `GraphTokenFilter#cachedTokenCount`
    /// (`GraphTokenFilter.java:177-179`), which returns `cacheSize`.
    pub fn cached_token_count(&self) -> i32 {
        self.cache_size as i32
    }

    /// Captures the input's current token into the cache, reusing a slot when
    /// one is free.
    ///
    /// Equivalent to `GraphTokenFilter.newToken`.
    fn new_token(&mut self) -> Result<usize> {
        let (state, pos_inc, pos_length) = {
            let source = self.input.attribute_source();
            let state = source.capture_state().ok_or_else(|| {
                LuceneError::illegal_state("token stream has no attributes to capture".to_string())
            })?;
            // `Token(AttributeSource)` (`GraphTokenFilter.java:247-251`): the
            // position length is 1 when the stream has no
            // `PositionLengthAttribute`.
            let pos_inc = pos_inc_of(source);
            let pos_length = source
                .get_attribute_interface::<dyn PositionLengthAttribute>()
                .map_or(1, |att| att.get_position_length());
            (state, pos_inc, pos_length)
        };

        if let Some(index) = self.token_pool.pop_front() {
            self.tokens[index] = Token {
                state,
                pos_inc,
                pos_length,
                next_token: None,
            };
            return Ok(index);
        }

        self.cache_size += 1;
        if self.cache_size > MAX_TOKEN_CACHE_SIZE {
            return Err(LuceneError::illegal_state(format!(
                "Too many cached tokens (> {MAX_TOKEN_CACHE_SIZE})"
            )));
        }
        self.tokens.push(Token {
            state,
            pos_inc,
            pos_length,
            next_token: None,
        });
        Ok(self.tokens.len() - 1)
    }

    /// Returns a cached slot to the pool.
    fn recycle_token(&mut self, token: Option<usize>) {
        if let Some(index) = token {
            self.tokens[index].next_token = None;
            self.token_pool.push_back(index);
        }
    }

    /// Returns the token that follows `token` in the input, reading one more if
    /// the cache does not already hold it.
    ///
    /// Equivalent to `GraphTokenFilter.nextTokenInStream`.
    fn next_token_in_stream(&mut self, token: Option<usize>) -> Result<Option<usize>> {
        if let Some(index) = token {
            if let Some(next) = self.tokens[index].next_token {
                return Ok(Some(next));
            }
        }
        if self.trailing_positions != -1 {
            // The end has already been reached.
            return Ok(None);
        }
        if !self.input.increment_token()? {
            self.input.end()?;
            let source = self.input.attribute_source();
            self.trailing_positions = pos_inc_of(source);
            self.final_offsets = source
                .get_attribute_interface::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute")
                .end_offset();
            return Ok(None);
        }
        let new_index = self.new_token()?;
        if let Some(index) = token {
            self.tokens[index].next_token = Some(new_index);
        }
        Ok(Some(new_index))
    }

    /// Skips forward to the token that starts where `token` ends, which is the
    /// next token along the same graph path.
    ///
    /// Equivalent to `GraphTokenFilter.nextTokenInGraph`.
    fn next_token_in_graph(&mut self, token: usize) -> Result<Option<usize>> {
        let mut remaining = self.tokens[token].pos_length;
        let mut current = Some(token);
        loop {
            current = self.next_token_in_stream(current)?;
            let Some(index) = current else {
                return Ok(None);
            };
            remaining -= self.tokens[index].pos_inc;
            if remaining <= 0 {
                return Ok(Some(index));
            }
        }
    }

    /// Returns whether no further token shares `token`'s position, which is
    /// what makes it the last alternative in its stack.
    ///
    /// Equivalent to `GraphTokenFilter.lastInStack`.
    fn last_in_stack(&mut self, token: usize) -> Result<bool> {
        let next = self.next_token_in_stream(Some(token))?;
        Ok(match next {
            None => true,
            Some(index) => self.tokens[index].pos_inc != 0,
        })
    }

    /// Restores the attributes of the token at `index` onto the input.
    fn restore(&self, index: usize) -> Result<()> {
        self.input
            .attribute_source()
            .restore_state(&self.tokens[index].state)
    }

    /// Advances to the next base token, resetting the graph walk.
    ///
    /// Equivalent to `GraphTokenFilter.incrementBaseToken`.
    pub fn increment_base_token(&mut self) -> Result<bool> {
        self.stack_size = 0;
        self.graph_depth = 0;
        self.graph_pos = 0;
        let old_base = self.base_token;
        self.base_token = self.next_token_in_stream(old_base)?;
        let Some(base) = self.base_token else {
            return Ok(false);
        };
        self.current_graph.clear();
        self.current_graph.push(base);
        self.restore(base)?;
        self.recycle_token(old_base);
        Ok(true)
    }

    /// Advances one token along the current path.
    ///
    /// Equivalent to `GraphTokenFilter.incrementGraphToken`.
    pub fn increment_graph_token(&mut self) -> Result<bool> {
        if self.graph_pos < self.graph_depth {
            self.graph_pos += 1;
            let index = self.current_graph[self.graph_pos];
            self.restore(index)?;
            return Ok(true);
        }
        let deepest = self.current_graph[self.graph_depth];
        let Some(token) = self.next_token_in_graph(deepest)? else {
            return Ok(false);
        };
        self.graph_depth += 1;
        self.graph_pos += 1;
        if self.current_graph.len() > self.graph_depth {
            self.current_graph[self.graph_depth] = token;
        } else {
            self.current_graph.push(token);
        }
        self.restore(token)?;
        Ok(true)
    }

    /// Backtracks to the next path through the graph.
    ///
    /// Equivalent to `GraphTokenFilter.incrementGraph`.
    pub fn increment_graph(&mut self) -> Result<bool> {
        if self.base_token.is_none() {
            return Ok(false);
        }
        self.graph_pos = 0;
        // Back out of the deepest position that still has an alternative.
        for i in (1..=self.graph_depth).rev() {
            let at_i = self.current_graph[i];
            if !self.last_in_stack(at_i)? {
                let Some(next) = self.next_token_in_stream(Some(at_i))? else {
                    continue;
                };
                self.current_graph[i] = next;
                for j in (i + 1)..self.graph_depth {
                    let prev = self.current_graph[j];
                    match self.next_token_in_graph(prev)? {
                        Some(token) => self.current_graph[j] = token,
                        None => break,
                    }
                }
                self.stack_size += 1;
                if self.stack_size > MAX_GRAPH_STACK_SIZE {
                    return Err(LuceneError::illegal_state(format!(
                        "Too many graph paths (> {MAX_GRAPH_STACK_SIZE})"
                    )));
                }
                let base = self.current_graph[0];
                self.restore(base)?;
                self.graph_depth = i;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Ends the stream.
    ///
    /// Equivalent to `GraphTokenFilter.end()` (`GraphTokenFilter.java:151-162`):
    /// when the input's end has not been reached yet, the input is ended and
    /// its final position increment and end offset are recorded; once they
    /// are recorded, the attributes are ended and the recorded position
    /// increment and final offset are set again.
    ///
    /// # Errors
    ///
    /// Returns whatever ending the input returns.
    pub fn end(&mut self) -> Result<()> {
        if self.trailing_positions == -1 {
            self.input.end()?;
            let source = self.input.attribute_source();
            self.trailing_positions = pos_inc_of(source);
            self.final_offsets = source
                .get_attribute_interface::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute")
                .end_offset();
        } else {
            let source = self.input.attribute_source();
            source.end_attributes();
            source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .set_position_increment(self.trailing_positions);
            source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute")
                .set_offset(self.final_offsets, self.final_offsets);
        }
        Ok(())
    }

    /// Resets the filter and its input.
    ///
    /// Equivalent to `GraphTokenFilter.reset()`.
    pub fn reset(&mut self) -> Result<()> {
        self.input.reset()?;
        self.tokens.clear();
        self.token_pool.clear();
        self.current_graph.clear();
        self.base_token = None;
        self.graph_depth = 0;
        self.graph_pos = 0;
        self.trailing_positions = -1;
        self.final_offsets = -1;
        self.stack_size = 0;
        self.cache_size = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::default_token_attribute_factory;
    use crate::analysis::tokenattributes::CharTermAttribute;
    use crate::error::LuceneErrorKind;
    use crate::util::attribute::AttributeSource;

    /// One token of the graph syntax: term, start and end offsets, position
    /// increment and position length.
    type GraphToken = (String, i32, i32, i32, i32);

    /// Port of the test fixture `TestGraphTokenizers.GraphTokenizer`
    /// (`TestGraphTokenizers.java:65-169`): tokens separated by spaces,
    /// alternatives at one position separated by `/`, and `:n` giving a
    /// token a position length of `n`.
    #[derive(Debug)]
    struct GraphTokenizer {
        source: AttributeSource,
        tokens: Vec<GraphToken>,
        upto: usize,
    }

    impl GraphTokenizer {
        fn new(text: &str) -> Self {
            let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
            source
                .add_attribute_interface::<dyn CharTermAttribute>()
                .expect("the default token factory implements CharTermAttribute");
            source
                .add_attribute_interface::<dyn OffsetAttribute>()
                .expect("the default token factory implements OffsetAttribute");
            source
                .add_attribute_interface::<dyn PositionIncrementAttribute>()
                .expect("the default token factory implements PositionIncrementAttribute");
            source
                .add_attribute_interface::<dyn PositionLengthAttribute>()
                .expect("the default token factory implements PositionLengthAttribute");
            let mut tokens = Vec::new();
            let (mut pos, mut offset) = (0i32, 0i32);
            for part in text.split(' ') {
                let mut first_at_pos = true;
                let mut min_pos_length = i32::MAX;
                for part2 in part.split('/') {
                    let (token, pos_length) = match part2.find(':') {
                        Some(colon) => (&part2[..colon], part2[colon + 1..].parse().unwrap()),
                        None => (part2, 1),
                    };
                    min_pos_length = min_pos_length.min(pos_length);
                    let pos_inc = if first_at_pos { 1 } else { 0 };
                    first_at_pos = false;
                    tokens.push((
                        token.to_string(),
                        offset,
                        offset + 2 * pos_length - 1,
                        pos_inc,
                        pos_length,
                    ));
                }
                pos += min_pos_length;
                offset = 2 * pos;
            }
            Self {
                source,
                tokens,
                upto: 0,
            }
        }
    }

    impl TokenStream for GraphTokenizer {
        fn increment_token(&mut self) -> Result<bool> {
            if self.upto == self.tokens.len() {
                return Ok(false);
            }
            let (term, start, end, pos_inc, pos_length) = self.tokens[self.upto].clone();
            self.upto += 1;
            self.source.clear_attributes();
            let source = &self.source;
            source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("added at construction")
                .append_string(Some(&term));
            source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("added at construction")
                .set_offset(start, end);
            source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("added at construction")
                .set_position_increment(pos_inc);
            source
                .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
                .expect("added at construction")
                .set_position_length(pos_length);
            Ok(true)
        }

        fn reset(&mut self) -> Result<()> {
            self.upto = 0;
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            self.source.end_attributes();
            Ok(())
        }

        fn attribute_source(&self) -> &AttributeSource {
            &self.source
        }

        fn attribute_source_mut(&mut self) -> &mut AttributeSource {
            &mut self.source
        }
    }

    /// The term the filter currently exposes.
    fn term(graph: &GraphTokenFilter) -> String {
        let term_att = graph
            .input
            .attribute_source()
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("added at construction");
        term_att.buffer()[..term_att.length()].iter().collect()
    }

    /// `cachedTokenCount` counts the tokens the lookahead cache has allocated,
    /// and recycling keeps it from growing. This is Lucene's own
    /// `TestGraphTokenFilter.testGraphTokenStream` (`TestGraphTokenFilter.java:42-164`),
    /// assertion for assertion, over the same input.
    #[test]
    fn cached_token_count_follows_lucene_graph_token_stream_test() {
        let mut graph =
            GraphTokenFilter::new(Box::new(GraphTokenizer::new("a b/c d e/f:3 g/h i j k")))
                .unwrap();
        graph.input.reset().unwrap();

        let step = |graph: &mut GraphTokenFilter, op: &str, expected: Option<&str>| {
            let moved = match op {
                "base" => graph.increment_base_token(),
                "token" => graph.increment_graph_token(),
                _ => graph.increment_graph(),
            }
            .unwrap();
            match expected {
                Some(t) => {
                    assert!(moved, "{op} should move to {t}");
                    assert_eq!(term(graph), t, "after {op}");
                }
                None => assert!(!moved, "{op} should not move"),
            }
        };

        step(&mut graph, "graph", None);
        assert_eq!(graph.cached_token_count(), 0);

        for (op, t) in [
            ("base", "a"),
            ("token", "b"),
            ("token", "d"),
            ("graph", "a"),
            ("token", "c"),
            ("token", "d"),
        ] {
            step(&mut graph, op, Some(t));
        }
        step(&mut graph, "graph", None);
        assert_eq!(graph.cached_token_count(), 5);

        for (op, t) in [
            ("base", "b"),
            ("token", "d"),
            ("token", "e"),
            ("graph", "b"),
            ("token", "d"),
            ("token", "f"),
        ] {
            step(&mut graph, op, Some(t));
        }
        step(&mut graph, "graph", None);
        assert_eq!(graph.cached_token_count(), 6);

        for (op, t) in [("base", "c"), ("token", "d")] {
            step(&mut graph, op, Some(t));
        }
        step(&mut graph, "graph", None);
        assert_eq!(graph.cached_token_count(), 6);

        for (op, t) in [
            ("base", "d"),
            ("token", "e"),
            ("token", "g"),
            ("graph", "d"),
            ("token", "e"),
            ("token", "h"),
            ("graph", "d"),
            ("token", "f"),
            ("token", "j"),
        ] {
            step(&mut graph, op, Some(t));
        }
        step(&mut graph, "graph", None);
        assert_eq!(graph.cached_token_count(), 8);

        for (op, t) in [
            ("base", "e"),
            ("token", "g"),
            ("token", "i"),
            ("token", "j"),
            ("graph", "e"),
            ("token", "h"),
        ] {
            step(&mut graph, op, Some(t));
        }
        step(&mut graph, "graph", None);
        assert_eq!(graph.cached_token_count(), 8);

        for (op, t) in [("base", "f"), ("token", "j"), ("token", "k")] {
            step(&mut graph, op, Some(t));
        }
        step(&mut graph, "token", None);
        step(&mut graph, "graph", None);
        assert_eq!(graph.cached_token_count(), 8);

        for (op, t) in [("base", "g"), ("token", "i")] {
            step(&mut graph, op, Some(t));
        }
        step(&mut graph, "graph", None);
        assert_eq!(graph.cached_token_count(), 8);

        step(&mut graph, "base", Some("h"));
        step(&mut graph, "graph", None);
        assert_eq!(graph.cached_token_count(), 8);

        step(&mut graph, "base", Some("i"));
        step(&mut graph, "base", Some("j"));
        step(&mut graph, "base", Some("k"));
        step(&mut graph, "token", None);
        assert_eq!(graph.get_trailing_positions(), 0);
        step(&mut graph, "graph", None);
        step(&mut graph, "base", None);
        assert_eq!(graph.cached_token_count(), 8);
    }

    /// The count is capped at `MAX_TOKEN_CACHE_SIZE`, and `reset()` clears
    /// it: Lucene's `TestGraphTokenFilter.testMaximumGraphCacheSize`
    /// (`TestGraphTokenFilter.java:186-210`), plus the count it implies.
    #[test]
    fn cached_token_count_is_capped_and_cleared_by_reset() {
        let text = vec!["a"; MAX_TOKEN_CACHE_SIZE + 5].join(" ");
        let mut graph = GraphTokenFilter::new(Box::new(GraphTokenizer::new(&text))).unwrap();
        graph.reset().unwrap();
        graph.increment_base_token().unwrap();
        let err = loop {
            if let Err(e) = graph.increment_graph_token() {
                break e;
            }
        };
        match err {
            LuceneError {
                kind: LuceneErrorKind::IllegalState { message, .. },
                ..
            } => {
                assert_eq!(message, "Too many cached tokens (> 100)")
            }
            other => panic!("expected IllegalStateException, got {other:?}"),
        }
        assert_eq!(graph.cached_token_count(), MAX_TOKEN_CACHE_SIZE as i32 + 1);

        graph.reset().unwrap();
        assert_eq!(graph.cached_token_count(), 0);
        // After reset the cache is clear, so the filter can read ahead again.
        assert!(graph.increment_base_token().unwrap());
        assert!(graph.increment_graph_token().unwrap());
        assert_eq!(graph.cached_token_count(), 2);
    }

    /// `MiscOracle` O6b (`oracle/h2/MiscOracle.java`): `end()` after the walk
    /// has reached the input's end ends the attributes and sets the recorded
    /// position increment and final offset again — over a stop filter that
    /// dropped the trailing `the`, Java printed `end(1,7,7)` both times, the
    /// second after the attributes had been changed.
    #[test]
    fn end_sets_the_recorded_trailing_state_again() {
        let mut tokenizer = crate::analysis::standard::new_standard_tokenizer();
        tokenizer
            .set_reader(Box::new(crate::analysis::StringCharReader::new("a b the")))
            .unwrap();
        let stop = crate::analysis::new_stop_filter(
            Box::new(tokenizer),
            crate::analysis::CharArraySet::from_iter(["the"], false),
        )
        .unwrap();
        let mut graph = GraphTokenFilter::new(Box::new(stop)).unwrap();
        graph.reset().unwrap();
        let mut tokens = 0;
        while graph.increment_base_token().unwrap() {
            tokens += 1;
        }
        assert_eq!(tokens, 2);
        let state = |graph: &GraphTokenFilter| {
            let source = graph.input.attribute_source();
            let pos_inc = pos_inc_of(source);
            let offset = source
                .get_attribute_interface::<dyn OffsetAttribute>()
                .unwrap();
            (pos_inc, offset.start_offset(), offset.end_offset())
        };
        graph.end().unwrap();
        assert_eq!(state(&graph), (1, 7, 7));
        {
            let source = graph.input.attribute_source();
            source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .unwrap()
                .set_position_increment(5);
            source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .unwrap()
                .set_offset(1, 2);
        }
        graph.end().unwrap();
        assert_eq!(state(&graph), (1, 7, 7));
        assert_eq!(graph.get_trailing_positions(), 1);
    }
}
