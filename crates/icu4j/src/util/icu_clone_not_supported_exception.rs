// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.ICUCloneNotSupportedException`.

use crate::util::icu_exception::icu_exception_class;

icu_exception_class!(
    /// Unchecked version of `CloneNotSupportedException`.
    ///
    /// Equivalent to `com.ibm.icu.util.ICUCloneNotSupportedException`,
    /// which extends `ICUException`
    /// ([`ICUException::is_instance`](crate::util::icu_exception::ICUException::is_instance)
    /// answers `true` for it).
    ICUCloneNotSupportedException,
    "com.ibm.icu.util.ICUCloneNotSupportedException",
    illegal_argument = false
);
