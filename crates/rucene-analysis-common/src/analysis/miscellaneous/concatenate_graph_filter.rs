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
//! Concatenates the tokens of a graph into whole strings, ported from
//! `org.apache.lucene.analysis.miscellaneous.ConcatenateGraphFilter`.

use std::any::{Any, TypeId};
use std::fmt;
use std::sync::{Arc, OnceLock};

use rucene::analysis::automaton_conversion::{TokenStreamToAutomaton, HOLE, POS_SEP};
use rucene::analysis::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, TermToBytesRefAttribute,
    TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{
    Attribute, AttributeImpl, AttributeInterface, AttributeReflector, AttributeSource,
};
use rucene::util::automaton::automaton::{Automaton, Transition, TransitionAccessor};
use rucene::util::automaton::limited_finite_strings_iterator::LimitedFiniteStringsIterator;
use rucene::util::automaton::operations::{Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};
use rucene::util::fst::Util;
use rucene::util::refs::CharsRefBuilder;
use rucene::util::{BytesRef, BytesRefBuilder};

use crate::jdk::character::utf16_to_term_chars;

/// Represents the separation between tokens, if `preserveSep` is true.
///
/// Equivalent to `ConcatenateGraphFilter.SEP_LABEL`,
/// `TokenStreamToAutomaton.POS_SEP`.
pub const SEP_LABEL: i32 = POS_SEP;

/// Equivalent to `ConcatenateGraphFilter.DEFAULT_MAX_GRAPH_EXPANSIONS`,
/// `Operations.DEFAULT_DETERMINIZE_WORK_LIMIT`.
pub const DEFAULT_MAX_GRAPH_EXPANSIONS: i32 = DEFAULT_DETERMINIZE_WORK_LIMIT;

/// Equivalent to `ConcatenateGraphFilter.DEFAULT_TOKEN_SEPARATOR`, the
/// `Character` [`SEP_LABEL`].
pub const DEFAULT_TOKEN_SEPARATOR: Option<char> = Some('\u{001F}');

/// Equivalent to `ConcatenateGraphFilter.DEFAULT_PRESERVE_SEP`.
pub const DEFAULT_PRESERVE_SEP: bool = true;

/// Equivalent to `ConcatenateGraphFilter.DEFAULT_PRESERVE_POSITION_INCREMENTS`.
pub const DEFAULT_PRESERVE_POSITION_INCREMENTS: bool = true;

/// Concatenates/Joins every incoming token with a separator into one output
/// token for every path through the token stream (which is a graph). In
/// simple cases this yields one token, but in the presence of any tokens
/// with a zero positionIncrmeent (e.g. synonyms) it will be more. This
/// filter uses the token bytes, position increment, and position length of
/// the incoming stream. Other attributes are not used or manipulated.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ConcatenateGraphFilter`
/// (`@lucene.experimental`), a `TokenStream` (not a `TokenFilter`) with an
/// attribute source of its own: it consumes its input entirely in the first
/// call to `incrementToken`.
pub struct ConcatenateGraphFilter {
    source: AttributeSource,
    input_token_stream: Box<dyn TokenStream>,
    token_separator: Option<char>,
    preserve_position_increments: bool,
    max_graph_expansions: i32,
    finite_strings: Option<LimitedFiniteStringsIterator<'static>>,
    /// Whether `getAttribute(CharTermAttribute.class)` found one at
    /// `reset()`, Java's non-`null` `charTermAttribute`.
    char_term_attribute: bool,
    was_reset: bool,
    end_offset: i32,
}

impl fmt::Debug for ConcatenateGraphFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConcatenateGraphFilter")
            .field("input_token_stream", &self.input_token_stream)
            .field("token_separator", &self.token_separator)
            .field(
                "preserve_position_increments",
                &self.preserve_position_increments,
            )
            .field("max_graph_expansions", &self.max_graph_expansions)
            .field("was_reset", &self.was_reset)
            .field("end_offset", &self.end_offset)
            .finish_non_exhaustive()
    }
}

