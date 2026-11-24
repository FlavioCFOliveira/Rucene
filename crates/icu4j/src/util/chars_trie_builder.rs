// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.CharsTrieBuilder`.

use std::sync::Arc;

use rucene::error::Result;

use crate::util::chars_trie::{
    CharsTrie, K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH, K_MAX_LINEAR_MATCH_LENGTH,
    K_MAX_ONE_UNIT_DELTA, K_MAX_ONE_UNIT_NODE_VALUE, K_MAX_ONE_UNIT_VALUE, K_MAX_TWO_UNIT_DELTA,
    K_MAX_TWO_UNIT_NODE_VALUE, K_MAX_TWO_UNIT_VALUE, K_MIN_LINEAR_MATCH, K_MIN_TWO_UNIT_DELTA_LEAD,
    K_MIN_TWO_UNIT_NODE_VALUE_LEAD, K_MIN_TWO_UNIT_VALUE_LEAD, K_THREE_UNIT_DELTA_LEAD,
    K_THREE_UNIT_NODE_VALUE_LEAD, K_THREE_UNIT_VALUE_LEAD, K_VALUE_IS_FINAL,
};
use crate::util::string_trie_builder::{
    StringTrieBuilder, StringTrieBuilderOption, StringTrieBuilderSpi,
};

/// Builder class for `CharsTrie`: collects (string, value) pairs and
/// serializes them into a `CharsTrie`.
///
/// Equivalent to `com.ibm.icu.util.CharsTrieBuilder`, which extends
/// `StringTrieBuilder` (the [`StringTrieBuilder`] it holds, with the
/// abstract methods implemented through [`StringTrieBuilderSpi`]).
#[derive(Debug, Default)]
pub struct CharsTrieBuilder {
    base: StringTrieBuilder,
    /// `chars`: the serialization, written backwards from the end; `None`
    /// before the first build and after `clear()`.
    chars: Option<Vec<u16>>,
    chars_length: i32,
}

impl CharsTrieBuilder {
    /// `CharsTrieBuilder()`: an empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// `add(CharSequence s, int value)`: adds a (string, value) pair.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` after a build; `IllegalArgumentException`
    /// for a duplicate string.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` for a string longer than 0xffff units.
    pub fn add(&mut self, s: &[u16], value: i32) -> Result<&mut Self> {
        StringTrieBuilder::add_impl(self, s, value)?;
        Ok(self)
    }

    /// `build(StringTrieBuilder.Option buildOption)`: a new `CharsTrie` over
    /// the serialization.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when a previous build failed.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when no pairs were added.
    pub fn build(&mut self, build_option: StringTrieBuilderOption) -> Result<CharsTrie> {
        let chars: Arc<[u16]> = Arc::from(self.build_char_sequence(build_option)?);
        Ok(CharsTrie::new(chars, 0))
    }

    /// `buildCharSequence(StringTrieBuilder.Option buildOption)`: the
    /// serialization (Java wraps the builder's array in a `CharBuffer`; the
    /// units are the same).
    ///
    /// # Errors
    ///
    /// As [`build`](Self::build).
    pub fn build_char_sequence(
        &mut self,
        build_option: StringTrieBuilderOption,
    ) -> Result<Vec<u16>> {
        self.build_chars(build_option)?;
        let chars = self.chars.as_ref().expect("buildChars allocates the array");
        Ok(chars[chars.len() - self.chars_length as usize..].to_vec())
    }

    /// `buildChars(StringTrieBuilder.Option buildOption)` (private).
    fn build_chars(&mut self, build_option: StringTrieBuilderOption) -> Result<()> {
        // Create and char-serialize the trie for the elements.
        if self.chars.is_none() {
            self.chars = Some(vec![0u16; 1024]);
        }
        StringTrieBuilder::build_impl(self, build_option)
    }

    /// `clear()`: removes all pairs, so that the builder can be reused.
    pub fn clear(&mut self) -> &mut Self {
        StringTrieBuilder::clear_impl(self);
        self.chars = None;
        self.chars_length = 0;
        self
    }

    /// `ensureCapacity(int length)` (private).
    fn ensure_capacity(&mut self, length: i32) {
        let chars = self.chars.as_mut().expect("buildChars allocates the array");
        if length > chars.len() as i32 {
            let mut new_capacity = chars.len() as i32;
            loop {
                new_capacity *= 2;
                if new_capacity > length {
                    break;
                }
            }
            let mut new_chars = vec![0u16; new_capacity as usize];
            let cl = self.chars_length as usize;
            let (old_len, new_len) = (chars.len(), new_chars.len());
            new_chars[new_len - cl..].copy_from_slice(&chars[old_len - cl..]);
            *chars = new_chars;
        }
    }

