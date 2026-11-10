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

//! Port of `org.apache.lucene.tests.store.MockDirectoryWrapper`: a directory
//! wrapper that tracks open files, injects random and deterministic
//! failures, simulates a full disk and a machine crash, and runs `CheckIndex`
//! on close.
//!
//! # Shape of the port
//!
//! Java's wrapper is one object that the streams it hands out keep a
//! reference to (`MockIndexInputWrapper.dir`, `MockIndexOutputWrapper.dir`),
//! and that `close()` passes to `CheckIndex` and `IndexWriter` as `this`.
//! Here [`MockDirectoryWrapper`] is a handle over shared state: the streams
//! hold handles of their own, and [`MockDirectoryWrapper::as_directory`]
//! gives the `Arc<dyn Directory>` that stands for `this`. Every `synchronized`
//! method enters one re-entrant monitor, as Java's methods lock the
//! wrapper's monitor.
//!
//! # Declared divergences
//!
//! * **Closing the delegate.** `close()` ends with `IOUtils.close(in)`. Rust
//!   closes a directory through `&mut`, so the delegate is closed only when
//!   the wrapper's state is no longer shared: every stream it opened closed
//!   or dropped (a closed input or output releases its handle; a clone keeps
//!   one until dropped) and no [`as_directory`](MockDirectoryWrapper::as_directory)
//!   copy alive. Otherwise a successful `close()` returns the
//!   `IllegalStateException` [`BaseDirectoryWrapper`](super::BaseDirectoryWrapper)
//!   returns for a shared delegate, and a failed one (where Java uses
//!   `IOUtils.closeWhileHandlingException`) leaves it open.
//! * **`openFileHandles` order.** Java's map is an `IdentityHashMap`, whose
//!   iteration order (the first open handle's trace, chosen as the cause of
//!   the close-time `RuntimeException`) depends on identity hash codes and
//!   is not reproducible; the port iterates in opening order.
//! * **`fillOpenTrace`.** Java attaches the trace of the handle that opened a
//!   file as the cause of an `IOException`, `AccessDeniedException` or
//!   `AssertionError`. Those Rust errors carry no cause, so the message is
//!   kept and the cause dropped.
//! * **`crash()` and open inputs.** Java's `crash()` closes every open
//!   input, which closes its delegate. A Rust input owns its delegate, so
//!   `crash()` marks it closed (later calls report `Abusing closed
//!   IndexInput!`), unregisters it and runs `maybeThrowDeterministicException`
//!   as Java's `close()` does, and the delegate is closed when the input is
//!   next closed (Java's double close also calls `in.close()`) or dropped.
//!   Open outputs are closed in full, delegate included.
//! * **`Failure.reset()`** returns nothing (Java returns `this` for
//!   chaining) and [`Failure`] is a trait whose `doFail` field is reached
//!   through [`Failure::do_fail_flag`].

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::ThreadId;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::error::{LuceneError, Result};
use rucene::index::check_index::Level;
use rucene::index::directory_reader::{self, index_exists};
use rucene::index::index_file_names::{is_codec_file, PENDING_SEGMENTS, SEGMENTS};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::NoDeletionPolicy;
use rucene::store::java_nio_file::{
    access_denied_exception, file_already_exists_exception, io_exception, no_such_file_exception,
    JavaIoException,
};
use rucene::store::{
    hints_of, BufferedChecksumIndexInput, Context, Directory, IOContext, IndexInput, IndexOutput,
    Lock, ReadOnceHint,
};
use rucene::util::java_collections::{java_compare_to, JavaHashMap, JavaHashSet};

use super::mock_index_input_wrapper::{InputCtl, MockIndexInputWrapper};
use super::mock_index_output_wrapper::{close_output, MockIndexOutputWrapper, OutputCell};
use super::slow_closing_mock_index_input_wrapper::SlowClosingMockIndexInputWrapper;
use super::slow_opening_mock_index_input_wrapper::SlowOpeningMockIndexInputWrapper;
use crate::util::lucene_test_case::{LuceneTestCase, TEST_NIGHTLY, VERBOSE};
use crate::util::{TestUtil, ThrottledIndexOutput};

/// Port of the enum `MockDirectoryWrapper.Throttling`: controls hard disk
/// throttling, set through [`MockDirectoryWrapper::set_throttling`].
///
/// WARNING: can make tests very slow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Throttling {
    /// `ALWAYS`: always emulate a slow hard disk. Could be very slow!
    Always,
    /// `SOMETIMES`: sometimes (0.5% of the time) emulate a slow hard disk.
    Sometimes,
    /// `NEVER`: never throttle output.
    Never,
}

impl fmt::Display for Throttling {
    /// `Enum.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Throttling::Always => "ALWAYS",
            Throttling::Sometimes => "SOMETIMES",
            Throttling::Never => "NEVER",
        })
    }
}

/// `MockDirectoryWrapper.FakeIOException` (`extends IOException`, message
/// `null`): the class name, binary form.
pub const FAKE_IO_EXCEPTION: &str =
    "org.apache.lucene.tests.store.MockDirectoryWrapper$FakeIOException";

/// `new FakeIOException()`: use this when throwing a fake `IOException`, for
/// example from a [`Failure`].
pub fn fake_io_exception() -> LuceneError {
    LuceneError::Io(JavaIoException::new(FAKE_IO_EXCEPTION, None).into_io_error())
}

/// Port of `MockDirectoryWrapper.Failure`: a fail-able condition registered
/// with [`MockDirectoryWrapper::fail_on`]; after registration it is evaluated
/// at every potential failure point
/// (`maybeThrowDeterministicException`), giving it a chance to throw.
///
/// An implementation keeps Java's `protected boolean doFail` in an
/// [`AtomicBool`] it returns from [`do_fail_flag`](Self::do_fail_flag);
/// [`PlainFailure`] is Java's `new Failure()`.
pub trait Failure: Send + Sync {
    /// `eval(dir)`: called at every potential failure point; an error is
    /// thrown from that point. The default does nothing.
    ///
    /// # Errors
    ///
    /// The error this failure injects.
    fn eval(&self, dir: &MockDirectoryWrapper) -> Result<()> {
        let _ = dir;
        Ok(())
    }

    /// `reset()`: sets this failure back to its freshly constructed state.
    /// The default does nothing.
    fn reset(&self) {}

    /// The `doFail` field.
    fn do_fail_flag(&self) -> &AtomicBool;

    /// `setDoFail()`.
    fn set_do_fail(&self) {
        self.do_fail_flag().store(true, Ordering::SeqCst);
    }

    /// `clearDoFail()`.
    fn clear_do_fail(&self) {
        self.do_fail_flag().store(false, Ordering::SeqCst);
    }

    /// Reads `doFail`.
    fn do_fail(&self) -> bool {
        self.do_fail_flag().load(Ordering::SeqCst)
    }
}

/// `new MockDirectoryWrapper.Failure()`: a failure whose `eval` never
/// throws.
#[derive(Debug, Default)]
pub struct PlainFailure {
    do_fail: AtomicBool,
}

impl PlainFailure {
    /// `new Failure()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Failure for PlainFailure {
    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// The private enum `MockDirectoryWrapper.Handle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Handle {
    Input,
    Output,
    #[allow(dead_code)]
    Slice,
}

impl Handle {
    /// `Enum.name()`.
    fn name(self) -> &'static str {
        match self {
            Handle::Input => "Input",
            Handle::Output => "Output",
            Handle::Slice => "Slice",
        }
    }
}

/// What `crash()` closes for one entry of `openFileHandles`.
#[derive(Clone)]
pub(crate) enum Closer {
    Input(Arc<InputCtl>),
    Output(Arc<OutputCell>),
}

/// One entry of `openFileHandles`: the stream and the
/// `RuntimeException("unclosed Index<handle>: <name>")` recorded when it was
/// opened.
#[derive(Clone)]
struct OpenHandle {
    closer: Closer,
    trace: Arc<LuceneError>,
}

/// A re-entrant monitor: what `synchronized` methods of one object lock.
struct Monitor {
    state: Mutex<(Option<ThreadId>, usize)>,
    released: Condvar,
}

struct MonitorGuard<'a>(&'a Monitor);

impl Monitor {
    fn new() -> Self {
        Monitor {
            state: Mutex::new((None, 0)),
            released: Condvar::new(),
        }
    }