impl ConcatenateGraphFilter {
    /// Creates a token stream to convert `input` to a token stream of
    /// accepted strings by its token stream graph.
    ///
    /// This constructor uses the default settings of the constants in this
    /// class.
    ///
    /// Equivalent to `ConcatenateGraphFilter(TokenStream)`.
    ///
    /// # Errors
    ///
    /// As [`with_separator`](Self::with_separator).
    pub fn new(input_token_stream: Box<dyn TokenStream>) -> Result<Self> {
        Self::with_separator(
            input_token_stream,
            DEFAULT_TOKEN_SEPARATOR,
            DEFAULT_PRESERVE_POSITION_INCREMENTS,
            DEFAULT_MAX_GRAPH_EXPANSIONS,
        )
    }

    /// Creates a token stream to convert `input` to a token stream of
    /// accepted strings by its token stream graph.
    ///
    /// * `input_token_stream` - The input/incoming TokenStream
    /// * `token_separator` - Separator to use for concatenation. Can be
    ///   `None`, in this case tokens will be concatenated without any
    ///   separators.
    /// * `preserve_position_increments` - Whether to add an empty token for
    ///   missing positions. The effect is a consecutive [`SEP_LABEL`]. When
    ///   false, it's as if there were no missing positions (we pretend the
    ///   surrounding tokens were adjacent).
    /// * `max_graph_expansions` - If the tokenStream graph has more than this
    ///   many possible paths through, then we'll throw
    ///   `TooComplexToDeterminizeException` to preserve the stability and
    ///   memory of the machine.
    ///
    /// Equivalent to `ConcatenateGraphFilter(TokenStream, Character, boolean,
    /// int)`, which does not call `super(input)`: this is a true delegate
    /// with a new attribute source, over the default token attribute
    /// factory, to which its field initializers add the
    /// `BytesRefBuilderTermAttribute`, the `PositionIncrementAttribute` and
    /// the `OffsetAttribute`.
    ///
    /// # Errors
    ///
    /// What the attribute factory raises for an interface it cannot
    /// implement, as Java's `addAttribute` throws.
    pub fn with_separator(
        input_token_stream: Box<dyn TokenStream>,
        token_separator: Option<char>,
        preserve_position_increments: bool,
        max_graph_expansions: i32,
    ) -> Result<Self> {
        let mut source =
            AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
        add_bytes_ref_builder_term_attribute(&mut source);
        source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        source.add_attribute_interface::<dyn OffsetAttribute>()?;
        Ok(Self {
            source,
            input_token_stream,
            token_separator,
            preserve_position_increments,
            max_graph_expansions,
            finite_strings: None,
            char_term_attribute: false,
            was_reset: false,
            end_offset: 0,
        })
    }

    /// Calls [`with_separator`](Self::with_separator) with
    /// [`DEFAULT_TOKEN_SEPARATOR`] if `preserve_sep` is true, otherwise
    /// `None`.
    ///
    /// Equivalent to the deprecated `ConcatenateGraphFilter(TokenStream,
    /// boolean, boolean, int)`.
    ///
    /// # Errors
    ///
    /// As [`with_separator`](Self::with_separator).
    #[deprecated(note = "Use with_separator, which takes the separator itself")]
    pub fn with_preserve_sep(
        input_token_stream: Box<dyn TokenStream>,
        preserve_sep: bool,
        preserve_position_increments: bool,
        max_graph_expansions: i32,
    ) -> Result<Self> {
        Self::with_separator(
            input_token_stream,
            if preserve_sep {
                DEFAULT_TOKEN_SEPARATOR
            } else {
                None
            },
            preserve_position_increments,
            max_graph_expansions,
        )
    }

    /// Converts the tokenStream to an automaton, treating the transition
    /// labels as utf-8. Does *not* close it.
    ///
    /// Equivalent to `ConcatenateGraphFilter.toAutomaton()`, `toAutomaton(false)`.
    ///
    /// # Errors
    ///
    /// As [`to_automaton_unicode_aware`](Self::to_automaton_unicode_aware).
    pub fn to_automaton(&mut self) -> Result<Automaton> {
        self.to_automaton_unicode_aware(false)
    }

