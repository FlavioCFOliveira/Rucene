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

//! Interface-level access to the attributes of a token stream.
//!
//! Java code holds an attribute by its interface — `CharTermAttribute
//! termAtt = addAttribute(CharTermAttribute.class)` — whatever class the
//! source's factory chose to implement it with: `PackedTokenAttributeImpl`
//! (`TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY`), [`Token`]
//! (`Token.TOKEN_ATTRIBUTE_FACTORY`), one `XImpl` per interface
//! (`AttributeFactory.DEFAULT_ATTRIBUTE_FACTORY`) or
//! [`MockUTF16TermAttributeImpl`]. Rust cannot cast a `dyn AttributeImpl` to
//! another trait object, so [`Att`] keeps the shared implementation cell
//! returned by [`AttributeSource::add_attribute_by_id`] and casts it to the
//! interface by trying each implementation the framework's factories can
//! produce.
//!
//! **Divergence from Lucene 10.5.0.** An implementation outside that closed
//! set cannot be cast; the accessors panic naming it.

use rucene::util::atomic_ref_cell::{AtomicRef, AtomicRefMut};
use std::any::TypeId;

use rucene::analysis::tokenattributes::{
    BytesTermAttribute, BytesTermAttributeImpl, CharTermAttribute, CharTermAttributeImpl,
    FlagsAttribute, FlagsAttributeImpl, KeywordAttribute, KeywordAttributeImpl, OffsetAttribute,
    OffsetAttributeImpl, PackedTokenAttributeImpl, PayloadAttribute, PayloadAttributeImpl,
    PositionIncrementAttribute, PositionIncrementAttributeImpl, PositionLengthAttribute,
    PositionLengthAttributeImpl, TermFrequencyAttribute, TermFrequencyAttributeImpl,
    TermToBytesRefAttribute, TypeAttribute, TypeAttributeImpl,
};
use rucene::error::Result;
use rucene::search::{BoostAttribute, BoostAttributeImpl};
use rucene::util::attribute::{AttributeCell, AttributeImpl, AttributeSource};

use super::base_token_stream_test_case::{
    CheckClearAttributesAttribute, CheckClearAttributesAttributeImpl,
};
use super::mock_utf16_term_attribute_impl::MockUTF16TermAttributeImpl;
use super::token::Token;

/// An attribute of a token stream, held by its interface.
///
/// The Java field `XAttribute xAtt = addAttribute(XAttribute.class)`.
#[derive(Clone)]
pub struct Att(AttributeCell);

impl std::fmt::Debug for Att {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Att").field(&self.0.borrow()).finish()
    }
}

macro_rules! accessors {
    ($(($get:ident, $get_mut:ident, $iface:ident, [$($imp:ty),+] $(, $cast:ident, $cast_mut:ident)?)),+ $(,)?) => {
        impl Att {
            $(
                #[doc = concat!("Borrows the implementation as `", stringify!($iface), "`.")]
                ///
                /// # Panics
                ///
                /// When the implementation is borrowed mutably, or is not one
                /// the framework can cast.
                pub fn $get(&self) -> AtomicRef<'_, dyn $iface> {
                    AtomicRef::map(self.0.borrow(), |att| {
                        let any = att.as_any();
                        $(
                            if let Some(x) = any.downcast_ref::<$imp>() {
                                return x as &dyn $iface;
                            }
                        )+
                        // any other implementation of the interface (Java's
                        // `getAttribute(X.class)` returns it whatever its class)
                        $(
                            if let Some(x) = att.$cast() {
                                return x;
                            }
                        )?
                        panic!(
                            "{:?} does not implement {}",
                            att,
                            stringify!($iface)
                        )
                    })
                }

                #[doc = concat!("Borrows the implementation mutably as `", stringify!($iface), "`.")]
                ///
                /// # Panics
                ///
                /// When the implementation is borrowed, or is not one the
                /// framework can cast.
                pub fn $get_mut(&self) -> AtomicRefMut<'_, dyn $iface> {
                    AtomicRefMut::map(self.0.borrow_mut(), |att| {
                        let name = format!("{:?}", att);
                        let known = {
                            let any = att.as_any();
                            false $(|| any.is::<$imp>())+
                        };
                        if !known {
                            // any other implementation of the interface
                            // (Java's `getAttribute(X.class)` returns it
                            // whatever its class)
                            $(
                                return att
                                    .$cast_mut()
                                    .unwrap_or_else(|| panic!("{} does not implement {}", name, stringify!($iface)));
                            )?
                            #[allow(unreachable_code)]
                            {
                                panic!("{} does not implement {}", name, stringify!($iface))
                            }
                        }
                        let any = att.as_any_mut();
                        $(
                            if any.is::<$imp>() {
                                return any.downcast_mut::<$imp>().unwrap() as &mut dyn $iface;
                            }
                        )+
                        unreachable!("INVARIANT: `known` holds")
                    })
                }
            )+
        }
    };
}

