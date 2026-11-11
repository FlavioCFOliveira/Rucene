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

//! Port of `org.apache.lucene.tests.store.MockIndexOutputWrapper`: the
//! output [`MockDirectoryWrapper`] hands out, which throws an `IOException`
//! on a fake full disk, tracks the maximum disk space actually used, and
//! maybe throws random `IOException`s.
//!
//! `crash()` closes every open output (Java's `openFileHandles` holds the
//! outputs themselves), so the delegate and the `closed` flag live in an
//! `OutputCell` the directory reaches too. No lock on it is held while
//! the directory's monitor is acquired.

use std::sync::{Arc, Mutex, MutexGuard};

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::store::java_nio_file::io_exception;
use rucene::store::{DataInput, DataOutput, IndexOutput};

use super::mock_directory_wrapper::{try_with_resources, MockDirectoryWrapper};
use crate::util::lucene_test_case::VERBOSE;

/// An output's delegate and `closed` flag.
pub(crate) struct OutputCell {
    id: u64,
    name: String,
    resource_description: String,
    inner: Mutex<OutputInner>,
}

struct OutputInner {
    closed: bool,
    out: Box<dyn IndexOutput>,
}

impl OutputCell {
    fn lock(&self) -> MutexGuard<'_, OutputInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// `MockIndexOutputWrapper.close()`, for the output itself and for
/// `crash()`: a second close only closes the delegate again (it does not
/// mask double-close bugs); the first runs `maybeThrowDeterministicException`,
/// closes the delegate, then (in a `finally`) unregisters the output and,
/// when tracking disk usage, records the peak size.
pub(crate) fn close_output(cell: &OutputCell, dir: &MockDirectoryWrapper) -> Result<()> {
    {
        let mut inner = cell.lock();
        if inner.closed {
            return inner.out.close(); // don't mask double-close bugs
        }
        inner.closed = true;
    }
    // try (Closeable out = this.out) { dir.maybeThrowDeterministicException(); }
    let body = dir.maybe_throw_deterministic_exception();
    let closed = cell.lock().out.close();
    let result = try_with_resources(body, [closed]);
    // finally:
    dir.remove_index_output_id(cell.id, &cell.name);
    if dir.track_disk_usage() {
        // Now compute actual disk usage & track the maxUsedSize
        // in the MockDirectoryWrapper:
        // A `finally` that throws replaces the pending exception.
        let size = dir.size_in_bytes()?;
        if size > dir.get_max_used_size_in_bytes() {
            dir.set_max_used_size_in_bytes(size);
        }
    }
    result
}

/// Port of `org.apache.lucene.tests.store.MockIndexOutputWrapper` (`extends
/// FilterIndexOutput`).
pub struct MockIndexOutputWrapper {
    cell: Arc<OutputCell>,
    /// `out.getName()`, the `FilterIndexOutput` name.
    out_name: String,
    /// `dir`. Released once closed.
    dir: Option<MockDirectoryWrapper>,
    first: bool,
}

impl MockIndexOutputWrapper {
    /// `MockIndexOutputWrapper(dir, out, name)`: resource description
    /// `MockIndexOutputWrapper(<out>)`, name `out.getName()`; `name` is the
    /// file name the directory tracks.
    pub fn new(dir: &MockDirectoryWrapper, out: Box<dyn IndexOutput>, name: &str) -> Self {
        let resource_description =
            format!("MockIndexOutputWrapper({})", out.resource_description());
        let out_name = out.name().to_string();
        MockIndexOutputWrapper {
            cell: Arc::new(OutputCell {
                id: dir.next_handle_id(),
                name: name.to_string(),
                resource_description,
                inner: Mutex::new(OutputInner { closed: false, out }),
            }),
            out_name,
            dir: Some(dir.handle()),
            first: true,
        }
    }

    pub(crate) fn registration(&self) -> (u64, Arc<OutputCell>) {
        (self.cell.id, Arc::clone(&self.cell))
    }

    pub(crate) fn handle_id(&self) -> u64 {
        self.cell.id
    }

    /// The package-private field `name`.
    pub fn file_name(&self) -> &str {
        &self.cell.name
    }

    fn dir(&self) -> &MockDirectoryWrapper {
        self.dir
            .as_ref()
            .expect("INVARIANT: an output keeps its directory until it is closed")
    }

    /// `checkCrashed()`: if crashed since we were opened, then don't write
    /// anything.
    fn check_crashed(&self) -> Result<()> {
        if self.dir().is_crashed() {
            return Err(LuceneError::from(LuceneErrorKind::Io(io_exception(
                format!(
                    "MockDirectoryWrapper has crashed; cannot write to {}",
                    self.cell.name
                ),
            ))));
        }
        Ok(())
    }

