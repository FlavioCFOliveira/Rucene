// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.ICUInputTooLongException`.

use crate::util::icu_exception::icu_exception_class;

icu_exception_class!(
    /// The input is too long.
    ///
    /// Equivalent to `com.ibm.icu.util.ICUInputTooLongException` (a
    /// subclass of `ICUException`).
    ICUInputTooLongException,
    "com.ibm.icu.util.ICUInputTooLongException",
    illegal_argument = false
);
