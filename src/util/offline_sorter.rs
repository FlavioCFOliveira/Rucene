//! External merge sort ported from `org.apache.lucene.util.OfflineSorter`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`OfflineSorter`] | `OfflineSorter` |
//! | [`BufferSize`] | `OfflineSorter.BufferSize` |
//! | [`SortInfo`] | `OfflineSorter.SortInfo` |
//! | [`ByteSequencesWriter`] | `OfflineSorter.ByteSequencesWriter` |
//! | [`ByteSequencesReader`] | `OfflineSorter.ByteSequencesReader` |
//!
//! On-disk sorter for byte sequences of at most 32767 bytes each, written as a
//! big-endian `short` length followed by the bytes, with a codec footer.
//!
//! # Divergences from Lucene 10.5.0
//!
//! * **Overridable methods.** Java subclasses override the protected
//!   `getWriter` and `getReader`; a sorter here holds an
//!   [`OfflineSorterHooks`] instead (see [`OfflineSorter::with_hooks`]).
//! * **`BufferSize::automatic`.** Java sizes the buffer from
//!   `Runtime.maxMemory()`, `totalMemory()` and `freeMemory()`. A Rust process
//!   has no managed heap to interrogate, so this port returns the same floor
//!   Java's heuristic is built around, `MIN_BUFFER_SIZE_MB` megabytes.

#![deny(unsafe_code)]

use std::fmt::{self, Display, Formatter};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use crate::codecs::codec_util;
use crate::error::{LuceneError, Result};
use crate::search::Executor;
use crate::store::{ChecksumIndexInput, Directory, IndexOutput, TrackingDirectoryWrapper};
use crate::util::bytes_ref_array::{
    BytesRefArray, BytesRefIterator, FixedLengthBytesRefArray, SortableBytesRefArray,
};
use crate::util::concurrent::Counter;
use crate::util::extra::{PriorityQueue, PriorityQueueComparator};
use crate::util::sorter::StringSorterComparator;
use crate::util::BytesRef;

/// One megabyte. `OfflineSorter.MB`.
pub const MB: i64 = 1024 * 1024;
/// One gigabyte. `OfflineSorter.GB`.
pub const GB: i64 = MB * 1024;
/// Minimum recommended buffer size in megabytes.
/// `OfflineSorter.MIN_BUFFER_SIZE_MB`.
pub const MIN_BUFFER_SIZE_MB: i64 = 32;
/// Absolute minimum required buffer size.
/// `OfflineSorter.ABSOLUTE_MIN_SORT_BUFFER_SIZE`.
pub const ABSOLUTE_MIN_SORT_BUFFER_SIZE: i64 = MB / 2;
/// `OfflineSorter.MIN_BUFFER_SIZE_MSG`.
const MIN_BUFFER_SIZE_MSG: &str = "At least 0.5MB RAM buffer is needed";
/// Maximum number of temporary files merged at once.
/// `OfflineSorter.MAX_TEMPFILES`.
pub const MAX_TEMPFILES: usize = 10;

// ---------------------------------------------------------------------------
// BufferSize
// ---------------------------------------------------------------------------

/// A sort buffer size in bytes.
///
/// Port of the nested class `OfflineSorter.BufferSize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferSize {
    /// The buffer size in bytes.
    pub bytes: usize,
}

impl BufferSize {
    fn new(bytes: i64) -> Result<Self> {
        if bytes > i32::MAX as i64 {
            return Err(LuceneError::illegal_argument(format!(
                "Buffer too large for Java ({}MB max): {bytes}",
                i32::MAX as i64 / MB
            )));
        }
        if bytes < ABSOLUTE_MIN_SORT_BUFFER_SIZE {
            return Err(LuceneError::illegal_argument(format!(
                "{MIN_BUFFER_SIZE_MSG}: {bytes}"
            )));
        }
        Ok(Self {
            bytes: bytes as usize,
        })
    }

    /// Creates a buffer size of `mb` megabytes.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the size is below
    /// [`ABSOLUTE_MIN_SORT_BUFFER_SIZE`] or above `i32::MAX`.
    pub fn megabytes(mb: i64) -> Result<Self> {
        Self::new(mb * MB)
    }

    /// Returns the default buffer size.
    ///
    /// See the module documentation for why this is a fixed value rather than
    /// Java's heap-derived heuristic.
    ///
    /// # Errors
    ///
    /// Never fails in practice; the signature mirrors [`BufferSize::megabytes`].
    pub fn automatic() -> Result<Self> {
        Self::new(MIN_BUFFER_SIZE_MB * MB)
    }
}

// ---------------------------------------------------------------------------
// SortInfo
// ---------------------------------------------------------------------------

/// Statistics about one [`OfflineSorter::sort`] run.
///
/// Port of the nested class `OfflineSorter.SortInfo`.
#[derive(Debug, Default, Clone)]
pub struct SortInfo {
    /// Number of temporary files created.
    pub temp_merge_files: i32,
    /// Number of merge rounds performed.
    pub merge_rounds: i32,
    /// Number of lines (values) sorted.
    pub line_count: i64,
    /// Time spent merging, in milliseconds.
    pub merge_time_ms: i64,
    /// Time spent sorting in memory, in milliseconds.
    pub sort_time_ms: i64,
    /// Total wall time, in milliseconds.
    pub total_time_ms: i64,
    /// Time spent reading, in milliseconds.
    pub read_time_ms: i64,
    /// The buffer size used, in bytes.
    pub buffer_size: usize,
}

impl SortInfo {
    fn new(buffer_size: usize) -> Self {
        Self {
            buffer_size,
            ..Default::default()
        }
    }
}

