// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.Padder`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::jdk::units;
use crate::r#impl::formatted_string_builder::FormattedStringBuilder;
use crate::r#impl::number::decimal_format_properties::DecimalFormatProperties;
use crate::r#impl::number::modifier::Modifier;
use crate::text::decimal_format::DecimalFormat;

/// `FALLBACK_PADDING_STRING`.
pub const FALLBACK_PADDING_STRING: &str = "\u{0020}"; // i.e. a space

/// Equivalent to `Padder.PadPosition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadPosition {
    /// `BEFORE_PREFIX`.
    BeforePrefix,
    /// `AFTER_PREFIX`.
    AfterPrefix,
    /// `BEFORE_SUFFIX`.
    BeforeSuffix,
    /// `AFTER_SUFFIX`.
    AfterSuffix,
}

impl PadPosition {
    /// Equivalent to `fromOld(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Don't know how to map <old>")`.
    pub fn from_old(old: i32) -> Result<PadPosition> {
        match old {
            DecimalFormat::PAD_BEFORE_PREFIX => Ok(PadPosition::BeforePrefix),
            DecimalFormat::PAD_AFTER_PREFIX => Ok(PadPosition::AfterPrefix),
            DecimalFormat::PAD_BEFORE_SUFFIX => Ok(PadPosition::BeforeSuffix),
            DecimalFormat::PAD_AFTER_SUFFIX => Ok(PadPosition::AfterSuffix),
            _ => Err(LuceneError::illegal_argument(format!("Don't know how to map {old}"))),
        }
    }

    /// Equivalent to `toOld()`.
    pub fn to_old(self) -> i32 {
        match self {
            PadPosition::BeforePrefix => DecimalFormat::PAD_BEFORE_PREFIX,
            PadPosition::AfterPrefix => DecimalFormat::PAD_AFTER_PREFIX,
            PadPosition::BeforeSuffix => DecimalFormat::PAD_BEFORE_SUFFIX,
            PadPosition::AfterSuffix => DecimalFormat::PAD_AFTER_SUFFIX,
        }
    }
}

/// The fields of a `Padder` object.
#[derive(Debug)]
struct PadderData {
    /// `paddingString`.
    padding_string: Vec<u16>,
    /// `targetWidth`.
    target_width: i32,
    /// `position`.
    position: PadPosition,
}

/// Equivalent to `com.ibm.icu.impl.number.Padder`. Java's objects are
/// immutable after construction and compared by identity; this handle
/// shares one object ([`ptr_eq`](Self::ptr_eq) is Java's `==` and
/// `equals`).
#[derive(Debug, Clone)]
pub struct Padder(Arc<PadderData>);

/// `NONE`.
pub static NONE: LazyLock<Padder> = LazyLock::new(|| Padder::new(None, -1, None));

impl Padder {
    /// Equivalent to `Padder(String, int, PadPosition)` (`None` for Java's
    /// `null`).
    pub fn new(padding_string: Option<&[u16]>, target_width: i32, position: Option<PadPosition>) -> Self {
        Padder(Arc::new(PadderData {
            padding_string: padding_string.map_or_else(|| units(FALLBACK_PADDING_STRING), <[u16]>::to_vec),
            target_width,
            position: position.unwrap_or(PadPosition::BeforePrefix),
        }))
    }

    /// Whether two handles are the same Java object.
    pub fn ptr_eq(&self, other: &Padder) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// `Object.hashCode()` (identity): see
    /// `java_identity_hash_code_stand_in`.
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.impl.number.Padder")
    }

    /// Equivalent to `none()`.
    pub fn none() -> Padder {
        NONE.clone()
    }

    /// Equivalent to `codePoints(int, int, PadPosition)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Padding width must not be negative")`,
    /// and the `IllegalArgumentException` of `Character.toChars` for an
    /// invalid code point.
    pub fn code_points(cp: i32, target_width: i32, position: Option<PadPosition>) -> Result<Padder> {
        // TODO: Validate the code point
        if target_width >= 0 {
            let mut padding_string = Vec::new();
            crate::jdk::append_code_point(&mut padding_string, cp)?;
            Ok(Padder::new(Some(&padding_string), target_width, position))
        } else {
            Err(LuceneError::illegal_argument("Padding width must not be negative"))
        }
    }

    /// Equivalent to `forProperties(DecimalFormatProperties)`.
    pub fn for_properties(properties: &DecimalFormatProperties) -> Padder {
        Padder::new(properties.get_pad_string(), properties.get_format_width(), properties.get_pad_position())
    }

    /// Equivalent to `isValid()`.
    pub fn is_valid(&self) -> bool {
        self.0.target_width > 0
    }

    /// Equivalent to `padAndApply(Modifier, Modifier, FormattedStringBuilder,
    /// int, int)`.
    ///
    /// # Errors
    ///
    /// The errors of the modifiers.
    pub fn pad_and_apply(
        &self,
        mod1: &dyn Modifier,
        mod2: &dyn Modifier,
        string: &mut FormattedStringBuilder,
        left_index: i32,
        right_index: i32,
    ) -> Result<i32> {
        let mod_length = mod1.get_code_point_count()? + mod2.get_code_point_count()?;
        let required_padding = self.0.target_width - mod_length - string.code_point_count();
        debug_assert!(left_index == 0 && right_index == string.length());

        let mut length = 0;
        if required_padding <= 0 {
            // Padding is not required.
            length += mod1.apply(string, left_index, right_index)?;
            length += mod2.apply(string, left_index, right_index + length)?;
            return Ok(length);
        }

        let position = self.0.position;
        let padding_string = &self.0.padding_string;
        if position == PadPosition::AfterPrefix {
            length += add_padding_helper(padding_string, required_padding, string, left_index);
        } else if position == PadPosition::BeforeSuffix {
            length += add_padding_helper(padding_string, required_padding, string, right_index + length);
        }
        length += mod1.apply(string, left_index, right_index + length)?;
        length += mod2.apply(string, left_index, right_index + length)?;
        if position == PadPosition::BeforePrefix {
            length += add_padding_helper(padding_string, required_padding, string, left_index);
        } else if position == PadPosition::AfterSuffix {
            length += add_padding_helper(padding_string, required_padding, string, right_index + length);
        }

        Ok(length)
    }
}

/// `addPaddingHelper(String, int, FormattedStringBuilder, int)`.
fn add_padding_helper(padding_string: &[u16], required_padding: i32, string: &mut FormattedStringBuilder, index: i32) -> i32 {
    for _ in 0..required_padding {
        // TODO: If appending to the end, this will cause actual insertion
        // operations. Improve.
        string.insert(index, padding_string, None);
    }
    padding_string.len() as i32 * required_padding
}
