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
//! * **Concurrency.** Java's eight-argument constructor accepts an
//!   `ExecutorService` and bounds the partitions held in RAM with a
//!   `Semaphore`; when none is supplied it installs a
//!   `SameThreadExecutorService` and forces `maxPartitionsInRAM` to 1. This
//!   port always runs that second configuration — every partition is sorted and
//!   every merge performed on the calling thread — because handing partitions
//!   to another thread would require the [`Directory`] and the comparator to be
//!   shared across threads, which is a design decision for the crate rather
//!   than for this module. The *sequence* of reads, sorts, cascading merges and
//!   final merges is unchanged, so the file produced is identical.
//! * **`BufferSize::automatic`.** Java sizes the buffer from
//!   `Runtime.maxMemory()`, `totalMemory()` and `freeMemory()`. A Rust process
//!   has no managed heap to interrogate, so this port returns the same floor
//!   Java's heuristic is built around, `MIN_BUFFER_SIZE_MB` megabytes.
//! * **Created-file tracking.** Java wraps the directory in a
//!   `TrackingDirectoryWrapper` so that a failure can delete the partial
//!   output. Rucene's `TrackingDirectoryWrapper` takes ownership of a
//!   `Box<dyn Directory>`, which a shared `Arc<dyn Directory>` cannot provide,
//!   so the created names are tracked in the sorter itself — which is all the
//!   wrapper does.

#![deny(unsafe_code)]

use std::collections::HashSet;
use std::fmt::{self, Display, Formatter};
use std::sync::Arc;
use std::time::Instant;

