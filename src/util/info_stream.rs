//! `InfoStream` implementations ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`PrintStreamInfoStream`] | `PrintStreamInfoStream` |
//! | [`JavaLoggingInfoStream`] | `JavaLoggingInfoStream` |
//!
//! The [`InfoStream`] trait itself is already ported in [`crate::util`].

#![deny(unsafe_code)]

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::util::InfoStream;

// ---------------------------------------------------------------------------
// PrintStreamInfoStream
// ---------------------------------------------------------------------------

/// Assigns each stream its own id. `PrintStreamInfoStream.MESSAGE_ID`.
static MESSAGE_ID: AtomicUsize = AtomicUsize::new(0);

/// An [`InfoStream`] that writes every message to a byte sink.
///
/// Port of `org.apache.lucene.util.PrintStreamInfoStream`, including its line
/// format: `component id [timestamp; thread]: message`.
///
/// **Divergences from Lucene 10.5.0.**
///
/// * Java writes to a `PrintStream`; this writes to any
///   [`std::io::Write`], held behind a [`Mutex`] because [`InfoStream`] is
///   `Send + Sync` and its `message` takes `&self`.
/// * Java's `isSystemStream()` compares the stream against `System.out` and
///   `System.err` so that `close()` does not close them. Rust has no such
///   identity to compare, so the flag is supplied at construction:
///   [`PrintStreamInfoStream::new`] marks the sink as owned (closed on
///   `close`), [`PrintStreamInfoStream::system`] marks it as shared.
/// * The timestamp is `Instant.now().toString()` in Java, an ISO-8601 UTC
///   instant. This port formats [`std::time::SystemTime::now`] with `chrono`,
///   which the crate already depends on for `DateTools`; `chrono` is built
///   without its `clock` feature here, hence the explicit epoch conversion.
pub struct PrintStreamInfoStream {
    message_id: usize,
    stream: Mutex<Box<dyn Write + Send>>,
    is_system_stream: bool,
}

impl std::fmt::Debug for PrintStreamInfoStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrintStreamInfoStream")
            .field("message_id", &self.message_id)
            .field("is_system_stream", &self.is_system_stream)
            .finish()
    }
}

impl PrintStreamInfoStream {
    /// Creates a stream over an owned sink, taking the next message id.
    ///
    /// Equivalent to `new PrintStreamInfoStream(PrintStream)`.
    pub fn new(stream: Box<dyn Write + Send>) -> Self {
        Self::with_message_id(stream, MESSAGE_ID.fetch_add(1, Ordering::SeqCst))
    }

    /// Creates a stream over an owned sink with an explicit message id.
    ///
    /// Equivalent to `new PrintStreamInfoStream(PrintStream, int)`.
    pub fn with_message_id(stream: Box<dyn Write + Send>, message_id: usize) -> Self {
        Self {
            message_id,
            stream: Mutex::new(stream),
            is_system_stream: false,
        }
    }

    /// Creates a stream over a process-wide sink such as standard output, which
    /// [`InfoStream::close`] must not close.
    pub fn system(stream: Box<dyn Write + Send>) -> Self {
        Self {
            message_id: MESSAGE_ID.fetch_add(1, Ordering::SeqCst),
            stream: Mutex::new(stream),
            is_system_stream: true,
        }
    }

    /// Returns this stream's message id.
    pub fn message_id(&self) -> usize {
        self.message_id
    }

    /// Returns whether the sink is a process-wide stream.
    ///
    /// Equivalent to `PrintStreamInfoStream.isSystemStream()`.
    pub fn is_system_stream(&self) -> bool {
        self.is_system_stream
    }

