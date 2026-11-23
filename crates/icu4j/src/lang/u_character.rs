// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.UCharacter`.
//!
//! Only the members block 1 of the port (rmp task #643) calls are here,
//! ported early; the rest of the class is ported by rmp task #645.

/// The ICU counterpart of `java.lang.Character`.
///
/// Equivalent to `com.ibm.icu.lang.UCharacter` (members ported so far:
/// [`to_code_point`](Self::to_code_point)).
pub struct UCharacter;

impl UCharacter {
    /// `toCodePoint(int high, int low)`: the code point of a surrogate pair,
    /// `(high << 10) + low - U16_SURROGATE_OFFSET` (`UCharacter.java:6499-6502`),
    /// in Java's wrapping `int` arithmetic.
    pub fn to_code_point(high: i32, low: i32) -> i32 {
        (high << 10)
            .wrapping_add(low)
            .wrapping_sub(Self::U16_SURROGATE_OFFSET)
    }

    /// `U16_SURROGATE_OFFSET` (private in Java):
    /// `(0xd800 << 10) + 0xdc00 - 0x10000`.
    const U16_SURROGATE_OFFSET: i32 = (0xd800 << 10) + 0xdc00 - 0x10000;
}
