//! Token-graph utilities, ported from `org.apache.lucene.util.graph`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`GraphTokenStreamFiniteStrings`] | `GraphTokenStreamFiniteStrings` |

#![deny(unsafe_code)]

use std::rc::Rc;

use crate::analysis::tokenattributes::{
    PackedTokenAttributeImpl, PositionIncrementAttribute, PositionLengthAttribute,
    TermToBytesRefAttribute,
};
use crate::analysis::TokenStream;
use crate::error::{LuceneError, Result};
use crate::index::Term;
use crate::util::automaton::{
    Automaton, AutomatonBuilder, FiniteStringsIterator, Operations, Transition, TransitionAccessor,
    DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use crate::util::{AttributeSource, IntsRef};

/// Maximum level of recursion allowed in recursive operations.
///
/// Equivalent to `GraphTokenStreamFiniteStrings.MAX_RECURSION_LEVEL`.
const MAX_RECURSION_LEVEL: i32 = 1000;

/// One finite string of the token graph, replayed as a [`TokenStream`].
///
/// Equivalent to the private `GraphTokenStreamFiniteStrings.FiniteStringsTokenStream`.
#[derive(Debug)]
pub struct FiniteStringsTokenStream {
    tokens: Rc<Vec<AttributeSource>>,
    atts: AttributeSource,
    ids: IntsRef,
    end: usize,
    offset: usize,
}

impl FiniteStringsTokenStream {
    fn new(tokens: Rc<Vec<AttributeSource>>, ids: IntsRef) -> Self {
        let atts = tokens[0].clone_attributes();
        let offset = ids.offset;
        let end = ids.offset + ids.length;
        Self {
            tokens,
            atts,
            ids,
            end,
            offset,
        }
    }
}

impl TokenStream for FiniteStringsTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.offset < self.end {
            self.atts.clear_attributes();
            let id = self.ids.ints[self.offset] as usize;
            self.tokens[id].copy_to(&self.atts)?;
            self.offset += 1;
            return Ok(true);
        }

        Ok(false)
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.atts
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.atts
    }
}

/// Iterator over the finite strings of a token graph, each replayed as a
/// [`TokenStream`].
///
/// Equivalent to the anonymous `Iterator<TokenStream>` returned by
/// `GraphTokenStreamFiniteStrings.getFiniteStrings`. Java's `Iterator` cannot
/// signal failure, so `has_next` and `next` return a `Result` here: the underlying
/// [`FiniteStringsIterator`] reports a cyclic automaton as an error rather than an
/// unchecked exception.
pub struct FiniteStringsTokenStreams<'a> {
    it: FiniteStringsIterator<'a>,
    tokens: Rc<Vec<AttributeSource>>,
    current: Option<IntsRef>,
    finished: bool,
}

impl FiniteStringsTokenStreams<'_> {
    /// Returns true while another finite string is available.
    ///
    /// # Errors
    ///
    /// Returns an error if the automaton has cycles.
    pub fn has_next(&mut self) -> Result<bool> {
        if !self.finished && self.current.is_none() {
            self.current = self.it.next()?;
            if self.current.is_none() {
                self.finished = true;
            }
        }
        Ok(self.current.is_some())
    }

    /// Returns the next finite string as a [`TokenStream`], or `None` when the
    /// iteration is finished.
    ///
    /// # Errors
    ///
    /// Returns an error if the automaton has cycles.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<FiniteStringsTokenStream>> {
        if self.current.is_none() {
            self.has_next()?;
        }
        match self.current.take() {
            Some(ids) => Ok(Some(FiniteStringsTokenStream::new(
                Rc::clone(&self.tokens),
                ids,
            ))),
            None => Ok(None),
        }
    }
}

/// Consumes a [`TokenStream`] and creates an [`Automaton`] where the transition
/// labels are the indices of the tokens.
///
/// Equivalent to `org.apache.lucene.util.graph.GraphTokenStreamFiniteStrings`. This
/// class also provides helpers to explore the different paths of the automaton.
///
/// # Divergences from Lucene 10.5.0
///
/// * Java reads the position increment and position length through
///   `in.addAttribute(PositionIncrementAttribute.class)`, which installs the
///   attribute when the stream lacks it, so an absent attribute reads as the
///   defaults `1` and `1`. This port reads
///   [`PackedTokenAttributeImpl`]
///   and falls back to the same defaults, which is the convention the rest of the
///   crate's analysis chain already follows.
/// * Java stores the cloned token attributes in an array that may hold trailing
///   `null`s; this port stores exactly one entry per token, which is the filled
///   prefix Java indexes into.
pub struct GraphTokenStreamFiniteStrings {
    tokens: Rc<Vec<AttributeSource>>,
    det: Automaton,
}

