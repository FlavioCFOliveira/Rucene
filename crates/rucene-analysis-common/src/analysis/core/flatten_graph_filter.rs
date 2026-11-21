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

//! Converts an incoming graph token stream into a flat form, ported from
//! `org.apache.lucene.analysis.core.FlattenGraphFilter`.

use rucene::analysis::{
    OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute, TokenFilter,
    TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::store::java_nio_file::throw_unchecked;
use rucene::util::attribute::{AttributeSource, CapturedState};
use rucene::util::ring_buffer::{Resettable, RollingBuffer};

/// Holds all tokens leaving a given input position.
///
/// Equivalent to the private static `FlattenGraphFilter.InputNode`, a
/// `RollingBuffer.Resettable`.
#[derive(Debug)]
pub(crate) struct InputNode {
    /// `List<AttributeSource.State>`: a captured state, `None` for Java's
    /// `null` state of a source without attributes.
    tokens: Vec<Option<CapturedState>>,

    /// Our input node, or -1 if we haven't been assigned yet
    node: i32,

    /// Maximum to input node for all tokens leaving here; we use this to know
    /// when we can freeze.
    max_to_node: i32,

    /// Minimum to input node for all tokens leaving here; we use this to check
    /// if holes exist.
    min_to_node: i32,

    /// Where we currently map to; this changes (can only increase as we see
    /// more input tokens), until we are finished with this position.
    output_node: i32,

    /// Which token (index into `tokens`) we will next output.
    next_out: usize,
}

impl InputNode {
    /// `new InputNode()`, the field initializers.
    fn new() -> Self {
        Self {
            tokens: Vec::new(),
            node: -1,
            max_to_node: -1,
            min_to_node: i32::MAX,
            output_node: -1,
            next_out: 0,
        }
    }
}

impl Resettable for InputNode {
    fn reset(&mut self) {
        self.tokens.clear();
        self.node = -1;
        self.output_node = -1;
        self.max_to_node = -1;
        self.min_to_node = i32::MAX;
        self.next_out = 0;
    }
}

/// Gathers merged input positions into a single output position, only for the
/// current "frontier" of nodes we've seen but can't yet output because they
/// are not frozen.
///
/// Equivalent to the private static `FlattenGraphFilter.OutputNode`, a
/// `RollingBuffer.Resettable`; its hppc `IntArrayList` is a `Vec<i32>`.
#[derive(Debug)]
pub(crate) struct OutputNode {
    input_nodes: Vec<i32>,

    /// Node ID for this output, or -1 if we haven't been assigned yet.
    node: i32,

    /// Which input node (index into `input_nodes`) we will next output.
    next_out: usize,

    /// Start offset of tokens leaving this node.
    start_offset: i32,

    /// End offset of tokens arriving to this node.
    end_offset: i32,
}

impl OutputNode {
    /// `new OutputNode()`, the field initializers.
    fn new() -> Self {
        Self {
            input_nodes: Vec::new(),
            node: -1,
            next_out: 0,
            start_offset: -1,
            end_offset: -1,
        }
    }

    /// hppc `IntArrayList.removeElement(int)`: removes the first element
    /// equal to `e` and answers whether there was one.
    fn remove_element(&mut self, e: i32) -> bool {
        match self.input_nodes.iter().position(|&n| n == e) {
            Some(index) => {
                self.input_nodes.remove(index);
                true
            }
            None => false,
        }
    }
}

impl Resettable for OutputNode {
    fn reset(&mut self) {
        self.input_nodes.clear();
        self.node = -1;
        self.next_out = 0;
        self.start_offset = -1;
        self.end_offset = -1;
    }
}

/// The body of [`FlattenGraphFilter`]: what Lucene's `FlattenGraphFilter`
/// adds to `TokenFilter`.
pub struct FlattenGraphFilterLogic {
    input_nodes: RollingBuffer<InputNode, fn() -> InputNode>,
    output_nodes: RollingBuffer<OutputNode, fn() -> OutputNode>,

    /// Which input node the last seen token leaves from
    input_from: i32,

    /// We are currently releasing tokens leaving from this output node
    output_from: i32,

    done: bool,

    last_output_from: i32,

    final_offset: i32,

    final_pos_inc: i32,

    max_lookahead_used: usize,

    last_start_offset: i32,
}

impl std::fmt::Debug for FlattenGraphFilterLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlattenGraphFilterLogic")
            .field("input_from", &self.input_from)
            .field("output_from", &self.output_from)
            .field("done", &self.done)
            .field("last_output_from", &self.last_output_from)
            .field("final_offset", &self.final_offset)
            .field("final_pos_inc", &self.final_pos_inc)
            .field("max_lookahead_used", &self.max_lookahead_used)
            .field("last_start_offset", &self.last_start_offset)
            .finish_non_exhaustive()
    }
}

