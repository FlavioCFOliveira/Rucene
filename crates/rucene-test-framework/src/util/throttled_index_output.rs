/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.util.ThrottledIndexOutput`.

use std::fmt;
use std::time::Instant;

use randomizedtesting::jdk::thread::Thread;
use rucene::error::{LuceneError, Result};
use rucene::store::{DataOutput, FilterIndexOutput, IndexOutput};
use rucene::util::concurrent::InterruptedException;

/// Intentionally slow `IndexOutput` for testing.
///
/// Port of `ThrottledIndexOutput` (`extends FilterIndexOutput`): every write
/// goes to the delegate, then sleeps for the delay that brings the observed
/// throughput down to `bytes_per_second` once more than `min_bytes_written`
/// bytes are pending; `close()` sleeps `close_delay_millis` plus the delay
/// of the pending bytes before it closes the delegate.
pub struct ThrottledIndexOutput {
    out: FilterIndexOutput,
    bytes_per_second: i32,
    flush_delay_millis: i64,
    close_delay_millis: i64,
    seek_delay_millis: i64,
    pending_bytes: i64,
    min_bytes_written: i64,
    time_elapsed: i64,
}

impl ThrottledIndexOutput {
    /// `DEFAULT_MIN_WRITTEN_BYTES`.
    pub const DEFAULT_MIN_WRITTEN_BYTES: i32 = 1024;

    /// `newFromDelegate(IndexOutput out)`: a throttled output with this
    /// one's settings over `out`.
    pub fn new_from_delegate(&self, out: Box<dyn IndexOutput>) -> ThrottledIndexOutput {
        ThrottledIndexOutput::with_delays(
            self.bytes_per_second,
            self.flush_delay_millis,
            self.close_delay_millis,
            self.seek_delay_millis,
            self.min_bytes_written,
            out,
        )
    }

    /// `ThrottledIndexOutput(int bytesPerSecond, long delayInMillis,
    /// IndexOutput out)`: every delay `delay_in_millis`,
    /// [`DEFAULT_MIN_WRITTEN_BYTES`](Self::DEFAULT_MIN_WRITTEN_BYTES).
    pub fn new(bytes_per_second: i32, delay_in_millis: i64, out: Box<dyn IndexOutput>) -> Self {
        Self::with_delays(
            bytes_per_second,
            delay_in_millis,
            delay_in_millis,
            delay_in_millis,
            i64::from(Self::DEFAULT_MIN_WRITTEN_BYTES),
            out,
        )
    }

    /// `ThrottledIndexOutput(int bytesPerSecond, long delays, int
    /// minBytesWritten, IndexOutput out)`.
    pub fn with_min_bytes_written(
        bytes_per_second: i32,
        delays: i64,
        min_bytes_written: i32,
        out: Box<dyn IndexOutput>,
    ) -> Self {
        Self::with_delays(
            bytes_per_second,
            delays,
            delays,
            delays,
            i64::from(min_bytes_written),
            out,
        )
    }

    /// `mBitsToBytes(int mbits)`: `mbits * 125000000`, wrapping as Java's
    /// `int` multiplication does.
    pub const fn m_bits_to_bytes(mbits: i32) -> i32 {
        mbits.wrapping_mul(125_000_000)
    }

    /// `ThrottledIndexOutput(int bytesPerSecond, long flushDelayMillis, long
    /// closeDelayMillis, long seekDelayMillis, long minBytesWritten,
    /// IndexOutput out)`: resource description `ThrottledIndexOutput(<out>)`
    /// (`out.toString()`, its resource description), name `out.getName()`.
    ///
    /// # Panics
    ///
    /// Java's `assert bytesPerSecond > 0`, under `debug_assertions`.
    pub fn with_delays(
        bytes_per_second: i32,
        flush_delay_millis: i64,
        close_delay_millis: i64,
        seek_delay_millis: i64,
        min_bytes_written: i64,
        out: Box<dyn IndexOutput>,
    ) -> Self {
        let description = format!("ThrottledIndexOutput({})", out.resource_description());
        let name = out.name().to_string();
        java_assert_unchecked!(bytes_per_second > 0);
        ThrottledIndexOutput {
            out: FilterIndexOutput::new(description, name, out),
            bytes_per_second,
            flush_delay_millis,
            close_delay_millis,
            seek_delay_millis,
            pending_bytes: 0,
            min_bytes_written,
            time_elapsed: 0,
        }
    }

    /// `getDelay(boolean closing)`: the milliseconds to sleep so that the
    /// pending bytes are written at `bytes_per_second`, 0 when none; a
    /// delay resets the pending bytes and the elapsed time.
    pub(crate) fn get_delay(&mut self, closing: bool) -> i64 {
        if self.pending_bytes > 0 && (closing || self.pending_bytes > self.min_bytes_written) {
            let actual_bps = (self.time_elapsed / self.pending_bytes).wrapping_mul(1_000_000_000); // nano to sec
            if actual_bps > i64::from(self.bytes_per_second) {
                let expected =
                    self.pending_bytes.wrapping_mul(1000) / i64::from(self.bytes_per_second);
                // TimeUnit.NANOSECONDS.toMillis: truncation.
                let delay = expected - self.time_elapsed / 1_000_000;
                self.pending_bytes = 0;
                self.time_elapsed = 0;
                return delay;
            }
        }
        0
    }

    /// `sleep(long ms)`: `Thread.sleep(ms)` when positive.
    ///
    /// # Errors
    ///
    /// `ThreadInterruptedException` when the thread is interrupted.
    fn sleep(ms: i64) -> Result<()> {
        if ms <= 0 {
            return Ok(());
        }
        Thread::try_sleep(ms).map_err(|_| LuceneError::thread_interrupted(InterruptedException))
    }
}

