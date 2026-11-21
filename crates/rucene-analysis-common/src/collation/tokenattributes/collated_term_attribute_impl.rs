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

//! Extension of `CharTermAttributeImpl` that encodes the term text as a
//! binary Unicode collation key, ported from
//! `org.apache.lucene.collation.tokenattributes.CollatedTermAttributeImpl`.

use std::any::{Any, TypeId};
use std::sync::OnceLock;

use rucene::analysis::tokenattributes::{
    CharTermAttribute, CharTermAttributeImpl, TermToBytesRefAttribute,
};
use rucene::util::attribute::{Attribute, AttributeImpl, AttributeReflector};
use rucene::util::BytesRef;

use crate::jdk::collator::RuleBasedCollator;

/// Extension of `CharTermAttributeImpl` that encodes the term text as a
/// binary Unicode collation key instead of as UTF-8 bytes.
///
/// Equivalent to
/// `org.apache.lucene.collation.tokenattributes.CollatedTermAttributeImpl`.
///
/// **Divergence from Lucene 10.5.0.** Java extends `CharTermAttributeImpl`;
/// here the implementation is a field the inherited members delegate to (the
/// shape of `MockUTF16TermAttributeImpl` in the test framework). Java's
/// `getBytesRef()` points the inherited builder's `BytesRef` at the key's
/// array; here the key's bytes are returned in a new `BytesRef`, as
/// `CharTermAttributeImpl.get_bytes_ref` returns an owned copy.
#[derive(Clone, Debug)]
pub struct CollatedTermAttributeImpl {
    inner: CharTermAttributeImpl,
    collator: RuleBasedCollator,
}

impl CollatedTermAttributeImpl {
    /// Create a new CollatedTermAttributeImpl
    ///
    /// * `collator`: Collation key generator
    ///
    /// Equivalent to `CollatedTermAttributeImpl(Collator)`, which keeps
    /// `(Collator) collator.clone()`.
    pub fn new(collator: &RuleBasedCollator) -> Self {
        Self {
            inner: CharTermAttributeImpl::new(),
            // clone in case JRE doesn't properly sync,
            // or to reduce contention in case they do
            collator: collator.clone(),
        }
    }

    /// The attribute interfaces this implementation covers: itself,
    /// `CharTermAttribute` and `TermToBytesRefAttribute`, those of
    /// `CharTermAttributeImpl`, which it extends.
    pub fn interfaces() -> &'static [TypeId] {
        static IDS: OnceLock<Vec<TypeId>> = OnceLock::new();
        IDS.get_or_init(|| {
            vec![
                TypeId::of::<CollatedTermAttributeImpl>(),
                TypeId::of::<dyn CharTermAttribute>(),
                TypeId::of::<dyn TermToBytesRefAttribute>(),
            ]
        })
    }

    /// The inherited `CharTermAttributeImpl.toString()`: the term.
    fn term(&self) -> String {
        self.inner.buffer()[..self.inner.length()].iter().collect()
    }
}

impl Attribute for CollatedTermAttributeImpl {}

impl CharTermAttribute for CollatedTermAttributeImpl {
    fn copy_buffer(&mut self, buffer: &[char], offset: usize, length: usize) {
        self.inner.copy_buffer(buffer, offset, length)
    }
    fn buffer(&self) -> &[char] {
        self.inner.buffer()
    }
    fn buffer_mut(&mut self) -> &mut [char] {
        self.inner.buffer_mut()
    }
    fn resize_buffer(&mut self, new_size: usize) -> &mut [char] {
        self.inner.resize_buffer(new_size)
    }
    fn set_length(&mut self, length: usize) {
        self.inner.set_length(length)
    }
    fn set_empty(&mut self) {
        self.inner.set_empty()
    }
    fn length(&self) -> usize {
        self.inner.length()
    }
    fn char_at(&self, index: usize) -> char {
        self.inner.char_at(index)
    }
    fn sub_sequence(&self, start: usize, end: usize) -> String {
        self.inner.sub_sequence(start, end)
    }
    fn append_string(&mut self, s: Option<&str>) {
        self.inner.append_string(s)
    }
    fn append_string_range(&mut self, s: Option<&str>, start: usize, end: usize) {
        self.inner.append_string_range(s, start, end)
    }
    fn append_char(&mut self, c: char) {
        self.inner.append_char(c)
    }
    fn append_char_term_attribute(&mut self, term_att: Option<&dyn CharTermAttribute>) {
        self.inner.append_char_term_attribute(term_att)
    }
}

impl TermToBytesRefAttribute for CollatedTermAttributeImpl {
    /// Equivalent to `CollatedTermAttributeImpl.getBytesRef()`:
    /// `collator.getCollationKey(toString()).toByteArray()`, offset 0, its
    /// whole length.
    fn get_bytes_ref(&self) -> BytesRef {
        BytesRef::new(
            self.collator
                .get_collation_key(&self.term())
                .to_byte_array(),
        )
    }
}

impl AttributeImpl for CollatedTermAttributeImpl {
    fn as_char_term_attribute(&self) -> Option<&dyn CharTermAttribute> {
        Some(self)
    }

    fn as_char_term_attribute_mut(&mut self) -> Option<&mut dyn CharTermAttribute> {
        Some(self)
    }

    fn as_term_to_bytes_ref_attribute(&self) -> Option<&dyn TermToBytesRefAttribute> {
        Some(self)
    }

    fn as_term_to_bytes_ref_attribute_mut(&mut self) -> Option<&mut dyn TermToBytesRefAttribute> {
        Some(self)
    }

    /// The inherited `CharTermAttributeImpl.clear()`.
    fn clear(&mut self) {
        self.inner.clear();
    }

    /// The inherited `CharTermAttributeImpl.copyTo`: `((CharTermAttribute)
    /// target).copyBuffer(termBuffer, 0, termLength)`.
    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target
            .as_any_mut()
            .downcast_mut::<CollatedTermAttributeImpl>()
        {
            t.inner
                .copy_buffer(self.inner.buffer(), 0, self.inner.length());
        } else {
            self.inner.copy_to(target);
        }
    }

    /// The inherited `CharTermAttributeImpl.reflectWith`, whose `bytes`
    /// property is the overridden `getBytesRef()`: the collation key.
    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn CharTermAttribute>(),
            std::any::type_name::<CollatedTermAttributeImpl>(),
            "term",
            &self.term(),
        );
        reflector.reflect(
            TypeId::of::<dyn TermToBytesRefAttribute>(),
            std::any::type_name::<CollatedTermAttributeImpl>(),
            "bytes",
            &self.get_bytes_ref(),
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
        Self::interfaces()
    }

    /// The inherited `CharTermAttributeImpl.equals`: the same term text.
    fn equals(&self, other: &dyn AttributeImpl) -> bool {
        match other.as_any().downcast_ref::<CollatedTermAttributeImpl>() {
            Some(o) => self.inner.equals(&o.inner),
            None => self.inner.equals(other),
        }
    }

    /// The inherited `CharTermAttributeImpl.hashCode()`.
    fn hash_code(&self) -> i32 {
        AttributeImpl::hash_code(&self.inner)
    }
}
