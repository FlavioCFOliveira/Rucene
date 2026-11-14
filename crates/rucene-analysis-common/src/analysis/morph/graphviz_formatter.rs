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

//! The types declared in `GraphvizFormatter.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use super::connection_costs::ConnectionCosts;
use super::dictionary::Dictionary;
use super::morph_data::MorphData;
use super::token_type::TokenType;
use super::viterbi::{ViterbiPosition, WrappedPositionArray};

const BOS_LABEL: &str = "BOS";

const EOS_LABEL: &str = "EOS";

const FONT_NAME: &str = "Helvetica";

// TODO: would be nice to show 2nd best path in a diff't
// color...

/// Outputs the dot (graphviz) string for the viterbi lattice.
///
/// Equivalent to `org.apache.lucene.analysis.morph.GraphvizFormatter<T
/// extends MorphData>`. A position is passed by its absolute position (see
/// [`Viterbi`](super::viterbi::Viterbi)), and the surface forms, Java
/// `String`s of UTF-16 units, are decoded into the Rust string (a lone
/// surrogate becomes U+FFFD).
pub struct GraphvizFormatter<T: ?Sized + MorphData> {
    costs: Arc<ConnectionCosts>,
    best_path_map: HashMap<String, String>,
    sb: String,
    morph_data: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized + MorphData> std::fmt::Debug for GraphvizFormatter<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphvizFormatter")
            .field("best_path_map", &self.best_path_map)
            .field("sb", &self.sb)
            .finish_non_exhaustive()
    }
}

impl<T: ?Sized + MorphData> GraphvizFormatter<T> {
    /// Equivalent to `GraphvizFormatter(ConnectionCosts)`.
    pub fn new(costs: Arc<ConnectionCosts>) -> Self {
        let mut sb = String::new();
        sb.push_str(&Self::format_header());
        sb.push_str("  init [style=invis]\n");
        sb.push_str(&format!("  init -> 0.0 [label=\"{BOS_LABEL}\"]\n"));
        Self {
            costs,
            best_path_map: HashMap::new(),
            sb,
            morph_data: PhantomData,
        }
    }

    /// Equivalent to `GraphvizFormatter.finish()`.
    pub fn finish(&mut self) -> String {
        self.sb.push_str(Self::format_trailer());
        self.sb.clone()
    }

    /// Backtraces another incremental fragment:
    ///
    /// Equivalent to `GraphvizFormatter.onBacktrace(DictionaryProvider<T>,
    /// WrappedPositionArray<? extends Position>, int, Position, int, char[],
    /// boolean)`; `end_pos_data` is the position's `pos`.
    #[allow(clippy::too_many_arguments)]
    pub fn on_backtrace<U: ViterbiPosition>(
        &mut self,
        dict_provider: &dyn DictionaryProvider<T>,
        positions: &mut WrappedPositionArray<U>,
        last_back_trace_pos: i32,
        end_pos_data: i32,
        from_idx: i32,
        fragment: &[u16],
        is_end: bool,
    ) {
        self.set_best_path_map(positions, last_back_trace_pos, end_pos_data, from_idx);
        let nodes = self.format_nodes(
            dict_provider,
            positions,
            last_back_trace_pos,
            end_pos_data,
            fragment,
        );
        self.sb.push_str(&nodes);
        if is_end {
            self.sb.push_str("  fini [style=invis]\n");
            self.sb.push_str("  ");
            self.sb.push_str(&Self::get_node_id(end_pos_data, from_idx));
            self.sb
                .push_str(&format!(" -> fini [label=\"{EOS_LABEL}\"]"));
        }
    }

    /// Records which arcs make up the best bath:
    ///
    /// Equivalent to the private `GraphvizFormatter.setBestPathMap(...)`.
    fn set_best_path_map<U: ViterbiPosition>(
        &mut self,
        positions: &mut WrappedPositionArray<U>,
        start_pos: i32,
        end_pos_data: i32,
        from_idx: i32,
    ) {
        self.best_path_map.clear();

        let mut pos = end_pos_data;
        let mut best_idx = from_idx;
        while pos > start_pos {
            let pos_data = positions.get(pos).position();
            let back_pos = pos_data.get_back_pos(best_idx);
            let back_idx = pos_data.get_back_index(best_idx);

            let to_node_id = Self::get_node_id(pos, best_idx);
            let from_node_id = Self::get_node_id(back_pos, back_idx);

            debug_assert!(!self.best_path_map.contains_key(&from_node_id));
            debug_assert!(!self.best_path_map.values().any(|v| *v == to_node_id));
            self.best_path_map.insert(from_node_id, to_node_id);
            pos = back_pos;
            best_idx = back_idx;
        }
    }