accessors!(
    (
        char_term,
        char_term_mut,
        CharTermAttribute,
        [
            PackedTokenAttributeImpl,
            Token,
            CharTermAttributeImpl,
            MockUTF16TermAttributeImpl
        ],
        as_char_term_attribute,
        as_char_term_attribute_mut
    ),
    (
        term_to_bytes_ref,
        term_to_bytes_ref_mut,
        TermToBytesRefAttribute,
        [
            PackedTokenAttributeImpl,
            Token,
            CharTermAttributeImpl,
            MockUTF16TermAttributeImpl,
            BytesTermAttributeImpl
        ],
        as_term_to_bytes_ref_attribute,
        as_term_to_bytes_ref_attribute_mut
    ),
    (
        bytes_term,
        bytes_term_mut,
        BytesTermAttribute,
        [BytesTermAttributeImpl],
        as_bytes_term_attribute,
        as_bytes_term_attribute_mut
    ),
    (
        offset,
        offset_mut,
        OffsetAttribute,
        [PackedTokenAttributeImpl, Token, OffsetAttributeImpl],
        as_offset_attribute,
        as_offset_attribute_mut
    ),
    (
        pos_inc,
        pos_inc_mut,
        PositionIncrementAttribute,
        [
            PackedTokenAttributeImpl,
            Token,
            PositionIncrementAttributeImpl
        ],
        as_position_increment_attribute,
        as_position_increment_attribute_mut
    ),
    (
        pos_len,
        pos_len_mut,
        PositionLengthAttribute,
        [PackedTokenAttributeImpl, Token, PositionLengthAttributeImpl],
        as_position_length_attribute,
        as_position_length_attribute_mut
    ),
    (
        type_att,
        type_att_mut,
        TypeAttribute,
        [PackedTokenAttributeImpl, Token, TypeAttributeImpl],
        as_type_attribute,
        as_type_attribute_mut
    ),
    (
        term_freq,
        term_freq_mut,
        TermFrequencyAttribute,
        [PackedTokenAttributeImpl, Token, TermFrequencyAttributeImpl],
        as_term_frequency_attribute,
        as_term_frequency_attribute_mut
    ),
    (
        flags,
        flags_mut,
        FlagsAttribute,
        [Token, FlagsAttributeImpl],
        as_flags_attribute,
        as_flags_attribute_mut
    ),
    (
        payload,
        payload_mut,
        PayloadAttribute,
        [Token, PayloadAttributeImpl],
        as_payload_attribute,
        as_payload_attribute_mut
    ),
    (
        keyword,
        keyword_mut,
        KeywordAttribute,
        [KeywordAttributeImpl],
        as_keyword_attribute,
        as_keyword_attribute_mut
    ),
    (boost, boost_mut, BoostAttribute, [BoostAttributeImpl]),
    (
        check_clear,
        check_clear_mut,
        CheckClearAttributesAttribute,
        [CheckClearAttributesAttributeImpl]
    ),
);

