// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.SkeletonSyntaxException`.

use rucene::error::LuceneError;

use crate::jdk::to_rust_string;

crate::util::icu_exception::icu_exception_class!(
    /// Exception used for illegal number skeleton strings.
    ///
    /// Equivalent to `com.ibm.icu.number.SkeletonSyntaxException`, which
    /// extends `IllegalArgumentException`; its two constructors are
    /// [`new_skeleton`] and [`new_skeleton_with_cause`], which compose
    /// Java's message.
    SkeletonSyntaxException,
    "com.ibm.icu.number.SkeletonSyntaxException",
    illegal_argument = true
);

/// `new SkeletonSyntaxException(String message, CharSequence token)`:
/// the message `Syntax error in skeleton string: <message>: <token>`.
pub fn new_skeleton(message: &str, token: &[u16]) -> LuceneError {
    SkeletonSyntaxException::with_message(format!("Syntax error in skeleton string: {}: {}", message, to_rust_string(token))).error()
}

/// `new SkeletonSyntaxException(String message, CharSequence token,
/// Throwable cause)`.
pub fn new_skeleton_with_cause(message: &str, token: &[u16], cause: LuceneError) -> LuceneError {
    SkeletonSyntaxException::with_message_and_cause(
        format!("Syntax error in skeleton string: {}: {}", message, to_rust_string(token)),
        cause,
    )
    .error()
}