impl GraphTokenStreamFiniteStrings {
    /// Builds the graph of `input` and determinizes it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] for a malformed token stream whose
    /// first token has a position increment below one, and a resource-limit error
    /// if determinizing the graph exceeds
    /// [`DEFAULT_DETERMINIZE_WORK_LIMIT`].
    pub fn new(input: &mut dyn TokenStream) -> Result<Self> {
        let (aut, tokens) = Self::build(input)?;
        let det = Operations::remove_dead_states(&Operations::determinize(
            &aut,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        )?);
        Ok(Self {
            tokens: Rc::new(tokens),
            det,
        })
    }

    /// Returns the determinized token graph.
    pub fn automaton(&self) -> &Automaton {
        &self.det
    }

    /// Returns whether the provided state is the start of multiple side paths of
    /// different length (e.g. "new york", "ny").
    pub fn has_side_path(&self, state: i32) -> bool {
        let mut transition = Transition::new();
        let num_t = self.det.init_transition(state, &mut transition);
        if num_t <= 1 {
            return false;
        }
        self.det.get_next_transition(&mut transition);
        let dest = transition.dest;
        for _ in 1..num_t {
            self.det.get_next_transition(&mut transition);
            if dest != transition.dest {
                return true;
            }
        }
        false
    }

    /// Returns the list of tokens that start at the provided state.
    pub fn get_terms(&self, state: i32) -> Vec<AttributeSource> {
        let mut transition = Transition::new();
        let num_t = self.det.init_transition(state, &mut transition);
        let mut tokens = Vec::new();
        for _ in 0..num_t {
            self.det.get_next_transition(&mut transition);
            for id in transition.min..=transition.max {
                tokens.push(self.tokens[id as usize].clone());
            }
        }
        tokens
    }

    /// Returns the list of terms that start at the provided state.
    pub fn get_terms_for_field(&self, field: &str, state: i32) -> Vec<Term> {
        self.get_terms(state)
            .into_iter()
            .map(|s| {
                let bytes = s
                    .get_attribute::<PackedTokenAttributeImpl>()
                    .map(|att| att.get_bytes_ref())
                    .unwrap_or_default();
                Term::new(field, bytes)
            })
            .collect()
    }