impl Att {
    /// `source.addAttribute(X.class)`: the implementation of the interface
    /// `iface` (`TypeId::of::<dyn X>()`), created through the source's
    /// factory when absent.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the factory cannot create it.
    pub fn add(source: &mut AttributeSource, iface: TypeId) -> Result<Att> {
        Ok(Att(source.add_attribute_by_id(iface)?))
    }

    /// `source.getAttribute(X.class)`: the implementation of `iface`, or
    /// `None` (Java's `null`).
    pub fn get(source: &AttributeSource, iface: TypeId) -> Option<Att> {
        source.get_attribute_by_id(iface).map(Att)
    }

    /// The shared implementation cell.
    pub fn cell(&self) -> &AttributeCell {
        &self.0
    }

    /// Java's `==` between two attribute references: the same instance.
    pub fn same(&self, other: &Att) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }

    /// The implementation, as an [`AttributeImpl`].
    pub fn implementation(&self) -> AtomicRef<'_, Box<dyn AttributeImpl>> {
        self.0.borrow()
    }

    /// `termAtt.toString()`: the term text.
    pub fn term_string(&self) -> String {
        let t = self.char_term();
        t.buffer()[..t.length()].iter().collect()
    }

    /// The UTF-16 length of the term, Java's `termAtt.length()`.
    pub fn term_length16(&self) -> i32 {
        let t = self.char_term();
        t.buffer()[..t.length()]
            .iter()
            .map(|c| c.len_utf16() as i32)
            .sum()
    }

    /// `termAtt.setEmpty().append(s)`.
    pub fn set_term(&self, s: &str) {
        let mut t = self.char_term_mut();
        t.set_empty();
        t.append_string(Some(s));
    }
}

/// `TypeId::of::<dyn CharTermAttribute>()`.
pub fn char_term_id() -> TypeId {
    TypeId::of::<dyn CharTermAttribute>()
}

/// `TypeId::of::<dyn TermToBytesRefAttribute>()`.
pub fn term_to_bytes_ref_id() -> TypeId {
    TypeId::of::<dyn TermToBytesRefAttribute>()
}

/// `TypeId::of::<dyn BytesTermAttribute>()`.
pub fn bytes_term_id() -> TypeId {
    TypeId::of::<dyn BytesTermAttribute>()
}

/// `TypeId::of::<dyn OffsetAttribute>()`.
pub fn offset_id() -> TypeId {
    TypeId::of::<dyn OffsetAttribute>()
}

/// `TypeId::of::<dyn PositionIncrementAttribute>()`.
pub fn pos_inc_id() -> TypeId {
    TypeId::of::<dyn PositionIncrementAttribute>()
}

/// `TypeId::of::<dyn PositionLengthAttribute>()`.
pub fn pos_len_id() -> TypeId {
    TypeId::of::<dyn PositionLengthAttribute>()
}

/// `TypeId::of::<dyn TypeAttribute>()`.
pub fn type_id() -> TypeId {
    TypeId::of::<dyn TypeAttribute>()
}

/// `TypeId::of::<dyn TermFrequencyAttribute>()`.
pub fn term_freq_id() -> TypeId {
    TypeId::of::<dyn TermFrequencyAttribute>()
}

/// `TypeId::of::<dyn FlagsAttribute>()`.
pub fn flags_id() -> TypeId {
    TypeId::of::<dyn FlagsAttribute>()
}

/// `TypeId::of::<dyn PayloadAttribute>()`.
pub fn payload_id() -> TypeId {
    TypeId::of::<dyn PayloadAttribute>()
}

/// `TypeId::of::<dyn KeywordAttribute>()`.
pub fn keyword_id() -> TypeId {
    TypeId::of::<dyn KeywordAttribute>()
}

/// `TypeId::of::<dyn BoostAttribute>()`.
pub fn boost_id() -> TypeId {
    TypeId::of::<dyn BoostAttribute>()
}