/// The `PositionIncrementAttribute` of `source`, which the constructor added.
fn pos_inc_att(
    source: &AttributeSource,
) -> rucene::util::atomic_ref_cell::AtomicRefMut<'_, dyn PositionIncrementAttribute> {
    source
        .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
        .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
}

/// The `PositionLengthAttribute` of `source`, which the constructor added.
fn pos_len_att(
    source: &AttributeSource,
) -> rucene::util::atomic_ref_cell::AtomicRefMut<'_, dyn PositionLengthAttribute> {
    source
        .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
        .expect("INVARIANT: the constructor added the PositionLengthAttribute")
}

/// The `OffsetAttribute` of `source`, which the constructor added.
fn offset_att(
    source: &AttributeSource,
) -> rucene::util::atomic_ref_cell::AtomicRefMut<'_, dyn OffsetAttribute> {
    source
        .get_attribute_interface_mut::<dyn OffsetAttribute>()
        .expect("INVARIANT: the constructor added the OffsetAttribute")
}

impl FlattenGraphFilterLogic {
    /// The fields of a new `FlattenGraphFilter`; `reset()` sets them before
    /// the stream is consumed.
    fn new() -> Self {
        Self {
            input_nodes: RollingBuffer::new(InputNode::new as fn() -> InputNode),
            output_nodes: RollingBuffer::new(OutputNode::new as fn() -> OutputNode),
            input_from: 0,
            output_from: 0,
            done: false,
            last_output_from: 0,
            final_offset: 0,
            final_pos_inc: 0,
            max_lookahead_used: 0,
            last_start_offset: 0,
        }
    }

    /// `releaseBufferedToken()`.
    fn release_buffered_token(&mut self, source: &AttributeSource) -> Result<bool> {
        // We only need the while loop (retry) if we have a hole (an output
        // node that has no tokens leaving):
        while self.output_from < self.output_nodes.get_max_pos() {
            let output_from = self.output_from;
            if self.output_nodes.get(output_from).input_nodes.is_empty() {
                // No tokens arrived to this node, which happens for the first
                // node after a hole:
                self.output_from += 1;
                continue;
            }

            let mut max_to_node = -1;
            let output_input_nodes = self.output_nodes.get(output_from).input_nodes.clone();
            for &input_node_id in &output_input_nodes {
                let input_node = self.input_nodes.get(input_node_id);
                debug_assert!(input_node.output_node == output_from);
                max_to_node = max_to_node.max(input_node.max_to_node);
            }

            // TODO: we could shrink the frontier here somewhat if we
            // always output posLen=1 as part of our "sausagizing":
            if max_to_node <= self.input_from || self.done {
                // These tokens are now frozen
                let (output_next_out, output_input_count) = {
                    let output = self.output_nodes.get(output_from);
                    (output.next_out, output.input_nodes.len())
                };
                debug_assert!(
                    output_next_out < output_input_count,
                    "output.nextOut={output_next_out} vs output.inputNodes.size()={output_input_count}"
                );
                let input_node_id = output_input_nodes[output_next_out];
                let (input_tokens_empty, input_next_out) = {
                    let input_node = self.input_nodes.get(input_node_id);
                    (input_node.tokens.is_empty(), input_node.next_out)
                };
                if self.done
                    && input_tokens_empty
                    && self.output_from >= self.output_nodes.get_max_pos()
                {
                    return Ok(false);
                }
                if input_tokens_empty {
                    debug_assert!(input_next_out == 0);
                    // Hole dest nodes should never be merged since 1) we always
                    // assign them to a new output position, and 2) since they
                    // never have arriving tokens they cannot be pushed. Skip
                    // them but don't free input until all are checked.
                    // Related tests testAltPathLastStepLongHole,
                    // testAltPathLastStepHoleFollowedByHole,
                    // testAltPathLastStepHoleWithoutEndToken
                    if output_input_count > 1 {
                        let output = self.output_nodes.get(output_from);
                        output.next_out += 1;
                        if output.next_out < output.input_nodes.len() {
                            continue;
                        }
                    }
                    // Don't free from a hole src. Since no edge leaves here
                    // bookkeeping may be incorrect. Later output nodes may
                    // point to earlier input nodes. So we don't want to free
                    // them yet.
                    self.free_before(output_from);
                    continue;
                }

                let input_node_node = {
                    let input_node = self.input_nodes.get(input_node_id);
                    debug_assert!(input_node.next_out < input_node.tokens.len());
                    // `restoreState(null)` does nothing.
                    if let Some(state) = &input_node.tokens[input_node.next_out] {
                        source.restore_state(state)?;
                    }
                    input_node.node
                };

                // Correct posInc
                debug_assert!(self.output_from >= self.last_output_from);
                pos_inc_att(source)
                    .set_position_increment(self.output_from - self.last_output_from);
                let to_input_node_id = input_node_node + pos_len_att(source).get_position_length();
                let to_input_node_output_node = self.input_nodes.get(to_input_node_id).output_node;

                // Correct posLen
                debug_assert!(to_input_node_output_node > self.output_from);
                pos_len_att(source)
                    .set_position_length(to_input_node_output_node - self.output_from);
                self.last_output_from = self.output_from;
                let input_node_next_out = {
                    let input_node = self.input_nodes.get(input_node_id);
                    input_node.next_out += 1;
                    input_node.next_out
                };

                let output_end_node_end_offset =
                    self.output_nodes.get(to_input_node_output_node).end_offset;

                // Correct offsets

                // This is a bit messy; we must do this so offset don't go
                // backwards, which would otherwise happen if the replacement
                // has more tokens than the input:
                let output_start_offset = self.output_nodes.get(output_from).start_offset;
                let start_offset = self.last_start_offset.max(output_start_offset);

                // We must do this in case the incoming tokens have broken
                // offsets:
                let end_offset = start_offset.max(output_end_node_end_offset);

                offset_att(source).set_offset(start_offset, end_offset);
                self.last_start_offset = start_offset;

                let input_tokens_len = self.input_nodes.get(input_node_id).tokens.len();
                if input_node_next_out == input_tokens_len {
                    let output = self.output_nodes.get(output_from);
                    output.next_out += 1;
                    if output.next_out == output.input_nodes.len() {
                        self.free_before(output_from);
                    }
                }

                return Ok(true);
            } else {
                return Ok(false);
            }
        }

        Ok(false)
    }

