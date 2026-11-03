//! Port of `com.carrotsearch.randomizedtesting.ResourceDisposalError`
//! (randomizedtesting 2.8.1, `ResourceDisposalError.java`).

use std::backtrace::Backtrace;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

/// Thrown when a resource registered with `RandomizedContext.closeAtEnd`
/// could not be released: `ResourceDisposalError extends Error`
/// (package-private). The runner builds it with the resource's registration
/// stack as its stack.
#[derive(Debug)]
#[allow(dead_code)] // Package-private: RandomizedRunner (a later task) calls it.
pub(crate) struct ResourceDisposalError {
    message: String,
    cause: Box<dyn Error + Send + Sync>,
    stack: Option<Arc<Backtrace>>,
}

#[allow(dead_code)] // Package-private: RandomizedRunner (a later task) calls it.
impl ResourceDisposalError {
    /// `new ResourceDisposalError(msg, cause)`.
    pub(crate) fn new(message: String, cause: Box<dyn Error + Send + Sync>) -> Self {
        ResourceDisposalError {
            message,
            cause,
            stack: None,
        }
    }

    /// `setStackTrace(stack)`.
    pub(crate) fn set_stack_trace(&mut self, stack: Arc<Backtrace>) {
        self.stack = Some(stack);
    }

    /// `getMessage()`.
    pub(crate) fn get_message(&self) -> &str {
        &self.message
    }

    /// `getStackTrace()`.
    pub(crate) fn get_stack_trace(&self) -> Option<&Arc<Backtrace>> {
        self.stack.as_ref()
    }
}

impl fmt::Display for ResourceDisposalError {
    /// `Throwable.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "com.carrotsearch.randomizedtesting.ResourceDisposalError: {}",
            self.message
        )
    }
}

impl Error for ResourceDisposalError {
    /// `getCause()`.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