use crate::codecs::codec_util;
use crate::error::{LuceneError, Result};
use crate::store::{ChecksumIndexInput, Directory, IndexOutput};
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
            return Err(LuceneError::IllegalArgument(format!(
                "Buffer too large for Java ({}MB max): {bytes}",
                i32::MAX as i64 / MB
            )));
        }
        if bytes < ABSOLUTE_MIN_SORT_BUFFER_SIZE {
            return Err(LuceneError::IllegalArgument(format!(
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
    /// Returns [`LuceneError::IllegalArgument`] when the size is below
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
    /// Returns [`LuceneError::IllegalArgument`] when the value is longer than
    /// `i16::MAX`, and propagates write errors.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > i16::MAX as usize {
            return Err(LuceneError::IllegalArgument(format!(
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

/// Reads the length-prefixed byte sequences [`ByteSequencesWriter`] produced.
///
/// Port of the nested class `OfflineSorter.ByteSequencesReader`.
pub struct ByteSequencesReader {
    name: String,
    input: Box<dyn ChecksumIndexInput>,
    end: i64,
}

impl ByteSequencesReader {
    /// Wraps `input`, reading up to the start of its codec footer.
    pub fn new(input: Box<dyn ChecksumIndexInput>, name: impl Into<String>) -> Self {
        let end = input.length() - codec_util::footer_length() as i64;
        Self {
            name: name.into(),
            input,
            end,
        }
    }

    /// Returns the name of the file being read.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the underlying input, so that the footer can be checked.
    pub fn input(&mut self) -> &mut dyn ChecksumIndexInput {
        self.input.as_mut()
    }
}

impl BytesRefIterator for ByteSequencesReader {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.input.file_pointer() >= self.end {
            return Ok(None);
        }
        let length = self.input.read_short()? as usize;
        let mut bytes = vec![0u8; length];
        self.input.read_bytes(&mut bytes, 0, length)?;
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
    buffer: Option<Box<dyn SortableBytesRefArray>>,
    exhausted: bool,
    count: i64,
    file_name: Option<String>,
}

impl Partition {
    fn in_memory(buffer: Box<dyn SortableBytesRefArray>, exhausted: bool) -> Self {
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

/// Sorts a file of byte sequences with an external merge sort.
///
/// Port of `org.apache.lucene.util.OfflineSorter`.
pub struct OfflineSorter {
    dir: Arc<dyn Directory>,
    value_length: i32,
    temp_file_name_prefix: String,
    ram_buffer_size: BufferSize,
    max_temp_files: usize,
    comparator: OfflineSorterComparator,
    sort_info: SortInfo,
    /// Files created during the current `sort`, so that a failure can remove
    /// them. Stands in for `TrackingDirectoryWrapper.getCreatedFiles()`.
    created_files: HashSet<String>,
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
        Self::with_options(
            dir,
            temp_file_name_prefix,
            OfflineSorterComparator::Natural,
            BufferSize::automatic()?,
            MAX_TEMPFILES,
            -1,
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
        Self::with_options(
            dir,
            temp_file_name_prefix,
            comparator,
            BufferSize::automatic()?,
            MAX_TEMPFILES,
            -1,
        )
    }

    /// Creates a fully configured sorter.
    ///
    /// `value_length` is `-1` for variable-length values, or the exact length
    /// every value has. See the module documentation for the two constructor
    /// parameters Java has that this port does not.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the buffer is below
    /// [`ABSOLUTE_MIN_SORT_BUFFER_SIZE`], when `max_tempfiles` is below 2, or
    /// when `value_length` is neither `-1` nor in `1..=i16::MAX`.
    pub fn with_options(
        dir: Arc<dyn Directory>,
        temp_file_name_prefix: &str,
        comparator: OfflineSorterComparator,
        ram_buffer_size: BufferSize,
        max_tempfiles: usize,
        value_length: i32,
    ) -> Result<Self> {
        if (ram_buffer_size.bytes as i64) < ABSOLUTE_MIN_SORT_BUFFER_SIZE {
            return Err(LuceneError::IllegalArgument(format!(
                "{MIN_BUFFER_SIZE_MSG}: {}",
                ram_buffer_size.bytes
            )));
        }
        if max_tempfiles < 2 {
            return Err(LuceneError::IllegalArgument(
                "maxTempFiles must be >= 2".to_string(),
            ));
        }
        if value_length != -1 && (value_length == 0 || value_length > i16::MAX as i32) {
            return Err(LuceneError::IllegalArgument(format!(
                "valueLength must be 1 .. {}; got: {value_length}",
                i16::MAX
            )));
        }
        Ok(Self {
            dir,
            value_length,
            temp_file_name_prefix: temp_file_name_prefix.to_string(),
            ram_buffer_size,
            max_temp_files: max_tempfiles,
            comparator,
            sort_info: SortInfo::new(ram_buffer_size.bytes),
            created_files: HashSet::new(),
        })
    }

    /// Returns the directory this sorter writes its temporary files to.
    pub fn get_directory(&self) -> &Arc<dyn Directory> {
        &self.dir
    }

    /// Returns the prefix of every temporary file this sorter creates.
    pub fn get_temp_file_name_prefix(&self) -> &str {
        &self.temp_file_name_prefix
    }

    /// Returns the comparator this sorter orders by.
    pub fn get_comparator(&self) -> &OfflineSorterComparator {
        &self.comparator
    }

    /// Returns the statistics of the last [`OfflineSorter::sort`] run.
    pub fn sort_info(&self) -> &SortInfo {
        &self.sort_info
    }

    /// Sorts `input_file_name` and returns the name of the sorted file.
    ///
    /// # Errors
    ///
    /// Propagates I/O and corruption errors; any partial output is deleted
    /// first.
    pub fn sort(&mut self, input_file_name: &str) -> Result<String> {
        self.sort_info = SortInfo::new(self.ram_buffer_size.bytes);
        self.created_files.clear();
        let start = Instant::now();

        let outcome = self.sort_inner(input_file_name, start);
        if outcome.is_err() {
            // Remove any partially written temp file, as Java's `finally` does.
            let files: Vec<String> = self.created_files.iter().cloned().collect();
            for name in files {
                let _ = self.dir.delete_file(&name);
            }
        }
        outcome
    }

    fn sort_inner(&mut self, input_file_name: &str, start: Instant) -> Result<String> {
        let mut segments: Vec<Partition> = Vec::new();
        let mut level_counts: Vec<i32> = vec![0];

        let mut is = self.get_reader(input_file_name)?;
        loop {
            let part = self.read_partition(&mut is)?;
            if part.count == 0 {
                debug_assert!(part.exhausted);
                break;
            }
            let exhausted = part.exhausted;
            let count = part.count;

            let sorted = self.sort_partition_task(part)?;
            segments.push(sorted);
            self.sort_info.temp_merge_files += 1;
            self.sort_info.line_count += count;
            level_counts[0] += 1;

            // Handle intermediate merges; the loop cascades them when needed.
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

        // Merge all partitions down to one, i.e. a forceMerge(1).
        while segments.len() > 1 {
            self.merge_partitions(&mut segments)?;
        }

        let result = if segments.is_empty() {
            let mut out = self.dir.create_temp_output(
                &self.temp_file_name_prefix,
                "sort",
                &*crate::store::DEFAULT_IO_CONTEXT,
            )?;
            let name = out.name().to_string();
            self.created_files.insert(name.clone());
            // Write the empty file footer.
            codec_util::write_footer(out.as_mut())?;
            out.close()?;
            name
        } else {
            segments[0]
                .file_name
                .clone()
                .expect("INVARIANT: a merged partition always has a file")
        };

        debug_assert!(self.created_files.len() == 1 && self.created_files.contains(&result));

        self.sort_info.total_time_ms = start.elapsed().as_millis() as i64;

        codec_util::check_footer(is.input())?;

        Ok(result)
    }

    /// Verifies the checksum of a reader whose `next()` already failed.
    ///
    /// Equivalent to the private `OfflineSorter.verifyChecksum`.
    fn verify_checksum(&self, prior: LuceneError, name: &str) -> LuceneError {
        match self.dir.open_checksum_input(name) {
            Ok(mut input) => codec_util::check_footer_with_prior(input.as_mut(), prior),
            Err(e) => e,
        }
    }

    /// Merges the newest partitions into one.
    ///
    /// Equivalent to `OfflineSorter.mergePartitions`.
    fn merge_partitions(&mut self, segments: &mut Vec<Partition>) -> Result<()> {
        let from = segments.len().saturating_sub(self.max_temp_files);
        let from = if segments.len() > self.max_temp_files {
            from
        } else {
            0
        };
        let to_merge: Vec<Partition> = segments.drain(from..).collect();

        self.sort_info.merge_rounds += 1;

        let merged = self.merge_partitions_task(to_merge)?;
        segments.push(merged);

        self.sort_info.temp_merge_files += 1;
        Ok(())
    }

    /// Reads one partition of the input into memory.
    ///
    /// Equivalent to `OfflineSorter.readPartition`.
    fn read_partition(&mut self, reader: &mut ByteSequencesReader) -> Result<Partition> {
        let start = Instant::now();
        let mut exhausted = false;
        let buffer: Box<dyn SortableBytesRefArray> = if self.value_length != -1 {
            // Fixed-length case.
            let value_length = self.value_length as usize;
            let mut buffer = FixedLengthBytesRefArray::new(value_length);
            let limit = self.ram_buffer_size.bytes / value_length;
            for _ in 0..limit {
                let item = match reader.next() {
                    Ok(item) => item,
                    Err(e) => {
                        let name = reader.name().to_string();
                        return Err(self.verify_checksum(e, &name));
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
                        return Err(self.verify_checksum(e, &name));
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
                // Account for the created objects; buffer slots do not count
                // towards the buffer size.
                if buffer_bytes_used.get() > self.ram_buffer_size.bytes as i64 {
                    break;
                }
            }
            Box::new(buffer)
        };
        self.sort_info.read_time_ms += start.elapsed().as_millis() as i64;
        Ok(Partition::in_memory(buffer, exhausted))
    }

    /// Returns a writer for a temporary output.
    ///
    /// Equivalent to the protected `OfflineSorter.getWriter`.
    pub fn get_writer(&self, out: Box<dyn IndexOutput>, _item_count: i64) -> ByteSequencesWriter {
        ByteSequencesWriter::new(out)
    }

    /// Returns a reader over `name`.
    ///
    /// Equivalent to the protected `OfflineSorter.getReader`.
    ///
    /// # Errors
    ///
    /// Propagates the directory's open errors.
    pub fn get_reader(&self, name: &str) -> Result<ByteSequencesReader> {
        let input = self.dir.open_checksum_input(name)?;
        Ok(ByteSequencesReader::new(input, name))
    }

    /// Sorts one in-memory partition and spills it to a temporary file.
    ///
    /// Equivalent to the nested `OfflineSorter.SortPartitionTask`.
    fn sort_partition_task(&mut self, part: Partition) -> Result<Partition> {
        let mut buffer = part
            .buffer
            .expect("INVARIANT: an unsorted partition is always in memory");
        let out = self.dir.create_temp_output(
            &self.temp_file_name_prefix,
            "sort",
            &*crate::store::DEFAULT_IO_CONTEXT,
        )?;
        let temp_name = out.name().to_string();
        self.created_files.insert(temp_name.clone());
        let mut writer = self.get_writer(out, buffer.size() as i64);

        let start = Instant::now();
        let mut count = 0i64;
        {
            let comparator = self.comparator.clone();
            let compare = move |a: &BytesRef, b: &BytesRef| comparator.compare(a, b);
            let mut iter = buffer.sorted_iterator(StringSorterComparator::Generic(&compare))?;
            self.sort_info.sort_time_ms += start.elapsed().as_millis() as i64;

            while let Some(spare) = iter.next()? {
                writer.write(spare.slice())?;
                count += 1;
            }
        }
        debug_assert_eq!(count, part.count);

        codec_util::write_footer(writer.out())?;
        buffer.clear();
        writer.close()?;

        Ok(Partition::on_disk(temp_name, part.count))
    }

    /// Merges several sorted partitions into one.
    ///
    /// Equivalent to the nested `OfflineSorter.MergePartitionsTask`.
    fn merge_partitions_task(&mut self, segments_to_merge: Vec<Partition>) -> Result<Partition> {
        let total_count: i64 = segments_to_merge.iter().map(|p| p.count).sum();

        let mut queue = PriorityQueue::new(
            segments_to_merge.len(),
            FileAndTopComparator {
                comparator: self.comparator.clone(),
            },
        )?;

        let start = Instant::now();
        let out = self.dir.create_temp_output(
            &self.temp_file_name_prefix,
            "sort",
            &*crate::store::DEFAULT_IO_CONTEXT,
        )?;
        let new_segment_name = out.name().to_string();
        self.created_files.insert(new_segment_name.clone());
        let mut writer = self.get_writer(out, total_count);

        // Open every stream and read its first value.
        let mut streams: Vec<ByteSequencesReader> = Vec::with_capacity(segments_to_merge.len());
        for segment in &segments_to_merge {
            let file_name = segment
                .file_name
                .as_deref()
                .expect("INVARIANT: a merged partition always has a file");
            let mut stream = self.get_reader(file_name)?;
            let item = match stream.next() {
                Ok(item) => item,
                Err(e) => {
                    let name = stream.name().to_string();
                    return Err(self.verify_checksum(e, &name));
                }
            };
            let item = item.expect("INVARIANT: a sorted partition is never empty");
            let fd = streams.len();
            queue.insert_with_overflow(FileAndTop { fd, current: item });
            streams.push(stream);
        }

        // A priority queue keeps the merge O(n log k); the process is I/O bound
        // anyway, as Lucene notes.
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

        self.sort_info.merge_time_ms += start.elapsed().as_millis() as i64;
        writer.close()?;

        for segment in &segments_to_merge {
            if let Some(name) = segment.file_name.as_deref() {
                self.dir.delete_file(name)?;
                self.created_files.remove(name);
            }
        }

        Ok(Partition::on_disk(new_segment_name, total_count))
    }
}