    /// Converts the tokenStream to an automaton. Does *not* close it.
    ///
    /// Equivalent to `ConcatenateGraphFilter.toAutomaton(boolean)`: the
    /// stream becomes an automaton through a `TokenStreamToAutomaton` (an
    /// `EscapingTokenStreamToAutomaton` when there is a separator), the
    /// separators are replaced, and the automaton is determinized.
    ///
    /// # Errors
    ///
    /// What the input stream raises, and `TooComplexToDeterminizeException`
    /// when determinizing takes more than `maxGraphExpansions` effort.
    pub fn to_automaton_unicode_aware(&mut self, unicode_aware: bool) -> Result<Automaton> {
        // TODO refactor this
        // maybe we could hook up a modified automaton from TermAutomatonQuery here?

        // Create corresponding automaton: labels are bytes
        // from each analyzed token, with byte 0 used as
        // separator between tokens:
        let mut tsta = TokenStreamToAutomaton::new();
        tsta.set_preserve_position_increments(self.preserve_position_increments);
        tsta.set_unicode_arcs(unicode_aware);

        let automaton = match self.token_separator {
            Some(token_separator) => {
                let mut escaping = EscapingTokenStreamToAutomaton::new(token_separator as i32);
                tsta.to_automaton_changing_tokens(self.input_token_stream.as_mut(), &mut |token| {
                    escaping.change_token(&token)
                })?
            }
            // When we're not preserving sep, we don't steal 0xff
            // byte, so we don't need to do any escaping:
            None => tsta.to_automaton(self.input_token_stream.as_mut())?,
        };

        // TODO: we can optimize this somewhat by determinizing
        // while we convert
        let automaton = replace_sep(&automaton, self.token_separator)?;
        // This automaton should not blow up during determinize:
        Ok(Operations::determinize(
            &automaton,
            self.max_graph_expansions,
        )?)
    }

    /// The `BytesRefBuilderTermAttribute` the constructor added.
    fn bytes_att(
        &self,
    ) -> rucene::util::atomic_ref_cell::AtomicRefMut<'_, dyn BytesRefBuilderTermAttribute> {
        self.source
            .get_attribute_interface_mut::<dyn BytesRefBuilderTermAttribute>()
            .expect("INVARIANT: the constructor added the BytesRefBuilderTermAttribute")
    }
}

impl TokenStream for ConcatenateGraphFilter {
    /// Equivalent to `ConcatenateGraphFilter.reset()`: Java captures
    /// `getAttribute(CharTermAttribute.class)`, which may be `null`, only
    /// if it really needs it, to save the UTF-8 to UTF-16 conversion.
    fn reset(&mut self) -> Result<()> {
        // we only capture this if we really need it to save the UTF-8 to UTF-16 conversion
        self.char_term_attribute = self
            .source
            .has_attribute_interface::<dyn CharTermAttribute>();
        // make sure the TermToBytesRefAttribute attribute is implemented by our class, not via
        // CharTermAttribute's
        debug_assert!(self
            .source
            .get_attribute_by_id(TypeId::of::<dyn TermToBytesRefAttribute>())
            .is_some_and(|cell| cell
                .borrow()
                .as_any()
                .is::<BytesRefBuilderTermAttributeImpl>()));
        self.was_reset = true;
        Ok(())
    }