    /// Equivalent to the private `GraphvizFormatter.formatNodes(...)`.
    fn format_nodes<U: ViterbiPosition>(
        &self,
        dict_provider: &dyn DictionaryProvider<T>,
        positions: &mut WrappedPositionArray<U>,
        start_pos: i32,
        end_pos_data: i32,
        fragment: &[u16],
    ) -> String {
        let mut sb = String::new();
        // Output nodes
        for pos in start_pos + 1..=end_pos_data {
            let pos_data = positions.get(pos).position();
            for idx in 0..pos_data.get_count() {
                sb.push_str("  ");
                sb.push_str(&Self::get_node_id(pos, idx));
                sb.push_str(" [label=\"");
                sb.push_str(&pos.to_string());
                sb.push_str(": ");
                sb.push_str(&pos_data.get_last_right_id(idx).to_string());
                sb.push_str("\"]\n");
            }
        }

        // Output arcs
        let mut pos = end_pos_data;
        while pos > start_pos {
            let pos_data = positions.get(pos).position().clone();
            for idx in 0..pos_data.get_count() {
                let back_last_right_id = positions
                    .get(pos_data.get_back_pos(idx))
                    .position()
                    .get_last_right_id(pos_data.get_back_index(idx));
                let to_node_id = Self::get_node_id(pos, idx);
                let from_node_id =
                    Self::get_node_id(pos_data.get_back_pos(idx), pos_data.get_back_index(idx));
                sb.push_str("  ");
                sb.push_str(&from_node_id);
                sb.push_str(" -> ");
                sb.push_str(&to_node_id);

                let attrs = if self.best_path_map.get(&from_node_id) == Some(&to_node_id) {
                    // This arc is on best path
                    " color=\"#40e050\" fontcolor=\"#40a050\" penwidth=3 fontsize=20"
                } else {
                    ""
                };

                let dict = dict_provider.get(pos_data.get_back_type(idx));
                let word_cost = dict.get_word_cost(pos_data.get_back_id(idx));
                let bg_cost = self.costs.get(
                    back_last_right_id,
                    dict.get_left_id(pos_data.get_back_id(idx)),
                );

                let start = (pos_data.get_back_pos(idx) - start_pos) as usize;
                let len = (pos - pos_data.get_back_pos(idx)) as usize;
                let surface_form = String::from_utf16_lossy(&fragment[start..start + len]);

                sb.push_str(" [label=\"");
                sb.push_str(&surface_form);
                sb.push(' ');
                sb.push_str(&word_cost.to_string());
                if bg_cost >= 0 {
                    sb.push('+');
                }
                sb.push_str(&bg_cost.to_string());
                sb.push('"');
                sb.push_str(attrs);
                sb.push_str("]\n");
            }
            pos -= 1;
        }
        sb
    }

    /// Equivalent to the private `GraphvizFormatter.formatHeader()`.
    fn format_header() -> String {
        format!(
            "digraph viterbi {{\n  graph [ fontsize=30 labelloc=\"t\" label=\"\" splines=true overlap=false rankdir = \"LR\"];\n  edge [ fontname=\"{FONT_NAME}\" fontcolor=\"red\" color=\"#606060\" ]\n  node [ style=\"filled\" fillcolor=\"#e8e8f0\" shape=\"Mrecord\" fontname=\"{FONT_NAME}\" ]\n"
        )
    }

    /// Equivalent to the private `GraphvizFormatter.formatTrailer()`.
    fn format_trailer() -> &'static str {
        "}"
    }

    /// Equivalent to the private `GraphvizFormatter.getNodeID(int, int)`.
    fn get_node_id(pos: i32, idx: i32) -> String {
        format!("{pos}.{idx}")
    }
}

/// `Dictionary` provider
///
/// Equivalent to `org.apache.lucene.analysis.morph.GraphvizFormatter$DictionaryProvider<T
/// extends MorphData>`; Java's `null` type (a position's unset back type) is
/// `None`.
pub trait DictionaryProvider<T: ?Sized + MorphData> {
    /// Equivalent to `DictionaryProvider.get(TokenType)`.
    fn get(&self, token_type: Option<TokenType>) -> &dyn Dictionary<T>;
}
