//! Port of `com.carrotsearch.randomizedtesting.listeners.ReproduceInfoPrinter`
//! (randomizedtesting 2.8.1, `listeners/ReproduceInfoPrinter.java`).

use crate::jdk::system;
use crate::junit::{Failure, RunListener, ASSUMPTION_VIOLATED_INTERNAL};
use crate::randomized_context::RandomizedContext;
use crate::reproduce_error_message_builder::ReproduceErrorMessageBuilder;
use crate::trace_formatting::TraceFormatting;

/// Prints to `System.err`, for every failure, the command-line options that
/// re-run it (`ReproduceInfoPrinter`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ReproduceInfoPrinter;

impl RunListener for ReproduceInfoPrinter {
    /// Assumption failures are skipped.
    fn test_failure(&self, failure: &Failure) {
        if failure
            .exception()
            .is_instance(ASSUMPTION_VIOLATED_INTERNAL)
        {
            return;
        }
        let d = failure.description();
        let mut b = String::new();
        b.push_str(&format!("FAILURE  : {}\n", d.display_name()));
        b.push_str(&format!(
            "Message  : {}\n",
            failure.message().unwrap_or("null")
        ));
        b.push_str("Reproduce: ");
        let mut builder = ReproduceErrorMessageBuilder::with_text(b);
        builder.append_all_opts(d);
        let mut b = builder.into_text();
        b.push('\n');
        b.push_str("Throwable:\n");
        let _quiet = crate::jdk::exception::QuietPanics::new();
        let traces = std::panic::catch_unwind(|| {
            RandomizedContext::current()
                .get_runner()
                .map(|r| r.get_trace_formatting().clone())
        })
        .ok()
        .flatten()
        .unwrap_or_else(TraceFormatting::new);
        traces.format_throwable_into(&mut b, failure.exception());
        system::err_println(&b);
    }
}
