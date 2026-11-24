// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! Ported from `com.ibm.icu.impl.IllegalIcuArgumentException`.

use crate::util::icu_exception::icu_exception_class;

icu_exception_class!(
    /// An `IllegalArgumentException` whose cause can be set.
    ///
    /// Equivalent to `com.ibm.icu.impl.IllegalIcuArgumentException`, which
    /// extends `IllegalArgumentException`. Java's constructors are
    /// `(String)`, `(Throwable)` and `(String, Throwable)`; the no-argument
    /// one of this declaration has no Java counterpart in use. Its
    /// `initCause` override only narrows the return type: the cause is
    /// given at construction here.
    IllegalIcuArgumentException,
    "com.ibm.icu.impl.IllegalIcuArgumentException",
    illegal_argument = true
);