impl fmt::Debug for ThrottledIndexOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.out.resource_description())
    }
}

impl DataOutput for ThrottledIndexOutput {
    /// `IndexOutput.toString()`: the resource description
    /// (`IndexOutput.java:83-86`).
    fn to_java_string(&self) -> String {
        self.resource_description().to_string()
    }

    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.write_bytes(&[b], 0, 1)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, length: usize) -> Result<()> {
        let before = Instant::now();
        // TODO: sometimes, write only half the bytes, then
        // sleep, then 2nd half, then sleep, so we sometimes
        // interrupt having only written not all bytes
        self.out.get_delegate_mut().write_bytes(b, offset, length)?;
        self.time_elapsed = self
            .time_elapsed
            .wrapping_add(before.elapsed().as_nanos() as i64);
        self.pending_bytes += length as i64;
        let delay = self.get_delay(false);
        Self::sleep(delay)
    }
}

impl IndexOutput for ThrottledIndexOutput {
    /// `close()`: sleeps, then (in a `finally`) closes the delegate.
    fn close(&mut self) -> Result<()> {
        let delay = self.close_delay_millis + self.get_delay(true);
        let slept = Self::sleep(delay);
        let closed = self.out.close();
        // A `finally` that throws replaces the pending exception.
        closed.and(slept)
    }

    fn file_pointer(&self) -> i64 {
        self.out.file_pointer()
    }

    fn checksum(&self) -> Result<i64> {
        self.out.checksum()
    }

    fn resource_description(&self) -> &str {
        self.out.resource_description()
    }

    fn name(&self) -> &str {
        self.out.name()
    }

    fn as_filter_index_output(&self) -> Option<&dyn IndexOutput> {
        Some(self.out.get_delegate())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use rucene::store::{
        ByteBuffersDirectory, DataOutput, Directory, IndexOutput, DEFAULT_IO_CONTEXT,
    };

    use super::ThrottledIndexOutput;

    fn delegate(dir: &ByteBuffersDirectory, name: &str) -> Box<dyn IndexOutput> {
        dir.create_output(name, &*DEFAULT_IO_CONTEXT).unwrap()
    }

    #[test]
    fn m_bits_to_bytes_wraps_as_java_int() {
        assert_eq!(ThrottledIndexOutput::m_bits_to_bytes(1), 125_000_000);
        assert_eq!(ThrottledIndexOutput::m_bits_to_bytes(17), 2_125_000_000);
        // 18 * 125000000 overflows `int`: Java wraps to -2044967296.
        assert_eq!(ThrottledIndexOutput::m_bits_to_bytes(18), -2_044_967_296);
    }

    #[test]
    fn writes_reach_the_delegate_and_names_wrap_it() {
        let dir = ByteBuffersDirectory::default();
        let inner = delegate(&dir, "a.bin");
        let inner_description = inner.resource_description().to_string();
        let mut out = ThrottledIndexOutput::new(i32::MAX, 0, inner);
        assert_eq!(out.name(), "a.bin");
        assert_eq!(
            out.resource_description(),
            format!("ThrottledIndexOutput({inner_description})")
        );
        out.write_bytes(b"hello", 0, 5).unwrap();
        out.write_byte(b'!').unwrap();
        assert_eq!(out.file_pointer(), 6);
        let derived = out.new_from_delegate(delegate(&dir, "b.bin"));
        assert_eq!(derived.name(), "b.bin");
        out.close().unwrap();
        assert_eq!(dir.file_length("a.bin").unwrap(), 6);
    }

    /// Throttling: once more than `minBytesWritten` bytes are pending at a
    /// rate above `bytesPerSecond`, the write sleeps
    /// `pendingBytes * 1000 / bytesPerSecond` minus the elapsed milliseconds,
    /// and resets the pending bytes.
    #[test]
    fn get_delay_follows_lucenes_formula() {
        let dir = ByteBuffersDirectory::default();
        let mut out = ThrottledIndexOutput::with_delays(1000, 0, 0, 0, 10, delegate(&dir, "c.bin"));
        // Below the minimum: no delay, the bytes stay pending.
        out.pending_bytes = 10;
        out.time_elapsed = 5_000_000;
        assert_eq!(out.get_delay(false), 0);
        assert_eq!(out.pending_bytes, 10);
        // Closing: 10 bytes at 1000 B/s is 10 ms, minus the 5 ms elapsed.
        assert_eq!(out.get_delay(true), 5);
        assert_eq!((out.pending_bytes, out.time_elapsed), (0, 0));
        // A slow writer (elapsed / pending < 1 ns per byte yields 0 B/s) is
        // never delayed.
        out.pending_bytes = 100;
        out.time_elapsed = 50;
        assert_eq!(out.get_delay(false), 0);
        let start = Instant::now();
        let mut slow =
            ThrottledIndexOutput::with_delays(20_000, 0, 0, 0, 0, delegate(&dir, "d.bin"));
        slow.time_elapsed = 0;
        slow.write_bytes(&[0u8; 400], 0, 400).unwrap();
        // 400 bytes at 20000 B/s: about 20 ms, whatever the write took.
        assert!(start.elapsed().as_millis() >= 15, "{:?}", start.elapsed());
        slow.close().unwrap();
        out.close().unwrap();
    }
}
