// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2012-2015, International Business Machines Corporation and    *
 * others. All Rights Reserved.                                                *
 *******************************************************************************
 */

//! `com.ibm.icu.text.DisplayContext`.

use std::fmt;

/// Display context settings. Note, the specific numeric values are
/// internal and may change.
///
/// Equivalent to `com.ibm.icu.text.DisplayContext`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisplayContext {
    /// `STANDARD_NAMES`: a possible setting for DIALECT_HANDLING: use
    /// standard names when generating a locale name, e.g. en_GB displays as
    /// 'English (United Kingdom)'.
    StandardNames,
    /// `DIALECT_NAMES`: use dialect names, when generating a locale name,
    /// e.g. en_GB displays as 'British English'.
    DialectNames,
    /// `CAPITALIZATION_NONE`: the capitalization context to be used is
    /// unknown (this is the default value).
    CapitalizationNone,
    /// `CAPITALIZATION_FOR_MIDDLE_OF_SENTENCE`: the capitalization context
    /// if a date, date symbol or display name is to be formatted with
    /// capitalization appropriate for the middle of a sentence.
    CapitalizationForMiddleOfSentence,
    /// `CAPITALIZATION_FOR_BEGINNING_OF_SENTENCE`: capitalization
    /// appropriate for the beginning of a sentence.
    CapitalizationForBeginningOfSentence,
    /// `CAPITALIZATION_FOR_UI_LIST_OR_MENU`: capitalization appropriate for
    /// a user-interface list or menu item.
    CapitalizationForUiListOrMenu,
    /// `CAPITALIZATION_FOR_STANDALONE`: capitalization appropriate for
    /// stand-alone usage such as an isolated name on a calendar page.
    CapitalizationForStandalone,
    /// `LENGTH_FULL`: use full names when generating a locale name, e.g.
    /// "United States" for US.
    LengthFull,
    /// `LENGTH_SHORT`: use short names when generating a locale name, e.g.
    /// "U.S." for US.
    LengthShort,
    /// `SUBSTITUTE`: return a fallback value when data is unavailable, e.g.
    /// "en_GB" for an unknown locale (this is the default value).
    Substitute,
    /// `NO_SUBSTITUTE`: return a null value when data is unavailable.
    NoSubstitute,
}

/// Type values for DisplayContext.
///
/// Equivalent to `DisplayContext.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    /// `DIALECT_HANDLING`: the type of dialect handling.
    DialectHandling,
    /// `CAPITALIZATION`: the type of capitalization context.
    Capitalization,
    /// `DISPLAY_LENGTH`: the type of display length.
    DisplayLength,
    /// `SUBSTITUTE_HANDLING`: the type of substitute handling.
    SubstituteHandling,
}

impl DisplayContext {
    /// `values()`.
    pub const VALUES: [DisplayContext; 11] = [
        DisplayContext::StandardNames,
        DisplayContext::DialectNames,
        DisplayContext::CapitalizationNone,
        DisplayContext::CapitalizationForMiddleOfSentence,
        DisplayContext::CapitalizationForBeginningOfSentence,
        DisplayContext::CapitalizationForUiListOrMenu,
        DisplayContext::CapitalizationForStandalone,
        DisplayContext::LengthFull,
        DisplayContext::LengthShort,
        DisplayContext::Substitute,
        DisplayContext::NoSubstitute,
    ];

    /// `type()`: the type part of the enum item.
    pub fn r#type(self) -> Type {
        match self {
            DisplayContext::StandardNames | DisplayContext::DialectNames => Type::DialectHandling,
            DisplayContext::CapitalizationNone
            | DisplayContext::CapitalizationForMiddleOfSentence
            | DisplayContext::CapitalizationForBeginningOfSentence
            | DisplayContext::CapitalizationForUiListOrMenu
            | DisplayContext::CapitalizationForStandalone => Type::Capitalization,
            DisplayContext::LengthFull | DisplayContext::LengthShort => Type::DisplayLength,
            DisplayContext::Substitute | DisplayContext::NoSubstitute => Type::SubstituteHandling,
        }
    }

    /// `value()`: the value part of the enum item.
    pub fn value(self) -> i32 {
        match self {
            DisplayContext::StandardNames => 0,
            DisplayContext::DialectNames => 1,
            DisplayContext::CapitalizationNone => 0,
            DisplayContext::CapitalizationForMiddleOfSentence => 1,
            DisplayContext::CapitalizationForBeginningOfSentence => 2,
            DisplayContext::CapitalizationForUiListOrMenu => 3,
            DisplayContext::CapitalizationForStandalone => 4,
            DisplayContext::LengthFull => 0,
            DisplayContext::LengthShort => 1,
            DisplayContext::Substitute => 0,
            DisplayContext::NoSubstitute => 1,
        }
    }

    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }
}

/// `toString()`: the constant name (`Enum.toString`).
impl fmt::Display for DisplayContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DisplayContext::StandardNames => "STANDARD_NAMES",
            DisplayContext::DialectNames => "DIALECT_NAMES",
            DisplayContext::CapitalizationNone => "CAPITALIZATION_NONE",
            DisplayContext::CapitalizationForMiddleOfSentence => "CAPITALIZATION_FOR_MIDDLE_OF_SENTENCE",
            DisplayContext::CapitalizationForBeginningOfSentence => "CAPITALIZATION_FOR_BEGINNING_OF_SENTENCE",
            DisplayContext::CapitalizationForUiListOrMenu => "CAPITALIZATION_FOR_UI_LIST_OR_MENU",
            DisplayContext::CapitalizationForStandalone => "CAPITALIZATION_FOR_STANDALONE",
            DisplayContext::LengthFull => "LENGTH_FULL",
            DisplayContext::LengthShort => "LENGTH_SHORT",
            DisplayContext::Substitute => "SUBSTITUTE",
            DisplayContext::NoSubstitute => "NO_SUBSTITUTE",
        })
    }
}

/// `toString()`: the constant name.
impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Type::DialectHandling => "DIALECT_HANDLING",
            Type::Capitalization => "CAPITALIZATION",
            Type::DisplayLength => "DISPLAY_LENGTH",
            Type::SubstituteHandling => "SUBSTITUTE_HANDLING",
        })
    }
}
