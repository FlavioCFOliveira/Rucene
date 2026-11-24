// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.ConstantAffixModifier`.

use rucene::error::Result;
use std::any::Any;
use std::sync::{Arc, LazyLock};

use crate::jdk::text::{same_field, FieldRef};
use crate::jdk::units;
use crate::lang::u_character::UCharacter;
use crate::r#impl::formatted_string_builder::{FieldObject, FormattedStringBuilder};
use crate::r#impl::number::modifier::{Modifier, Parameters};

/// `EMPTY`: a constant affix modifier with empty prefix and suffix.
pub static EMPTY: LazyLock<Arc<ConstantAffixModifier>> =
    LazyLock::new(|| Arc::new(ConstantAffixModifier::new()));

/// The canonical Modifier implementation, containing a prefix and suffix
/// string.
///
/// Equivalent to `com.ibm.icu.impl.number.ConstantAffixModifier`.
#[derive(Debug, Clone)]
pub struct ConstantAffixModifier {
    prefix: Vec<u16>,
    suffix: Vec<u16>,
    field: Option<FieldRef>,
    strong: bool,
}

impl Default for ConstantAffixModifier {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstantAffixModifier {
    /// Constructs an instance with the given strings. A `None` prefix or
    /// suffix is the empty string. Equivalent to
    /// `ConstantAffixModifier(String, String, Field, boolean)`.
    pub fn with_affixes(
        prefix: Option<&[u16]>,
        suffix: Option<&[u16]>,
        field: Option<FieldRef>,
        strong: bool,
    ) -> Self {
        // Use an empty string instead of null if we are given null
        // TODO: Consider returning a null modifier if both prefix and suffix
        // are empty.
        Self {
            prefix: prefix.map(|p| p.to_vec()).unwrap_or_default(),
            suffix: suffix.map(|s| s.to_vec()).unwrap_or_default(),
            field,
            strong,
        }
    }

    /// Constructs a new instance with an empty prefix, suffix, and field.
    /// Equivalent to `ConstantAffixModifier()`.
    pub fn new() -> Self {
        Self {
            prefix: Vec::new(),
            suffix: Vec::new(),
            field: None,
            strong: false,
        }
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> Vec<u16> {
        let mut s = units("<ConstantAffixModifier prefix:'");
        s.extend_from_slice(&self.prefix);
        s.extend_from_slice(&units("' suffix:'"));
        s.extend_from_slice(&self.suffix);
        s.extend_from_slice(&units("'>"));
        s
    }
}

impl Modifier for ConstantAffixModifier {
    fn apply(
        &self,
        output: &mut FormattedStringBuilder,
        left_index: i32,
        right_index: i32,
    ) -> Result<i32> {
        // Insert the suffix first since inserting the prefix will change the
        // rightIndex
        let field = self.field.map(FieldObject::Field);
        let mut length = output.insert(right_index, &self.suffix, field.clone());
        length += output.insert(left_index, &self.prefix, field);
        Ok(length)
    }

    fn get_prefix_length(&self) -> Result<i32> {
        Ok(self.prefix.len() as i32)
    }

    fn get_code_point_count(&self) -> Result<i32> {
        Ok(
            UCharacter::code_point_count(&self.prefix, 0, self.prefix.len() as i32)
                + UCharacter::code_point_count(&self.suffix, 0, self.suffix.len() as i32),
        )
    }

    fn is_strong(&self) -> bool {
        self.strong
    }

    fn contains_field(&self, _field: FieldRef) -> bool {
        // This method is not currently used.
        debug_assert!(false);
        false
    }

    fn get_parameters(&self) -> Option<&Parameters> {
        None
    }

    fn strict_equals(&self, other: &dyn Modifier) -> bool {
        let other = match other.as_any().downcast_ref::<ConstantAffixModifier>() {
            Some(o) => o,
            None => return false,
        };
        self.prefix == other.prefix
            && self.suffix == other.suffix
            && same_field(self.field, other.field)
            && self.strong == other.strong
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
