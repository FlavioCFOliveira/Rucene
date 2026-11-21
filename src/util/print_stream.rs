//! The `java.io.PrintStream` surface Lucene's command-line tools write to.
//!
//! | Rucene | Java |
//! | --- | --- |
//! | [`PrintStream`] | the `print`/`println`/`flush` methods of `java.io.PrintStream` |
//! | [`WriterPrintStream`] | `new PrintStream(OutputStream, autoFlush, UTF_8)`, `System.out`, `System.err` |
//! | [`ByteArrayPrintStream`] | `new PrintStream(new ByteArrayOutputStream(), true, UTF_8)` |
//!
//! `CheckIndex` reports to a `PrintStream` (`CheckIndex.java:111`, `:504-524`)
//! and mixes `print` — a check's label, written before the check runs — with
//! `println` — its `OK [...]` or `FAILED` outcome — so that both land on one
//! line. An [`InfoStream`](crate::util::InfoStream) cannot express that: it is
//! line-oriented and component-tagged. This module is the port of the Java
//! class those tools hold instead.

#![deny(unsafe_code)]

use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};

/// The line separator `println` writes: `System.lineSeparator()` on the
/// platforms this crate targets.
pub const LINE_SEPARATOR: &str = "\n";

/// A text sink with `java.io.PrintStream`'s `print`, `println` and `flush`.
///
/// Equivalent to the methods of `java.io.PrintStream` Lucene calls. Java's
/// `PrintStream` never throws: it swallows `IOException`s and records them for
/// `checkError()`; implementations swallow I/O errors in the same way.
pub trait PrintStream: Send + Sync {
    /// Writes `s` without a line separator.
    ///
    /// Equivalent to `PrintStream.print(String)`.
    fn print(&self, s: &str);

    /// Writes `s` followed by the line separator, as one write that another
    /// thread's output cannot interleave.
    ///
    /// Equivalent to `PrintStream.println(String)`, which writes both under
    /// the stream's lock.
    fn println(&self, s: &str) {
        let mut line = String::with_capacity(s.len() + LINE_SEPARATOR.len());
        line.push_str(s);
        line.push_str(LINE_SEPARATOR);
        self.print(&line);
    }

    /// Flushes the underlying sink.
    ///
    /// Equivalent to `PrintStream.flush()`.
    fn flush(&self) {}
}

/// A [`PrintStream`] over any [`Write`] sink.
///
/// Equivalent to `new PrintStream(OutputStream, boolean autoFlush, Charset)`
/// with UTF-8, and to `System.out` and `System.err` through
/// [`stdout`](Self::stdout) and [`stderr`](Self::stderr). With `auto_flush`,
/// every `println` flushes, as Java's auto-flushing stream does.
pub struct WriterPrintStream {
    sink: Mutex<Box<dyn Write + Send>>,
    auto_flush: bool,
}

impl std::fmt::Debug for WriterPrintStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterPrintStream")
            .field("auto_flush", &self.auto_flush)
            .finish_non_exhaustive()
    }
}

impl WriterPrintStream {
    /// Wraps `sink`.
    ///
    /// Equivalent to `new PrintStream(sink, autoFlush, UTF_8)`.
    pub fn new(sink: Box<dyn Write + Send>, auto_flush: bool) -> Self {
        Self {
            sink: Mutex::new(sink),
            auto_flush,
        }
    }

    /// Standard output: `System.out`, which the JVM creates auto-flushing on
    /// `println`.
    pub fn stdout() -> Self {
        Self::new(Box::new(std::io::stdout()), true)
    }

    /// Standard error: `System.err`, auto-flushing as `System.out`.
    pub fn stderr() -> Self {
        Self::new(Box::new(std::io::stderr()), true)
    }

    fn write_str(&self, s: &str, line: bool) {
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        // Java's `PrintStream` swallows I/O errors; so does this.
        let _ = sink.write_all(s.as_bytes());
        if line && self.auto_flush {
            let _ = sink.flush();
        }
    }
}

impl PrintStream for WriterPrintStream {
    fn print(&self, s: &str) {
        self.write_str(s, false);
    }

    fn println(&self, s: &str) {
        let mut line = String::with_capacity(s.len() + LINE_SEPARATOR.len());
        line.push_str(s);
        line.push_str(LINE_SEPARATOR);
        self.write_str(&line, true);
    }

    fn flush(&self) {
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = sink.flush();
    }
}

/// A [`PrintStream`] over an in-memory byte buffer.
///
/// Equivalent to `new PrintStream(new ByteArrayOutputStream(), true, UTF_8)`,
/// the buffer `CheckIndex.checkIndex(List, ExecutorService)` gives each
/// concurrently checked segment (`CheckIndex.java:827-828`) and that tests hand
/// to `CheckIndex.setInfoStream`. The buffer is shared, so a clone of
/// [`buffer`](Self::buffer) reads what was written, as Java reads the
/// `ByteArrayOutputStream` the stream wraps.
#[derive(Debug, Clone, Default)]
pub struct ByteArrayPrintStream {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl ByteArrayPrintStream {
    /// Creates a stream over an empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a stream over `buffer`.
    pub fn with_buffer(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { buffer }
    }

    /// The shared buffer: Java's `ByteArrayOutputStream`.
    pub fn buffer(&self) -> &Arc<Mutex<Vec<u8>>> {
        &self.buffer
    }

    /// The buffer decoded as UTF-8.
    ///
    /// Equivalent to `ByteArrayOutputStream.toString(UTF_8)`, which replaces a
    /// malformed sequence with U+FFFD.
    pub fn to_string_utf8(&self) -> String {
        let buffer = self.buffer.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&buffer).into_owned()
    }
}

impl PrintStream for ByteArrayPrintStream {
    fn print(&self, s: &str) {
        self.buffer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(s.as_bytes());
    }
}
