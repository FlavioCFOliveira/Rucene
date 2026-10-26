//! Conversions between token streams and automata, ported from
//! `org.apache.lucene.analysis`.
//!
//! A token stream with synonyms is a graph, not a line, and an automaton is the
//! natural way to hold that graph — which is what lets a phrase query match
//! through a synonym expansion.

use crate::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PackedTokenAttributeImpl, PositionIncrementAttribute,
    PositionLengthAttribute, TermToBytesRefAttribute,
};
use crate::analysis::TokenStream;
use crate::error::{LuceneError, Result};
use crate::util::automaton::{Automaton, AutomatonBuilder};
use crate::util::ring_buffer::{Resettable, RollingBuffer};
use crate::util::{AttributeSource, BytesRef};

/// Arc label separating one position from the next.
///
/// Equivalent to `TokenStreamToAutomaton.POS_SEP`.
pub const POS_SEP: i32 = 0x001f;
/// Arc label standing for a hole — a position no token occupies.
///
/// Equivalent to `TokenStreamToAutomaton.HOLE`.
pub const HOLE: i32 = 0x001e;

/// What is known about one position while the automaton is built.
///
/// Equivalent to `TokenStreamToAutomaton.Position`.
#[derive(Clone, Copy, Debug)]
struct Position {
    /// State tokens arriving at this position end in, or `-1`.
    arriving: i32,
    /// State tokens leaving this position start from, or `-1`.
    leaving: i32,
}

impl Default for Position {
    fn default() -> Self {
        Self {
            arriving: -1,
            leaving: -1,
        }
    }
}

impl Resettable for Position {
    /// Equivalent to `TokenStreamToAutomaton.Position.reset()`.
    fn reset(&mut self) {
        self.arriving = -1;
        self.leaving = -1;
    }
}

/// Turns a token stream into the automaton that accepts exactly the phrases the
/// stream encodes.
///
/// Equivalent to `org.apache.lucene.analysis.TokenStreamToAutomaton`. The
/// positions are held in a [`RollingBuffer`] freed behind the cursor, as Java's
/// `Positions` is, so a very long stream costs constant memory.
#[derive(Clone, Copy, Debug)]
pub struct TokenStreamToAutomaton {
    preserve_position_increments: bool,
    final_offset_gap_as_hole: bool,
    unicode_arcs: bool,
}

impl Default for TokenStreamToAutomaton {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStreamToAutomaton {
    /// Creates a converter that preserves position increments.
    pub fn new() -> Self {
        Self {
            preserve_position_increments: true,
            final_offset_gap_as_hole: false,
            unicode_arcs: false,
        }
    }

    /// Sets whether a position increment above one becomes a hole.
    ///
    /// Equivalent to `setPreservePositionIncrements(boolean)`.
    pub fn set_preserve_position_increments(&mut self, value: bool) -> &mut Self {
        self.preserve_position_increments = value;
        self
    }

    /// Sets whether a trailing offset gap becomes a hole.
    ///
    /// Equivalent to `setFinalOffsetGapAsHole(boolean)`.
    pub fn set_final_offset_gap_as_hole(&mut self, value: bool) -> &mut Self {
        self.final_offset_gap_as_hole = value;
        self
    }

    /// Sets whether arcs carry Unicode code points rather than UTF-8 bytes.
    ///
    /// Equivalent to `setUnicodeArcs(boolean)`.
    pub fn set_unicode_arcs(&mut self, value: bool) -> &mut Self {
        self.unicode_arcs = value;
        self
    }

    /// Returns the token bytes to put into the graph.
    ///
    /// The token is returned unchanged. Lucene declares this so a subclass can
    /// rewrite a token — escaping bytes that would otherwise collide with
    /// [`POS_SEP`] or [`HOLE`], for instance — before it becomes arcs.
    ///
    /// Divergence: Rust has no implementation inheritance, so nothing can
    /// replace this behaviour the way a Java subclass overrides
    /// `changeToken` (`TokenStreamToAutomaton.java` lines 93-95). The default
    /// body is ported exactly, and it is applied at the same point of
    /// [`to_automaton`](Self::to_automaton) that Java applies it
    /// (`TokenStreamToAutomaton.java` line 176).
    ///
    /// Equivalent to `org.apache.lucene.analysis.TokenStreamToAutomaton#changeToken`.
    pub fn change_token(&self, input: BytesRef) -> BytesRef {
        input
    }