impl Display for SortInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "time={:.2} sec. total ({:.2} reading, {:.2} sorting, {:.2} merging), lines={}, temp files={}, merges={}, soft ram limit={:.2} MB",
            self.total_time_ms as f64 / 1000.0,
            self.read_time_ms as f64 / 1000.0,
            self.sort_time_ms as f64 / 1000.0,
            self.merge_time_ms as f64 / 1000.0,
            self.line_count,
            self.temp_merge_files,
            self.merge_rounds,
            self.buffer_size as f64 / MB as f64
        )
    }
}

// ---------------------------------------------------------------------------
// ByteSequences reader/writer
// ---------------------------------------------------------------------------

/// Writes length-prefixed byte sequences to an [`IndexOutput`].
///
/// Port of the nested class `OfflineSorter.ByteSequencesWriter`.
pub struct ByteSequencesWriter {
    out: Box<dyn IndexOutput>,
}

impl ByteSequencesWriter {
    /// Wraps `out`.
    pub fn new(out: Box<dyn IndexOutput>) -> Self {
        Self { out }
    }

    /// Returns the underlying output.
    pub fn out(&mut self) -> &mut dyn IndexOutput {
        self.out.as_mut()
    }

    /// Writes one value.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the value is longer than
    /// `i16::MAX`, and propagates write errors.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > i16::MAX as usize {
            return Err(LuceneError::illegal_argument(format!(
                "len must be <= {}; got {}",
                i16::MAX,
                bytes.len()
            )));
        }
        self.out.write_short(bytes.len() as i16)?;
        self.out.write_bytes(bytes, 0, bytes.len())
    }

    /// Closes the underlying output.
    ///
    /// # Errors
    ///
    /// Propagates close errors.
    pub fn close(mut self) -> Result<()> {
        self.out.close()
    }
}

/// The methods a subclass of `ByteSequencesReader` overrides: `next()` and
/// `close()`.
///
/// Java's subclass passes its `in` to the super constructor and may hand the
/// same input to another reader it delegates to; Rust cannot hold one input in
/// two readers, so the overriding object owns it and exposes it as the `in`
/// field through [`input`](Self::input).
pub trait ByteSequencesReaderOverride: Send + Sync {
    /// The overriding `next()`.
    ///
    /// # Errors
    ///
    /// Propagates what the override raises.
    fn next(&mut self) -> Result<Option<BytesRef>>;

    /// The overriding `close()`.
    ///
    /// # Errors
    ///
    /// Propagates what the override raises.
    fn close(&mut self) -> Result<()>;

    /// The field `in`.
    fn input(&mut self) -> &mut dyn ChecksumIndexInput;
}

/// What a [`ByteSequencesReader`] reads through.
enum ReaderBody {
    /// Java's own `next()` and `close()` over `in`, up to `end`.
    Base {
        input: Box<dyn ChecksumIndexInput>,
        end: i64,
    },
    /// A subclass's.
    Override(Box<dyn ByteSequencesReaderOverride>),
}

/// Reads the length-prefixed byte sequences [`ByteSequencesWriter`] produced.
///
/// Port of the nested class `OfflineSorter.ByteSequencesReader`.
pub struct ByteSequencesReader {
    name: String,
    body: ReaderBody,
}

impl ByteSequencesReader {
    /// Wraps `input`, reading up to the start of its codec footer.
    pub fn new(input: Box<dyn ChecksumIndexInput>, name: impl Into<String>) -> Self {
        let end = input.length() - codec_util::footer_length() as i64;
        Self {
            name: name.into(),
            body: ReaderBody::Base { input, end },
        }
    }

    /// The reader of a subclass that overrides `next()` and `close()`.
    pub fn with_override(
        name: impl Into<String>,
        overriding: Box<dyn ByteSequencesReaderOverride>,
    ) -> Self {
        Self {
            name: name.into(),
            body: ReaderBody::Override(overriding),
        }
    }

    /// Returns the name of the file being read.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the underlying input, so that the footer can be checked.
    pub fn input(&mut self) -> &mut dyn ChecksumIndexInput {
        match &mut self.body {
            ReaderBody::Base { input, .. } => input.as_mut(),
            ReaderBody::Override(overriding) => overriding.input(),
        }
    }

    /// Closes the underlying input.
    ///
    /// Equivalent to `ByteSequencesReader.close()`, `in.close()`.
    ///
    /// # Errors
    ///
    /// Propagates the failure of closing the input.
    pub fn close(&mut self) -> Result<()> {
        match &mut self.body {
            ReaderBody::Base { input, .. } => input.close(),
            ReaderBody::Override(overriding) => overriding.close(),
        }
    }
}

impl BytesRefIterator for ByteSequencesReader {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        let (input, end) = match &mut self.body {
            ReaderBody::Base { input, end } => (input, *end),
            ReaderBody::Override(overriding) => return overriding.next(),
        };
        if input.file_pointer()? >= end {
            return Ok(None);
        }
        let length = input.read_short()? as usize;
        let mut bytes = vec![0u8; length];
        input.read_bytes(Some(&mut bytes), 0, length)?;
        Ok(Some(BytesRef::new(bytes)))
    }
}

// ---------------------------------------------------------------------------
// OfflineSorter
// ---------------------------------------------------------------------------

/// A caller-supplied `Comparator<BytesRef>`.
pub type BytesRefComparatorFn = Arc<dyn Fn(&BytesRef, &BytesRef) -> i32 + Send + Sync>;

/// The order [`OfflineSorter`] sorts by.
///
/// Java holds a `Comparator<BytesRef>`; Rust cannot store a bare closure
/// without a type parameter, so the default and the custom case are an enum.
#[derive(Clone)]
pub enum OfflineSorterComparator {
    /// `OfflineSorter.DEFAULT_COMPARATOR`, i.e. `Comparator.naturalOrder()`.
    Natural,
    /// A caller-supplied comparator.
    Custom(BytesRefComparatorFn),
}

