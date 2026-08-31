//! A rate-limiting [`IndexOutput`].
//!
//! Ported from `org.apache.lucene.store.RateLimitedIndexOutput`.

use std::sync::Arc;

use super::{DataOutput, FilterIndexOutput, IndexOutput, RateLimiter};
use crate::error::Result;

/// An [`IndexOutput`] that pauses periodically so that the write throughput
/// stays under the limit imposed by a [`RateLimiter`].
///
/// Equivalent to `org.apache.lucene.store.RateLimitedIndexOutput`, which Lucene
/// marks `@lucene.internal` and uses to throttle merge writes.
///
/// The output counts the bytes handed to it and calls [`RateLimiter::pause`]
/// once more than [`RateLimiter::get_min_pause_check_bytes`] have accumulated.
/// A single `write_bytes` call is never split, so the instantaneous write rate
/// can briefly exceed the limit after an idle period; Lucene accepts the same
/// trade-off (LUCENE-10448).
///
/// # Divergence from Lucene 10.5.0
///
/// Java extends `FilterIndexOutput`; Rust has no inheritance, so this type
/// contains one and delegates to it, overriding the same five write methods
/// Java overrides. The rate limiter is held behind an [`Arc`] because a single
/// limiter is shared by every output taking part in one merge, which Java
/// expresses with a shared object reference.
pub struct RateLimitedIndexOutput {
    inner: FilterIndexOutput,
    rate_limiter: Arc<dyn RateLimiter>,
    /// How many bytes have been written since `rate_limiter.pause` was last
    /// called.
    bytes_since_last_pause: i64,
    /// Cached so that the shared limiter's atomically-read minimum is not
    /// re-read on every write.
    current_min_pause_check_bytes: i64,
}

impl RateLimitedIndexOutput {
    /// Wraps `out`, throttling it with `rate_limiter`.
    ///
    /// Equivalent to `RateLimitedIndexOutput(RateLimiter, IndexOutput)`.
    pub fn new(rate_limiter: Arc<dyn RateLimiter>, out: Box<dyn IndexOutput>) -> Self {
        let resource_description =
            format!("RateLimitedIndexOutput({})", out.resource_description());
        let name = out.name().to_string();
        let current_min_pause_check_bytes = rate_limiter.get_min_pause_check_bytes();
        Self {
            inner: FilterIndexOutput::new(resource_description, name, out),
            rate_limiter,
            bytes_since_last_pause: 0,
            current_min_pause_check_bytes,
        }
    }

    /// Returns the wrapped output.
    ///
    /// Equivalent to `FilterIndexOutput.getDelegate()`.
    pub fn get_delegate(&self) -> &dyn IndexOutput {
        self.inner.get_delegate()
    }

    /// Returns the rate limiter throttling this output.
    pub fn rate_limiter(&self) -> &Arc<dyn RateLimiter> {
        &self.rate_limiter
    }

    /// Pauses if enough bytes have accumulated since the last pause.
    ///
    /// Equivalent to `RateLimitedIndexOutput.checkRate()`.
    fn check_rate(&mut self) -> Result<()> {
        if self.bytes_since_last_pause > self.current_min_pause_check_bytes {
            self.rate_limiter.pause(self.bytes_since_last_pause)?;
            self.bytes_since_last_pause = 0;
            self.current_min_pause_check_bytes = self.rate_limiter.get_min_pause_check_bytes();
        }
        Ok(())
    }
}

impl DataOutput for RateLimitedIndexOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.bytes_since_last_pause += 1;
        self.check_rate()?;
        self.inner.get_delegate_mut().write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, length: usize) -> Result<()> {
        self.bytes_since_last_pause += length as i64;
        self.check_rate()?;
        // The byte slice is written without pauses. This can cause the instant
        // write rate to breach the limit if there have been no writes for long
        // enough to keep the average within it. See LUCENE-10448.
        self.inner.get_delegate_mut().write_bytes(b, offset, length)
    }

    fn write_short(&mut self, i: i16) -> Result<()> {
        self.bytes_since_last_pause += std::mem::size_of::<i16>() as i64;
        self.check_rate()?;
        self.inner.get_delegate_mut().write_short(i)
    }

    fn write_int(&mut self, i: i32) -> Result<()> {
        self.bytes_since_last_pause += std::mem::size_of::<i32>() as i64;
        self.check_rate()?;
        self.inner.get_delegate_mut().write_int(i)
    }

    fn write_long(&mut self, i: i64) -> Result<()> {
        self.bytes_since_last_pause += std::mem::size_of::<i64>() as i64;
        self.check_rate()?;
        self.inner.get_delegate_mut().write_long(i)
    }
}

impl IndexOutput for RateLimitedIndexOutput {
    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn file_pointer(&self) -> i64 {
        self.inner.file_pointer()
    }

    fn checksum(&self) -> Result<i64> {
        self.inner.checksum()
    }

    fn resource_description(&self) -> &str {
        self.inner.resource_description()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

impl std::fmt::Debug for RateLimitedIndexOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimitedIndexOutput")
            .field("name", &self.inner.name())
            .field("resource_description", &self.inner.resource_description())
            .field("bytes_since_last_pause", &self.bytes_since_last_pause)
            .finish()
    }
}
