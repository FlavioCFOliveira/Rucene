// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UPropertyAliases`.
//!
//! Ported whole by rmp task #644 ahead of its owner (#645, block 3):
//! `LSR` (block 2) reads script names through `UScript.getShortName` /
//! `getCodeFromName`, which read this class's data.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};

use crate::jdk::ByteBuffer;
use crate::lang::u_property::UProperty;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::illegal_icu_argument_exception::IllegalIcuArgumentException;
use crate::util::bytes_trie::{BytesTrie, Result as TrieResult};
use crate::util::u_resource_bundle::missing_resource;

// Byte offsets from the start of the data, after the generic header.
const IX_VALUE_MAPS_OFFSET: usize = 0;
const IX_BYTE_TRIES_OFFSET: usize = 1;
const IX_NAME_GROUPS_OFFSET: usize = 2;
const IX_RESERVED3_OFFSET: usize = 3;

/// `DATA_FORMAT`: "pnam".
const DATA_FORMAT: i32 = 0x706E616D;

fn is_acceptable(version: &[u8; 4]) -> bool {
    version[0] == 2
}

/// Wrapper for the `pnames.icu` binary data file: the property and
/// property-value names and their enums.
///
/// Equivalent to `com.ibm.icu.impl.UPropertyAliases` (formatVersion 2).
#[derive(Debug)]
pub struct UPropertyAliases {
    value_maps: Vec<i32>,
    bytes_tries: Arc<[i8]>,
    name_groups: Vec<u16>,
}

/// `INSTANCE`, or the `MissingResourceException` its static initializer
/// throws.
static INSTANCE: LazyLock<std::result::Result<Arc<UPropertyAliases>, String>> =
    LazyLock::new(|| {
        UPropertyAliases::new()
            .map(Arc::new)
            .map_err(|e| e.to_string())
    });

impl UPropertyAliases {
    #[allow(clippy::needless_range_loop)] // indexed loop mirrors the Java for-loop
    fn load(bytes: &mut ByteBuffer) -> Result<Self> {
        //dataVersion=ICUBinary.readHeaderAndDataVersion(bytes, DATA_FORMAT, IS_ACCEPTABLE);
        ICUBinary::read_header(
            bytes,
            DATA_FORMAT,
            Some(&is_acceptable as &dyn Fn(&[u8; 4]) -> bool),
        )?;
        let indexes_length = bytes.get_int()? / 4; // inIndexes[IX_VALUE_MAPS_OFFSET]/4
        if indexes_length < 8 {
            // formatVersion 2 initially has 8 indexes
            return Err(LuceneError::from(std::io::Error::other(
                "pnames.icu: not enough indexes",
            )));
        }
        let mut in_indexes = vec![0i32; indexes_length as usize];
        in_indexes[0] = indexes_length * 4;
        for i in 1..indexes_length as usize {
            in_indexes[i] = bytes.get_int()?;
        }

        // Read the valueMaps.
        let mut offset = in_indexes[IX_VALUE_MAPS_OFFSET];
        let mut next_offset = in_indexes[IX_BYTE_TRIES_OFFSET];
        let num_ints = (next_offset - offset) / 4;
        let value_maps = ICUBinary::get_ints(bytes, num_ints, 0)?;

        // Read the bytesTries.
        offset = next_offset;
        next_offset = in_indexes[IX_NAME_GROUPS_OFFSET];
        let num_bytes = next_offset - offset;
        let mut bytes_tries = vec![0u8; num_bytes as usize];
        bytes.get_bytes(&mut bytes_tries)?;
        let bytes_tries: Vec<i8> = bytes_tries.into_iter().map(|b| b as i8).collect();

        // Read the nameGroups and turn them from ASCII bytes into a Java String.
        offset = next_offset;
        next_offset = in_indexes[IX_RESERVED3_OFFSET];
        let num_bytes = next_offset - offset;
        let mut sb = Vec::with_capacity(num_bytes as usize);
        for _ in 0..num_bytes {
            sb.push(bytes.get()? as i16 as u16);
        }
        Ok(Self {
            value_maps,
            bytes_tries: Arc::from(bytes_tries),
            name_groups: sb,
        })
    }

    fn new() -> Result<Self> {
        let mut bytes = ICUBinary::get_required_data("pnames.icu")?;
        Self::load(&mut bytes)
    }