    /// Free inputs nodes before the minimum input node for the given output.
    ///
    /// Equivalent to `freeBefore(OutputNode)`, for the output node at
    /// `output`, the position `outputFrom` held when it was fetched.
    fn free_before(&mut self, output: i32) {
        // We've released all the tokens that end at the current output, so
        // free all output nodes before this. Input nodes are more complex. The
        // second shingled tokens with alternate paths can appear later in the
        // output graph than some of their alternate path tokens. Because of
        // this case we can only free from the minimum because the minimum node
        // will have come from before the second shingled token. This means we
        // have to hold onto input nodes whose tokens get stacked on previous
        // nodes until we've completely passed those inputs. Related tests
        // testShingledGap, testShingledGapWithHoles
        self.output_from += 1;
        let free_before = match self.output_nodes.get(output).input_nodes.iter().min() {
            Some(&min) => min,
            None => throw_unchecked("java.util.NoSuchElementException", Some("No value present")),
        };
        // This will catch a node being freed early if it is input to the next
        // output. Could a freed early node be input to a later output?
        debug_assert!(
            self.output_nodes
                .get(self.output_from)
                .input_nodes
                .iter()
                .all(|&n| free_before <= n),
            "FreeBefore {free_before} will free in use nodes"
        );
        self.input_nodes.free_before(free_before);
        self.output_nodes.free_before(self.output_from);
    }