impl std::fmt::Debug for OfflineSorterComparator {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Natural => f.write_str("Natural"),
            Self::Custom(_) => f.write_str("Custom"),
        }
    }
}

impl OfflineSorterComparator {
    /// Compares two values.
    pub fn compare(&self, a: &BytesRef, b: &BytesRef) -> i32 {
        match self {
            Self::Natural => natural_compare(a, b),
            Self::Custom(f) => f(a, b),
        }
    }
}

/// `Comparator.naturalOrder()` over [`BytesRef`], which orders by unsigned
/// bytes and then by length.
fn natural_compare(a: &BytesRef, b: &BytesRef) -> i32 {
    match a.cmp(b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// One partition of the input: either an in-memory buffer or a sorted file.
///
/// Port of the nested class `OfflineSorter.Partition`.
struct Partition {
    buffer: Option<Box<dyn SortableBytesRefArray + Send>>,
    exhausted: bool,
    count: i64,
    file_name: Option<String>,
}

impl Partition {
    fn in_memory(buffer: Box<dyn SortableBytesRefArray + Send>, exhausted: bool) -> Self {
        let count = buffer.size() as i64;
        Self {
            buffer: Some(buffer),
            exhausted,
            count,
            file_name: None,
        }
    }

    fn on_disk(file_name: String, count: i64) -> Self {
        Self {
            buffer: None,
            exhausted: true,
            count,
            file_name: Some(file_name),
        }
    }
}

/// The queue entry of the merge step.
///
/// Port of the nested class `OfflineSorter.FileAndTop`.
struct FileAndTop {
    fd: usize,
    current: BytesRef,
}

struct FileAndTopComparator {
    comparator: OfflineSorterComparator,
}

impl PriorityQueueComparator<FileAndTop> for FileAndTopComparator {
    fn less_than(&self, a: &FileAndTop, b: &FileAndTop) -> bool {
        self.comparator.compare(&a.current, &b.current) < 0
    }
}

/// The overridable methods of `OfflineSorter`: `getWriter(IndexOutput,
/// long)` and `getReader(ChecksumIndexInput, String)`.
///
/// Java subclasses `OfflineSorter` and overrides these protected methods;
/// Rust has no implementation inheritance, so a sorter holds an implementation
/// of this trait, whose defaults are Java's bodies.
pub trait OfflineSorterHooks: Send + Sync {
    /// Subclasses can override to change how byte sequences are written to
    /// disk.
    ///
    /// Equivalent to the protected `OfflineSorter.getWriter`.
    ///
    /// # Errors
    ///
    /// Propagates what the override raises.
    fn get_writer(
        &self,
        out: Box<dyn IndexOutput>,
        item_count: i64,
    ) -> Result<ByteSequencesWriter> {
        let _ = item_count;
        Ok(ByteSequencesWriter::new(out))
    }

    /// Subclasses can override to change how byte sequences are read from
    /// disk.
    ///
    /// Equivalent to the protected `OfflineSorter.getReader`.
    ///
    /// # Errors
    ///
    /// Propagates what the override raises.
    fn get_reader(
        &self,
        input: Box<dyn ChecksumIndexInput>,
        name: &str,
    ) -> Result<ByteSequencesReader> {
        Ok(ByteSequencesReader::new(input, name))
    }
}

/// The hooks of a sorter that overrides nothing.
#[derive(Debug, Default)]
pub struct DefaultOfflineSorterHooks;

impl OfflineSorterHooks for DefaultOfflineSorterHooks {}

/// `java.util.concurrent.Semaphore`, the permits bounding the partitions held
/// in RAM.
#[derive(Debug)]
struct Semaphore {
    permits: Mutex<i32>,
    available: Arc<Condvar>,
}

impl Semaphore {
    /// `new Semaphore(permits)`.
    fn new(permits: i32) -> Self {
        Self {
            permits: Mutex::new(permits),
            available: Arc::new(Condvar::new()),
        }
    }

    /// `acquire()`: waits for a permit; interruptible.
    fn acquire(&self) -> std::result::Result<(), crate::util::InterruptedException> {
        let mut permits = self
            .permits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while *permits <= 0 {
            let (reacquired, waited) =
                crate::util::wait_interruptibly(&self.available, permits, None)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            permits = reacquired;
            waited?;
        }
        *permits -= 1;
        Ok(())
    }

    /// `release()`.
    fn release(&self) {
        let mut permits = self
            .permits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *permits += 1;
        self.available.notify_one();
    }
}

/// What a submitted task produced: its value, the error it raised, or the
/// panic it threw.
enum TaskOutcome {
    Value(Partition),
    Error(LuceneError),
    Panic(Box<dyn std::any::Any + Send>),
}

/// The `Future<Partition>` `ExecutorService.submit(Callable)` returns.
///
/// **Divergence from Lucene 10.5.0.** Java's future rethrows the very same
/// cause on every `get()`. [`LuceneError`] cannot be copied, so the first
/// `get()` of a failed task returns its error and any later one an error
/// carrying its text; no code path of the sorter gets a failed future twice.
#[derive(Clone)]
struct PartitionFuture {
    state: Arc<(Mutex<Option<TaskOutcome>>, Arc<Condvar>)>,
}

impl PartitionFuture {
    fn new() -> Self {
        Self {
            state: Arc::new((Mutex::new(None), Arc::new(Condvar::new()))),
        }
    }

    /// Completes the future with what `task` produced.
    fn run(&self, task: impl FnOnce() -> Result<Partition>) {
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(task)) {
            Ok(Ok(partition)) => TaskOutcome::Value(partition),
            Ok(Err(error)) => TaskOutcome::Error(error),
            Err(payload) => TaskOutcome::Panic(payload),
        };
        let (slot, done) = &*self.state;
        *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(outcome);
        done.notify_all();
    }

    /// Equivalent to the private `OfflineSorter.getPartition(Future)`:
    /// `future.get()`, an interrupt as `ThreadInterruptedException`, and the
    /// task's own failure rethrown (`IOUtils.rethrowAlways(cause)`).
    fn get_partition(&self) -> Result<PartitionInfo> {
        let (slot, done) = &*self.state;
        let mut outcome = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        while outcome.is_none() {
            let (reacquired, waited) = crate::util::wait_interruptibly(done, outcome, None)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            outcome = reacquired;
            waited.map_err(LuceneError::thread_interrupted)?;
        }
        match outcome.as_ref() {
            Some(TaskOutcome::Value(partition)) => Ok(PartitionInfo {
                file_name: partition.file_name.clone(),
                count: partition.count,
            }),
            Some(TaskOutcome::Error(error)) => {
                let text = error.to_string();
                match outcome.replace(TaskOutcome::Error(LuceneError::illegal_state(text))) {
                    Some(TaskOutcome::Error(error)) => Err(error),
                    _ => unreachable!("INVARIANT: the slot held an error"),
                }
            }
            Some(TaskOutcome::Panic(_)) => match outcome.take() {
                Some(TaskOutcome::Panic(payload)) => std::panic::resume_unwind(payload),
                _ => unreachable!("INVARIANT: the slot held a panic"),
            },
            None => unreachable!("INVARIANT: the wait ends with an outcome"),
        }
    }
}

/// What `getPartition` hands back of a finished task: an on-disk partition.
struct PartitionInfo {
    file_name: Option<String>,
    count: i64,
}

/// The state the sorter shares with the tasks it submits: Java's inner
/// classes read their enclosing `OfflineSorter`'s fields.
struct Shared {
    dir: Arc<dyn Directory>,
    temp_file_name_prefix: String,
    comparator: OfflineSorterComparator,
    partitions_in_ram: Semaphore,
    hooks: Arc<dyn OfflineSorterHooks>,
    sort_info: Mutex<SortInfo>,
    /// The `TrackingDirectoryWrapper trackingDir` the current `sort` creates
    /// around `dir` (`OfflineSorter.java:291`) and hands its tasks, so that a
    /// failure can delete every file it created.
    tracking_dir: Mutex<Arc<TrackingDirectoryWrapper>>,
}

impl Shared {
    fn sort_info(&self) -> std::sync::MutexGuard<'_, SortInfo> {
        self.sort_info
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The tracking directory of the current `sort`.
    fn tracking_dir(&self) -> Arc<TrackingDirectoryWrapper> {
        Arc::clone(
            &self
                .tracking_dir
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    /// `new TrackingDirectoryWrapper(dir)`.
    fn new_tracking_dir(dir: &Arc<dyn Directory>) -> Arc<TrackingDirectoryWrapper> {
        Arc::new(TrackingDirectoryWrapper::new(Box::new(
            crate::index::documents_writer::SharedDirectory(Arc::clone(dir)),
        )))
    }

    /// `trackingDir.createTempOutput(prefix, "sort", IOContext.DEFAULT)`.
    fn create_tracked_temp_output(&self) -> Result<Box<dyn IndexOutput>> {
        self.tracking_dir().create_temp_output(
            &self.temp_file_name_prefix,
            "sort",
            &*crate::store::DEFAULT_IO_CONTEXT,
        )
    }

    /// `getReader(dir.openChecksumInput(name), name)`.
    fn open_reader(&self, name: &str) -> Result<ByteSequencesReader> {
        let input = self.dir.open_checksum_input(name)?;
        self.hooks.get_reader(input, name)
    }

    /// Verifies the checksum of a reader whose `next()` already failed.
    ///
    /// Equivalent to the private `OfflineSorter.verifyChecksum`.
    fn verify_checksum(&self, prior: LuceneError, name: &str) -> LuceneError {
        // `try (ChecksumIndexInput in = dir.openChecksumInput(reader.name))`:
        // closed on every exit.
        match self.dir.open_checksum_input(name) {
            Ok(mut input) => {
                let error = codec_util::check_footer_with_prior(input.as_mut(), prior);
                // A failure closing it is suppressed into `error` in Java, dropped
                // here (see `close_after`).
                let _ = crate::store::IndexInput::close(input.as_mut());
                error
            }
            Err(e) => e,
        }
    }

    /// Sorts one in-memory partition and spills it to a temporary file.
    ///
    /// Equivalent to `OfflineSorter.SortPartitionTask.call()`; `finally {
    /// partitionsInRAM.release(); }`.
    fn sort_partition_task(&self, part: Partition) -> Result<Partition> {
        let outcome = self.sort_partition(part);
        self.partitions_in_ram.release();
        outcome
    }

    fn sort_partition(&self, part: Partition) -> Result<Partition> {
        let mut buffer = part
            .buffer
            .expect("INVARIANT: an unsorted partition is always in memory");
        let temp_file = self.create_tracked_temp_output()?;
        let temp_name = temp_file.name().to_string();
        let mut out = self.hooks.get_writer(temp_file, buffer.size() as i64)?;

        let body = (|| -> Result<()> {
            let start = Instant::now();
            let comparator = self.comparator.clone();
            let compare = move |a: &BytesRef, b: &BytesRef| comparator.compare(a, b);
            let mut iter = buffer.sorted_iterator(StringSorterComparator::Generic(&compare))?;
            self.sort_info().sort_time_ms += start.elapsed().as_millis() as i64;

            let mut count = 0i64;
            while let Some(spare) = iter.next()? {
                out.write(spare.slice())?;
                count += 1;
            }
            debug_assert_eq!(count, part.count);
            drop(iter);

            codec_util::write_footer(out.out())?;
            buffer.clear();
            Ok(())
        })();
        // The try-with-resources closes the writer (and so the temp file); a
        // failure to close is suppressed when the body failed.
        match body {
            Ok(()) => out.close()?,
            Err(error) => {
                let _ = out.close();
                return Err(error);
            }
        }
        Ok(Partition::on_disk(temp_name, part.count))
    }

    /// Merges several sorted partitions into one.
    ///
    /// Equivalent to `OfflineSorter.MergePartitionsTask.call()`.
    fn merge_partitions_task(&self, segments_to_merge: Vec<PartitionFuture>) -> Result<Partition> {
        let mut total_count = 0i64;
        for segment in &segments_to_merge {
            total_count += segment.get_partition()?.count;
        }

        let mut queue = PriorityQueue::new(
            segments_to_merge.len(),
            FileAndTopComparator {
                comparator: self.comparator.clone(),
            },
        )?;

        let start = Instant::now();
        let out = self.create_tracked_temp_output()?;
        let mut writer = self.hooks.get_writer(out, total_count)?;
        let new_segment_name = writer.out().name().to_string();

        // `try (ByteSequencesWriter writer = ...) { ... } finally {
        // IOUtils.close(streams); }` (`OfflineSorter.java:701-752`).
        let mut streams: Vec<ByteSequencesReader> = Vec::with_capacity(segments_to_merge.len());
        let body = (|| -> Result<()> {
            // Open streams and read the top for each file
            for segment in &segments_to_merge {
                let segment = segment.get_partition()?;
                let file_name = segment
                    .file_name
                    .as_deref()
                    .expect("INVARIANT: a merged partition always has a file");
                streams.push(self.open_reader(file_name)?);
                let fd = streams.len() - 1;
                let item = match streams[fd].next() {
                    Ok(item) => item,
                    Err(e) => {
                        let name = streams[fd].name().to_string();
                        return Err(self.verify_checksum(e, &name));
                    }
                };
                let item = item.expect("INVARIANT: a sorted partition is never empty");
                queue.insert_with_overflow(FileAndTop { fd, current: item });
            }

            // Unix utility sort() uses ordered array of files to pick the next
            // line from, updating it as it reads new lines. The PQ used here is
            // a more elegant solution and has a nicer theoretical complexity
            // bound :) The entire sorting process is I/O bound anyway so it
            // shouldn't make much of a difference (didn't check).
            while let Some((fd, current)) = queue.top().map(|top| (top.fd, top.current.clone())) {
                writer.write(current.slice())?;
                let next = match streams[fd].next() {
                    Ok(next) => next,
                    Err(e) => {
                        let name = streams[fd].name().to_string();
                        return Err(self.verify_checksum(e, &name));
                    }
                };
                match next {
                    Some(item) => {
                        queue.update_top_with(FileAndTop { fd, current: item });
                    }
                    None => {
                        queue.pop();
                    }
                }
            }

            codec_util::write_footer(writer.out())?;

            for stream in streams.iter_mut() {
                codec_util::check_footer(stream.input())?;
            }

            self.sort_info().merge_time_ms += start.elapsed().as_millis() as i64;
            Ok(())
        })();
        // The try-with-resources closes the writer after the body; a failure
        // to close is suppressed when the body failed.
        let body = match body {
            Ok(()) => writer.close(),
            Err(error) => {
                let _ = writer.close();
                Err(error)
            }
        };
        // `finally { IOUtils.close(streams); }`: a failure to close a stream
        // replaces the one in flight.
        crate::util::IOUtils::close(streams.iter_mut().map(|stream| Some(|| stream.close())))?;
        body?;

        let mut to_delete = Vec::with_capacity(segments_to_merge.len());
        for segment in &segments_to_merge {
            to_delete.push(segment.get_partition()?.file_name);
        }
        // `IOUtils.deleteFiles(dir, toDelete)` on the tracking directory
        // (`OfflineSorter.java:757`, `IOUtils.java:247-262`): every non-null
        // name is attempted, the first failure is thrown with the later ones
        // suppressed into it.
        let tracking_dir = self.tracking_dir();
        let mut th: Option<LuceneError> = None;
        for name in to_delete.into_iter().flatten() {
            if let Err(t) = tracking_dir.delete_file(&name) {
                match th.as_mut() {
                    None => th = Some(t),
                    Some(first) => first.add_suppressed(t),
                }
            }
        }
        if let Some(th) = th {
            return Err(th);
        }

        Ok(Partition::on_disk(new_segment_name, total_count))
    }
}

/// Sorts a file of byte sequences with an external merge sort.
///
/// Port of `org.apache.lucene.util.OfflineSorter`.
pub struct OfflineSorter {
    value_length: i32,
    ram_buffer_size: BufferSize,
    max_temp_files: usize,
    exec: Arc<dyn Executor>,
    shared: Arc<Shared>,
}

impl OfflineSorter {
    /// Creates a sorter with the default comparator, an automatic buffer size,
    /// [`MAX_TEMPFILES`] temporary files and variable-length values.
    ///
    /// Equivalent to `new OfflineSorter(Directory, String)`.
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`OfflineSorter::with_options`].
    pub fn new(dir: Arc<dyn Directory>, temp_file_name_prefix: &str) -> Result<Self> {
        Self::with_executor(
            dir,
            temp_file_name_prefix,
            OfflineSorterComparator::Natural,
            BufferSize::automatic()?,
            MAX_TEMPFILES,
            -1,
            None,
            0,
        )
    }

    /// Creates a sorter with the given comparator and otherwise default
    /// settings.
    ///
    /// Equivalent to `new OfflineSorter(Directory, String, Comparator)`.
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`OfflineSorter::with_options`].
    pub fn with_comparator(
        dir: Arc<dyn Directory>,
        temp_file_name_prefix: &str,
        comparator: OfflineSorterComparator,
    ) -> Result<Self> {
        Self::with_executor(
            dir,
            temp_file_name_prefix,
            comparator,
            BufferSize::automatic()?,
            MAX_TEMPFILES,
            -1,
            None,
            0,
        )
    }

    /// Creates a fully configured sorter that sorts on the calling thread.
    ///
    /// Equivalent to the all-details constructor with a `null`
    /// `ExecutorService`: see [`OfflineSorter::with_executor`].
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`OfflineSorter::with_executor`].
    pub fn with_options(
        dir: Arc<dyn Directory>,
        temp_file_name_prefix: &str,
        comparator: OfflineSorterComparator,
        ram_buffer_size: BufferSize,
        max_tempfiles: usize,
        value_length: i32,
    ) -> Result<Self> {
        Self::with_executor(
            dir,
            temp_file_name_prefix,
            comparator,
            ram_buffer_size,
            max_tempfiles,
            value_length,
            None,
            0,
        )
    }

    /// All-details constructor.
    ///
    /// Equivalent to `OfflineSorter(Directory, String, Comparator, BufferSize,
    /// int, int, ExecutorService, int)` (`OfflineSorter.java:225-262`). If
    /// `value_length` is `-1` (the default), the length of each value
    /// differs; otherwise, all values have the specified length. A non-`None`
    /// `exec` runs the sorting operations that can run concurrently, and
    /// `max_partitions_in_ram` is the maximum of concurrent in-memory
    /// partitions; without one, a `SameThreadExecutorService` runs them and
    /// `max_partitions_in_ram` is 1. The maximum RAM this sorter uses is
    /// therefore `max_partitions_in_ram * ram_buffer_size`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when an executor is given with
    /// a non-positive `max_partitions_in_ram`, when the buffer is below
    /// [`ABSOLUTE_MIN_SORT_BUFFER_SIZE`], when `max_tempfiles` is below 2, or
    /// when `value_length` is neither `-1` nor in `1..=i16::MAX`.
    #[allow(clippy::too_many_arguments)]
    pub fn with_executor(
        dir: Arc<dyn Directory>,
        temp_file_name_prefix: &str,
        comparator: OfflineSorterComparator,
        ram_buffer_size: BufferSize,
        max_tempfiles: usize,
        value_length: i32,
        exec: Option<Arc<dyn Executor>>,
        max_partitions_in_ram: i32,
    ) -> Result<Self> {
        Self::with_hooks(
            dir,
            temp_file_name_prefix,
            comparator,
            ram_buffer_size,
            max_tempfiles,
            value_length,
            exec,
            max_partitions_in_ram,
            Arc::new(DefaultOfflineSorterHooks),
        )
    }

    /// The all-details constructor of a subclass that overrides
    /// [`OfflineSorterHooks`]' methods.
    ///
    /// # Errors
    ///
    /// See [`OfflineSorter::with_executor`].
    #[allow(clippy::too_many_arguments)]
    pub fn with_hooks(
        dir: Arc<dyn Directory>,
        temp_file_name_prefix: &str,
        comparator: OfflineSorterComparator,
        ram_buffer_size: BufferSize,
        max_tempfiles: usize,
        value_length: i32,
        exec: Option<Arc<dyn Executor>>,
        max_partitions_in_ram: i32,
        hooks: Arc<dyn OfflineSorterHooks>,
    ) -> Result<Self> {
        let mut max_partitions_in_ram = max_partitions_in_ram;
        let exec: Arc<dyn Executor> = match exec {
            Some(exec) => {
                if max_partitions_in_ram <= 0 {
                    return Err(LuceneError::illegal_argument(format!(
                        "maxPartitionsInRAM must be > 0; got {max_partitions_in_ram}"
                    )));
                }
                exec
            }
            None => {
                max_partitions_in_ram = 1;
                Arc::new(crate::util::SameThreadExecutorService::new())
            }
        };
        let partitions_in_ram = Semaphore::new(max_partitions_in_ram);

        if (ram_buffer_size.bytes as i64) < ABSOLUTE_MIN_SORT_BUFFER_SIZE {
            return Err(LuceneError::illegal_argument(format!(
                "{MIN_BUFFER_SIZE_MSG}: {}",
                ram_buffer_size.bytes
            )));
        }
        if max_tempfiles < 2 {
            return Err(LuceneError::illegal_argument(
                "maxTempFiles must be >= 2".to_string(),
            ));
        }
        if value_length != -1 && (value_length == 0 || value_length > i16::MAX as i32) {
            return Err(LuceneError::illegal_argument(format!(
                "valueLength must be 1 .. {}; got: {value_length}",
                i16::MAX
            )));
        }
        let tracking_dir = Shared::new_tracking_dir(&dir);
        Ok(Self {
            value_length,
            ram_buffer_size,
            max_temp_files: max_tempfiles,
            exec,
            shared: Arc::new(Shared {
                dir,
                temp_file_name_prefix: temp_file_name_prefix.to_string(),
                comparator,
                partitions_in_ram,
                hooks,
                sort_info: Mutex::new(SortInfo::new(ram_buffer_size.bytes)),
                tracking_dir: Mutex::new(tracking_dir),
            }),
        })
    }

    /// Returns the directory this sorter writes its temporary files to.
    pub fn get_directory(&self) -> &Arc<dyn Directory> {
        &self.shared.dir
    }

    /// Returns the prefix of every temporary file this sorter creates.
    pub fn get_temp_file_name_prefix(&self) -> &str {
        &self.shared.temp_file_name_prefix
    }

    /// Returns the comparator this sorter orders by.
    pub fn get_comparator(&self) -> &OfflineSorterComparator {
        &self.shared.comparator
    }

    /// Returns the statistics of the last [`OfflineSorter::sort`] run.
    ///
    /// Java reads the field `sortInfo`, whose two timing counters the tasks
    /// update concurrently (`AtomicLong`s); the record is kept under a lock
    /// here, and this returns a copy of it.
    pub fn sort_info(&self) -> SortInfo {
        self.shared.sort_info().clone()
    }

    /// Sort input to a new temp file, returning its name.
    ///
    /// Equivalent to `OfflineSorter.sort(String)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O and corruption errors; any partial output is deleted
    /// first.
    pub fn sort(&mut self, input_file_name: &str) -> Result<String> {
        *self.shared.sort_info() = SortInfo::new(self.ram_buffer_size.bytes);
        // `TrackingDirectoryWrapper trackingDir = new
        // TrackingDirectoryWrapper(dir)` (`OfflineSorter.java:291`).
        *self
            .shared
            .tracking_dir
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Shared::new_tracking_dir(&self.shared.dir);
        let start = Instant::now();

        let outcome = self.sort_inner(input_file_name, start);
        if outcome.is_err() {
            // `IOUtils.deleteFilesIgnoringExceptions(trackingDir,
            // trackingDir.getCreatedFiles())`.
            let tracking_dir = self.shared.tracking_dir();
            let files: Vec<String> = tracking_dir
                .get_created_files_ordered()
                .iter()
                .cloned()
                .collect();
            crate::util::IOUtils::delete_files_ignoring_exceptions_in(tracking_dir.as_ref(), files);
        }
        outcome
    }

    fn sort_inner(&mut self, input_file_name: &str, start: Instant) -> Result<String> {
        let mut segments: Vec<PartitionFuture> = Vec::new();
        let mut level_counts: Vec<i32> = vec![0];

        // `try (ByteSequencesReader is = getReader(dir.openChecksumInput(
        // inputFileName), inputFileName))`.
        let mut is = self.shared.open_reader(input_file_name)?;
        let body = (|| -> Result<String> {
            loop {
                let part = self.read_partition(&mut is)?;
                if part.count == 0 {
                    self.shared.partitions_in_ram.release();
                    debug_assert!(part.exhausted);
                    break;
                }
                let exhausted = part.exhausted;
                let count = part.count;

                let shared = Arc::clone(&self.shared);
                segments.push(self.submit(move || shared.sort_partition_task(part))?);
                {
                    let mut sort_info = self.shared.sort_info();
                    sort_info.temp_merge_files += 1;
                    sort_info.line_count += count;
                }
                level_counts[0] += 1;

                // Handle intermediate merges; we need a while loop to
                // "cascade" the merge when necessary:
                let mut merge_level = 0usize;
                while level_counts[merge_level] == self.max_temp_files as i32 {
                    self.merge_partitions(&mut segments)?;
                    if merge_level + 2 > level_counts.len() {
                        level_counts.resize(merge_level + 2, 0);
                    }
                    level_counts[merge_level + 1] += 1;
                    level_counts[merge_level] = 0;
                    merge_level += 1;
                }

                if exhausted {
                    break;
                }
            }

            // Merge all partitions down to 1 (basically a forceMerge(1)):
            while segments.len() > 1 {
                self.merge_partitions(&mut segments)?;
            }

            let result = if segments.is_empty() {
                let mut out = self.shared.create_tracked_temp_output()?;
                let name = out.name().to_string();
                // Write empty file footer
                let written = codec_util::write_footer(out.as_mut());
                let closed = out.close();
                written.and(closed)?;
                name
            } else {
                segments[0]
                    .get_partition()?
                    .file_name
                    .expect("INVARIANT: a merged partition always has a file")
            };

            // We should be explicitly removing all intermediate files ourselves
            // unless there is an exception:
            debug_assert!({
                let created = self.shared.tracking_dir().get_created_files();
                created.len() == 1 && created.contains(&result)
            });

            self.shared.sort_info().total_time_ms = start.elapsed().as_millis() as i64;

            codec_util::check_footer(is.input())?;

            Ok(result)
        })();
        // The try-with-resources closes the reader; a failure to close is
        // suppressed when the body failed.
        match body {
            Ok(result) => {
                is.close()?;
                Ok(result)
            }
            Err(error) => {
                let _ = is.close();
                Err(error)
            }
        }
    }

    /// `exec.submit(job)`: runs `job` on the executor and returns its future.
    fn submit(
        &self,
        job: impl FnOnce() -> Result<Partition> + Send + 'static,
    ) -> Result<PartitionFuture> {
        let future = PartitionFuture::new();
        let completer = future.clone();
        // **Divergence from Lucene 10.5.0.** Java's `submit` throws the
        // unchecked `RejectedExecutionException`; the error surface has no
        // such variant, so the rejection is an `IllegalStateException`
        // carrying its message.
        self.exec
            .execute(Box::new(move || completer.run(job)))
            .map_err(|rejected| LuceneError::illegal_state(rejected.to_string()))?;
        Ok(future)
    }

    /// Merge the most recent `maxTempFile` partitions into a new partition.
    ///
    /// Equivalent to `OfflineSorter.mergePartitions`.
    fn merge_partitions(&mut self, segments: &mut Vec<PartitionFuture>) -> Result<()> {
        let from = if segments.len() > self.max_temp_files {
            segments.len() - self.max_temp_files
        } else {
            0
        };

        self.shared.sort_info().merge_rounds += 1;

        let segments_to_merge: Vec<PartitionFuture> = segments.drain(from..).collect();
        let shared = Arc::clone(&self.shared);
        segments.push(self.submit(move || shared.merge_partitions_task(segments_to_merge))?);

        self.shared.sort_info().temp_merge_files += 1;
        Ok(())
    }

    /// Read in a single partition of data, setting `exhausted` when there are
    /// no more items.
    ///
    /// Equivalent to `OfflineSorter.readPartition`: a permit of
    /// `partitionsInRAM` is acquired first and released again on failure.
    fn read_partition(&mut self, reader: &mut ByteSequencesReader) -> Result<Partition> {
        self.shared
            .partitions_in_ram
            .acquire()
            .map_err(LuceneError::thread_interrupted)?;
        let outcome = self.read_partition_body(reader);
        if outcome.is_err() {
            self.shared.partitions_in_ram.release();
        }
        outcome
    }

    fn read_partition_body(&mut self, reader: &mut ByteSequencesReader) -> Result<Partition> {
        let start = Instant::now();
        let mut exhausted = false;
        let buffer: Box<dyn SortableBytesRefArray + Send> = if self.value_length != -1 {
            // fixed length case
            let value_length = self.value_length as usize;
            let mut buffer = FixedLengthBytesRefArray::new(value_length);
            let limit = self.ram_buffer_size.bytes / value_length;
            for _ in 0..limit {
                let item = match reader.next() {
                    Ok(item) => item,
                    Err(e) => {
                        let name = reader.name().to_string();
                        return Err(self.shared.verify_checksum(e, &name));
                    }
                };
                match item {
                    None => {
                        exhausted = true;
                        break;
                    }
                    Some(item) => {
                        buffer.append(&item)?;
                    }
                }
            }
            Box::new(buffer)
        } else {
            let buffer_bytes_used = Arc::new(Counter::new_counter());
            let mut buffer = BytesRefArray::new(Arc::clone(&buffer_bytes_used));
            loop {
                let item = match reader.next() {
                    Ok(item) => item,
                    Err(e) => {
                        let name = reader.name().to_string();
                        return Err(self.shared.verify_checksum(e, &name));
                    }
                };
                match item {
                    None => {
                        exhausted = true;
                        break;
                    }
                    Some(item) => {
                        buffer.append(&item)?;
                    }
                }
                // Account for the created objects.
                // (buffer slots do not account to buffer size.)
                if buffer_bytes_used.get() > self.ram_buffer_size.bytes as i64 {
                    break;
                }
            }
            Box::new(buffer)
        };
        self.shared.sort_info().read_time_ms += start.elapsed().as_millis() as i64;
        Ok(Partition::in_memory(buffer, exhausted))
    }

    /// Returns a writer for a temporary output.
    ///
    /// Equivalent to the protected `OfflineSorter.getWriter`, as the hooks
    /// of this sorter define it.
    ///
    /// # Errors
    ///
    /// Propagates what the hooks raise.
    pub fn get_writer(
        &self,
        out: Box<dyn IndexOutput>,
        item_count: i64,
    ) -> Result<ByteSequencesWriter> {
        self.shared.hooks.get_writer(out, item_count)
    }

    /// Returns a reader over `name`: `getReader(dir.openChecksumInput(name),
    /// name)`, as the hooks of this sorter define `getReader`.
    ///
    /// # Errors
    ///
    /// Propagates the directory's open errors and what the hooks raise.
    pub fn get_reader(&self, name: &str) -> Result<ByteSequencesReader> {
        self.shared.open_reader(name)
    }
}