    fn enter(&self) -> MonitorGuard<'_> {
        let me = std::thread::current().id();
        let mut state = lock(&self.state);
        loop {
            match state.0 {
                None => {
                    *state = (Some(me), 1);
                    break;
                }
                Some(owner) if owner == me => {
                    state.1 += 1;
                    break;
                }
                Some(_) => {
                    state = self
                        .released
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
        }
        MonitorGuard(self)
    }
}

impl Drop for MonitorGuard<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.0.state);
        state.1 -= 1;
        if state.1 == 0 {
            state.0 = None;
            self.0.released.notify_one();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The collections the wrapper's `synchronized` methods guard.
struct State {
    un_synced_files: JavaHashSet,
    created_files: JavaHashSet,
    open_files_for_write: JavaHashSet,
    /// `openLocks` (a `ConcurrentHashMap<String, RuntimeException>`). Lucene
    /// 10.5.0 never adds to it; `close()` checks it and `clearCrash()`
    /// clears it.
    open_locks: JavaHashMap<Arc<LuceneError>>,
    /// `openFileHandles`, by opening order (see the module docs).
    open_file_handles: BTreeMap<u64, OpenHandle>,
    open_files: JavaHashMap<i32>,
    open_files_deleted: JavaHashSet,
}

/// The state every handle of one wrapper shares.
pub(crate) struct Core {
    delegate: Arc<dyn Directory>,
    monitor: Monitor,
    state: Mutex<State>,
    failures: Mutex<Option<Vec<Arc<dyn Failure>>>>,
    random_state: JdkRandom,
    /// `throttledOutput`'s settings: `bytesPerSecond` and the delay (Java
    /// keeps a `ThrottledIndexOutput` over a `null` delegate as the template
    /// `newFromDelegate` copies; Rust has no null output).
    throttle_bytes_per_second: i32,
    throttle_delay_millis: i64,
    // BaseDirectoryWrapper's state.
    is_open: Arc<AtomicBool>,
    check_index_on_close: AtomicBool,
    level_for_check_on_close: AtomicI32,
    // MockDirectoryWrapper's fields.
    max_size: AtomicI64,
    max_used_size: AtomicI64,
    random_io_exception_rate: AtomicU64,
    random_io_exception_rate_on_open: AtomicU64,
    assert_no_delete_open_file: AtomicBool,
    track_disk_usage: AtomicBool,
    use_slow_open_closers: AtomicBool,
    allow_random_file_not_found_exception: AtomicBool,
    allow_reading_files_still_open_for_write: AtomicBool,
    crashed: AtomicBool,
    throttling: Mutex<Throttling>,
    always_corrupt: AtomicBool,
    input_clone_count: AtomicI32,
    verbose_clone: AtomicBool,
    fail_on_create_output: AtomicBool,
    fail_on_open_input: AtomicBool,
    assert_no_unreferenced_files_on_close: AtomicBool,
    next_handle_id: AtomicU64,
}

/// Port of `org.apache.lucene.tests.store.MockDirectoryWrapper` (`extends
/// BaseDirectoryWrapper`): a directory wrapper for tests.
///
/// * When closed, it fails if any file it opened is still open, with the
///   trace of where it was opened as the cause.
/// * When closed, it runs `CheckIndex` to test if the index was corrupted
///   (after simulating a crash of the unsynced files).
/// * It simulates some "features" of Windows, such as refusing to write or
///   delete open files.
///
/// See the module docs for the shape of the port and its divergences.
pub struct MockDirectoryWrapper {
    core: Arc<Core>,
}

impl MockDirectoryWrapper {
    /// `MockDirectoryWrapper(random, delegate)`: a private
    /// `new Random(random.nextInt())` (the methods are called from several
    /// threads), and the throttled output template
    /// `ThrottledIndexOutput(mBitsToBytes(40 + randomState.nextInt(10)), 1 +
    /// randomState.nextInt(5), null)`.
    pub fn new(random: &dyn Random, delegate: Arc<dyn Directory>) -> Self {
        // must make a private random since our methods are
        // called from different threads; else test failures may
        // not be reproducible from the original seed
        let random_state = JdkRandom::new(i64::from(random.next_int()));
        let bytes_per_second =
            ThrottledIndexOutput::m_bits_to_bytes(40 + random_state.next_int_bound(10));
        let delay = i64::from(1 + random_state.next_int_bound(5));
        // `ThrottledIndexOutput`'s constructor: `assert bytesPerSecond > 0`.
        randomizedtesting::jdk::exception::java_assert_no_message(bytes_per_second > 0);
        let nightly = *TEST_NIGHTLY;
        MockDirectoryWrapper {
            core: Arc::new(Core {
                delegate,
                monitor: Monitor::new(),
                state: Mutex::new(State {
                    un_synced_files: JavaHashSet::new(),
                    created_files: JavaHashSet::new(),
                    open_files_for_write: JavaHashSet::new(),
                    open_locks: JavaHashMap::new(),
                    open_file_handles: BTreeMap::new(),
                    open_files: JavaHashMap::new(),
                    open_files_deleted: JavaHashSet::new(),
                }),
                failures: Mutex::new(None),
                random_state,
                throttle_bytes_per_second: bytes_per_second,
                throttle_delay_millis: delay,
                is_open: Arc::new(AtomicBool::new(true)),
                check_index_on_close: AtomicBool::new(true),
                level_for_check_on_close: AtomicI32::new(Level::MIN_LEVEL_FOR_SLOW_CHECKS),
                max_size: AtomicI64::new(0),
                max_used_size: AtomicI64::new(0),
                random_io_exception_rate: AtomicU64::new(0f64.to_bits()),
                random_io_exception_rate_on_open: AtomicU64::new(0f64.to_bits()),
                assert_no_delete_open_file: AtomicBool::new(false),
                track_disk_usage: AtomicBool::new(false),
                use_slow_open_closers: AtomicBool::new(nightly),
                allow_random_file_not_found_exception: AtomicBool::new(true),
                allow_reading_files_still_open_for_write: AtomicBool::new(false),
                crashed: AtomicBool::new(false),
                throttling: Mutex::new(if nightly {
                    Throttling::Sometimes
                } else {
                    Throttling::Never
                }),
                always_corrupt: AtomicBool::new(false),
                input_clone_count: AtomicI32::new(0),
                verbose_clone: AtomicBool::new(false),
                fail_on_create_output: AtomicBool::new(true),
                fail_on_open_input: AtomicBool::new(true),
                assert_no_unreferenced_files_on_close: AtomicBool::new(false),
                next_handle_id: AtomicU64::new(0),
            }),
        }
    }

    /// Another handle over the same wrapper (Java's `this`, held by a stream
    /// or passed to a `Failure`).
    pub(crate) fn handle(&self) -> MockDirectoryWrapper {
        MockDirectoryWrapper {
            core: Arc::clone(&self.core),
        }
    }

    /// This wrapper as the `Arc<dyn Directory>` that Java's `this` is where
    /// `close()` hands it to `CheckIndex`, `SegmentInfos.readCommit`,
    /// `DirectoryReader.open` and `IndexWriter`, and where a test opens a
    /// writer or a reader over it.
    ///
    /// It has no counterpart in Lucene. A copy alive when the wrapper is
    /// closed keeps the delegate from being closed (see the module docs).
    pub fn as_directory(&self) -> Arc<dyn Directory> {
        Arc::new(self.handle())
    }

    pub(crate) fn next_handle_id(&self) -> u64 {
        self.core.next_handle_id.fetch_add(1, Ordering::SeqCst)
    }

    // -------------------------------------------------------------------------
    // BaseDirectoryWrapper
    // -------------------------------------------------------------------------

    /// `getDelegate()`: the wrapped directory, Java's `in`.
    pub fn get_delegate(&self) -> &Arc<dyn Directory> {
        &self.core.delegate
    }

    /// `isOpen()`.
    pub fn is_open(&self) -> bool {
        self.core.is_open.load(Ordering::SeqCst)
    }

    /// A probe of [`is_open`](Self::is_open) that, unlike a copy of the
    /// wrapper, does not keep its delegate from being closed (see the module
    /// docs), for [`CloseableDirectory`](crate::util::CloseableDirectory).
    pub fn open_probe(&self) -> crate::util::closeable_directory::DirectoryOpenProbe {
        crate::util::closeable_directory::DirectoryOpenProbe::new(
            Arc::clone(&self.core.is_open),
            self.to_string(),
        )
    }

    /// `setCheckIndexOnClose(value)`: whether `CheckIndex` runs on close.
    pub fn set_check_index_on_close(&self, value: bool) {
        self.core
            .check_index_on_close
            .store(value, Ordering::SeqCst);
    }

    /// `getCheckIndexOnClose()`.
    pub fn get_check_index_on_close(&self) -> bool {
        self.core.check_index_on_close.load(Ordering::SeqCst)
    }

    /// `setCrossCheckTermVectorsOnClose(value)`: `true` enables the slow
    /// checks (`MIN_LEVEL_FOR_SLOW_CHECKS`), `false` keeps the integrity
    /// checks (`MIN_LEVEL_FOR_INTEGRITY_CHECKS`).
    pub fn set_cross_check_term_vectors_on_close(&self, value: bool) {
        let level = if value {
            Level::MIN_LEVEL_FOR_SLOW_CHECKS
        } else {
            Level::MIN_LEVEL_FOR_INTEGRITY_CHECKS
        };
        self.core
            .level_for_check_on_close
            .store(level, Ordering::SeqCst);
    }

    /// `getLevelForCheckOnClose()`.
    pub fn get_level_for_check_on_close(&self) -> i32 {
        self.core.level_for_check_on_close.load(Ordering::SeqCst)
    }

    // -------------------------------------------------------------------------
    // Settings
    // -------------------------------------------------------------------------

    /// `getInputCloneCount()`.
    pub fn get_input_clone_count(&self) -> i32 {
        self.core.input_clone_count.load(Ordering::SeqCst)
    }

    pub(crate) fn increment_input_clone_count(&self) {
        self.core.input_clone_count.fetch_add(1, Ordering::SeqCst);
    }

    /// `setVerboseClone(v)`: print a fake exception with the file name and a
    /// stack trace on every `IndexInput` clone.
    pub fn set_verbose_clone(&self, v: bool) {
        self.core.verbose_clone.store(v, Ordering::SeqCst);
    }

    pub(crate) fn verbose_clone(&self) -> bool {
        self.core.verbose_clone.load(Ordering::SeqCst)
    }

    /// `setTrackDiskUsage(v)`.
    pub fn set_track_disk_usage(&self, v: bool) {
        self.core.track_disk_usage.store(v, Ordering::SeqCst);
    }

    pub(crate) fn track_disk_usage(&self) -> bool {
        self.core.track_disk_usage.load(Ordering::SeqCst)
    }

    /// `setAllowRandomFileNotFoundException(value)`: when `true` (the
    /// default), a random exception on `openInput` or `createOutput` may be
    /// a `FileNotFoundException` or a `NoSuchFileException`.
    pub fn set_allow_random_file_not_found_exception(&self, value: bool) {
        self.core
            .allow_random_file_not_found_exception
            .store(value, Ordering::SeqCst);
    }

    /// `setAllowReadingFilesStillOpenForWrite(value)`: when `true`, a file
    /// still open for writing can be opened for reading.
    pub fn set_allow_reading_files_still_open_for_write(&self, value: bool) {
        self.core
            .allow_reading_files_still_open_for_write
            .store(value, Ordering::SeqCst);
    }

    /// `setThrottling(throttling)`.
    pub fn set_throttling(&self, throttling: Throttling) {
        *lock(&self.core.throttling) = throttling;
    }

    /// `setUseSlowOpenClosers(v)`: add a rare small sleep to catch race
    /// conditions in open and close.
    pub fn set_use_slow_open_closers(&self, v: bool) {
        self.core.use_slow_open_closers.store(v, Ordering::SeqCst);
    }

    /// `setMaxSizeInBytes(maxSize)`.
    pub fn set_max_size_in_bytes(&self, max_size: i64) {
        self.core.max_size.store(max_size, Ordering::SeqCst);
    }

    /// `getMaxSizeInBytes()`.
    pub fn get_max_size_in_bytes(&self) -> i64 {
        self.core.max_size.load(Ordering::SeqCst)
    }

    /// `getMaxUsedSizeInBytes()`: the peak actual storage used (bytes) in
    /// this directory.
    pub fn get_max_used_size_in_bytes(&self) -> i64 {
        self.core.max_used_size.load(Ordering::SeqCst)
    }

    pub(crate) fn set_max_used_size_in_bytes(&self, value: i64) {
        self.core.max_used_size.store(value, Ordering::SeqCst);
    }

    /// `resetMaxUsedSizeInBytes()`: `maxUsedSize = sizeInBytes()`.
    ///
    /// # Errors
    ///
    /// What listing the delegate raises.
    pub fn reset_max_used_size_in_bytes(&self) -> Result<()> {
        let size = self.size_in_bytes()?;
        self.set_max_used_size_in_bytes(size);
        Ok(())
    }

    /// `setAssertNoDeleteOpenFile(value)`: trip a test assert if there is an
    /// attempt to delete an open file.
    pub fn set_assert_no_delete_open_file(&self, value: bool) {
        self.core
            .assert_no_delete_open_file
            .store(value, Ordering::SeqCst);
    }

    /// `getAssertNoDeleteOpenFile()`.
    pub fn get_assert_no_delete_open_file(&self) -> bool {
        self.core.assert_no_delete_open_file.load(Ordering::SeqCst)
    }

    /// `setRandomIOExceptionRate(rate)`: if 0.0, no exceptions are thrown;
    /// else a rate in [0.0, 1.0] at which the first write to an output throws
    /// a random `IOException`.
    pub fn set_random_io_exception_rate(&self, rate: f64) {
        self.core
            .random_io_exception_rate
            .store(rate.to_bits(), Ordering::SeqCst);
    }

    /// `getRandomIOExceptionRate()`.
    pub fn get_random_io_exception_rate(&self) -> f64 {
        f64::from_bits(self.core.random_io_exception_rate.load(Ordering::SeqCst))
    }

    /// `setRandomIOExceptionRateOnOpen(rate)`: if 0.0, no exceptions are
    /// thrown during `openInput` and `createOutput`; else the rate at which
    /// they throw a random `IOException`.
    pub fn set_random_io_exception_rate_on_open(&self, rate: f64) {
        self.core
            .random_io_exception_rate_on_open
            .store(rate.to_bits(), Ordering::SeqCst);
    }

    /// `getRandomIOExceptionRateOnOpen()`.
    pub fn get_random_io_exception_rate_on_open(&self) -> f64 {
        f64::from_bits(
            self.core
                .random_io_exception_rate_on_open
                .load(Ordering::SeqCst),
        )
    }

    /// `setFailOnCreateOutput(v)`.
    pub fn set_fail_on_create_output(&self, v: bool) {
        self.core.fail_on_create_output.store(v, Ordering::SeqCst);
    }

    /// `setFailOnOpenInput(v)`.
    pub fn set_fail_on_open_input(&self, v: bool) {
        self.core.fail_on_open_input.store(v, Ordering::SeqCst);
    }

    /// `setAssertNoUnrefencedFilesOnClose(v)` (Java's spelling): off by
    /// default (LUCENE-5574).
    pub fn set_assert_no_unrefenced_files_on_close(&self, v: bool) {
        self.core
            .assert_no_unreferenced_files_on_close
            .store(v, Ordering::SeqCst);
    }

    /// The package-private field `alwaysCorrupt` ("for testing"): when set,
    /// `corruptFiles` never leaves a file unchanged.
    pub fn set_always_corrupt(&self, value: bool) {
        self.core.always_corrupt.store(value, Ordering::SeqCst);
    }

    /// The package-private field `randomState`, the wrapper's private
    /// `Random`, which every random choice it makes draws from.
    pub fn random_state(&self) -> &dyn Random {
        &self.core.random_state
    }

    /// The package-private field `crashed`.
    pub fn is_crashed(&self) -> bool {
        self.core.crashed.load(Ordering::SeqCst)
    }

    // -------------------------------------------------------------------------
    // Disk usage, crash and corruption
    // -------------------------------------------------------------------------

    /// `sizeInBytes()`: the sum of the delegate's file lengths, files whose
    /// name starts with `extra` excluded (`ExtrasFS`).
    ///
    /// # Errors
    ///
    /// What listing the delegate or reading a length raises.
    pub fn size_in_bytes(&self) -> Result<i64> {
        let _monitor = self.core.monitor.enter();
        let mut size = 0i64;
        for file in self.core.delegate.list_all()? {
            // hack 2: see TODO in ExtrasFS (ideally it would always return 0 byte
            // size for extras it creates, even though the size of non-regular files is not defined)
            if !file.starts_with("extra") {
                size += self.core.delegate.file_length(&file)?;
            }
        }
        Ok(size)
    }

    /// `corruptUnknownFiles()`: corrupts the codec and `pending_segments`
    /// files no commit references (`write.lock` excluded).
    ///
    /// # Errors
    ///
    /// What reading a commit raises, and what [`corrupt_files`](Self::corrupt_files)
    /// raises.
    pub fn corrupt_unknown_files(&self) -> Result<()> {
        let _monitor = self.core.monitor.enter();
        if *VERBOSE {
            println!("MDW: corrupt unknown files");
        }
        let mut known_files = std::collections::HashSet::new();
        for file_name in self.list_all()? {
            if file_name.starts_with(SEGMENTS) {
                if *VERBOSE {
                    println!("MDW: read {file_name} to gather files it references");
                }
                let infos = match SegmentInfos::read_commit(self.as_directory(), &file_name) {
                    Ok(infos) => infos,
                    Err(e) => {
                        if *VERBOSE {
                            println!(
                                "MDW: exception reading segment infos {file_name}; files: [{}]",
                                self.list_all()?.join(", ")
                            );
                        }
                        return Err(e);
                    }
                };
                known_files.extend(infos.files(true)?);
            }
        }

        let mut to_corrupt = Vec::new();
        for file_name in self.list_all()? {
            if !known_files.contains(&file_name)
                && !file_name.ends_with("write.lock")
                && (is_codec_file(&file_name) || file_name.starts_with(PENDING_SEGMENTS))
            {
                to_corrupt.push(file_name);
            }
        }

        self.corrupt_files(&to_corrupt)
    }

    /// `corruptFiles(files)`: with the virus checker disabled, gives each
    /// file (in sorted order) one random damage: deleted, zeroed, partially
    /// truncated, unchanged, one bit flipped, or fully truncated.
    ///
    /// # Errors
    ///
    /// `RuntimeException` ("hit unexpected IOException while trying to
    /// corrupt file …") for an I/O failure while rewriting a file, and what
    /// deleting a file raises.
    pub fn corrupt_files(&self, files: &[String]) -> Result<()> {
        let _monitor = self.core.monitor.enter();
        let disabled = TestUtil::disable_virus_checker(self.core.delegate.as_ref())?;
        let result = self.corrupt_files_internal(files);
        if disabled {
            TestUtil::enable_virus_checker(self.core.delegate.as_ref())?;
        }
        result
    }

    /// `_corruptFiles(files)`.
    fn corrupt_files_internal(&self, files: &[String]) -> Result<()> {
        let _monitor = self.core.monitor.enter();
        // TODO: we should also mess with any recent file renames, file deletions, if
        // syncMetaData was not called!!

        // Must make a copy because we change the incoming unsyncedFiles
        // when we create temp files, delete, etc., below:
        let mut files_to_corrupt = files.to_vec();
        // sort the files otherwise we have reproducibility issues
        // across JVMs if the incoming collection is a hashSet etc.
        files_to_corrupt.sort_by(|a, b| java_compare_to(a, b));
        let random = &self.core.random_state;
        let delegate = &self.core.delegate;
        for name in &files_to_corrupt {
            let mut damage = random.next_int_bound(6);
            if self.core.always_corrupt.load(Ordering::SeqCst) && damage == 3 {
                damage = 4;
            }
            let action: String;
            let unexpected = |e: LuceneError| -> LuceneError {
                if e.is_io_exception() {
                    LuceneError::Unchecked {
                        class: "java.lang.RuntimeException",
                        message: Some(format!(
                            "hit unexpected IOException while trying to corrupt file {name}"
                        )),
                        cause: Some(Arc::new(e)),
                    }
                } else {
                    e
                }
            };

            match damage {
                0 => {
                    action = "deleted".to_string();
                    self.delete_file(name)?;
                }
                1 => {
                    action = "zeroed".to_string();
                    // Zero out file entirely
                    let length = self.file_length(name).map_err(unexpected)?;

                    // Delete original and write zeros back:
                    self.delete_file(name)?;

                    let zeroes = [0u8; 256];
                    let mut upto = 0i64;
                    let context = LuceneTestCase::new_io_context(random);
                    let mut out = delegate
                        .create_output(name, &*context)
                        .map_err(unexpected)?;
                    let mut written = || -> Result<()> {
                        while upto < length {
                            let limit = (length - upto).min(zeroes.len() as i64) as usize;
                            out.write_bytes(&zeroes, 0, limit)?;
                            upto += limit as i64;
                        }
                        Ok(())
                    };
                    let body = written();
                    let closed = out.close();
                    try_with_resources(body, [closed]).map_err(unexpected)?;
                }
                2 => {
                    action = "partially truncated".to_string();
                    // Partially Truncate the file:

                    // First, make temp file and copy only half this
                    // file over:
                    let temp_file_name = self
                        .copy_to_temp(name, |temp_out, ii| {
                            let half = ii.length() / 2;
                            temp_out.copy_bytes(ii, half)
                        })
                        .map_err(unexpected)?;

                    // Delete original and copy bytes back:
                    self.delete_file(name)?;
                    self.copy_back(name, &temp_file_name).map_err(unexpected)?;
                    self.delete_file(&temp_file_name)?;
                }
                3 => {
                    // The file survived intact:
                    action = "didn't change".to_string();
                }
                4 => {
                    // Corrupt one bit randomly in the file:
                    let mut flipped = "didn't change".to_string();
                    let temp_file_name = self
                        .copy_to_temp(name, |temp_out, ii| {
                            let length = ii.length();
                            if length > 0 {
                                // Copy first part unchanged:
                                let byte_to_corrupt = (random.next_double() * length as f64) as i64;
                                if byte_to_corrupt > 0 {
                                    temp_out.copy_bytes(ii, byte_to_corrupt)?;
                                }

                                // Randomly flip one bit from this byte:
                                let mut b = ii.read_byte()?;
                                let bit_to_flip = random.next_int_bound(8);
                                b ^= 1u8 << bit_to_flip;
                                temp_out.write_byte(b)?;

                                flipped = format!(
                                    "flip bit {bit_to_flip} of byte {byte_to_corrupt} out of {length} bytes"
                                );

                                // Copy last part unchanged:
                                let bytes_left = length - byte_to_corrupt - 1;
                                if bytes_left > 0 {
                                    temp_out.copy_bytes(ii, bytes_left)?;
                                }
                            } else {
                                flipped = "didn't change".to_string();
                            }
                            Ok(())
                        })
                        .map_err(unexpected)?;
                    action = flipped;

                    // Delete original and copy bytes back:
                    self.delete_file(name)?;
                    self.copy_back(name, &temp_file_name).map_err(unexpected)?;
                    self.delete_file(&temp_file_name)?;
                }
                5 => {
                    action = "fully truncated".to_string();
                    // Totally truncate the file to zero bytes
                    self.delete_file(name)?;

                    let context = LuceneTestCase::new_io_context(random);
                    let mut out = delegate
                        .create_output(name, &*context)
                        .map_err(unexpected)?;
                    let _ = out.file_pointer(); // just fake access to prevent compiler warning
                    out.close().map_err(unexpected)?;
                }
                _ => return Err(LuceneError::AssertionError(String::new())),
            }

            if *VERBOSE {
                println!("MockDirectoryWrapper: {action} unsynced file: {name}");
            }
        }
        Ok(())
    }

    /// The first `try` of damages 2 and 4: `in.createTempOutput("name",
    /// "mdw_corrupt", newIOContext(randomState))` then `in.openInput(name,
    /// newIOContext(randomState))`, `body` over both, closed in reverse
    /// order; the temporary file's name.
    fn copy_to_temp(
        &self,
        name: &str,
        body: impl FnOnce(&mut dyn IndexOutput, &mut dyn IndexInput) -> Result<()>,
    ) -> Result<String> {
        let random = &self.core.random_state;
        let delegate = &self.core.delegate;
        let context = LuceneTestCase::new_io_context(random);
        let mut temp_out = delegate.create_temp_output("name", "mdw_corrupt", &*context)?;
        let context = LuceneTestCase::new_io_context(random);
        let mut ii = match delegate.open_input(name, &*context) {
            Ok(ii) => ii,
            Err(e) => {
                let closed = temp_out.close();
                return try_with_resources::<String, 1>(Err(e), [closed]);
            }
        };
        let temp_file_name = temp_out.name().to_string();
        let result = body(temp_out.as_mut(), ii.as_mut());
        let closed_in = ii.close();
        let closed_out = temp_out.close();
        try_with_resources(result, [closed_in, closed_out])?;
        Ok(temp_file_name)
    }

    /// The second `try` of damages 2 and 4: copies `temp_file_name` back to
    /// `name` in the delegate.
    fn copy_back(&self, name: &str, temp_file_name: &str) -> Result<()> {
        let random = &self.core.random_state;
        let delegate = &self.core.delegate;
        let context = LuceneTestCase::new_io_context(random);
        let mut out = delegate.create_output(name, &*context)?;
        let context = LuceneTestCase::new_io_context(random);
        let mut ii = match delegate.open_input(temp_file_name, &*context) {
            Ok(ii) => ii,
            Err(e) => {
                let closed = out.close();
                return try_with_resources(Err(e), [closed]);
            }
        };
        let length = ii.length();
        let result = out.copy_bytes(ii.as_mut(), length);
        let closed_in = ii.close();
        let closed_out = out.close();
        try_with_resources(result, [closed_in, closed_out])
    }

    /// `crash()`: simulates a crash of OS or machine by overwriting unsynced
    /// files. Force-closes every open file first (ignoring failures), then
    /// corrupts the unsynced files.
    ///
    /// # Errors
    ///
    /// What [`corrupt_files`](Self::corrupt_files) raises.
    pub fn crash(&self) -> Result<()> {
        let _monitor = self.core.monitor.enter();
        {
            let mut state = lock(&self.core.state);
            state.open_files = JavaHashMap::new();
            state.open_files_for_write = JavaHashSet::new();
            state.open_files_deleted = JavaHashSet::new();
        }
        // first force-close all files, so we can corrupt on windows etc.
        // clone the file map, as these guys want to remove themselves on close.
        let handles: Vec<OpenHandle> = lock(&self.core.state)
            .open_file_handles
            .values()
            .cloned()
            .collect();
        for handle in handles {
            let _ignored = match &handle.closer {
                Closer::Input(ctl) => ctl.close_from_crash(self),
                Closer::Output(cell) => close_output(cell, self),
            };
        }
        let un_synced: Vec<String> = lock(&self.core.state)
            .un_synced_files
            .iter()
            .cloned()
            .collect();
        self.corrupt_files(&un_synced)?;
        self.core.crashed.store(true, Ordering::SeqCst);
        lock(&self.core.state).un_synced_files = JavaHashSet::new();
        Ok(())
    }

    /// `clearCrash()`.
    pub fn clear_crash(&self) {
        let _monitor = self.core.monitor.enter();
        self.core.crashed.store(false, Ordering::SeqCst);
        lock(&self.core.state).open_locks.clear();
    }

    // -------------------------------------------------------------------------
    // Failure injection
    // -------------------------------------------------------------------------

    /// `maybeThrowIOException(message)`: throws `IOException("a random
    /// IOException (<message>)")` at the random IOException rate.
    ///
    /// # Errors
    ///
    /// The random `IOException`.
    pub(crate) fn maybe_throw_io_exception(&self, message: Option<&str>) -> Result<()> {
        if self.core.random_state.next_double() < self.get_random_io_exception_rate() {
            let suffix = message.map_or(String::new(), |m| format!(" ({m})"));
            let ioe = io_exception(format!("a random IOException{suffix}"));
            if *VERBOSE {
                println!(
                    "{}: MockDirectoryWrapper: now throw random exception{suffix}",
                    current_thread_name()
                );
                println!("java.io.IOException: a random IOException{suffix}");
            }
            return Err(LuceneError::Io(ioe));
        }
        Ok(())
    }

    /// `maybeThrowIOExceptionOnOpen(name)`: at the random-on-open rate,
    /// throws `IOException`, or (when allowed, half of the time)
    /// `FileNotFoundException` or `NoSuchFileException`, all with the
    /// message `a random IOException (<name>)`.
    fn maybe_throw_io_exception_on_open(&self, name: &str) -> Result<()> {
        let random = &self.core.random_state;
        if random.next_double() < self.get_random_io_exception_rate_on_open() {
            if *VERBOSE {
                println!(
                    "{}: MockDirectoryWrapper: now throw random exception during open file={name}",
                    current_thread_name()
                );
                println!("java.lang.Throwable");
            }
            let message = format!("a random IOException ({name})");
            if !self
                .core
                .allow_random_file_not_found_exception
                .load(Ordering::SeqCst)
                || random.next_boolean()
            {
                return Err(LuceneError::Io(io_exception(message)));
            } else {
                return Err(if random.next_boolean() {
                    file_not_found_exception(message)
                } else {
                    LuceneError::Io(no_such_file_exception(message))
                });
            }
        }
        Ok(())
    }

    /// `getFileHandleCount()`: the current open file handle count.
    pub fn get_file_handle_count(&self) -> i64 {
        let _monitor = self.core.monitor.enter();
        lock(&self.core.state).open_file_handles.len() as i64
    }

    /// `maybeYield()`: `Thread.yield()` half of the time.
    fn maybe_yield(&self) {
        if self.core.random_state.next_boolean() {
            std::thread::yield_now();
        }
    }

    /// `getOpenDeletedFiles()`: the open files a delete was attempted on.
    pub fn get_open_deleted_files(&self) -> std::collections::HashSet<String> {
        let _monitor = self.core.monitor.enter();
        lock(&self.core.state).open_files_deleted.to_std()
    }

    /// `failOn(fail)`: adds a [`Failure`] to the ones evaluated at every
    /// potential failure point.
    pub fn fail_on(&self, fail: Arc<dyn Failure>) {
        let _monitor = self.core.monitor.enter();
        lock(&self.core.failures)
            .get_or_insert_with(Vec::new)
            .push(fail);
    }

    /// `maybeThrowDeterministicException()`: gives each registered failure,
    /// in order, a chance to throw.
    ///
    /// # Errors
    ///
    /// The first failure's error, rethrown as is.
    pub(crate) fn maybe_throw_deterministic_exception(&self) -> Result<()> {
        let _monitor = self.core.monitor.enter();
        let mut i = 0;
        loop {
            let failure = match lock(&self.core.failures).as_ref() {
                Some(failures) if i < failures.len() => Arc::clone(&failures[i]),
                _ => return Ok(()),
            };
            if let Err(t) = failure.eval(self) {
                if *VERBOSE {
                    println!("MockDirectoryWrapper: throw exc");
                    println!("{t}");
                }
                return Err(t);
            }
            i += 1;
        }
    }

    // -------------------------------------------------------------------------
    // Open-file tracking
    // -------------------------------------------------------------------------

    /// `addFileHandle(c, name, handle)`.
    pub(crate) fn add_file_handle(&self, id: u64, closer: Closer, name: &str, handle: Handle) {
        let _monitor = self.core.monitor.enter();
        let mut state = lock(&self.core.state);
        let v = state.open_files.get(name).copied();
        match v {
            Some(v) => {
                state.open_files.insert(name.to_string(), v + 1);
            }
            None => {
                state.open_files.insert(name.to_string(), 1);
            }
        }
        let trace = Arc::new(runtime_exception(
            format!("unclosed Index{}: {name}", handle.name()),
            None,
        ));
        state
            .open_file_handles
            .insert(id, OpenHandle { closer, trace });
    }

    /// `removeOpenFile(c, name)`.
    fn remove_open_file(&self, id: u64, name: &str) {
        let _monitor = self.core.monitor.enter();
        let mut state = lock(&self.core.state);
        let v = state.open_files.get(name).copied();
        // Could be null when crash() was called
        if let Some(v) = v {
            if v == 1 {
                state.open_files.remove(name);
            } else {
                state.open_files.insert(name.to_string(), v - 1);
            }
        }
        state.open_file_handles.remove(&id);
    }

    /// `removeIndexOutput(out, name)`.
    pub fn remove_index_output(&self, out: &MockIndexOutputWrapper, name: &str) {
        self.remove_index_output_id(out.handle_id(), name);
    }

    pub(crate) fn remove_index_output_id(&self, id: u64, name: &str) {
        let _monitor = self.core.monitor.enter();
        lock(&self.core.state).open_files_for_write.remove(name);
        self.remove_open_file(id, name);
    }

    /// `removeIndexInput(in, name)`.
    pub fn remove_index_input(&self, input: &MockIndexInputWrapper, name: &str) {
        self.remove_index_input_id(input.handle_id(), name);
    }

    pub(crate) fn remove_index_input_id(&self, id: u64, name: &str) {
        self.remove_open_file(id, name);
    }

    /// `fillOpenTrace(t, name, input)`: Java sets the cause of `t` to the
    /// trace of the first input (or output) open on `name`; see the module
    /// docs for why the port returns `t` unchanged.
    fn fill_open_trace(&self, t: LuceneError, _name: &str, _input: bool) -> LuceneError {
        let _monitor = self.core.monitor.enter();
        t
    }

    /// `maybeThrottle(name, output)`: wraps the output in a throttled copy
    /// of the template always, or for `SOMETIMES` once in 200 draws.
    fn maybe_throttle(&self, name: &str, output: Box<dyn IndexOutput>) -> Box<dyn IndexOutput> {
        let throttling = *lock(&self.core.throttling);
        // throttling REALLY slows down tests, so don't do it very often for SOMETIMES.
        if throttling == Throttling::Always
            || (throttling == Throttling::Sometimes
                && self.core.random_state.next_int_bound(200) == 0)
        {
            if *VERBOSE {
                println!("MockDirectoryWrapper: throttling indexOutput ({name})");
            }
            Box::new(ThrottledIndexOutput::with_delays(
                self.core.throttle_bytes_per_second,
                self.core.throttle_delay_millis,
                self.core.throttle_delay_millis,
                self.core.throttle_delay_millis,
                i64::from(ThrottledIndexOutput::DEFAULT_MIN_WRITTEN_BYTES),
                output,
            ))
        } else {
            output
        }
    }

    fn check_crashed(&self, what: &str) -> Result<()> {
        if self.is_crashed() {
            return Err(LuceneError::Io(io_exception(format!(
                "cannot {what} after crash"
            ))));
        }
        Ok(())
    }

    /// The `IOUtils.close(in)` that ends `close()`: closes the delegate when
    /// the wrapper's state is no longer shared (see the module docs).
    fn close_delegate(&mut self) -> Result<()> {
        match Arc::get_mut(&mut self.core).and_then(|core| Arc::get_mut(&mut core.delegate)) {
            Some(delegate) => delegate.close(),
            None => Err(LuceneError::illegal_state(format!(
                "MockDirectoryWrapper: the delegate is still shared and cannot be closed B2DEBUG core={} delegate={}",
                Arc::strong_count(&self.core),
                Arc::strong_count(&self.core.delegate)
            ))),
        }
    }

    /// The body of `close()` between `isOpen = false` and the closing of the
    /// delegate.
    fn close_checks(&self) -> Result<()> {
        // files that we tried to delete, but couldn't because readers were open.
        // all that matters is that we tried! (they will eventually go away)
        //   still open when we tried to delete
        self.maybe_yield();
        {
            let state = lock(&self.core.state);
            if !state.open_files.is_empty() {
                // print the first one as it's very verbose otherwise
                let cause = state
                    .open_file_handles
                    .values()
                    .next()
                    .map(|h| Arc::clone(&h.trace));
                // RuntimeException instead of IOException because
                // super() does not throw IOException currently:
                return Err(runtime_exception(
                    format!(
                        "MockDirectoryWrapper: cannot close: there are still {} open files: {}",
                        state.open_files.len(),
                        java_map_to_string(&state.open_files, |v| v.to_string())
                    ),
                    cause,
                ));
            }
            if !state.open_locks.is_empty() {
                let cause = state.open_locks.values().next().map(Arc::clone);
                return Err(runtime_exception(
                    format!(
                        "MockDirectoryWrapper: cannot close: there are still open locks: {}",
                        java_map_to_string(&state.open_locks, |v| throwable_to_string(v))
                    ),
                    cause,
                ));
            }
        }
        self.set_random_io_exception_rate(0.0);
        self.set_random_io_exception_rate_on_open(0.0);

        let assert_no_unreferenced = self
            .core
            .assert_no_unreferenced_files_on_close
            .load(Ordering::SeqCst);
        if (self.get_check_index_on_close() || assert_no_unreferenced) && index_exists(self)? {
            if self.get_check_index_on_close() {
                if *VERBOSE {
                    println!("\nNOTE: MockDirectoryWrapper: now crush");
                }
                self.crash()?; // corrupt any unsynced-files
                if *VERBOSE {
                    println!("\nNOTE: MockDirectoryWrapper: now run CheckIndex");
                }

                // Methods in MockDirectoryWrapper hold locks on this, which will cause deadlock when
                // TestUtil#checkIndex checks segment concurrently using another thread, but making
                // call back to synchronized methods such as MockDirectoryWrapper#fileLength.
                // Hence passing concurrent = false to this method to turn off concurrent checks.
                TestUtil::check_index_with(
                    self.as_directory(),
                    self.get_level_for_check_on_close(),
                    true,
                    false,
                    None,
                )?;
            }

            // TODO: factor this out / share w/ TestIW.assertNoUnreferencedFiles
            if assert_no_unreferenced {
                self.assert_no_unreferenced_files()?;
            }
        }
        Ok(())
    }

    /// The `assertNoUnreferencedFilesOnClose` block of `close()`.
    fn assert_no_unreferenced_files(&self) -> Result<()> {
        if *VERBOSE {
            println!("MDW: now assert no unref'd files at close");
        }
        // now look for unreferenced files: discount ones that we tried to delete but could not
        let start_files = self.list_all()?;
        let mut iwc = IndexWriterConfig::with_analyzer(None);
        iwc.set_index_deletion_policy(Some(NoDeletionPolicy::instance()))?;

        // We must do this before opening writer otherwise writer will be angry if there are
        // pending deletions:
        TestUtil::disable_virus_checker(self.core.delegate.as_ref())?;

        IndexWriter::new(Arc::clone(&self.core.delegate), iwc)?.rollback()?;
        let end_files = self.core.delegate.list_all()?;

        let mut start_files = start_files;
        start_files.sort_by(|a, b| java_compare_to(a, b));
        start_files.dedup();
        let mut end_files = end_files;
        end_files.sort_by(|a, b| java_compare_to(a, b));
        end_files.dedup();

        if start_files != end_files {
            let removed: Vec<&String> = start_files
                .iter()
                .filter(|f| !end_files.contains(f))
                .collect();
            let added: Vec<&String> = end_files
                .iter()
                .filter(|f| !start_files.contains(f))
                .collect();

            let mut extras = if !removed.is_empty() {
                format!("\n\nThese files were removed: {}", java_list(&removed))
            } else {
                String::new()
            };
            if !added.is_empty() {
                extras.push_str(&format!(
                    "\n\nThese files were added (waaaaaaaaaat!): {}",
                    java_list(&added)
                ));
            }

            return Err(runtime_exception(
                format!(
                    "unreferenced files: before delete:\n    {}\n  after delete:\n    {}{extras}",
                    java_list(&start_files),
                    java_list(&end_files)
                ),
                None,
            ));
        }

        let ir1 = directory_reader::open(self.as_directory())?;
        let num_docs1 = ir1.num_docs();
        ir1.close()?;
        drop(ir1);
        // Don't commit on close, so that no merges will be scheduled.
        let mut config = IndexWriterConfig::with_analyzer(None);
        config.set_commit_on_close(false);
        IndexWriter::new(self.as_directory(), config)?.close()?;
        let ir2 = directory_reader::open(self.as_directory())?;
        let num_docs2 = ir2.num_docs();
        ir2.close()?;
        drop(ir2);
        if cfg!(debug_assertions) && num_docs1 != num_docs2 {
            return Err(LuceneError::AssertionError(format!(
                "numDocs changed after opening/closing IW: before={num_docs1} after={num_docs2}"
            )));
        }
        Ok(())
    }
}

impl Directory for MockDirectoryWrapper {
    /// `instanceof` (see [`Directory::as_any`]).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// `listAll()`.
    fn list_all(&self) -> Result<Vec<String>> {
        let _monitor = self.core.monitor.enter();
        self.maybe_yield();
        self.core.delegate.list_all()
    }

    /// `deleteFile(name)`: refused after a crash; deleting an open file is
    /// recorded in the open deleted files, and refused when
    /// [`set_assert_no_delete_open_file`](MockDirectoryWrapper::set_assert_no_delete_open_file)
    /// is set.
    fn delete_file(&self, name: &str) -> Result<()> {
        let _monitor = self.core.monitor.enter();
        self.maybe_yield();

        self.maybe_throw_deterministic_exception()?;

        self.check_crashed("delete")?;

        let is_open_file = lock(&self.core.state).open_files.contains_key(name);
        if is_open_file {
            lock(&self.core.state)
                .open_files_deleted
                .insert(name.to_string());
            if self.get_assert_no_delete_open_file() {
                return Err(self.fill_open_trace(
                    LuceneError::Io(io_exception(format!(
                        "MockDirectoryWrapper: file \"{name}\" is still open: cannot delete"
                    ))),
                    name,
                    true,
                ));
            }
        } else {
            lock(&self.core.state).open_files_deleted.remove(name);
        }

        lock(&self.core.state).un_synced_files.remove(name);
        self.core.delegate.delete_file(name)?;
        lock(&self.core.state).created_files.remove(name);
        Ok(())
    }

    /// `fileLength(name)`.
    fn file_length(&self, name: &str) -> Result<i64> {
        let _monitor = self.core.monitor.enter();
        self.maybe_yield();
        self.core.delegate.file_length(name)
    }

    /// `createOutput(name, context)`: refuses a name already written to
    /// (`FileAlreadyExistsException`), records the file as created and
    /// unsynced, and wraps the delegate's output in a
    /// [`MockIndexOutputWrapper`], maybe throttled.
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        let _monitor = self.core.monitor.enter();
        self.maybe_throw_deterministic_exception()?;
        self.maybe_throw_io_exception_on_open(name)?;
        self.maybe_yield();
        if self.core.fail_on_create_output.load(Ordering::SeqCst) {
            self.maybe_throw_deterministic_exception()?;
        }
        self.check_crashed("createOutput")?;

        {
            let mut state = lock(&self.core.state);
            if state.created_files.contains(name) {
                return Err(LuceneError::Io(file_already_exists_exception(format!(
                    "File \"{name}\" was already written to."
                ))));
            }

            if self.get_assert_no_delete_open_file() && state.open_files.contains_key(name) {
                return Err(LuceneError::AssertionError(format!(
                    "MockDirectoryWrapper: file \"{name}\" is still open: cannot overwrite"
                )));
            }

            state.un_synced_files.insert(name.to_string());
            state.created_files.insert(name.to_string());
        }

        let context = LuceneTestCase::new_io_context_from(&self.core.random_state, context);
        let delegate_output = self.core.delegate.create_output(name, &*context)?;
        let io = MockIndexOutputWrapper::new(self, delegate_output, name);
        let (id, cell) = io.registration();
        self.add_file_handle(id, Closer::Output(cell), name, Handle::Output);
        lock(&self.core.state)
            .open_files_for_write
            .insert(name.to_string());
        Ok(self.maybe_throttle(name, Box::new(io)))
    }

    /// `createTempOutput(prefix, suffix, context)`: as `createOutput`, for
    /// the delegate's temporary file, which must use the `.tmp` extension.
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        let _monitor = self.core.monitor.enter();
        self.maybe_throw_deterministic_exception()?;
        self.maybe_throw_io_exception_on_open(&format!("temp: prefix={prefix} suffix={suffix}"))?;
        self.maybe_yield();
        if self.core.fail_on_create_output.load(Ordering::SeqCst) {
            self.maybe_throw_deterministic_exception()?;
        }
        self.check_crashed("createTempOutput")?;

        let context = LuceneTestCase::new_io_context_from(&self.core.random_state, context);
        let delegate_output = self
            .core
            .delegate
            .create_temp_output(prefix, suffix, &*context)?;
        let name = delegate_output.name().to_string();
        if !name.to_lowercase().ends_with(".tmp") {
            return Err(LuceneError::illegal_state(format!(
                "wrapped directory failed to use .tmp extension: got: {name}"
            )));
        }

        {
            let mut state = lock(&self.core.state);
            state.un_synced_files.insert(name.clone());
            state.created_files.insert(name.clone());
        }
        let io = MockIndexOutputWrapper::new(self, delegate_output, &name);
        let (id, cell) = io.registration();
        self.add_file_handle(id, Closer::Output(cell), &name, Handle::Output);
        lock(&self.core.state)
            .open_files_for_write
            .insert(name.clone());

        Ok(self.maybe_throttle(&name, Box::new(io)))
    }

    /// `sync(names)`: refused after a crash; each file may throw a random
    /// `IOException`, then is synced in the delegate and no longer unsynced.
    fn sync(&self, names: &[String]) -> Result<()> {
        let _monitor = self.core.monitor.enter();
        self.maybe_yield();
        self.maybe_throw_deterministic_exception()?;
        self.check_crashed("sync")?;
        // always pass thru fsync, directories rely on this.
        // 90% of time, we use DisableFsyncFS which omits the real calls.
        for name in names {
            // randomly fail with IOE on any file
            self.maybe_throw_io_exception(Some(name))?;
            self.core.delegate.sync(std::slice::from_ref(name))?;
            lock(&self.core.state).un_synced_files.remove(name);
        }
        Ok(())
    }

    /// `syncMetaData()`.
    fn sync_metadata(&self) -> Result<()> {
        let _monitor = self.core.monitor.enter();
        self.maybe_yield();
        self.maybe_throw_deterministic_exception()?;
        self.check_crashed("sync metadata")?;
        self.core.delegate.sync_metadata()
    }

    /// `rename(source, dest)`: refused after a crash, and for an open file
    /// when [`set_assert_no_delete_open_file`](MockDirectoryWrapper::set_assert_no_delete_open_file)
    /// is set; moves the file's created and unsynced state.
    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        let _monitor = self.core.monitor.enter();
        self.maybe_yield();
        self.maybe_throw_deterministic_exception()?;

        if self.is_crashed() {
            return Err(LuceneError::Io(io_exception("cannot rename after crash")));
        }

        let (source_open, dest_open) = {
            let state = lock(&self.core.state);
            (
                state.open_files.contains_key(source),
                state.open_files.contains_key(dest),
            )
        };
        if source_open && self.get_assert_no_delete_open_file() {
            return Err(self.fill_open_trace(
                LuceneError::AssertionError(format!(
                    "MockDirectoryWrapper: source file \"{source}\" is still open: cannot rename"
                )),
                source,
                true,
            ));
        }

        if dest_open && self.get_assert_no_delete_open_file() {
            return Err(self.fill_open_trace(
                LuceneError::AssertionError(format!(
                    "MockDirectoryWrapper: dest file \"{dest}\" is still open: cannot rename"
                )),
                dest,
                true,
            ));
        }

        self.core.delegate.rename(source, dest)?;
        // we don't do this stuff with lucene's commit, but it's just for completeness
        let mut state = lock(&self.core.state);
        if state.un_synced_files.contains(source) {
            state.un_synced_files.remove(source);
            state.un_synced_files.insert(dest.to_string());
        }
        state.open_files_deleted.remove(source);
        state.created_files.remove(source);
        state.created_files.insert(dest.to_string());
        Ok(())
    }

    /// `openInput(name, context)`: refuses a missing file
    /// (`FileNotFoundException` or `NoSuchFileException`, at random), a file
    /// still open for writing (`AccessDeniedException`) and a `segments`
    /// file opened without `READONCE`; wraps the delegate's input in a
    /// [`MockIndexInputWrapper`] (or, with slow open/closers, rarely one of
    /// the slow wrappers).
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        let _monitor = self.core.monitor.enter();
        self.maybe_throw_deterministic_exception()?;
        self.maybe_throw_io_exception_on_open(name)?;
        self.maybe_yield();
        if self.core.fail_on_open_input.load(Ordering::SeqCst) {
            self.maybe_throw_deterministic_exception()?;
        }
        let delegate = &self.core.delegate;
        if !LuceneTestCase::slow_file_exists(delegate.as_ref(), name)? {
            let message = format!("{name} in dir={}", delegate.directory_to_string());
            return Err(if self.core.random_state.next_boolean() {
                file_not_found_exception(message)
            } else {
                LuceneError::Io(no_such_file_exception(message))
            });
        }

        // cannot open a file for input if it's still open for output.
        if !self
            .core
            .allow_reading_files_still_open_for_write
            .load(Ordering::SeqCst)
            && lock(&self.core.state).open_files_for_write.contains(name)
        {
            return Err(self.fill_open_trace(
                LuceneError::Io(access_denied_exception(format!(
                    "MockDirectoryWrapper: file \"{name}\" is still open for writing"
                ))),
                name,
                false,
            ));
        }

        let context = LuceneTestCase::new_io_context_from(&self.core.random_state, context);
        let confined = !hints_of::<ReadOnceHint>(&*context).is_empty();
        if name.starts_with(SEGMENTS) && !confined {
            return Err(runtime_exception(
                format!(
                    "MockDirectoryWrapper: opening segments file [{name}] with a non-READONCE context[{}]",
                    io_context_to_string(&*context)
                ),
                None,
            ));
        }
        let delegate_input = delegate.open_input(name, &*context)?;

        let random_int = self.core.random_state.next_int_bound(500);
        let use_slow = self.core.use_slow_open_closers.load(Ordering::SeqCst);
        let ii: Box<dyn IndexInput> = if use_slow && random_int == 0 {
            if *VERBOSE {
                println!(
                    "MockDirectoryWrapper: using SlowClosingMockIndexInputWrapper for file {name}"
                );
            }
            let ii = SlowClosingMockIndexInputWrapper::new(self, name, delegate_input, confined);
            let (id, ctl) = ii.registration();
            self.add_file_handle(id, Closer::Input(ctl), name, Handle::Input);
            Box::new(ii)
        } else if use_slow && random_int == 1 {
            if *VERBOSE {
                println!(
                    "MockDirectoryWrapper: using SlowOpeningMockIndexInputWrapper for file {name}"
                );
            }
            let ii = SlowOpeningMockIndexInputWrapper::new(self, name, delegate_input, confined)?;
            let (id, ctl) = ii.registration();
            self.add_file_handle(id, Closer::Input(ctl), name, Handle::Input);
            Box::new(ii)
        } else {
            let ii = MockIndexInputWrapper::new(self, name, delegate_input, None, confined);
            let (id, ctl) = ii.registration();
            self.add_file_handle(id, Closer::Input(ctl), name, Handle::Input);
            Box::new(ii)
        };
        Ok(ii)
    }

    /// `obtainLock(name)`: the delegate's lock.
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        let _monitor = self.core.monitor.enter();
        self.maybe_yield();
        self.core.delegate.obtain_lock(name)
        // TODO: consider mocking locks, but not all the time, can hide bugs
    }

    /// `close()`: the first close fails (`RuntimeException`) while files or
    /// locks are open; then, when an index exists, crashes the unsynced files
    /// and runs `CheckIndex` (and the unreferenced-files check when enabled);
    /// finally closes the delegate. A second close closes the delegate
    /// again.
    fn close(&mut self) -> Result<()> {
        let first = {
            let _monitor = self.core.monitor.enter();
            self.core.is_open.swap(false, Ordering::SeqCst)
        };
        if !first {
            return self.close_delegate(); // but call it again on our wrapped dir
        }
        let result = {
            let _monitor = self.core.monitor.enter();
            self.close_checks()
        };
        match result {
            Ok(()) => self.close_delegate(),
            Err(e) => {
                let _ = self.close_delegate();
                Err(e)
            }
        }
    }

    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.core.delegate.get_pending_deletions()
    }

    /// `ensureOpen()` (final): `super.ensureOpen()`, which neither
    /// `BaseDirectoryWrapper` nor `FilterDirectory` overrides: `Directory`'s
    /// no-op.
    fn ensure_open(&self) -> Result<()> {
        Ok(())
    }

    /// `toString()`: with a maximum size, `MockDirectoryWrapper(<in>,
    /// current=<maxUsedSize>,max=<maxSize>)`; else `FilterDirectory`'s
    /// `MockDirectoryWrapper(<in>)`.
    fn directory_to_string(&self) -> String {
        let max_size = self.get_max_size_in_bytes();
        if max_size != 0 {
            format!(
                "MockDirectoryWrapper({}, current={},max={max_size})",
                self.core.delegate.directory_to_string(),
                self.get_max_used_size_in_bytes()
            )
        } else {
            format!(
                "MockDirectoryWrapper({})",
                self.core.delegate.directory_to_string()
            )
        }
    }

    /// A `FilterDirectory`, over its `in`.
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.core.delegate.as_ref())
    }

    /// `openChecksumInput(name)` (final): `Directory`'s default, over this
    /// wrapper's `openInput`.
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        Ok(Box::new(BufferedChecksumIndexInput::new(
            self.open_input(name, &*rucene::store::READONCE_IO_CONTEXT)?,
        )))
    }
}