    /// `INSTANCE`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException("Could not construct UPropertyAliases.
    /// Missing pnames.icu")` when the data cannot be read.
    pub fn instance() -> Result<Arc<UPropertyAliases>> {
        match &*INSTANCE {
            Ok(i) => Ok(i.clone()),
            Err(_) => Err(missing_resource(
                "Could not construct UPropertyAliases. Missing pnames.icu",
                "",
                "",
            )),
        }
    }

    fn find_property(&self, property: i32) -> usize {
        let vm = &self.value_maps;
        let mut i = 1usize; // valueMaps index, initially after numRanges
        let mut num_ranges = vm[0];
        while num_ranges > 0 {
            // Read and skip the start and limit of this range.
            let start = vm[i];
            let limit = vm[i + 1];
            i += 2;
            if property < start {
                break;
            }
            if property < limit {
                return i + ((property - start) * 2) as usize;
            }
            i += ((limit - start) * 2) as usize; // Skip all entries for this range.
            num_ranges -= 1;
        }
        0
    }

    fn find_property_value_name_group(&self, value_map_index: i32, value: i32) -> i32 {
        if value_map_index == 0 {
            return 0; // The property does not have named values.
        }
        let vm = &self.value_maps;
        let mut value_map_index = value_map_index as usize + 1; // Skip the BytesTrie offset.
        let mut num_ranges = vm[value_map_index];
        value_map_index += 1;
        if num_ranges < 0x10 {
            // Ranges of values.
            while num_ranges > 0 {
                // Read and skip the start and limit of this range.
                let start = vm[value_map_index];
                let limit = vm[value_map_index + 1];
                value_map_index += 2;
                if value < start {
                    break;
                }
                if value < limit {
                    return vm[value_map_index + (value - start) as usize];
                }
                value_map_index += (limit - start) as usize; // Skip all entries for this range.
                num_ranges -= 1;
            }
        } else {
            // List of values.
            let values_start = value_map_index;
            let name_group_offsets_start = value_map_index + (num_ranges - 0x10) as usize;
            loop {
                let v = vm[value_map_index];
                if value < v {
                    break;
                }
                if value == v {
                    return vm[name_group_offsets_start + value_map_index - values_start];
                }
                value_map_index += 1;
                if value_map_index >= name_group_offsets_start {
                    break;
                }
            }
        }
        0
    }

    fn get_name(&self, name_groups_index: i32, mut name_index: i32) -> Result<Option<String>> {
        let ng = &self.name_groups;
        let mut i = name_groups_index as usize;
        let num_names = ng[i] as i32;
        i += 1;
        if name_index < 0 || num_names <= name_index {
            return Err(IllegalIcuArgumentException::with_message(
                "Invalid property (value) name choice",
            )
            .error());
        }
        // Skip nameIndex names.
        while name_index > 0 {
            loop {
                let c = ng[i];
                i += 1;
                if c == 0 {
                    break;
                }
            }
            name_index -= 1;
        }
        // Find the end of this name.
        let name_start = i;
        while ng[i] != 0 {
            i += 1;
        }
        if name_start == i {
            return Ok(None); // no name (Property[Value]Aliases.txt has "n/a")
        }
        Ok(Some(String::from_utf16_lossy(&ng[name_start..i])))
    }

    fn ascii_to_lowercase(c: i32) -> i32 {
        if (b'A' as i32..=b'Z' as i32).contains(&c) {
            c + 0x20
        } else {
            c
        }
    }

    fn contains_name(trie: &mut BytesTrie, name: &[u16]) -> bool {
        let mut result = TrieResult::NoValue;
        for &u in name {
            let mut c = u as i32;
            // Ignore delimiters '-', '_', and ASCII White_Space.
            if c == b'-' as i32
                || c == b'_' as i32
                || c == b' ' as i32
                || (0x09..=0x0d).contains(&c)
            {
                continue;
            }
            if !result.has_next() {
                return false;
            }
            c = Self::ascii_to_lowercase(c);
            result = trie.next(c);
        }
        result.has_value()
    }

    fn invalid_property(property: i32) -> LuceneError {
        LuceneError::illegal_argument(format!("Invalid property enum {property} (0x{property:x})"))
    }

    fn no_named_values(property: i32) -> LuceneError {
        LuceneError::illegal_argument(format!(
            "Property {property} (0x{property:x}) does not have named values"
        ))
    }