    /// Fills the gap before `pos` with hole arcs, so a phrase cannot match
    /// across a position nothing occupies.
    ///
    /// Equivalent to `TokenStreamToAutomaton.addHoles(Automaton.Builder,
    /// RollingBuffer, int)` (`TokenStreamToAutomaton.java:270-298`).
    fn add_holes<F: FnMut() -> Position>(
        builder: &mut AutomatonBuilder,
        positions: &mut RollingBuffer<Position, F>,
        mut pos: i32,
    ) {
        while positions.get(pos).arriving == -1 || positions.get(pos - 1).leaving == -1 {
            if positions.get(pos).arriving == -1 {
                let arriving = builder.create_state();
                positions.get(pos).arriving = arriving;
                let leaving = positions.get(pos).leaving;
                builder.add_transition(arriving, leaving, POS_SEP);
            }
            if positions.get(pos - 1).leaving == -1 {
                let leaving = if pos == 1 { 0 } else { builder.create_state() };
                positions.get(pos - 1).leaving = leaving;
                let arriving = positions.get(pos - 1).arriving;
                if arriving != -1 {
                    builder.add_transition(arriving, leaving, POS_SEP);
                }
            }
            let prev_leaving = positions.get(pos - 1).leaving;
            let arriving = positions.get(pos).arriving;
            builder.add_transition(prev_leaving, arriving, HOLE);
            pos -= 1;
            if pos <= 0 {
                break;
            }
        }
    }