#[cfg(test)]
mod merge_close_tests {
    //! `MergePartitionsTask.call()` (`OfflineSorter.java:701-752`): `finally {
    //! IOUtils.close(streams); }` closes every partition the merge read.
    use super::*;
    use crate::codecs::tests::CloseTrackingDirectory;

    #[test]
    fn a_merge_closes_every_partition_it_read() -> Result<()> {
        let tracking = Arc::new(CloseTrackingDirectory::default());
        let log = Arc::clone(&tracking.log);
        let dir: Arc<dyn Directory> = tracking;
        // Enough values for several partitions of the smallest buffer, and
        // two temporary files at most, so that partitions are merged.
        {
            let out = dir.create_output("unsorted", &*crate::store::DEFAULT_IO_CONTEXT)?;
            let mut writer = ByteSequencesWriter::new(out);
            for i in 0..40_000u32 {
                let value = format!(
                    "{:08}-{}",
                    i.wrapping_mul(2_654_435_761) % 1_000_003,
                    "x".repeat(40)
                );
                writer.write(value.as_bytes())?;
            }
            codec_util::write_footer(writer.out())?;
            writer.close()?;
        }
        let mut sorter = OfflineSorter::with_options(
            Arc::clone(&dir),
            "merge_close",
            OfflineSorterComparator::Natural,
            BufferSize {
                bytes: ABSOLUTE_MIN_SORT_BUFFER_SIZE as usize,
            },
            2,
            -1,
        )?;
        sorter.sort("unsorted")?;
        assert!(
            sorter.sort_info().merge_rounds > 0,
            "the sort merged partitions"
        );
        let partition_closes: Vec<String> = log
            .closed()
            .into_iter()
            .filter(|name| name.starts_with("merge_close"))
            .collect();
        assert!(
            !partition_closes.is_empty(),
            "the merged partitions are closed: {:?}",
            log.closed()
        );
        Ok(())
    }
}