    /// `recoverFromHole(InputNode, int, int)`, for the input node `src`, at
    /// position `inputFrom`; answers the position of the output node it
    /// returns.
    fn recover_from_hole(&mut self, start_offset: i32, posinc: i32) -> i32 {
        // This means the "from" node of this token was never seen as a "to"
        // node, which should only happen if we just crossed a hole. This is a
        // challenging case for us because we normally rely on the full
        // dependencies expressed by the arcs to assign outgoing node IDs. It
        // would be better if tokens were never dropped but instead just marked
        // deleted with a new TermDeletedAttribute (boolean valued) ... but
        // until that future, we have a hack here to forcefully jump the output
        // node ID:
        let input_from = self.input_from;
        debug_assert!(self.input_nodes.get(input_from).output_node == -1);
        self.input_nodes.get(input_from).node = input_from;

        let out_index;
        let previous_input_from = input_from - posinc;
        if previous_input_from >= 0 {
            let offset_src_min_to_node = self.input_nodes.get(previous_input_from).min_to_node;
            // Select output src node. Need to make sure the new output node
            // isn't placed too far ahead. If a disconnected node is placed at
            // the end of the output graph that may place it after output nodes
            // that map to input nodes that are after src in the input. Since
            // it is disconnected there is no path to it, and there could be
            // holes after meaning no paths to following nodes. This "floating"
            // edge will cause problems in FreeBefore. In the following section
            // make sure the edge connects to something. Related test
            // testLongHole testAltPathLastStepHoleFollowedByHole,
            // testAltPathFirstStepHole, testShingledGapWithHoles
            if offset_src_min_to_node < input_from {
                // There is a possible path to this node.
                // place this node one position off from the possible path
                // keeping a 1 inc gap. Can't be larger than 1 inc or risk
                // getting disconnected.
                out_index = self.input_nodes.get(offset_src_min_to_node).output_node + 1;
            } else {
                // no information about how the current node was previously
                // connected. Connect it to the end.
                out_index = self.output_nodes.get_max_pos();
            }
        } else {
            // in case the first token in the stream is a hole we have no input
            // node to increment from.
            out_index = self.output_nodes.get_max_pos() + 1;
        }
        self.input_nodes.get(input_from).output_node = out_index;
        let out_src = self.output_nodes.get(out_index);

        // OutSrc may have other inputs
        if out_src.node == -1 {
            out_src.node = out_index;
            out_src.start_offset = start_offset;
        } else {
            out_src.start_offset = start_offset.max(out_src.start_offset);
        }
        out_src.input_nodes.push(input_from);
        out_index
    }

    /// For testing.
    ///
    /// Equivalent to `FlattenGraphFilter.getMaxLookaheadUsed()`.
    pub fn get_max_lookahead_used(&self) -> usize {
        self.max_lookahead_used
    }
}

impl TokenFilterLogic for FlattenGraphFilterLogic {
    /// Equivalent to `FlattenGraphFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        loop {
            if self.release_buffered_token(source)? {
                return Ok(true);
            } else if self.done {
                return Ok(false);
            }

