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
use crate::util::automaton::Automaton;
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

/// Turns a token stream into the automaton that accepts exactly the phrases the
/// stream encodes.
///
/// Equivalent to `org.apache.lucene.analysis.TokenStreamToAutomaton`.
///
/// **Divergence from Lucene 10.5.0.** Java holds the positions in a
/// `RollingBuffer` it frees behind the cursor, so a very long stream costs
/// constant memory. `RollingBuffer` is not ported, so this port keeps a `Vec`
/// that grows to the stream's position count. The automaton produced is
/// identical; only the peak memory differs.
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

    /// Returns the position record at `pos`, growing the buffer as needed.
    fn position_at(positions: &mut Vec<Position>, pos: usize) -> &mut Position {
        if positions.len() <= pos {
            positions.resize(pos + 1, Position::default());
        }
        &mut positions[pos]
    }

    /// Fills the gap before `pos` with hole arcs, so a phrase cannot match
    /// across a position nothing occupies.
    ///
    /// Equivalent to `TokenStreamToAutomaton.addHoles`.
    fn add_holes(builder: &mut Automaton, positions: &mut Vec<Position>, pos: usize) {
        let mut pos_data_arriving = Self::position_at(positions, pos).arriving;
        let mut idx = pos;
        while idx > 0 {
            let prev = Self::position_at(positions, idx - 1);
            if prev.leaving != -1 {
                break;
            }
            if prev.arriving == -1 {
                prev.arriving = builder.create_state();
            }
            let prev_arriving = prev.arriving;
            let prev_leaving = builder.create_state();
            Self::position_at(positions, idx - 1).leaving = prev_leaving;
            builder.add_transition(prev_arriving, prev_leaving, POS_SEP);
            if pos_data_arriving == -1 {
                pos_data_arriving = builder.create_state();
                Self::position_at(positions, idx).arriving = pos_data_arriving;
            }
            builder.add_transition(prev_leaving, pos_data_arriving, HOLE);
            pos_data_arriving = Self::position_at(positions, idx - 1).arriving;
            idx -= 1;
        }
    }

    /// Builds the automaton for `input`.
    ///
    /// Equivalent to `TokenStreamToAutomaton.toAutomaton(TokenStream)`.
    pub fn to_automaton(&self, input: &mut dyn TokenStream) -> Result<Automaton> {
        let mut builder = Automaton::new();
        builder.create_state();

        input.reset()?;

        let mut positions: Vec<Position> = vec![Position::default()];
        let mut pos: i64 = -1;
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

            if pos_inc > 0 {
                pos += i64::from(pos_inc);
                let p = pos as usize;
                let arriving = Self::position_at(&mut positions, p).arriving;
                if arriving == -1 {
                    if p == 0 {
                        Self::position_at(&mut positions, p).leaving = 0;
                    } else {
                        let leaving = builder.create_state();
                        Self::position_at(&mut positions, p).leaving = leaving;
                        Self::add_holes(&mut builder, &mut positions, p);
                    }
                } else {
                    let leaving = builder.create_state();
                    Self::position_at(&mut positions, p).leaving = leaving;
                    builder.add_transition(arriving, leaving, POS_SEP);
                    if pos_inc > 1 {
                        Self::add_holes(&mut builder, &mut positions, p);
                    }
                }
            }

            let end_pos = (pos + i64::from(pos_len)) as usize;
            let end_arriving = Self::position_at(&mut positions, end_pos).arriving;
            let end_arriving = if end_arriving == -1 {
                let s = builder.create_state();
                Self::position_at(&mut positions, end_pos).arriving = s;
                s
            } else {
                end_arriving
            };

            // Each byte (or code point) of the term is one arc.
            let labels: Vec<i32> = if self.unicode_arcs {
                String::from_utf8_lossy(term_bytes.slice())
                    .chars()
                    .map(|c| c as i32)
                    .collect()
            } else {
                term_bytes.slice().iter().map(|&b| i32::from(b)).collect()
            };

            let mut state = Self::position_at(&mut positions, pos.max(0) as usize).leaving;
            for (i, &label) in labels.iter().enumerate() {
                let next_state = if i == labels.len() - 1 {
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
            // A trailing gap becomes a run of hole arcs ending in the accept
            // state.
            let end_state = builder.create_state();
            let mut last_state = end_state;
            loop {
                let state = builder.create_state();
                builder.add_transition(last_state, state, HOLE);
                end_pos_inc -= 1;
                if end_pos_inc == 0 {
                    builder.set_accept(state, true);
                    break;
                }
                last_state = state;
            }
            end_state
        } else {
            let p = pos.max(0) as usize;
            let leaving = Self::position_at(&mut positions, p).leaving;
            if leaving != -1 {
                builder.set_accept(leaving, true);
            }
            leaving
        };

        // Join every open position to the end state.
        if end_pos_inc > 0 || end_state != -1 {
            for idx in 0..positions.len() {
                let arriving = positions[idx].arriving;
                if arriving != -1 && positions[idx].leaving == -1 && end_state != -1 {
                    builder.add_transition(arriving, end_state, POS_SEP);
                }
            }
        }

        builder.finish();
        Ok(builder)
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
