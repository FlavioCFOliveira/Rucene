// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! Ported from `com.ibm.icu.util.ICUUncheckedIOException`.

use crate::util::icu_exception::icu_exception_class;

icu_exception_class!(
    /// Unchecked version of `IOException`.
    ///
    /// Equivalent to `com.ibm.icu.util.ICUUncheckedIOException`, which
    /// extends `RuntimeException`.
    ICUUncheckedIOException,
    "com.ibm.icu.util.ICUUncheckedIOException",
    illegal_argument = false
);