    /// Equivalent to `ConcatenateGraphFilter.incrementToken()`.
    fn increment_token(&mut self) -> Result<bool> {
        if self.finite_strings.is_none() {
            if !self.was_reset {
                return Err(LuceneError::illegal_state(
                    "reset() missing before incrementToken",
                ));
            }
            // lazy init/consume
            // calls reset(), incrementToken() repeatedly, and end() on inputTokenStream
            let automaton = self.to_automaton()?;
            self.finite_strings = Some(LimitedFiniteStringsIterator::new_shared(
                Arc::new(automaton),
                self.max_graph_expansions,
            )?);
            // note: would be nice to know the startOffset but toAutomaton doesn't capture it.
            // We'll assume 0
            self.end_offset = self
                .input_token_stream
                .attribute_source()
                .get_attribute_interface::<dyn OffsetAttribute>()
                .ok_or_else(|| {
                    LuceneError::illegal_argument(
                        "This AttributeSource does not have the attribute 'org.apache.lucene.analysis.tokenattributes.OffsetAttribute'.",
                    )
                })?
                .end_offset();
        }

        let finite_strings = self
            .finite_strings
            .as_mut()
            .expect("INVARIANT: initialized above");
        let Some(string) = finite_strings.next()? else {
            return Ok(false);
        };
        // if number of iterated strings so far is more than one...
        let stacked = finite_strings.size() > 1;

        self.source.clear_attributes();

        if stacked {
            self.source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .set_position_increment(0); // stacked
        }

        self.source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(0, self.end_offset);

        let utf16 = {
            let mut bytes_att = self.bytes_att();
            Util::to_bytes_ref(&string, bytes_att.builder_mut())?; // now we have UTF-8
            if self.char_term_attribute {
                Some(bytes_att.to_utf16())
            } else {
                None
            }
        };
        if let Some(utf16) = utf16 {
            let chars = utf16_to_term_chars(&utf16);
            let mut char_term_attribute = self
                .source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: reset() found the CharTermAttribute");
            char_term_attribute.set_length(0);
            let length = chars.len();
            char_term_attribute.resize_buffer(length)[..length].copy_from_slice(&chars);
            char_term_attribute.set_length(length);
        }

        Ok(true)
    }

    /// Equivalent to `ConcatenateGraphFilter.end()`.
    fn end(&mut self) -> Result<()> {
        self.source.end_attributes();
        if self.finite_strings.is_none() {
            // thus inputTokenStream hasn't yet received end()
            // the input TS may really want to see "end()" called even if incrementToken
            // hasn't.
            self.input_token_stream.end()?;
        } // else we already eagerly consumed inputTokenStream including end()
        if self.end_offset != -1 {
            self.source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute")
                .set_offset(0, self.end_offset);
        }
        Ok(())
    }