    /// Returns the timestamp prefix of a message.
    ///
    /// Equivalent to the protected `PrintStreamInfoStream.getTimestamp()`,
    /// which renders `Instant.now()` as an ISO-8601 UTC instant.
    pub fn get_timestamp(&self) -> String {
        let since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        match chrono::DateTime::from_timestamp(
            since_epoch.as_secs() as i64,
            since_epoch.subsec_nanos(),
        ) {
            Some(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            // Unreachable for any clock this side of the year 262143; render
            // the raw instant rather than panicking.
            None => format!("{}s since epoch", since_epoch.as_secs()),
        }
    }
}

impl InfoStream for PrintStreamInfoStream {
    fn message(&self, component: &str, message: &str) {
        let thread = std::thread::current().name().unwrap_or("main").to_string();
        let line = format!(
            "{component} {} [{}; {thread}]: {message}\n",
            self.message_id,
            self.get_timestamp()
        );
        if let Ok(mut stream) = self.stream.lock() {
            // Java's `PrintStream.println` swallows I/O errors; so does this.
            let _ = stream.write_all(line.as_bytes());
            let _ = stream.flush();
        }
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    fn close(&self) {
        if !self.is_system_stream {
            if let Ok(mut stream) = self.stream.lock() {
                let _ = stream.flush();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// JavaLoggingInfoStream
// ---------------------------------------------------------------------------

/// An [`InfoStream`] that forwards every message to the `log` crate.
///
/// Port of `org.apache.lucene.util.JavaLoggingInfoStream`.
///
/// **Divergence from Lucene 10.5.0.** Java writes to `java.util.logging` at a
/// configurable `Level`, caching one `Logger` per component. The Rust
/// equivalent of that facade is the `log` crate, which this crate already
/// depends on: the component is mapped to a *target* by the same
/// `componentToLoggerName` function, and the `Level` becomes a
/// [`log::Level`]. Because `log` resolves targets without allocating a logger
/// object, the `ConcurrentHashMap` cache has nothing to hold and is gone; the
/// mapping function is still applied on every call, as Java's is.
pub struct JavaLoggingInfoStream {
    component_to_logger_name: Box<dyn Fn(&str) -> String + Send + Sync>,
    level: log::Level,
}

impl std::fmt::Debug for JavaLoggingInfoStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JavaLoggingInfoStream")
            .field("level", &self.level)
            .finish()
    }
}

impl JavaLoggingInfoStream {
    /// Creates a stream logging under the `org.apache.lucene.` prefix.
    ///
    /// Equivalent to `new JavaLoggingInfoStream(Level)`.
    pub fn new(level: log::Level) -> Self {
        Self::with_prefix("org.apache.lucene.", level)
    }

    /// Creates a stream logging under `name_prefix`.
    ///
    /// Equivalent to `new JavaLoggingInfoStream(String, Level)`.
    pub fn with_prefix(name_prefix: impl Into<String>, level: log::Level) -> Self {
        let prefix = name_prefix.into();
        Self::with_mapper(move |component| format!("{prefix}{component}"), level)
    }

    /// Creates a stream mapping components to logger names with
    /// `component_to_logger_name`.
    ///
    /// Equivalent to `new JavaLoggingInfoStream(Function, Level)`.
    pub fn with_mapper<F>(component_to_logger_name: F, level: log::Level) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self {
            component_to_logger_name: Box::new(component_to_logger_name),
            level,
        }
    }

    /// Returns the logger name a component maps to.
    ///
    /// Equivalent to the private `JavaLoggingInfoStream.getLogger`.
    pub fn logger_name(&self, component: &str) -> String {
        (self.component_to_logger_name)(component)
    }
}

impl InfoStream for JavaLoggingInfoStream {
    fn message(&self, component: &str, message: &str) {
        // Java passes a null class and method name, which prevents the logging
        // framework from inspecting the stack; `log` never inspects it.
        log::log!(target: "lucene", self.level, "{}: {}", self.logger_name(component), message);
    }

    fn is_enabled(&self, component: &str) -> bool {
        let _ = self.logger_name(component);
        log::log_enabled!(target: "lucene", self.level)
    }

    fn close(&self) {
        // Java clears its logger cache here; this port holds no cache.
    }
}
