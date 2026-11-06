//! Port of `com.carrotsearch.randomizedtesting.ResourceDisposalError`
//! (randomizedtesting 2.8.1, `ResourceDisposalError.java`).

use std::backtrace::Backtrace;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::jdk::exception::Throwable;

/// Thrown when a resource registered with `RandomizedContext.closeAtEnd`
/// could not be released: `ResourceDisposalError extends Error`
/// (package-private). The runner builds it with the resource's registration
/// stack as its stack.
#[derive(Debug)]
pub(crate) struct ResourceDisposalError {
    message: String,
    /// The throwable `close()` threw: the `IOException` it declares, or any
    /// other throwable (`catch (Throwable t)`), such as the `AssertionError`
    /// of Lucene's `CloseableDirectory`.
    cause: Throwable,
    stack: Option<Arc<Backtrace>>,
}

impl ResourceDisposalError {
    /// `new ResourceDisposalError(msg, cause)`.
    pub(crate) fn new(message: String, cause: Throwable) -> Self {
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

    /// The error as the runner reports it: a
    /// `com.carrotsearch.randomizedtesting.ResourceDisposalError` throwable
    /// caused by what the resource's `close()` threw.
    pub(crate) fn to_throwable(&self) -> Throwable {
        Throwable::with_cause(
            crate::randomized_runner::RESOURCE_DISPOSAL_ERROR,
            Some(self.message.clone()),
            self.cause.clone(),
        )
    }

    /// `getMessage()`.
    #[allow(dead_code)] // Java declares it; the runner reports `to_throwable`.
    pub(crate) fn get_message(&self) -> &str {
        &self.message
    }

    /// `getStackTrace()`.
    #[allow(dead_code)] // Java declares it; the runner reports `to_throwable`.
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

impl Error for ResourceDisposalError {}