    /// Equivalent to `ConcatenateGraphFilter.close()`: delegate lifecycle
    /// (`toAutomaton` does not close the stream).
    fn close(&mut self) -> Result<()> {
        self.input_token_stream.close()?;
        self.finite_strings = None;
        self.was_reset = false; // reset
        self.end_offset = -1; // reset
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// Just escapes the [`SEP_LABEL`] byte with an extra.
///
/// Equivalent to the private `org.apache.lucene.analysis.miscellaneous.ConcatenateGraphFilter$EscapingTokenStreamToAutomaton`,
/// a `TokenStreamToAutomaton` subclass overriding `changeToken`: the
/// override is [`change_token`](Self::change_token), which
/// `ConcatenateGraphFilter` hands to
/// `TokenStreamToAutomaton::to_automaton_changing_tokens`.
#[derive(Debug)]
pub struct EscapingTokenStreamToAutomaton {
    spare: BytesRefBuilder,
    sep_label: u8,
}

impl EscapingTokenStreamToAutomaton {
    /// Equivalent to `EscapingTokenStreamToAutomaton(int)`: `assert sepLabel
    /// <= Byte.MAX_VALUE; this.sepLabel = (byte) sepLabel`, the narrowing
    /// cast keeping the low byte.
    pub fn new(sep_label: i32) -> Self {
        debug_assert!(sep_label <= i32::from(i8::MAX));
        Self {
            spare: BytesRefBuilder::new(),
            sep_label: sep_label as u8,
        }
    }

    /// Equivalent to `EscapingTokenStreamToAutomaton.changeToken(BytesRef)`.
    pub fn change_token(&mut self, input: &BytesRef) -> BytesRef {
        let mut upto = 0;
        for &b in input.slice() {
            if b == self.sep_label {
                self.spare.grow(upto + 2);
                self.spare.set_byte_at(upto, self.sep_label);
                upto += 1;
                self.spare.set_byte_at(upto, b);
                upto += 1;
            } else {
                self.spare.grow(upto + 1);
                self.spare.set_byte_at(upto, b);
                upto += 1;
            }
        }
        self.spare.set_length(upto);
        self.spare.get()
    }
}

/// Replaces SEP with epsilon or remaps them if we were asked to preserve
/// them.
///
/// Equivalent to the private static `ConcatenateGraphFilter.replaceSep(Automaton,
/// Character)`.
fn replace_sep(a: &Automaton, token_separator: Option<char>) -> Result<Automaton> {
    let mut result = Automaton::new();

    // Copy all states over
    let num_states = a.get_num_states();
    for s in 0..num_states {
        result.create_state();
        result.set_accept(s, a.is_accept(s));
    }

    // Go in reverse topo sort so we know we only have to
    // make one pass:
    let mut t = Transition::new();
    let topo_sort_states = Operations::topo_sort_states(a)?;
    for i in 0..topo_sort_states.len() {
        let state = topo_sort_states[topo_sort_states.len() - 1 - i];
        let count = a.init_transition(state, &mut t);
        for _ in 0..count {
            a.get_next_transition(&mut t);
            if t.min == POS_SEP {
                debug_assert!(t.max == POS_SEP);
                if let Some(token_separator) = token_separator {
                    // Remap to tokenSeparator:
                    result.add_transition(state, t.dest, token_separator as i32);
                } else {
                    result.add_epsilon(state, t.dest);
                }
            } else if t.min == HOLE {
                debug_assert!(t.max == HOLE);

                // Just remove the hole: there will then be two
                // SEP tokens next to each other, which will only
                // match another hole at search time.  Note that
                // it will also match an empty-string token ... if
                // that's somehow a problem we can always map HOLE
                // to a dedicated byte (and escape it in the
                // input).
                result.add_epsilon(state, t.dest);
            } else {
                result.add_transition_range(state, t.dest, t.min, t.max);
            }
        }
    }

    result.finish_state();

    Ok(result)
}

/// Attribute providing access to the term builder and UTF-16 conversion.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ConcatenateGraphFilter$BytesRefBuilderTermAttribute`
/// (`@lucene.internal`), which extends `TermToBytesRefAttribute`.
pub trait BytesRefBuilderTermAttribute: TermToBytesRefAttribute {
    /// Returns the builder from which the term is derived.
    ///
    /// Equivalent to `BytesRefBuilderTermAttribute.builder()`.
    fn builder(&self) -> &BytesRefBuilder;

    /// The builder, to fill it; Java's `builder()` returns the mutable
    /// builder itself.
    fn builder_mut(&mut self) -> &mut BytesRefBuilder;

    /// Returns the term represented as UTF-16 units.
    ///
    /// Equivalent to `BytesRefBuilderTermAttribute.toUTF16()`, whose
    /// `CharSequence` is the UTF-16 units of the term here.
    fn to_utf16(&mut self) -> Vec<u16>;
}

impl AttributeInterface for dyn BytesRefBuilderTermAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_any()
            .downcast_ref::<BytesRefBuilderTermAttributeImpl>()
            .map(|att| att as &dyn BytesRefBuilderTermAttribute)
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_any_mut()
            .downcast_mut::<BytesRefBuilderTermAttributeImpl>()
            .map(|att| att as &mut dyn BytesRefBuilderTermAttribute)
    }
}

/// Implementation of [`BytesRefBuilderTermAttribute`].
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ConcatenateGraphFilter$BytesRefBuilderTermAttributeImpl`
/// (`@lucene.internal`).
#[derive(Debug, Default)]
pub struct BytesRefBuilderTermAttributeImpl {
    bytes: BytesRefBuilder,
    /// Java's `transient CharsRefBuilder charsRef`, created on first use.
    chars_ref: Option<CharsRefBuilder>,
}

impl BytesRefBuilderTermAttributeImpl {
    /// Sole constructor no-op.
    ///
    /// Equivalent to `BytesRefBuilderTermAttributeImpl()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Attribute for BytesRefBuilderTermAttributeImpl {}