    /// `checkDiskFull(b, offset, in, len)`: when the write would exceed the
    /// directory's maximum size, writes what still fits (from `b`, or from
    /// `input` when `b` is `None`), records the peak usage, and throws
    /// `IOException("fake disk full at …")`.
    fn check_disk_full(
        &self,
        b: Option<(&[u8], usize)>,
        input: Option<&mut dyn DataInput>,
        len: i64,
    ) -> Result<()> {
        let dir = self.dir();
        let max_size = dir.get_max_size_in_bytes();
        let mut free_space = if max_size == 0 {
            0
        } else {
            max_size - dir.size_in_bytes()?
        };
        let mut real_usage = 0i64;

        // Enforce disk full:
        if max_size != 0 && free_space <= len {
            // Compute the real disk free.  This will greatly slow
            // down our test but makes it more accurate:
            real_usage = dir.size_in_bytes()?;
            free_space = max_size - real_usage;
        }

        if max_size != 0 && free_space <= len {
            if free_space > 0 {
                real_usage += free_space;
                let mut inner = self.cell.lock();
                match (b, input) {
                    (Some((b, offset)), _) => {
                        inner
                            .out
                            .write_bytes(b, offset, free_space as i32 as usize)?
                    }
                    (None, Some(input)) => {
                        inner.out.copy_bytes(input, i64::from(free_space as i32))?
                    }
                    (None, None) => unreachable!("INVARIANT: checkDiskFull has bytes or an input"),
                }
            }
            if real_usage > dir.get_max_used_size_in_bytes() {
                dir.set_max_used_size_in_bytes(real_usage);
            }
            let size = dir.size_in_bytes()?;
            let file_pointer = self.cell.lock().out.file_pointer();
            let mut message = format!(
                "fake disk full at {size} bytes when writing {} (file length={file_pointer}",
                self.cell.name
            );
            if free_space > 0 {
                message.push_str(&format!("; wrote {free_space} of {len} bytes"));
            }
            message.push(')');
            if *VERBOSE {
                println!(
                    "{}: MDW: now throw fake disk full",
                    std::thread::current().name().unwrap_or("main")
                );
                println!("java.lang.Throwable");
            }
            return Err(LuceneError::from(LuceneErrorKind::Io(io_exception(
                message,
            ))));
        }
        Ok(())
    }

    /// `ensureOpen()`: `AlreadyClosedException("Already closed: " + this)`.
    fn ensure_open(&self) -> Result<()> {
        if self.cell.lock().closed {
            return Err(LuceneError::already_closed(format!(
                "Already closed: {}",
                self.cell.resource_description
            )));
        }
        Ok(())
    }
}

impl DataOutput for MockIndexOutputWrapper {
    /// `writeByte(b)`: `writeBytes(singleByte, 0, 1)`.
    fn write_byte(&mut self, b: u8) -> Result<()> {
        let single_byte = [b];
        self.write_bytes(&single_byte, 0, 1)
    }

    /// `writeBytes(b, offset, len)`: after the open, crash and disk-full
    /// checks, writes the bytes (once in 200 draws in two halves, with a
    /// `Thread.yield()` between), runs `maybeThrowDeterministicException`,
    /// and on the first write to the file maybe throws a random
    /// `IOException`.
    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.ensure_open()?;
        self.check_crashed()?;
        self.check_disk_full(Some((b, offset)), None, len as i64)?;

        if self.dir().random_state().next_int_bound(200) == 0 {
            let half = len / 2;
            self.cell.lock().out.write_bytes(b, offset, half)?;
            std::thread::yield_now();
            self.cell
                .lock()
                .out
                .write_bytes(b, offset + half, len - half)?;
        } else {
            self.cell.lock().out.write_bytes(b, offset, len)?;
        }

        self.dir().maybe_throw_deterministic_exception()?;

        if self.first {
            // Maybe throw random exception; only do this on first
            // write to a new file:
            self.first = false;
            self.dir().maybe_throw_io_exception(Some(&self.cell.name))?;
        }
        Ok(())
    }

    /// `copyBytes(input, numBytes)`: after the open, crash and disk-full
    /// checks, copies through the delegate and runs
    /// `maybeThrowDeterministicException`.
    fn copy_bytes(&mut self, input: &mut dyn DataInput, num_bytes: i64) -> Result<()> {
        self.ensure_open()?;
        self.check_crashed()?;
        self.check_disk_full(None, Some(&mut *input), num_bytes)?;
        self.cell.lock().out.copy_bytes(input, num_bytes)?;
        self.dir().maybe_throw_deterministic_exception()
    }
}

impl IndexOutput for MockIndexOutputWrapper {
    fn close(&mut self) -> Result<()> {
        let result = match &self.dir {
            Some(dir) => close_output(&self.cell, dir),
            // Closed before and released the directory: only close the
            // delegate again.
            None => self.cell.lock().out.close(),
        };
        if self.cell.lock().closed {
            self.dir = None;
        }
        result
    }

    fn file_pointer(&self) -> i64 {
        self.cell.lock().out.file_pointer()
    }

    fn checksum(&self) -> Result<i64> {
        self.cell.lock().out.checksum()
    }

    fn resource_description(&self) -> &str {
        &self.cell.resource_description
    }

    fn name(&self) -> &str {
        &self.out_name
    }
}

impl std::fmt::Debug for MockIndexOutputWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.cell.resource_description)
    }
}
