// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.CharacterProperties`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};

use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::lang::u_script::UScript;
use crate::r#impl::character_properties_impl::CharacterPropertiesImpl;
use crate::r#impl::emoji_props::EmojiProps;
use crate::text::unicode_set::UnicodeSet;
use crate::util::code_point_trie::{CodePointTrie, Type, ValueWidth};
use crate::util::mutable_code_point_trie::MutableCodePointTrie;

/// Cached sets per binary property (`sets`), guarded as Java's
/// `synchronized(sets)`.
static SETS: Mutex<Vec<Option<Arc<UnicodeSet>>>> = Mutex::new(Vec::new());
/// Cached maps per int property (`maps`), guarded as Java's
/// `synchronized(maps)`.
static MAPS: Mutex<Vec<Option<Arc<CodePointTrie>>>> = Mutex::new(Vec::new());

/// Sets and maps for Unicode properties.
///
/// Equivalent to `com.ibm.icu.lang.CharacterProperties`.
/// `getIntPropertyMap` returns the immutable `CodePointTrie` Java returns
/// as a `CodePointMap`.
pub struct CharacterProperties;

impl CharacterProperties {
    fn make_set(property: i32) -> Result<UnicodeSet> {
        let mut set = UnicodeSet::new();
        if UProperty::BASIC_EMOJI <= property && property <= UProperty::RGI_EMOJI {
            // property of strings
            EmojiProps::instance().add_strings(property, &mut set)?;
            if property != UProperty::BASIC_EMOJI && property != UProperty::RGI_EMOJI {
                // property of _only_ strings
                set.freeze();
                return Ok(set);
            }
        }

        let inclusions = CharacterPropertiesImpl::get_inclusions_for_property(property)?;
        let num_ranges = inclusions.get_range_count();
        let mut start_has_property = -1;

        for i in 0..num_ranges {
            let range_end = inclusions.get_range_end(i);
            for c in inclusions.get_range_start(i)..=range_end {
                // TODO: Get a UCharacterProperty.BinaryProperty to avoid the property dispatch.
                if UCharacter::has_binary_property(c, property) {
                    if start_has_property < 0 {
                        // Transition from false to true.
                        start_has_property = c;
                    }
                } else if start_has_property >= 0 {
                    // Transition from true to false.
                    set.add_range(start_has_property, c - 1)?;
                    start_has_property = -1;
                }
            }
        }
        if start_has_property >= 0 {
            set.add_range(start_has_property, 0x10FFFF)?;
        }

        set.freeze();
        Ok(set)
    }

    fn make_map(property: i32) -> Result<CodePointTrie> {
        let null_value = if property == UProperty::SCRIPT { UScript::UNKNOWN } else { 0 };
        let mut mutable_trie = MutableCodePointTrie::new(null_value, null_value);
        let inclusions = CharacterPropertiesImpl::get_inclusions_for_property(property)?;
        let num_ranges = inclusions.get_range_count();
        let mut start = 0;
        let mut value = null_value;

        for i in 0..num_ranges {
            let range_end = inclusions.get_range_end(i);
            for c in inclusions.get_range_start(i)..=range_end {
                // TODO: Get a UCharacterProperty.IntProperty to avoid the property dispatch.
                let next_value = UCharacter::get_int_property_value(c, property);
                if value != next_value {
                    if value != null_value {
                        mutable_trie.set_range(start, c - 1, value)?;
                    }
                    start = c;
                    value = next_value;
                }
            }
        }
        if value != 0 {
            mutable_trie.set_range(start, 0x10FFFF, value)?;
        }

        let ty = if property == UProperty::BIDI_CLASS || property == UProperty::GENERAL_CATEGORY {
            Type::Fast
        } else {
            Type::Small
        };
        // Optimize the data size.
        let max = UCharacter::get_int_property_max_value(property);
        let value_width = if max <= 0xff {
            ValueWidth::Bits8
        } else if max <= 0xffff {
            ValueWidth::Bits16
        } else {
            ValueWidth::Bits32
        };
        mutable_trie.build_immutable(Some(ty), Some(value_width))
    }

    /// `getBinaryPropertySet(int property)`: a frozen set for a binary
    /// property.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("<p> is not a constant for a UProperty
    /// binary property")`; the errors of the property data.
    pub fn get_binary_property_set(property: i32) -> Result<Arc<UnicodeSet>> {
        if property < 0 || UProperty::BINARY_LIMIT <= property {
            return Err(LuceneError::illegal_argument(format!(
                "{property} is not a constant for a UProperty binary property"
            )));
        }
        let mut sets = SETS.lock().unwrap_or_else(|e| e.into_inner());
        if sets.is_empty() {
            sets.resize(UProperty::BINARY_LIMIT as usize, None);
        }
        if let Some(set) = &sets[property as usize] {
            return Ok(set.clone());
        }
        let set = Arc::new(Self::make_set(property)?);
        sets[property as usize] = Some(set.clone());
        Ok(set)
    }

    /// `getIntPropertyMap(int property)`: an immutable code point map for
    /// an enumerated/catalog/int-valued property.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("<p> is not a constant for a UProperty int
    /// property")`; the errors of the property data.
    pub fn get_int_property_map(property: i32) -> Result<Arc<CodePointTrie>> {
        if !(UProperty::INT_START..UProperty::INT_LIMIT).contains(&property) {
            return Err(LuceneError::illegal_argument(format!(
                "{property} is not a constant for a UProperty int property"
            )));
        }
        let mut maps = MAPS.lock().unwrap_or_else(|e| e.into_inner());
        if maps.is_empty() {
            maps.resize((UProperty::INT_LIMIT - UProperty::INT_START) as usize, None);
        }
        let i = (property - UProperty::INT_START) as usize;
        if let Some(map) = &maps[i] {
            return Ok(map.clone());
        }
        let map = Arc::new(Self::make_map(property)?);
        maps[i] = Some(map.clone());
        Ok(map)
    }
}