    /// `write(char[] s, int length)` (private).
    fn write_chars(&mut self, s: &[u16], length: i32) -> i32 {
        let new_length = self.chars_length + length;
        self.ensure_capacity(new_length);
        self.chars_length = new_length;
        let chars = self.chars.as_mut().expect("buildChars allocates the array");
        let start = chars.len() - self.chars_length as usize;
        chars[start..start + length as usize].copy_from_slice(&s[..length as usize]);
        self.chars_length
    }
}

impl StringTrieBuilderSpi for CharsTrieBuilder {
    fn base(&self) -> &StringTrieBuilder {
        &self.base
    }

    fn base_mut(&mut self) -> &mut StringTrieBuilder {
        &mut self.base
    }

    fn match_nodes_can_have_values(&self) -> bool {
        true
    }

    fn get_max_branch_linear_sub_node_length(&self) -> i32 {
        K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH
    }

    fn get_min_linear_match(&self) -> i32 {
        K_MIN_LINEAR_MATCH
    }

    fn get_max_linear_match_length(&self) -> i32 {
        K_MAX_LINEAR_MATCH_LENGTH
    }

    fn write_unit(&mut self, unit: i32) -> i32 {
        let new_length = self.chars_length + 1;
        self.ensure_capacity(new_length);
        self.chars_length = new_length;
        let chars = self.chars.as_mut().expect("buildChars allocates the array");
        let i = chars.len() - self.chars_length as usize;
        chars[i] = unit as u16;
        self.chars_length
    }

    fn write_offset_length(&mut self, mut offset: i32, mut length: i32) -> i32 {
        let new_length = self.chars_length + length;
        self.ensure_capacity(new_length);
        self.chars_length = new_length;
        let chars = self.chars.as_mut().expect("buildChars allocates the array");
        let mut chars_offset = chars.len() - self.chars_length as usize;
        while length > 0 {
            chars[chars_offset] = self.base.strings[offset as usize];
            chars_offset += 1;
            offset += 1;
            length -= 1;
        }
        self.chars_length
    }

    #[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
    fn write_value_and_final(&mut self, i: i32, is_final: bool) -> i32 {
        if (0..=K_MAX_ONE_UNIT_VALUE).contains(&i) {
            return self.write_unit(i | if is_final { K_VALUE_IS_FINAL } else { 0 });
        }
        let mut int_units = [0u16; 3];
        let length;
        if !(0..=K_MAX_TWO_UNIT_VALUE).contains(&i) {
            int_units[0] = K_THREE_UNIT_VALUE_LEAD as u16;
            int_units[1] = (i >> 16) as u16;
            int_units[2] = i as u16;
            length = 3;
        // } else if(i<=CharsTrie.kMaxOneUnitValue) {
        //     intUnits[0]=(char)(i);
        //     length=1;
        } else {
            int_units[0] = (K_MIN_TWO_UNIT_VALUE_LEAD + (i >> 16)) as u16;
            int_units[1] = i as u16;
            length = 2;
        }
        int_units[0] |= if is_final { K_VALUE_IS_FINAL as u16 } else { 0 };
        self.write_chars(&int_units, length)
    }

    #[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
    fn write_value_and_type(&mut self, has_value: bool, value: i32, node: i32) -> i32 {
        if !has_value {
            return self.write_unit(node);
        }
        let mut int_units = [0u16; 3];
        let length;
        if !(0..=K_MAX_TWO_UNIT_NODE_VALUE).contains(&value) {
            int_units[0] = K_THREE_UNIT_NODE_VALUE_LEAD as u16;
            int_units[1] = (value >> 16) as u16;
            int_units[2] = value as u16;
            length = 3;
        } else if value <= K_MAX_ONE_UNIT_NODE_VALUE {
            int_units[0] = ((value + 1) << 6) as u16;
            length = 1;
        } else {
            int_units[0] = (K_MIN_TWO_UNIT_NODE_VALUE_LEAD + ((value >> 10) & 0x7fc0)) as u16;
            int_units[1] = value as u16;
            length = 2;
        }
        int_units[0] |= node as u16;
        self.write_chars(&int_units, length)
    }

    fn write_delta_to(&mut self, jump_target: i32) -> i32 {
        let i = self.chars_length - jump_target;
        debug_assert!(i >= 0);
        if i <= K_MAX_ONE_UNIT_DELTA {
            return self.write_unit(i);
        }
        let mut int_units = [0u16; 3];
        let mut length;
        if i <= K_MAX_TWO_UNIT_DELTA {
            int_units[0] = (K_MIN_TWO_UNIT_DELTA_LEAD + (i >> 16)) as u16;
            length = 1;
        } else {
            int_units[0] = K_THREE_UNIT_DELTA_LEAD as u16;
            int_units[1] = (i >> 16) as u16;
            length = 2;
        }
        int_units[length] = i as u16;
        length += 1;
        self.write_chars(&int_units, length as i32)
    }
}
