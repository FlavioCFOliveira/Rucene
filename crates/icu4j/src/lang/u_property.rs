// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.UProperty`.
//!
//! Only the constants block 2 of the port (rmp task #644) uses are here,
//! ported early; the rest of the interface is ported by rmp task #645.

/// Selection constants for Unicode properties.
///
/// Equivalent to the interface `com.ibm.icu.lang.UProperty` (constants
/// ported so far).
pub struct UProperty;

impl UProperty {
    /// `UNDEFINED`.
    pub const UNDEFINED: i32 = -1;
    /// `CANONICAL_COMBINING_CLASS`.
    pub const CANONICAL_COMBINING_CLASS: i32 = 0x1002;
    /// `SCRIPT`.
    pub const SCRIPT: i32 = 0x100A;
    /// `LEAD_CANONICAL_COMBINING_CLASS`.
    pub const LEAD_CANONICAL_COMBINING_CLASS: i32 = 0x1010;
    /// `TRAIL_CANONICAL_COMBINING_CLASS`.
    pub const TRAIL_CANONICAL_COMBINING_CLASS: i32 = 0x1011;
}

/// Selector constants for property names and property value names.
///
/// Equivalent to the interface `UProperty.NameChoice`.
pub struct NameChoice;

impl NameChoice {
    /// `SHORT`.
    pub const SHORT: i32 = 0;
    /// `LONG`.
    pub const LONG: i32 = 1;
    /// `COUNT` (deprecated).
    pub const COUNT: i32 = 2;
}