    /// Builds the automaton for `input`.
    ///
    /// Equivalent to `TokenStreamToAutomaton.toAutomaton(TokenStream)`
    /// (`TokenStreamToAutomaton.java:113-257`).
    ///
    /// # Errors
    ///
    /// Propagates the stream's errors, and returns
    /// [`LuceneError::IllegalState`] when the first token has a position
    /// increment of zero, where Java's `assert pos > -1 || posInc > 0` fails
    /// and its `posData.leaving` then dereferences `null`.
    pub fn to_automaton(&self, input: &mut dyn TokenStream) -> Result<Automaton> {
        let mut builder = AutomatonBuilder::new();
        builder.create_state();

        input.reset()?;

        // Only temporarily holds states ahead of our current position.
        let mut positions = RollingBuffer::new(Position::default);

        let mut pos: i32 = -1;
        let mut freed_pos: i32 = 0;
        let mut max_offset = 0i32;
        while input.increment_token()? {
            let (term_bytes, mut pos_inc, pos_len, end_offset) = {
                let source = input.attribute_source();
                match source.get_attribute::<PackedTokenAttributeImpl>() {
                    Some(att) => (
                        att.get_bytes_ref(),
                        att.get_position_increment(),
                        att.get_position_length(),
                        att.end_offset(),
                    ),
                    None => (BytesRef::default(), 1, 1, 0),
                }
            };
            if !self.preserve_position_increments && pos_inc > 1 {
                pos_inc = 1;
            }
            if pos == -1 && pos_inc <= 0 {
                return Err(LuceneError::IllegalState(
                    "the first token must have a positive position increment".to_string(),
                ));
            }

            if pos_inc > 0 {
                // New node:
                pos += pos_inc;

                let arriving = positions.get(pos).arriving;
                if arriving == -1 {
                    // No token ever arrived to this position
                    if pos == 0 {
                        // OK: this is the first token
                        positions.get(pos).leaving = 0;
                    } else {
                        // This means there's a hole (eg, StopFilter does this):
                        let leaving = builder.create_state();
                        positions.get(pos).leaving = leaving;
                        Self::add_holes(&mut builder, &mut positions, pos);
                    }
                } else {
                    let leaving = builder.create_state();
                    positions.get(pos).leaving = leaving;
                    builder.add_transition(arriving, leaving, POS_SEP);
                    if pos_inc > 1 {
                        // A token spanned over a hole; add holes "under" it:
                        Self::add_holes(&mut builder, &mut positions, pos);
                    }
                }
                while freed_pos <= pos {
                    let free_pos_data = *positions.get(freed_pos);
                    // don't free this position yet if we may still need to fill
                    // holes over it:
                    if free_pos_data.arriving == -1 || free_pos_data.leaving == -1 {
                        break;
                    }
                    positions.free_before(freed_pos);
                    freed_pos += 1;
                }
            }

            let end_pos = pos + pos_len;

            let term_bytes = self.change_token(term_bytes);
            if positions.get(end_pos).arriving == -1 {
                let arriving = builder.create_state();
                positions.get(end_pos).arriving = arriving;
            }
            let end_arriving = positions.get(end_pos).arriving;

            // Each byte (or code point) of the term is one arc.
            let labels: Vec<i32> = if self.unicode_arcs {
                String::from_utf8_lossy(term_bytes.slice())
                    .chars()
                    .map(|c| c as i32)
                    .collect()
            } else {
                term_bytes.slice().iter().map(|&b| i32::from(b)).collect()
            };

            let mut state = positions.get(pos).leaving;
            for (byte_idx, &label) in labels.iter().enumerate() {
                let next_state = if byte_idx == labels.len() - 1 {
                    end_arriving
                } else {
                    builder.create_state()
                };
                builder.add_transition(state, next_state, label);
                state = next_state;
            }

            max_offset = max_offset.max(end_offset);
        }

        input.end()?;

        let (mut end_pos_inc, final_end_offset) = {
            let source = input.attribute_source();
            match source.get_attribute::<PackedTokenAttributeImpl>() {
                Some(att) => (att.get_position_increment(), att.end_offset()),
                None => (0, 0),
            }
        };
        if end_pos_inc == 0 && self.final_offset_gap_as_hole && final_end_offset > max_offset {
            end_pos_inc = 1;
        } else if end_pos_inc > 0 && !self.preserve_position_increments {
            end_pos_inc = 0;
        }

        let end_state = if end_pos_inc > 0 {
            // there were hole(s) after the last token
            let end_state = builder.create_state();

            // add trailing holes now:
            let mut last_state = end_state;
            loop {
                let state1 = builder.create_state();
                builder.add_transition(last_state, state1, HOLE);
                end_pos_inc -= 1;
                if end_pos_inc == 0 {
                    builder.set_accept(state1, true);
                    break;
                }
                let state2 = builder.create_state();
                builder.add_transition(state1, state2, POS_SEP);
                last_state = state2;
            }
            end_state
        } else {
            -1
        };

        pos += 1;
        while pos <= positions.get_max_pos() {
            let arriving = positions.get(pos).arriving;
            if arriving != -1 {
                if end_state != -1 {
                    builder.add_transition(arriving, end_state, POS_SEP);
                } else {
                    builder.set_accept(arriving, true);
                }
            }
            pos += 1;
        }

        Ok(builder.finish())
    }
}

/// One arc of the automaton, as the stream will emit it.
///
/// Equivalent to `AutomatonToTokenStream.EdgeToken`.
#[derive(Clone, Copy, Debug)]
struct EdgeToken {
    destination: usize,
    value: i32,
}

/// A token stream over the arcs of an automaton, laid out in topological
/// layers.
///
/// Equivalent to `AutomatonToTokenStream.TopoTokenStream`.
pub struct TopoTokenStream {
    edges_by_pos: Vec<Vec<EdgeToken>>,
    current_pos: usize,
    current_edge_index: usize,
    source: AttributeSource,
}

impl std::fmt::Debug for TopoTokenStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TopoTokenStream")
            .field("positions", &self.edges_by_pos.len())
            .finish_non_exhaustive()
    }
}

impl TopoTokenStream {
    fn new(edges_by_pos: Vec<Vec<EdgeToken>>) -> Result<Self> {
        let mut source = AttributeSource::new();
        source.add_attribute::<PackedTokenAttributeImpl>()?;
        Ok(Self {
            edges_by_pos,
            current_pos: 0,
            current_edge_index: 0,
            source,
        })
    }
}

