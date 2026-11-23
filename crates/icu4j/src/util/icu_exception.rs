// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.ICUException`.

use std::sync::Arc;

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

/// Declares an ICU exception class: a `RuntimeException` with Java's four
/// constructors (`()`, `(String)`, `(Throwable)`, `(String, Throwable)`),
/// carried in [`LuceneErrorKind::Throwable`].
macro_rules! icu_exception_class {
    ($(#[$doc:meta])* $name:ident, $class:literal, illegal_argument = $iae:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone)]
        pub struct $name {
            message: Option<String>,
            cause: Option<std::sync::Arc<rucene::error::LuceneError>>,
        }

        impl $name {
            /// The Java binary class name.
            pub const CLASS: &'static str = $class;

            /// `new X()`: no message, no cause.
            pub fn new() -> Self {
                Self { message: None, cause: None }
            }

            /// `new X(String message)`.
            pub fn with_message(message: impl Into<String>) -> Self {
                Self { message: Some(message.into()), cause: None }
            }

            /// `new X(Throwable cause)`: the message is `cause.toString()`,
            /// as `Throwable(Throwable)` sets it.
            pub fn with_cause(cause: rucene::error::LuceneError) -> Self {
                Self { message: Some(cause.to_string()), cause: Some(std::sync::Arc::new(cause)) }
            }

            /// `new X(String message, Throwable cause)`.
            pub fn with_message_and_cause(
                message: impl Into<String>,
                cause: rucene::error::LuceneError,
            ) -> Self {
                Self { message: Some(message.into()), cause: Some(std::sync::Arc::new(cause)) }
            }

            /// This exception as the error it is thrown as.
            pub fn error(self) -> rucene::error::LuceneError {
                rucene::error::LuceneError::from(rucene::error::LuceneErrorKind::Throwable(
                    std::sync::Arc::new(self),
                ))
            }

            /// The exception of this class `error` stands for, if any.
            pub fn of(error: &rucene::error::LuceneError) -> Option<&Self> {
                match error.kind() {
                    rucene::error::LuceneErrorKind::Throwable(t) => t.as_any().downcast_ref(),
                    _ => None,
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl rucene::error::JavaThrowable for $name {
            fn java_class(&self) -> &'static str {
                $class
            }

            fn get_message(&self) -> Option<&str> {
                self.message.as_deref()
            }

            fn get_cause(&self) -> Option<&std::sync::Arc<rucene::error::LuceneError>> {
                self.cause.as_ref()
            }

            fn is_illegal_argument_exception(&self) -> bool {
                $iae
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
    };
}
pub(crate) use icu_exception_class;

icu_exception_class!(
    /// Base class for unchecked, ICU-specific exceptions.
    ///
    /// Equivalent to `com.ibm.icu.util.ICUException` (`ICUException.java`),
    /// which extends `RuntimeException`.
    ICUException,
    "com.ibm.icu.util.ICUException",
    illegal_argument = false
);

impl ICUException {
    /// Whether `error` is an `ICUException` or one of its subclasses
    /// (`ICUCloneNotSupportedException`).
    pub fn is_instance(error: &LuceneError) -> bool {
        match error.kind() {
            LuceneErrorKind::Throwable(t) => {
                let t: &Arc<dyn JavaThrowable> = t;
                t.as_any().is::<ICUException>()
                    || t.as_any()
                        .is::<crate::util::icu_clone_not_supported_exception::ICUCloneNotSupportedException>()
            }
            _ => false,
        }
    }
}
