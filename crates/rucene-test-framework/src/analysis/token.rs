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

//! Port of `org.apache.lucene.tests.analysis.Token`.

use std::any::{Any, TypeId};
use std::sync::{Arc, OnceLock};

use rucene::analysis::default_attribute_factory;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, FlagsAttribute, OffsetAttribute, PackedTokenAttributeImpl, PayloadAttribute,
    PositionIncrementAttribute, PositionLengthAttribute, TermFrequencyAttribute,
    TermToBytesRefAttribute, TypeAttribute,
};
use rucene::util::attribute::{
    get_static_implementation, Attribute, AttributeFactory, AttributeImpl, AttributeReflector,
};
use rucene::util::BytesRef;

/// A Token is an occurrence of a term from the text of a field. It consists
/// of a term's text, the start and end offset of the term in the text of the
/// field, and a type string.
///
/// The start and end offsets permit applications to re-associate a token
/// with its source text, e.g., to display highlighted query terms in a
/// document browser, or to show matching text fragments in a KWIC display,
/// etc.
///
/// The type is a string, assigned by a lexical analyzer (a.k.a. tokenizer),
/// naming the lexical or syntactic class that the token belongs to. The
/// default token type is "word".
///
/// A Token can optionally have metadata (a.k.a. payload) in the form of a
/// variable length byte array.
///
/// Equivalent to `org.apache.lucene.tests.analysis.Token`.
///
/// **Divergence from Lucene 10.5.0.** Java's `Token` extends
/// `PackedTokenAttributeImpl`; Rust has no implementation inheritance, so the
/// packed implementation is a field every inherited member delegates to. A
/// `Token` is therefore not found by a lookup of `PackedTokenAttributeImpl`
/// itself (Java has no such lookup: attributes are keyed by interface).
#[derive(Clone, Debug, Default)]
pub struct Token {
    packed: PackedTokenAttributeImpl,
    flags: i32,
    payload: Option<BytesRef>,
}

impl Token {
    /// Constructs a Token will null text.
    pub fn new() -> Token {
        Token::default()
    }

    /// Constructs a Token with the given term text, start and end offsets.
    /// The type defaults to "word."
    pub fn with_offsets(text: &str, start: i32, end: i32) -> Token {
        let mut t = Token::new();
        t.append_string(text);
        t.set_offset(start, end);
        t
    }

    /// Constructs a Token with the given term text, position increment,
    /// start and end offsets.
    pub fn with_pos_inc(text: &str, pos_inc: i32, start: i32, end: i32) -> Token {
        let mut t = Token::with_offsets(text, start, end);
        t.set_position_increment(pos_inc);
        t
    }

    /// `Token(CharSequence text, int posInc, int start, int end, int
    /// posLength)`.
    pub fn with_pos_len(text: &str, pos_inc: i32, start: i32, end: i32, pos_length: i32) -> Token {
        let mut t = Token::with_pos_inc(text, pos_inc, start, end);
        t.set_position_length(pos_length);
        t
    }

    /// `toString()`: the term text.
    pub fn term(&self) -> String {
        self.packed.term()
    }

    /// Copy the prototype token's fields into this one.
    ///
    /// Java shares the payload object ("a bad hack to emulate no cloning of
    /// payload"); a Rust `BytesRef` is a value, so it is copied — no caller
    /// can tell the difference without mutating the shared array.
    pub fn reinit(&mut self, prototype: &Token) {
        prototype.packed.copy_to(&mut self.packed);
        self.flags = prototype.flags;
        self.payload = prototype.payload.clone();
    }

    /// Convenience factory that returns `Token` as implementation for the
    /// basic attributes and return the default impl (with "Impl" appended)
    /// for all other attributes.
    ///
    /// Equivalent to `Token.TOKEN_ATTRIBUTE_FACTORY`.
    pub fn token_attribute_factory() -> Arc<dyn AttributeFactory> {
        static FACTORY: OnceLock<Arc<dyn AttributeFactory>> = OnceLock::new();
        Arc::clone(FACTORY.get_or_init(|| {
            Arc::new(get_static_implementation::<Token>(
                default_attribute_factory(),
            ))
        }))
    }

    /// `AttributeFactory.getStaticImplementation(delegate, Token.class)`
    /// over another delegate.
    ///
    /// Java's `DEFAULT_ATTRIBUTE_FACTORY` finds any `XImpl` by reflection;
    /// a caller whose attribute lies outside `lucene-core` passes a delegate
    /// that registers it.
    pub fn token_attribute_factory_over(
        delegate: Arc<dyn AttributeFactory>,
    ) -> Arc<dyn AttributeFactory> {
        Arc::new(get_static_implementation::<Token>(delegate))
    }
}

impl Attribute for Token {}

impl CharTermAttribute for Token {
    fn copy_buffer(&mut self, buffer: &[char], offset: usize, length: usize) {
        self.packed.copy_buffer(buffer, offset, length)
    }
    fn buffer(&self) -> &[char] {
        self.packed.buffer()
    }
    fn buffer_mut(&mut self) -> &mut [char] {
        self.packed.buffer_mut()
    }
    fn resize_buffer(&mut self, new_size: usize) -> &mut [char] {
        self.packed.resize_buffer(new_size)
    }
    fn set_length(&mut self, length: usize) {
        self.packed.set_length(length)
    }
    fn set_empty(&mut self) {
        self.packed.set_empty()
    }
    fn length(&self) -> usize {
        self.packed.length()
    }
    fn char_at(&self, index: usize) -> char {
        self.packed.char_at(index)
    }
    fn sub_sequence(&self, start: usize, end: usize) -> String {
        self.packed.sub_sequence(start, end)
    }
    fn append_string(&mut self, s: &str) {
        self.packed.append_string(s)
    }
    fn append_string_range(&mut self, s: &str, start: usize, end: usize) {
        self.packed.append_string_range(s, start, end)
    }
    fn append_char(&mut self, c: char) {
        self.packed.append_char(c)
    }
    fn append_char_term_attribute(&mut self, term_att: &dyn CharTermAttribute) {
        self.packed.append_char_term_attribute(term_att)
    }
}