    /// `getPropertyName(int property, int nameChoice)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid property or name choice.
    pub fn get_property_name(&self, property: i32, name_choice: i32) -> Result<Option<String>> {
        let value_map_index = self.find_property(property);
        if value_map_index == 0 {
            return Err(Self::invalid_property(property));
        }
        self.get_name(self.value_maps[value_map_index], name_choice)
    }

    /// `getPropertyValueName(int property, int value, int nameChoice)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid property, a property
    /// without named values, or an invalid name choice.
    pub fn get_property_value_name(
        &self,
        property: i32,
        value: i32,
        name_choice: i32,
    ) -> Result<Option<String>> {
        let value_map_index = self.find_property(property);
        if value_map_index == 0 {
            return Err(Self::invalid_property(property));
        }
        let name_group_offset =
            self.find_property_value_name_group(self.value_maps[value_map_index + 1], value);
        if name_group_offset == 0 {
            return Err(Self::no_named_values(property));
        }
        self.get_name(name_group_offset, name_choice)
    }

    fn get_property_or_value_enum(&self, bytes_trie_offset: i32, alias: &[u16]) -> i32 {
        let mut trie = BytesTrie::new(self.bytes_tries.clone(), bytes_trie_offset);
        if Self::contains_name(&mut trie, alias) {
            trie.get_value()
        } else {
            UProperty::UNDEFINED
        }
    }

    /// `getPropertyEnum(CharSequence alias)`.
    pub fn get_property_enum(&self, alias: &[u16]) -> i32 {
        self.get_property_or_value_enum(0, alias)
    }

    /// `getPropertyValueEnum(int property, CharSequence alias)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid property or one without
    /// named values.
    pub fn get_property_value_enum(&self, property: i32, alias: &[u16]) -> Result<i32> {
        let value_map_index = self.find_property(property);
        if value_map_index == 0 {
            return Err(Self::invalid_property(property));
        }
        let value_map_index = self.value_maps[value_map_index + 1];
        if value_map_index == 0 {
            return Err(Self::no_named_values(property));
        }
        // valueMapIndex is the start of the property's valueMap,
        // where the first word is the BytesTrie offset.
        Ok(self.get_property_or_value_enum(self.value_maps[value_map_index as usize], alias))
    }

    /// `getPropertyValueEnumNoThrow(int property, CharSequence alias)`:
    /// `UProperty.UNDEFINED` where `getPropertyValueEnum` throws.
    pub fn get_property_value_enum_no_throw(&self, property: i32, alias: &[u16]) -> i32 {
        let value_map_index = self.find_property(property);
        if value_map_index == 0 {
            return UProperty::UNDEFINED;
        }
        let value_map_index = self.value_maps[value_map_index + 1];
        if value_map_index == 0 {
            return UProperty::UNDEFINED;
        }
        // valueMapIndex is the start of the property's valueMap,
        // where the first word is the BytesTrie offset.
        self.get_property_or_value_enum(self.value_maps[value_map_index as usize], alias)
    }

    /// `compare(String stra, String strb)`: compares property names
    /// ignoring delimiters and ASCII case (`uprv_comparePropertyNames`).
    pub fn compare(stra: &[u16], strb: &[u16]) -> i32 {
        // Note: This implementation is a literal copy of
        // uprv_comparePropertyNames.  It can probably be improved.
        let mut istra = 0usize;
        let mut istrb = 0usize;
        let mut cstra: i32 = 0;
        let mut cstrb: i32 = 0;
        let is_delim = |c: i32| matches!(c, 0x2d | 0x5f | 0x20 | 0x09 | 0x0a | 0x0b | 0x0c | 0x0d);
        loop {
            /* Ignore delimiters '-', '_', and ASCII White_Space */
            while istra < stra.len() {
                cstra = stra[istra] as i32;
                if is_delim(cstra) {
                    istra += 1;
                    continue;
                }
                break;
            }

            while istrb < strb.len() {
                cstrb = strb[istrb] as i32;
                if is_delim(cstrb) {
                    istrb += 1;
                    continue;
                }
                break;
            }

            /* If we reach the ends of both strings then they match */
            let endstra = istra == stra.len();
            let endstrb = istrb == strb.len();
            if endstra {
                if endstrb {
                    return 0;
                }
                cstra = 0;
            } else if endstrb {
                cstrb = 0;
            }

            let rc = Self::ascii_to_lowercase(cstra) - Self::ascii_to_lowercase(cstrb);
            if rc != 0 {
                return rc;
            }

            istra += 1;
            istrb += 1;
        }
    }
}
