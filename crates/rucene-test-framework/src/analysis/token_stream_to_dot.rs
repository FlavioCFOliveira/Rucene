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

//! Port of `org.apache.lucene.tests.analysis.TokenStreamToDot`.

use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::attributes::{char_term_id, offset_id, pos_inc_id, pos_len_id, Att};

const FONT_NAME: &str = "Helvetica";

/// Consumes a TokenStream and outputs the dot (graphviz) string (graph).
///
/// Equivalent to `org.apache.lucene.tests.analysis.TokenStreamToDot`; Java's
/// `PrintWriter` is the [`String`] [`to_dot`](Self::to_dot) appends to.
pub struct TokenStreamToDot<'a> {
    input: &'a mut dyn TokenStream,
    term_att: Att,
    pos_inc_att: Att,
    pos_length_att: Att,
    offset_att: Option<Att>,
    input_text: Option<String>,
    /// The output (Java's `protected final PrintWriter out`).
    pub out: String,
}

impl<'a> TokenStreamToDot<'a> {
    /// If inputText is non-null, and the TokenStream has offsets, we include
    /// the surface form in each arc's label.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory lacks one of the
    /// attributes this adds.
    pub fn new(input_text: Option<&str>, input: &'a mut dyn TokenStream) -> Result<Self> {
        let source = input.attribute_source_mut();
        let term_att = Att::add(source, char_term_id())?;
        let pos_inc_att = Att::add(source, pos_inc_id())?;
        let pos_length_att = Att::add(source, pos_len_id())?;
        let offset_att = if source.has_attribute_by_id(offset_id()) {
            Some(Att::add(source, offset_id())?)
        } else {
            None
        };
        Ok(TokenStreamToDot {
            input,
            term_att,
            pos_inc_att,
            pos_length_att,
            offset_att,
            input_text: input_text.map(str::to_string),
            out: String::new(),
        })
    }

    /// `toDot()`: writes the graph of the stream to [`out`](Self::out).
    ///
    /// # Errors
    ///
    /// What the stream throws.
    pub fn to_dot(&mut self) -> Result<()> {
        self.input.reset()?;
        self.write_header();

        // TODO: is there some way to tell dot that it should
        // make the "main path" a straight line and have the
        // non-sausage arcs not affect node placement...

        let mut pos = -1;
        let mut last_end_pos = -1;
        while self.input.increment_token()? {
            let is_first = pos == -1;
            let mut pos_inc = self.pos_inc_att.pos_inc().get_position_increment();
            if is_first && pos_inc == 0 {
                // TODO: hmm are TS's still allowed to do this...?
                eprintln!("WARNING: first posInc was 0; correcting to 1");
                pos_inc = 1;
            }

            if pos_inc > 0 {
                // New node:
                pos += pos_inc;
                self.write_node(pos, Some(&pos.to_string()));
            }

            if pos_inc > 1 {
                // Gap!
                self.write_arc(last_end_pos, pos, None, Some("dotted"));
            }

            if is_first {
                self.write_node(-1, None);
                self.write_arc(-1, pos, None, None);
            }

            let term = self.term_att.term_string();
            let mut arc_label = term.clone();
            if let Some(offset_att) = &self.offset_att {
                let (start_offset, end_offset) = {
                    let o = offset_att.offset();
                    (o.start_offset(), o.end_offset())
                };
                match &self.input_text {
                    Some(input_text) => {
                        let units: Vec<u16> = input_text.encode_utf16().collect();
                        let fragment = String::from_utf16_lossy(
                            &units[start_offset as usize..end_offset as usize],
                        );
                        if fragment != term {
                            arc_label.push_str(" / ");
                            arc_label.push_str(&fragment);
                        }
                    }
                    None => {
                        arc_label.push_str(&format!(" / {}-{}", start_offset, end_offset));
                    }
                }
            }

            let pos_length = self.pos_length_att.pos_len().get_position_length();
            self.write_arc(pos, pos + pos_length, Some(&arc_label), None);
            last_end_pos = pos + pos_length;
        }

        self.input.end()?;

        if last_end_pos != -1 {
            // TODO: should we output any final text (from end
            // offsets) on this arc...?
            self.write_node(-2, None);
            self.write_arc(last_end_pos, -2, None, None);
        }

        self.write_trailer();
        Ok(())
    }

    /// `writeArc(int fromNode, int toNode, String label, String style)`.
    pub fn write_arc(
        &mut self,
        from_node: i32,
        to_node: i32,
        label: Option<&str>,
        style: Option<&str>,
    ) {
        self.out
            .push_str(&format!("  {} -> {} [", from_node, to_node));
        if let Some(label) = label {
            self.out.push_str(&format!(" label=\"{}\"", label));
        }
        if let Some(style) = style {
            self.out.push_str(&format!(" style=\"{}\"", style));
        }
        self.out.push_str("]\n");
    }

    /// `writeNode(int name, String label)`.
    pub fn write_node(&mut self, name: i32, label: Option<&str>) {
        self.out.push_str(&format!("  {}", name));
        match label {
            Some(label) => self.out.push_str(&format!(" [label=\"{}\"]", label)),
            None => self.out.push_str(" [shape=point color=white]"),
        }
        self.out.push('\n');
    }

    /// Override to customize.
    pub fn write_header(&mut self) {
        self.out.push_str("digraph tokens {\n");
        self.out.push_str(
            "  graph [ fontsize=30 labelloc=\"t\" label=\"\" splines=true overlap=false rankdir = \"LR\" ];\n",
        );
        self.out.push_str("  // A2 paper size\n");
        self.out.push_str("  size = \"34.4,16.5\";\n");
        // out.println("  // try to fill paper");
        // out.println("  ratio = fill;");
        self.out.push_str(&format!(
            "  edge [ fontname=\"{}\" fontcolor=\"red\" color=\"#606060\" ]\n",
            FONT_NAME
        ));
        self.out.push_str(&format!(
            "  node [ style=\"filled\" fillcolor=\"#e8e8f0\" shape=\"Mrecord\" fontname=\"{}\" ]\n",
            FONT_NAME
        ));
        self.out.push('\n');
    }

    /// Override to customize.
    pub fn write_trailer(&mut self) {
        self.out.push_str("}\n");
    }
}