impl fmt::Display for MockDirectoryWrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.directory_to_string())
    }
}

impl fmt::Debug for MockDirectoryWrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.directory_to_string())
    }
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// `new RuntimeException(message, cause)`.
pub(crate) fn runtime_exception(message: String, cause: Option<Arc<LuceneError>>) -> LuceneError {
    LuceneError::Unchecked {
        class: "java.lang.RuntimeException",
        message: Some(message),
        cause,
    }
}

/// `new FileNotFoundException(message)`, of kind `NotFound` as the
/// `catch (NoSuchFileException | FileNotFoundException e)` sites of the port
/// test it.
pub(crate) fn file_not_found_exception(message: String) -> LuceneError {
    LuceneError::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        JavaIoException::new("java.io.FileNotFoundException", Some(message)),
    ))
}

/// Java's `try`-with-resources outcome: the body's error, or else the first
/// error closing a resource (the others are suppressed).
pub(crate) fn try_with_resources<T, const N: usize>(
    body: Result<T>,
    closes: [Result<()>; N],
) -> Result<T> {
    let mut first_close_error = None;
    for close in closes {
        if let Err(e) = close {
            first_close_error.get_or_insert(e);
        }
    }
    match (body, first_close_error) {
        (Err(e), _) => Err(e),
        (Ok(_), Some(e)) => Err(e),
        (Ok(v), None) => Ok(v),
    }
}