            if input.increment_token()? {
                // Input node this token leaves from:
                let position_increment = pos_inc_att(source).get_position_increment();
                self.input_from += position_increment;

                let (start_offset, end_offset) = {
                    let offset_att = offset_att(source);
                    (offset_att.start_offset(), offset_att.end_offset())
                };

                // Input node this token goes to:
                let input_to = self.input_from + pos_len_att(source).get_position_length();

                let input_from = self.input_from;
                if self.input_nodes.get(input_from).node == -1 {
                    self.recover_from_hole(start_offset, position_increment);
                } else {
                    let mut out_src_index = self.input_nodes.get(input_from).output_node;
                    // If positionIncrement > 1 and the position we're
                    // incrementing from doesn't come to the current node we've
                    // crossed a hole. The long edge will point too far back and
                    // not account for the holes unless it gets fixed. A long
                    // edge may have already made this fix though, if src is
                    // more than 1 position ahead in the output there's no
                    // additional work to do.
                    if position_increment > 1 {
                        let previous = input_from - position_increment;
                        let (previous_output_node, previous_min_to_node) = {
                            let node = self.input_nodes.get(previous);
                            (node.output_node, node.min_to_node)
                        };
                        if out_src_index - previous_output_node <= 1
                            && previous_min_to_node != input_from
                        {
                            // If there was a hole at the end of an alternate
                            // path then the input and output nodes have been
                            // created, but the offsets and increments have not
                            // been maintained correctly. Here we go back and
                            // fix them. Related test testAltPathLastStepHole
                            // The last node in the alt path didn't arrive to
                            // remove this reference.
                            debug_assert!(
                                self.input_nodes.get(input_from).tokens.is_empty(),
                                "about to remove non empty edge"
                            );
                            self.output_nodes
                                .get(out_src_index)
                                .remove_element(input_from);
                            self.input_nodes.get(input_from).output_node = -1;
                            let prev_end_offset = self.output_nodes.get(out_src_index).end_offset;

                            out_src_index =
                                self.recover_from_hole(start_offset, position_increment);
                            self.output_nodes.get(out_src_index).end_offset = prev_end_offset;
                        }
                    }

                    let out_src = self.output_nodes.get(out_src_index);
                    if out_src.start_offset == -1 || start_offset > out_src.start_offset {
                        // "shrink wrap" the offsets so the original tokens
                        // (with most restrictive offsets) win:
                        out_src.start_offset = start_offset.max(out_src.start_offset);
                    }
                }

                // Buffer this token:
                let state = source.capture_state();
                {
                    let src = self.input_nodes.get(input_from);
                    src.tokens.push(state);
                    src.max_to_node = src.max_to_node.max(input_to);
                    src.min_to_node = src.min_to_node.min(input_to);
                }
                self.max_lookahead_used = self
                    .max_lookahead_used
                    .max(self.input_nodes.get_buffer_size());

                let src_output_node = self.input_nodes.get(input_from).output_node;
                let dest_output_node = {
                    let dest = self.input_nodes.get(input_to);
                    if dest.node == -1 {
                        // Common case: first time a token is arriving to this
                        // input position:
                        dest.node = input_to;
                    }
                    dest.output_node
                };

                // Always number output nodes sequentially:
                let output_end_node = src_output_node + 1;

                let mut dest_output_node = dest_output_node;
                if output_end_node > dest_output_node {
                    if dest_output_node != -1 {
                        let removed = self
                            .output_nodes
                            .get(dest_output_node)
                            .remove_element(input_to);
                        debug_assert!(removed);
                    }
                    self.output_nodes
                        .get(output_end_node)
                        .input_nodes
                        .push(input_to);
                    self.input_nodes.get(input_to).output_node = output_end_node;
                    dest_output_node = output_end_node;

                    // Since all we ever do is merge incoming nodes together,
                    // and then renumber the merged nodes sequentially, we
                    // should only ever assign smaller node numbers:
                    debug_assert!(
                        output_end_node <= input_to,
                        "outputEndNode={output_end_node} vs inputTo={input_to}"
                    );
                }

                let out_dest = self.output_nodes.get(dest_output_node);
                // "shrink wrap" the offsets so the original tokens (with most
                // restrictive offsets) win:
                if out_dest.end_offset == -1 || end_offset < out_dest.end_offset {
                    out_dest.end_offset = end_offset;
                }
            } else {
                input.end()?;
                self.final_pos_inc = pos_inc_att(source).get_position_increment();
                self.final_offset = offset_att(source).end_offset();
                self.done = true;
                // Don't return false here: we need to force release any
                // buffered tokens now
            }
        }
    }

    /// Equivalent to `FlattenGraphFilter.end()`. When the input was not
    /// exhausted (`done` is false), Java calls `super.end()` — the input's
    /// `end()` — twice, once before and once after `clearAttributes()`, and
    /// so does this port.
    fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        if !self.done {
            input.end()?;
        } else {
            // NOTE, shady: don't call super.end, because we did already from
            // incrementToken
        }

        source.clear_attributes();
        if self.done {
            // On exc, done is false, and we will not have set these:
            pos_inc_att(source).set_position_increment(self.final_pos_inc);
            offset_att(source).set_offset(self.final_offset, self.final_offset);
        } else {
            input.end()?;
        }
        Ok(())
    }

    /// Equivalent to `FlattenGraphFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.input_from = -1;
        self.input_nodes.reset();
        {
            let r#in = self.input_nodes.get(0);
            r#in.node = 0;
            r#in.output_node = 0;
        }

        self.output_nodes.reset();
        {
            let out = self.output_nodes.get(0);
            out.node = 0;
            out.input_nodes.push(0);
            out.start_offset = 0;
        }
        self.output_from = 0;
        self.last_output_from = -1;
        self.done = false;
        self.final_pos_inc = -1;
        self.final_offset = -1;
        self.last_start_offset = 0;
        self.max_lookahead_used = 0;
        Ok(())
    }
}

/// Converts an incoming graph token stream, such as one from
/// `SynonymGraphFilter`, into a flat form so that all nodes form a single
/// linear chain with no side paths. Every path through the graph touches
/// every node. This is necessary when indexing a graph token stream, because
/// the index does not save `PositionLengthAttribute` and so it cannot
/// preserve the graph structure. However, at search time, query parsers can
/// correctly handle the graph and this token filter should **not** be used.
///
/// If the graph was not already flat to start, this is likely a lossy
/// process, i.e. it will often cause the graph to accept token sequences it
/// should not, and to reject token sequences it should not.
///
/// However, when applying synonyms during indexing, this is necessary because
/// Lucene already does not index a graph and so the indexing process is
/// already lossy (it ignores the `PositionLengthAttribute`).
///
/// Equivalent to `org.apache.lucene.analysis.core.FlattenGraphFilter`
/// (`@lucene.experimental`).
pub type FlattenGraphFilter = TokenFilter<FlattenGraphFilterLogic>;

/// Equivalent to `FlattenGraphFilter(TokenStream)`, whose fields add the
/// `PositionIncrementAttribute`, the `PositionLengthAttribute` and the
/// `OffsetAttribute`, in that order.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_flatten_graph_filter(input: Box<dyn TokenStream>) -> Result<FlattenGraphFilter> {
    let mut filter = TokenFilter::new(input, FlattenGraphFilterLogic::new());
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    Ok(filter)
}
