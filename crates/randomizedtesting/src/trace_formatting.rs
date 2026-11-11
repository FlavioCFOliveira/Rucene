//! Port of `com.carrotsearch.randomizedtesting.TraceFormatting`
//! (randomizedtesting 2.8.1, `TraceFormatting.java`).

use crate::jdk::exception::{StackTraceElement, Throwable};

/// Formats throwables and their stacks, collapsing filtered frames
/// (`TraceFormatting`).
#[derive(Debug, Clone, Default)]
pub struct TraceFormatting {
    filtered_prefixes: Vec<String>,
}

impl TraceFormatting {
    /// `new TraceFormatting()`: no filtering.
    pub fn new() -> TraceFormatting {
        TraceFormatting::default()
    }

    /// `new TraceFormatting(filteredPrefixes)`.
    pub fn with_filters(filtered_prefixes: &[&str]) -> TraceFormatting {
        TraceFormatting {
            filtered_prefixes: filtered_prefixes.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// `formatThrowable(t)`.
    pub fn format_throwable(&self, t: &Throwable) -> String {
        let mut b = String::new();
        self.format_throwable_into(&mut b, t);
        b
    }

    /// `formatThrowable(StringBuilder, t)`: the throwable, its stack and,
    /// after `Caused by: `, its causes.
    pub fn format_throwable_into(&self, b: &mut String, t: &Throwable) {
        b.push_str(&t.to_string());
        b.push('\n');
        self.format_stack_trace_into(b, t.stack_trace());
        if let Some(c) = t.cause() {
            b.push_str("Caused by: ");
            self.format_throwable_into(b, c);
        }
    }

    /// `formatStackTrace(stackTrace)`.
    pub fn format_stack_trace(&self, stack: &[StackTraceElement]) -> String {
        let mut b = String::new();
        self.format_stack_trace_into(&mut b, stack);
        b
    }

    /// `formatStackTrace(StringBuilder, stackTrace)`: each frame indented;
    /// runs of frames that start with a filtered prefix collapse into
    /// `    [...prefix*, ...]`.
    pub fn format_stack_trace_into(&self, b: &mut String, stack: &[StackTraceElement]) {
        // The run's prefixes, a `HashSet<String>` printed in its iteration
        // order.
        let mut filtered_set: Vec<String> = Vec::new();
        for e in stack {
            let line = e.to_string();
            let filtered = self
                .filtered_prefixes
                .iter()
                .find(|p| line.starts_with(p.as_str()));
            match filtered {
                Some(prefix) => {
                    if filtered_set.is_empty() {
                        b.push_str("    [...");
                    }
                    if !filtered_set.contains(prefix) {
                        filtered_set.push(prefix.clone());
                    }
                }
                None => {
                    append_filtered(b, &mut filtered_set);
                    b.push_str("    ");
                    b.push_str(&line);
                    b.push('\n');
                }
            }
        }
        append_filtered(b, &mut filtered_set);
    }
}

fn append_filtered(b: &mut String, filtered_set: &mut Vec<String>) {
    if !filtered_set.is_empty() {
        let keyed: Vec<(&String, i32)> = filtered_set
            .iter()
            .map(|p| (p, crate::jdk::lang::string_hash_code(p)))
            .collect();
        let order = crate::jdk::util::hash_iteration_order(&keyed, |a, b| a == b);
        let joined: Vec<String> = order
            .iter()
            .map(|&i| format!("{}*", filtered_set[i]))
            .collect();
        b.push_str(&joined.join(", "));
        b.push_str("]\n");
        filtered_set.clear();
    }
}