impl TermToBytesRefAttribute for Token {
    fn get_bytes_ref(&self) -> BytesRef {
        self.packed.get_bytes_ref()
    }
}

impl OffsetAttribute for Token {
    fn start_offset(&self) -> i32 {
        self.packed.start_offset()
    }
    fn set_offset(&mut self, start_offset: i32, end_offset: i32) {
        self.packed.set_offset(start_offset, end_offset)
    }
    fn end_offset(&self) -> i32 {
        self.packed.end_offset()
    }
}

impl PositionIncrementAttribute for Token {
    fn set_position_increment(&mut self, position_increment: i32) {
        self.packed.set_position_increment(position_increment)
    }
    fn get_position_increment(&self) -> i32 {
        self.packed.get_position_increment()
    }
}

impl PositionLengthAttribute for Token {
    fn set_position_length(&mut self, position_length: i32) {
        self.packed.set_position_length(position_length)
    }
    fn get_position_length(&self) -> i32 {
        self.packed.get_position_length()
    }
}

impl TypeAttribute for Token {
    fn type_value(&self) -> &str {
        self.packed.type_value()
    }
    fn set_type(&mut self, type_value: String) {
        self.packed.set_type(type_value)
    }
}

impl TermFrequencyAttribute for Token {
    fn set_term_frequency(&mut self, term_frequency: i32) {
        self.packed.set_term_frequency(term_frequency)
    }
    fn get_term_frequency(&self) -> i32 {
        self.packed.get_term_frequency()
    }
}

impl FlagsAttribute for Token {
    /// `getFlags()`.
    fn get_flags(&self) -> i32 {
        self.flags
    }
    /// `setFlags(int)`.
    fn set_flags(&mut self, flags: i32) {
        self.flags = flags;
    }
}

impl PayloadAttribute for Token {
    /// `getPayload()`.
    fn get_payload(&self) -> Option<&BytesRef> {
        self.payload.as_ref()
    }
    /// `setPayload(BytesRef)`.
    fn set_payload(&mut self, payload: Option<BytesRef>) {
        self.payload = payload;
    }
}

impl AttributeImpl for Token {
    /// Resets the term text, payload, flags, positionIncrement,
    /// positionLength, startOffset, endOffset and token type to default.
    fn clear(&mut self) {
        self.packed.clear();
        self.flags = 0;
        self.payload = None;
    }

    /// `PackedTokenAttributeImpl.end()`, which `Token` inherits:
    /// `super.end()` calls the overridden `clear()` — this class's — and the
    /// position increment becomes `0`.
    fn end(&mut self) {
        AttributeImpl::clear(self);
        self.packed.set_position_increment(0);
    }

    /// `copyTo(AttributeImpl)`: the packed fields, the flags and a deep copy
    /// of the payload.
    ///
    /// # Panics
    ///
    /// Java's `ClassCastException` when `target` implements neither
    /// `FlagsAttribute` nor `PayloadAttribute` as a `Token`.
    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        match target.as_any_mut().downcast_mut::<Token>() {
            Some(t) => {
                self.packed.copy_to(&mut t.packed);
                t.flags = self.flags;
                t.payload = self.payload.as_ref().map(BytesRef::deep_copy_of);
            }
            None => panic!(
                "java.lang.ClassCastException: {:?} cannot be cast to org.apache.lucene.analysis.tokenattributes.FlagsAttribute",
                target
            ),
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        self.packed.reflect_with(reflector);
        reflector.reflect(
            TypeId::of::<dyn FlagsAttribute>(),
            std::any::type_name::<Token>(),
            "flags",
            &self.flags,
        );
        reflector.reflect(
            TypeId::of::<dyn PayloadAttribute>(),
            std::any::type_name::<Token>(),
            "payload",
            &self.payload,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
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
                TypeId::of::<Token>(),
                TypeId::of::<dyn CharTermAttribute>(),
                TypeId::of::<dyn TermToBytesRefAttribute>(),
                TypeId::of::<dyn OffsetAttribute>(),
                TypeId::of::<dyn PositionIncrementAttribute>(),
                TypeId::of::<dyn PositionLengthAttribute>(),
                TypeId::of::<dyn TypeAttribute>(),
                TypeId::of::<dyn TermFrequencyAttribute>(),
                TypeId::of::<dyn FlagsAttribute>(),
                TypeId::of::<dyn PayloadAttribute>(),
            ]
        })
    }

    /// `equals(Object)`: another `Token` with the same flags, payload and
    /// packed fields.
    fn equals(&self, other: &dyn AttributeImpl) -> bool {
        match other.as_any().downcast_ref::<Token>() {
            Some(o) => {
                std::ptr::eq(self, o)
                    || (self.flags == o.flags
                        && self.payload == o.payload
                        && self.packed.equals(&o.packed))
            }
            None => false,
        }
    }

    /// `hashCode()`: the packed hash, then the flags and the payload's hash.
    fn hash_code(&self) -> i32 {
        let mut code = AttributeImpl::hash_code(&self.packed);
        code = code.wrapping_mul(31).wrapping_add(self.flags);
        if let Some(payload) = &self.payload {
            code = code.wrapping_mul(31).wrapping_add(payload.hash_code());
        }
        code
    }
}