    /// Gets all finite strings from the automaton.
    pub fn get_finite_strings(&self) -> FiniteStringsTokenStreams<'_> {
        self.get_finite_strings_between(0, -1)
    }

    /// Gets all finite strings that start at `start_state` and end at `end_state`.
    pub fn get_finite_strings_between(
        &self,
        start_state: i32,
        end_state: i32,
    ) -> FiniteStringsTokenStreams<'_> {
        FiniteStringsTokenStreams {
            it: FiniteStringsIterator::with_bounds(&self.det, start_state, end_state),
            tokens: Rc::clone(&self.tokens),
            current: None,
            finished: false,
        }
    }

    /// Returns the articulation points (or cut vertices) of the graph.
    ///
    /// See <https://en.wikipedia.org/wiki/Biconnected_component>.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the graph is deeper than the
    /// maximum recursion level Lucene allows (1000).
    pub fn articulation_points(&self) -> Result<Vec<i32>> {
        if self.det.get_num_states() == 0 {
            return Ok(Vec::new());
        }
        let mut undirect = AutomatonBuilder::new();
        undirect.copy(&self.det);
        let mut transition = Transition::new();
        for i in 0..self.det.get_num_states() {
            let num_t = self.det.init_transition(i, &mut transition);
            for _ in 0..num_t {
                self.det.get_next_transition(&mut transition);
                undirect.add_transition(transition.dest, i, transition.min);
            }
        }
        let num_states = self.det.get_num_states() as usize;
        let mut visited = vec![false; num_states];
        let mut depth = vec![0i32; num_states];
        let mut low = vec![0i32; num_states];
        let mut parent = vec![-1i32; num_states];
        let mut points: Vec<i32> = Vec::new();
        let undirected = undirect.finish();
        Self::articulation_points_recurse(
            &undirected,
            0,
            0,
            &mut depth,
            &mut low,
            &mut parent,
            &mut visited,
            &mut points,
        )?;
        points.reverse();
        Ok(points)
    }

    /// Builds an automaton from the provided [`TokenStream`], returning it together
    /// with the cloned attributes of every token.
    fn build(input: &mut dyn TokenStream) -> Result<(Automaton, Vec<AttributeSource>)> {
        let mut builder = AutomatonBuilder::new();
        let mut tokens: Vec<AttributeSource> = Vec::new();

        input.reset()?;

        let mut pos: i32 = -1;
        let mut prev_incr: i32 = 1;
        let mut state: i32 = -1;
        let mut gap: i32 = 0;
        while input.increment_token()? {
            let (current_incr, pos_length) = {
                let source = input.attribute_source();
                match source.get_attribute::<PackedTokenAttributeImpl>() {
                    Some(att) => (att.get_position_increment(), att.get_position_length()),
                    None => (1, 1),
                }
            };

            if pos == -1 && current_incr < 1 {
                return Err(LuceneError::IllegalState(
                    "Malformed TokenStream, start token can't have increment less than 1"
                        .to_string(),
                ));
            }

            if current_incr == 0 {
                if gap > 0 {
                    pos -= gap;
                }
            } else {
                pos += 1;
                gap = current_incr - 1;
            }

            let end_pos = pos + pos_length + gap;
            while state < end_pos {
                state = builder.create_state();
            }

            let id = tokens.len() as i32;
            let cloned = input.attribute_source().clone_attributes();
            tokens.push(cloned);
            builder.add_transition(pos, end_pos, id);
            pos += gap;

            // We always produce linear token graphs from get_finite_strings(), so we
            // need to adjust posLength and posIncrement accordingly.
            {
                let token = &tokens[id as usize];
                if let Some(mut att) = token.get_attribute_mut::<PackedTokenAttributeImpl>() {
                    att.set_position_length(1);
                    if current_incr == 0 {
                        // A stacked token should have the same increment as the
                        // original token at this position.
                        att.set_position_increment(prev_incr);
                    }
                }
            }

            // Only save the last increment on a non-zero increment, in case we have
            // multiple stacked tokens.
            if current_incr > 0 {
                prev_incr = current_incr;
            }
        }

        input.end()?;
        if state != -1 {
            builder.set_accept(state, true);
        }
        Ok((builder.finish(), tokens))
    }

    #[allow(clippy::too_many_arguments)]
    fn articulation_points_recurse(
        a: &Automaton,
        state: i32,
        d: i32,
        depth: &mut [i32],
        low: &mut [i32],
        parent: &mut [i32],
        visited: &mut [bool],
        points: &mut Vec<i32>,
    ) -> Result<()> {
        visited[state as usize] = true;
        depth[state as usize] = d;
        low[state as usize] = d;
        let mut child_count = 0;
        let mut is_articulation = false;
        let mut t = Transition::new();
        let num_t = a.init_transition(state, &mut t);
        for _ in 0..num_t {
            a.get_next_transition(&mut t);
            if !visited[t.dest as usize] {
                parent[t.dest as usize] = state;
                if d < MAX_RECURSION_LEVEL {
                    Self::articulation_points_recurse(
                        a,
                        t.dest,
                        d + 1,
                        depth,
                        low,
                        parent,
                        visited,
                        points,
                    )?;
                } else {
                    return Err(LuceneError::IllegalArgument(
                        "Exceeded maximum recursion level during graph analysis".to_string(),
                    ));
                }
                child_count += 1;
                if low[t.dest as usize] >= depth[state as usize] {
                    is_articulation = true;
                }
                low[state as usize] = low[state as usize].min(low[t.dest as usize]);
            } else if t.dest != parent[state as usize] {
                low[state as usize] = low[state as usize].min(depth[t.dest as usize]);
            }
        }
        if (parent[state as usize] != -1 && is_articulation)
            || (parent[state as usize] == -1 && child_count > 1)
        {
            points.push(state);
        }
        Ok(())
    }
}
