// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! Ported from `com.ibm.icu.util.UResourceTypeMismatchException`.

use rucene::error::LuceneError;

crate::util::icu_exception::icu_exception_class!(
    /// Exception thrown when the requested resource type is not the same
    /// type as the available resource.
    ///
    /// Equivalent to `com.ibm.icu.util.UResourceTypeMismatchException`
    /// (`UResourceTypeMismatchException.java`), which extends
    /// `RuntimeException`; Java has only the `(String msg)` constructor,
    /// [`with_message`](Self::with_message).
    UResourceTypeMismatchException,
    "com.ibm.icu.util.UResourceTypeMismatchException",
    illegal_argument = false
);

/// `new UResourceTypeMismatchException(msg)` as the error it is thrown as.
pub fn type_mismatch(msg: impl Into<String>) -> LuceneError {
    UResourceTypeMismatchException::with_message(msg).error()
}
