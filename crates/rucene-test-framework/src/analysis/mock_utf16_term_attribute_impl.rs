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

//! Port of `org.apache.lucene.tests.analysis.MockUTF16TermAttributeImpl`.

use std::any::{Any, TypeId};
use std::sync::{Arc, OnceLock};

use rucene::analysis::default_attribute_factory;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, CharTermAttributeImpl, TermToBytesRefAttribute,
};
use rucene::util::attribute::{
    get_static_implementation, Attribute, AttributeFactory, AttributeImpl, AttributeReflector,
};
use rucene::util::BytesRef;

/// Extension of [`CharTermAttributeImpl`] that encodes the term text as
/// UTF-16 bytes instead of as UTF-8 bytes.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockUTF16TermAttributeImpl`.
///
/// **Divergence from Lucene 10.5.0.** Java extends `CharTermAttributeImpl`;
/// here the implementation is a field the inherited members delegate to.
#[derive(Clone, Debug, Default)]
pub struct MockUTF16TermAttributeImpl {
    inner: CharTermAttributeImpl,
}

impl MockUTF16TermAttributeImpl {
    /// Factory that returns an instance of this class for
    /// `CharTermAttribute`.
    ///
    /// Equivalent to `MockUTF16TermAttributeImpl.UTF16_TERM_ATTRIBUTE_FACTORY`.
    pub fn utf16_term_attribute_factory() -> Arc<dyn AttributeFactory> {
        static FACTORY: OnceLock<Arc<dyn AttributeFactory>> = OnceLock::new();
        Arc::clone(FACTORY.get_or_init(|| {
            Arc::new(get_static_implementation::<MockUTF16TermAttributeImpl>(
                default_attribute_factory(),
            ))
        }))
    }

    fn term(&self) -> String {
        self.inner.buffer()[..self.inner.length()].iter().collect()
    }
}

impl Attribute for MockUTF16TermAttributeImpl {}

impl CharTermAttribute for MockUTF16TermAttributeImpl {
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
    fn append_string(&mut self, s: &str) {
        self.inner.append_string(s)
    }
    fn append_string_range(&mut self, s: &str, start: usize, end: usize) {
        self.inner.append_string_range(s, start, end)
    }
    fn append_char(&mut self, c: char) {
        self.inner.append_char(c)
    }
    fn append_char_term_attribute(&mut self, term_att: &dyn CharTermAttribute) {
        self.inner.append_char_term_attribute(term_att)
    }
}

impl TermToBytesRefAttribute for MockUTF16TermAttributeImpl {
    /// `getBytesRef()`: `toString().getBytes(StandardCharsets.UTF_16LE)`.
    fn get_bytes_ref(&self) -> BytesRef {
        let bytes: Vec<u8> = self
            .term()
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        BytesRef::new(bytes)
    }
}

impl AttributeImpl for MockUTF16TermAttributeImpl {
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

    fn clear(&mut self) {
        self.inner.clear();
    }

    /// `CharTermAttributeImpl.copyTo`: `((CharTermAttribute)
    /// target).copyBuffer(termBuffer, 0, termLength)`.
    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target
            .as_any_mut()
            .downcast_mut::<MockUTF16TermAttributeImpl>()
        {
            t.inner
                .copy_buffer(self.inner.buffer(), 0, self.inner.length());
        } else {
            self.inner.copy_to(target);
        }
    }

    /// `CharTermAttributeImpl.reflectWith`, whose `bytes` property is the
    /// overridden `getBytesRef()`: the UTF-16LE bytes.
    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn CharTermAttribute>(),
            std::any::type_name::<MockUTF16TermAttributeImpl>(),
            "term",
            &self.term(),
        );
        reflector.reflect(
            TypeId::of::<dyn TermToBytesRefAttribute>(),
            std::any::type_name::<MockUTF16TermAttributeImpl>(),
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
        static IDS: OnceLock<Vec<TypeId>> = OnceLock::new();
        IDS.get_or_init(|| {
            vec![
                TypeId::of::<MockUTF16TermAttributeImpl>(),
                TypeId::of::<dyn CharTermAttribute>(),
                TypeId::of::<dyn TermToBytesRefAttribute>(),
            ]
        })
    }

    /// `CharTermAttributeImpl.equals`: the same term text.
    fn equals(&self, other: &dyn AttributeImpl) -> bool {
        match other.as_any().downcast_ref::<MockUTF16TermAttributeImpl>() {
            Some(o) => self.inner.equals(&o.inner),
            None => self.inner.equals(other),
        }
    }

    fn hash_code(&self) -> i32 {
        AttributeImpl::hash_code(&self.inner)
    }
}