impl TokenStream for TopoTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        // Skip past any layer that has no arcs left.
        while self.current_pos < self.edges_by_pos.len()
            && self.current_edge_index == self.edges_by_pos[self.current_pos].len()
        {
            self.current_edge_index = 0;
            self.current_pos += 1;
        }
        if self.current_pos == self.edges_by_pos.len() {
            return Ok(false);
        }

        let edge = self.edges_by_pos[self.current_pos][self.current_edge_index];
        let pos = self.current_pos;
        let first_of_layer = self.current_edge_index == 0;
        self.source.clear_attributes();
        if let Some(mut att) = self.source.get_attribute_mut::<PackedTokenAttributeImpl>() {
            if let Some(c) = char::from_u32(edge.value as u32) {
                att.append_char(c);
            }
            // Only the first arc of a layer advances the position.
            att.set_position_increment(if first_of_layer { 1 } else { 0 });
            att.set_position_length((edge.destination - pos) as i32);
            att.set_offset(pos as i32, edge.destination as i32);
        }
        self.current_edge_index += 1;
        Ok(true)
    }

    fn reset(&mut self) -> Result<()> {
        self.current_pos = 0;
        self.current_edge_index = 0;
        Ok(())
    }

    fn end(&mut self) -> Result<()> {
        let last = self.edges_by_pos.len().saturating_sub(1) as i32;
        self.source.clear_attributes();
        if let Some(mut att) = self.source.get_attribute_mut::<PackedTokenAttributeImpl>() {
            att.set_position_increment(0);
            att.set_offset(last, last);
        }
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// Turns an automaton back into a token stream.
///
/// Equivalent to `org.apache.lucene.analysis.AutomatonToTokenStream`. The
/// automaton must be acyclic: each state becomes one position, and each arc one
/// token.
pub struct AutomatonToTokenStream;

impl AutomatonToTokenStream {
    /// Builds the stream that walks `automaton`.
    ///
    /// Equivalent to `AutomatonToTokenStream.toTokenStream(Automaton)`. Fails
    /// when the automaton has a cycle, which no token stream can represent.
    pub fn to_token_stream(automaton: &Automaton) -> Result<TopoTokenStream> {
        let transitions = automaton.get_sorted_transitions();
        let mut indegree = vec![0i32; transitions.len()];
        for state_transitions in &transitions {
            for t in state_transitions {
                indegree[t.dest as usize] += 1;
            }
        }
        if indegree.first().copied().unwrap_or(0) != 0 {
            return Err(LuceneError::IllegalArgument(
                "Start node has incoming edges, creating cycle".to_string(),
            ));
        }

        // Kahn's topological sort: a state's layer is its distance from the
        // start once every predecessor has been placed.
        let mut position_nodes: Vec<Vec<usize>> = Vec::new();
        let mut id_to_pos: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();
        let mut queue: std::collections::VecDeque<(usize, usize)> =
            std::collections::VecDeque::new();
        queue.push_back((0, 0));

        while let Some((id, pos)) = queue.pop_front() {
            for t in &transitions[id] {
                let dest = t.dest as usize;
                indegree[dest] -= 1;
                if indegree[dest] == 0 {
                    queue.push_back((dest, pos + 1));
                }
            }
            if position_nodes.len() == pos {
                position_nodes.push(vec![id]);
            } else {
                position_nodes[pos].push(id);
            }
            id_to_pos.insert(id, pos);
        }

        if indegree.iter().any(|&d| d != 0) {
            return Err(LuceneError::IllegalArgument(
                "Cycle found in automaton".to_string(),
            ));
        }

        let last_layer = position_nodes.len().saturating_sub(1);
        let mut edges_by_layer: Vec<Vec<EdgeToken>> = Vec::with_capacity(position_nodes.len());
        for layer in &position_nodes {
            let mut edges = Vec::new();
            for &state in layer {
                for t in &transitions[state] {
                    for value in t.min..=t.max {
                        let dest_layer = *id_to_pos.get(&(t.dest as usize)).unwrap_or(&0);
                        edges.push(EdgeToken {
                            destination: dest_layer,
                            value,
                        });
                        // An accepting state that is not the last layer also
                        // reaches the end, so the stream can stop there.
                        if automaton.is_accept(t.dest) && dest_layer != last_layer {
                            edges.push(EdgeToken {
                                destination: last_layer,
                                value,
                            });
                        }
                    }
                }
            }
            edges_by_layer.push(edges);
        }

        TopoTokenStream::new(edges_by_layer)
    }
}