/// `Thread.currentThread().getName()`.
fn current_thread_name() -> String {
    std::thread::current()
        .name()
        .map_or_else(|| "main".to_string(), str::to_string)
}

/// `AbstractMap.toString()` over a map's iteration order.
fn java_map_to_string<V>(map: &JavaHashMap<V>, value: impl Fn(&V) -> String) -> String {
    let entries: Vec<String> = map
        .iter()
        .map(|(k, v)| format!("{k}={}", value(v)))
        .collect();
    format!("{{{}}}", entries.join(", "))
}

/// `Arrays.toString` / `AbstractCollection.toString` of strings.
fn java_list<S: AsRef<str>>(items: &[S]) -> String {
    let items: Vec<&str> = items.iter().map(AsRef::as_ref).collect();
    format!("[{}]", items.join(", "))
}

/// `Throwable.toString()` of an unchecked exception.
fn throwable_to_string(e: &LuceneError) -> String {
    match e {
        LuceneError::Unchecked {
            class,
            message: Some(message),
            ..
        } => format!("{class}: {message}"),
        LuceneError::Unchecked { class, .. } => class.to_string(),
        other => other.to_string(),
    }
}

/// `IOContext.toString()` of the contexts `newIOContext` makes: the record
/// `DefaultIOContext[hints=[…]]`, or `Object.toString()` of the anonymous
/// merge (`IOContext$1`) and flush (`IOContext$2`) classes, whose identity
/// hash the address of the context stands for.
fn io_context_to_string(context: &dyn IOContext) -> String {
    let hash = context as *const dyn IOContext as *const () as usize as i32;
    match context.context() {
        Context::Merge => format!("org.apache.lucene.store.IOContext$1@{:x}", hash),
        Context::Flush => format!("org.apache.lucene.store.IOContext$2@{:x}", hash),
        Context::Default => {
            let hints: Vec<String> = context.hints().iter().map(|h| format!("{h:?}")).collect();
            format!("DefaultIOContext[hints=[{}]]", hints.join(", "))
        }
    }
}
