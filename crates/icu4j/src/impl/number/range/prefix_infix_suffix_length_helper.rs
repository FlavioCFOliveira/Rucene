// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.range.PrefixInfixSuffixLengthHelper`.

/// The lengths of the parts of a formatted range (prefix, first number,
/// infix, second number, suffix) and the indexes they imply.
///
/// Equivalent to
/// `com.ibm.icu.impl.number.range.PrefixInfixSuffixLengthHelper`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrefixInfixSuffixLengthHelper {
    /// `lengthPrefix`.
    pub length_prefix: i32,
    /// `length1`.
    pub length1: i32,
    /// `lengthInfix`.
    pub length_infix: i32,
    /// `length2`.
    pub length2: i32,
    /// `lengthSuffix`.
    pub length_suffix: i32,
}

impl PrefixInfixSuffixLengthHelper {
    /// `index0()`.
    pub fn index0(&self) -> i32 {
        self.length_prefix
    }

    /// `index1()`.
    pub fn index1(&self) -> i32 {
        self.length_prefix + self.length1
    }

    /// `index2()`.
    pub fn index2(&self) -> i32 {
        self.length_prefix + self.length1 + self.length_infix
    }

    /// `index3()`.
    pub fn index3(&self) -> i32 {
        self.length_prefix + self.length1 + self.length_infix + self.length2
    }

    /// `index4()`.
    pub fn index4(&self) -> i32 {
        self.length_prefix + self.length1 + self.length_infix + self.length2 + self.length_suffix
    }
}
