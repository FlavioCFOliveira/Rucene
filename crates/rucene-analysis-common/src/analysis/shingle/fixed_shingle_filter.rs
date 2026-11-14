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

//! Constructs shingles of a fixed size from a token graph, ported from
//! `org.apache.lucene.analysis.shingle.FixedShingleFilter`.

use rucene::analysis::graph_token_filter::GraphTokenFilter;
use rucene::analysis::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, TokenStream, TypeAttribute,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

/// Equivalent to the private `FixedShingleFilter.MAX_SHINGLE_SIZE`.
const MAX_SHINGLE_SIZE: i32 = 4;

/// A FixedShingleFilter constructs shingles (token n-grams) from a token
/// stream. In other words, it creates combinations of tokens as a single
/// token.
///
/// Unlike the `ShingleFilter`, FixedShingleFilter only emits shingles of a
/// fixed size, and never emits unigrams, even at the end of a TokenStream.
/// In addition, if the filter encounters stacked tokens (eg synonyms), then
/// it will output stacked shingles
///
/// For example, the sentence "please divide this sentence into shingles"
/// might be tokenized into shingles "please divide", "divide this", "this
/// sentence", "sentence into", and "into shingles".
///
/// This filter handles position increments > 1 by inserting filler tokens
/// (tokens with termtext "_").
///
/// Equivalent to `org.apache.lucene.analysis.shingle.FixedShingleFilter`
/// (`@lucene.experimental`), a `GraphTokenFilter`: the inherited state is
/// the [`GraphTokenFilter`] it holds, whose attributes are the stream's. Its
/// private `CharTermAttributeImpl buffer` is the `buffer` of `char`s.
#[derive(Debug)]
pub struct FixedShingleFilter {
    graph: GraphTokenFilter,
    shingle_size: i32,
    token_separator: String,
    filler_token: String,
    buffer: Vec<char>,
}

impl FixedShingleFilter {
    /// Creates a FixedShingleFilter over an input token stream
    ///
    /// * `input` the input stream
    /// * `shingle_size` the shingle size
    ///
    /// Equivalent to `FixedShingleFilter(TokenStream, int)`, `this(input,
    /// shingleSize, " ", "_")`.
    ///
    /// # Errors
    ///
    /// As [`with_separators`](Self::with_separators).
    pub fn new(input: Box<dyn TokenStream>, shingle_size: i32) -> Result<Self> {
        Self::with_separators(input, shingle_size, " ", "_")
    }

    /// Creates a FixedShingleFilter over an input token stream
    ///
    /// * `input` the input tokenstream
    /// * `shingle_size` the shingle size
    /// * `token_separator` a String to use as a token separator
    /// * `filler_token` a String to use to represent gaps in the input stream
    ///   (due to eg stopwords)
    ///
    /// Equivalent to `FixedShingleFilter(TokenStream, int, String, String)`:
    /// `GraphTokenFilter` adds the `PositionIncrementAttribute` and the
    /// `OffsetAttribute`, then the fields the `PositionIncrementAttribute`,
    /// `OffsetAttribute`, `CharTermAttribute` and `TypeAttribute`.
    ///
    /// # Errors
    ///
    /// What the stream's attribute factory raises for an interface it cannot
    /// implement, as Java's `addAttribute` throws, and
    /// `IllegalArgumentException` "Shingle size must be between 2 and 4, got
    /// N".
    pub fn with_separators(
        input: Box<dyn TokenStream>,
        shingle_size: i32,
        token_separator: &str,
        filler_token: &str,
    ) -> Result<Self> {
        let mut graph = GraphTokenFilter::new(input)?;
        let source = graph.attribute_source_mut();
        source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        source.add_attribute_interface::<dyn OffsetAttribute>()?;
        source.add_attribute_interface::<dyn CharTermAttribute>()?;
        source.add_attribute_interface::<dyn TypeAttribute>()?;
        if shingle_size <= 1 || shingle_size > MAX_SHINGLE_SIZE {
            return Err(LuceneError::illegal_argument(format!(
                "Shingle size must be between 2 and {MAX_SHINGLE_SIZE}, got {shingle_size}"
            )));
        }
        Ok(Self {
            graph,
            shingle_size,
            token_separator: token_separator.to_string(),
            filler_token: filler_token.to_string(),
            buffer: Vec::new(),
        })
    }

