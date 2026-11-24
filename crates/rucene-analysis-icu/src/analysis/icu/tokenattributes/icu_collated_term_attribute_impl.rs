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
//! `org.apache.lucene.analysis.icu.tokenattributes.ICUCollatedTermAttributeImpl`.

use std::any::{Any, TypeId};
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use icu4j::text::collator::Collator;
use icu4j::text::raw_collation_key::RawCollationKey;
use rucene::analysis::character_utils::term_to_utf16;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, CharTermAttributeImpl, TermToBytesRefAttribute,
};
use rucene::error::LuceneError;
use rucene::util::attribute::{Attribute, AttributeImpl, AttributeReflector};
use rucene::util::{BytesRef, IOUtils};

/// Extension of `CharTermAttributeImpl` that encodes the term text as a
/// binary Unicode collation key instead of as UTF-8 bytes.
///
/// Equivalent to
/// `org.apache.lucene.analysis.icu.tokenattributes.ICUCollatedTermAttributeImpl`.
///
/// **Divergence from Lucene 10.5.0 (inherited).** Java extends
/// `CharTermAttributeImpl`; here the implementation is a field the
/// inherited members delegate to, the shape of `CollatedTermAttributeImpl`
/// of `rucene-analysis-common`. Java's `getBytesRef()` points the inherited
/// builder's `BytesRef` at the key's array; here the key's first `size`
/// bytes are returned in a new `BytesRef`, as
/// `CharTermAttributeImpl.get_bytes_ref` returns an owned copy. Java's
/// `clone()` (the shallow `Object.clone` under `CharTermAttributeImpl`'s)
/// shares the collator and the key with the copy; here the copy shares the
/// collator (`Arc`) and has its own key, the key being scratch space that
/// `getBytesRef()` overwrites before every read.
pub struct ICUCollatedTermAttributeImpl {
    inner: CharTermAttributeImpl,
    collator: Arc<dyn Collator>,
    key: Mutex<RawCollationKey>,
}

impl Clone for ICUCollatedTermAttributeImpl {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            collator: Arc::clone(&self.collator),
            key: Mutex::new(RawCollationKey::new()),
        }
    }
}

impl fmt::Debug for ICUCollatedTermAttributeImpl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ICUCollatedTermAttributeImpl")
            .field("inner", &self.inner)
            .field("collator", &self.collator)
            .finish_non_exhaustive()
    }
}

impl ICUCollatedTermAttributeImpl {
    /// Create a new ICUCollatedTermAttributeImpl
    ///
    /// * `collator`: Collation key generator
    ///
    /// Equivalent to `ICUCollatedTermAttributeImpl(Collator)`, which keeps
    /// `(Collator) collator.clone()`.
    ///
    /// # Panics
    ///
    /// With the `RuntimeException` Java wraps a `CloneNotSupportedException`
    /// of the collator in.
    pub fn new(collator: &dyn Collator) -> Self {
        // clone the collator: see http://userguide.icu-project.org/collation/architecture
        let collator: Arc<dyn Collator> = Arc::from(
            collator
                .clone_box()
                .unwrap_or_else(|e| IOUtils::rethrow_always(LuceneError::runtime_exception(e))),
        );
        Self {
            inner: CharTermAttributeImpl::new(),
            collator,
            key: Mutex::new(RawCollationKey::new()),
        }
    }

    /// The attribute interfaces this implementation covers: itself,
    /// `CharTermAttribute` and `TermToBytesRefAttribute`, those of
    /// `CharTermAttributeImpl`, which it extends.
    pub fn interfaces() -> &'static [TypeId] {
        static IDS: OnceLock<Vec<TypeId>> = OnceLock::new();
        IDS.get_or_init(|| {
            vec![
                TypeId::of::<ICUCollatedTermAttributeImpl>(),
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

impl Attribute for ICUCollatedTermAttributeImpl {}

impl CharTermAttribute for ICUCollatedTermAttributeImpl {
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

impl TermToBytesRefAttribute for ICUCollatedTermAttributeImpl {
    /// Equivalent to `ICUCollatedTermAttributeImpl.getBytesRef()`:
    /// `collator.getRawCollationKey(toString(), key)`, then the key's bytes,
    /// offset 0, length `key.size`.
    fn get_bytes_ref(&self) -> BytesRef {
        let mut units = Vec::new();
        term_to_utf16(&self.inner.buffer()[..self.inner.length()], &mut units);
        let mut key = self.key.lock().unwrap_or_else(PoisonError::into_inner);
        let filled = self
            .collator
            .get_raw_collation_key(Some(&units), Some(std::mem::take(&mut *key)))
            .expect("INVARIANT: a non-null source yields the key");
        let size = filled.size.max(0) as usize;
        let bytes: Vec<u8> = filled
            .bytes
            .as_deref()
            .map(|b| b[..size].iter().map(|&x| x as u8).collect())
            .unwrap_or_default();
        *key = filled;
        BytesRef::new(bytes)
    }
}

impl AttributeImpl for ICUCollatedTermAttributeImpl {
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
            .downcast_mut::<ICUCollatedTermAttributeImpl>()
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
            std::any::type_name::<ICUCollatedTermAttributeImpl>(),
            "term",
            &self.term(),
        );
        reflector.reflect(
            TypeId::of::<dyn TermToBytesRefAttribute>(),
            std::any::type_name::<ICUCollatedTermAttributeImpl>(),
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
        match other
            .as_any()
            .downcast_ref::<ICUCollatedTermAttributeImpl>()
        {
            Some(o) => self.inner.equals(&o.inner),
            None => self.inner.equals(other),
        }
    }

    /// The inherited `CharTermAttributeImpl.hashCode()`.
    fn hash_code(&self) -> i32 {
        AttributeImpl::hash_code(&self.inner)
    }
}
