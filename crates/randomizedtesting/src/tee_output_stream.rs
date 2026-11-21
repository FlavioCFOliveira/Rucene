//! Port of `com.carrotsearch.randomizedtesting.TeeOutputStream`
//! (randomizedtesting 2.8.1, `TeeOutputStream.java`).

use std::fmt;
use std::io::{self, Write};

/// A stream that writes to two streams, every call to both, and reports
/// the first failure with the second one suppressed (`TeeOutputStream`).
///
/// **Divergence.** Java's `IOException` carries suppressed exceptions
/// (`TeeOutputStream.java:33`); a
/// Rust `io::Error` does not, so the error is a [`TeeError`] holding the
/// first error and the suppressed second one. Java's `equals`, `hashCode`
/// and `clone` throw; the port does not implement them.
pub struct TeeOutputStream<A: Write, B: Write> {
    first: A,
    second: B,
}

/// The first failure of a [`TeeOutputStream`] call, with the second
/// stream's failure suppressed.
#[derive(Debug)]
pub struct TeeError {
    /// The first stream's error, or the second's when only it failed.
    pub error: io::Error,
    /// The second stream's error when both failed (`addSuppressed`).
    pub suppressed: Option<io::Error>,
}

impl fmt::Display for TeeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for TeeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl<A: Write, B: Write> TeeOutputStream<A, B> {
    /// `new TeeOutputStream(first, second)`.
    pub fn new(first: A, second: B) -> TeeOutputStream<A, B> {
        TeeOutputStream { first, second }
    }

    /// The two streams.
    pub fn into_inner(self) -> (A, B) {
        (self.first, self.second)
    }

    fn both(&mut self, f: impl Fn(&mut dyn Write) -> io::Result<()>) -> io::Result<()> {
        let e1 = f(&mut self.first).err();
        let e2 = f(&mut self.second).err();
        match (e1, e2) {
            (None, None) => Ok(()),
            (Some(e), suppressed) => Err(io::Error::new(
                e.kind(),
                TeeError {
                    error: e,
                    suppressed,
                },
            )),
            (None, Some(e)) => Err(io::Error::new(
                e.kind(),
                TeeError {
                    error: e,
                    suppressed: None,
                },
            )),
        }
    }

    /// `close()`: flushes both streams (a Rust writer closes when dropped).
    pub fn close(&mut self) -> io::Result<()> {
        self.both(|w| w.flush())
    }
}

impl<A: Write, B: Write> Write for TeeOutputStream<A, B> {
    /// `write(b, off, len)`: the whole buffer to each stream.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.both(|w| w.write_all(buf))?;
        Ok(buf.len())
    }

    /// `flush()`.
    fn flush(&mut self) -> io::Result<()> {
        self.both(|w| w.flush())
    }
}

impl<A: Write + fmt::Display, B: Write + fmt::Display> fmt::Display for TeeOutputStream<A, B> {
    /// `toString()`: `[tee: 1:<first>, 2:<second>]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[tee: 1:{}, 2:{}]", self.first, self.second)
    }
}