impl TermToBytesRefAttribute for BytesRefBuilderTermAttributeImpl {
    /// Equivalent to `BytesRefBuilderTermAttributeImpl.getBytesRef()`,
    /// `bytes.get()`.
    fn get_bytes_ref(&self) -> BytesRef {
        self.bytes.get()
    }
}

impl BytesRefBuilderTermAttribute for BytesRefBuilderTermAttributeImpl {
    fn builder(&self) -> &BytesRefBuilder {
        &self.bytes
    }

    fn builder_mut(&mut self) -> &mut BytesRefBuilder {
        &mut self.bytes
    }

    /// Equivalent to `BytesRefBuilderTermAttributeImpl.toUTF16()`.
    fn to_utf16(&mut self) -> Vec<u16> {
        let bytes = self.bytes.get();
        let chars_ref = self.chars_ref.get_or_insert_with(CharsRefBuilder::new);
        chars_ref.copy_utf8_bytes_ref(&bytes);
        chars_ref.chars()[..chars_ref.length()].to_vec()
    }
}

impl AttributeImpl for BytesRefBuilderTermAttributeImpl {
    fn as_term_to_bytes_ref_attribute(&self) -> Option<&dyn TermToBytesRefAttribute> {
        Some(self)
    }

    fn as_term_to_bytes_ref_attribute_mut(&mut self) -> Option<&mut dyn TermToBytesRefAttribute> {
        Some(self)
    }

    /// Equivalent to `BytesRefBuilderTermAttributeImpl.clear()`.
    fn clear(&mut self) {
        self.bytes.clear();
    }

    /// Equivalent to `BytesRefBuilderTermAttributeImpl.copyTo(AttributeImpl)`.
    ///
    /// # Panics
    ///
    /// With the `ClassCastException` Java's cast throws for a target of
    /// another class.
    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        match target
            .as_any_mut()
            .downcast_mut::<BytesRefBuilderTermAttributeImpl>()
        {
            Some(other) => other.bytes.copy_ref(&self.bytes.get()),
            None => {
                rucene::store::java_nio_file::throw_unchecked("java.lang.ClassCastException", None)
            }
        }
    }

    /// Equivalent to `BytesRefBuilderTermAttributeImpl.reflectWith(AttributeReflector)`.
    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn TermToBytesRefAttribute>(),
            std::any::type_name::<BytesRefBuilderTermAttributeImpl>(),
            "bytes",
            &self.get_bytes_ref(),
        );
    }

    /// Equivalent to `BytesRefBuilderTermAttributeImpl.clone()`: a new
    /// instance the bytes are copied to.
    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        let mut other = BytesRefBuilderTermAttributeImpl::new();
        self.copy_to(&mut other);
        Box::new(other)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static IDS: OnceLock<Vec<TypeId>> = OnceLock::new();
        IDS.get_or_init(|| {
            vec![
                TypeId::of::<BytesRefBuilderTermAttributeImpl>(),
                TypeId::of::<dyn BytesRefBuilderTermAttribute>(),
                TypeId::of::<dyn TermToBytesRefAttribute>(),
            ]
        })
    }
}

/// `addAttribute(BytesRefBuilderTermAttribute.class)`.
///
/// **Divergence from Lucene 10.5.0.** Java's `DefaultAttributeFactory`
/// resolves the `*Impl` class reflectively from the interface name; this
/// port's factories are explicit registries, so the implementation is
/// installed directly, which is what the reflective lookup produces.
fn add_bytes_ref_builder_term_attribute(source: &mut AttributeSource) {
    if !source.has_attribute_interface::<dyn BytesRefBuilderTermAttribute>() {
        source.add_attribute_impl_instance(Box::new(BytesRefBuilderTermAttributeImpl::new()));
    }
}