    fn source(&self) -> &AttributeSource {
        self.graph.attribute_source()
    }

    fn pos_inc(&self) -> i32 {
        self.source()
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
            .get_position_increment()
    }

    fn offsets(&self) -> (i32, i32) {
        let offset = self
            .source()
            .get_attribute_interface::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute");
        (offset.start_offset(), offset.end_offset())
    }

    fn term(&self) -> Vec<char> {
        let term = self
            .source()
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        term.buffer()[..term.length()].to_vec()
    }

    /// `buffer.append(tokenSeparator).append(fillerToken)`.
    fn append_filler(&mut self) {
        self.buffer.extend(self.token_separator.chars());
        self.buffer.extend(self.filler_token.chars());
    }
}

impl TokenStream for FixedShingleFilter {
    /// Equivalent to `FixedShingleFilter.incrementToken()`.
    fn increment_token(&mut self) -> Result<bool> {
        let mut shingle_pos_inc;
        let start_offset;
        let mut end_offset;
        'outer: loop {
            if !self.graph.increment_graph()? {
                if !self.graph.increment_base_token()? {
                    return Ok(false);
                }
                // starting a shingle at a new base position, use base position increment
                shingle_pos_inc = self.pos_inc();
            } else {
                // starting a new shingle at the same base with a different graph, use a 0
                // position increment
                shingle_pos_inc = 0;
            }

            let (start, end) = self.offsets();
            end_offset = end;
            self.buffer.clear();
            let term = self.term();
            self.buffer.extend(term);

            // build the shingle by iterating over the current graph, adding
            // filler tokens if we encounter gaps
            let mut i = 1;
            while i < self.shingle_size {
                if !self.graph.increment_graph_token()? {
                    // we've reached the end of the token stream, check for trailing
                    // positions and add fillers if necessary
                    let trailing_positions = self.graph.get_trailing_positions();
                    if i + trailing_positions < self.shingle_size {
                        // not enough trailing positions to make a full shingle
                        // start again at a different graph
                        continue 'outer;
                    }
                    while i < self.shingle_size {
                        self.append_filler();
                        i += 1;
                    }
                    break;
                }
                let mut pos_inc = self.pos_inc();
                if pos_inc > 1 {
                    // if we have a posInc > 1, we need to fill in the gaps
                    if i + pos_inc > self.shingle_size {
                        // if the posInc is greater than the shingle size, we need to add fillers
                        // up to the shingle size but no further
                        while i < self.shingle_size {
                            self.append_filler();
                            i += 1;
                        }
                        break;
                    }
                    // otherwise just add them in as far as we need
                    while pos_inc > 1 {
                        self.append_filler();
                        pos_inc -= 1;
                        i += 1;
                    }
                }
                self.buffer.extend(self.token_separator.chars());
                let term = self.term();
                self.buffer.extend(term);
                end_offset = self.offsets().1;
                i += 1;
            }
            start_offset = start;
            break;
        }
        let source = self.graph.attribute_source();
        source.clear_attributes();
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(start_offset, end_offset);
        source
            .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
            .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
            .set_position_increment(shingle_pos_inc);
        source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute")
            .copy_buffer(&self.buffer, 0, self.buffer.len());
        source
            .get_attribute_interface_mut::<dyn TypeAttribute>()
            .expect("INVARIANT: the constructor added the TypeAttribute")
            .set_type_static("shingle");
        Ok(true)
    }

    /// Equivalent to the inherited `GraphTokenFilter.end()`.
    fn end(&mut self) -> Result<()> {
        self.graph.end()
    }

    /// Equivalent to the inherited `GraphTokenFilter.reset()`.
    fn reset(&mut self) -> Result<()> {
        self.graph.reset()
    }

    /// Equivalent to the inherited `TokenFilter.close()`.
    fn close(&mut self) -> Result<()> {
        self.graph.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        self.graph.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.graph.attribute_source_mut()
    }
}
