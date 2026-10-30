#![allow(unsafe_code)]

//! Memory-mapped directory implementation for trusted indexes.
//!
//! This module provides [`MMapDirectory`], gated by the crate feature `mmap`. It uses the `memmap2` crate to memory-map index files
//! into the process address space, which is the read path used by
//! `org.apache.lucene.store.MMapDirectory` in Lucene 10.5.0 on 64-bit
//! platforms.
//!
//! # Safety note
//!
//! Memory mapping keeps the underlying file accessible through process memory.
//! This is generally safe for indexes produced or verified by this process,
//! mirroring Lucene's own trust model. The module intentionally does not expose
//! any `unsafe` API surface; `memmap2` encapsulates the platform `mmap` call.
//! Users who need to read untrusted indexes should use [`FSDirectory`] or
//! [`NIOFSDirectory`] instead.
//!
//! [`FSDirectory`]: super::FSDirectory
//! [`NIOFSDirectory`]: super::NIOFSDirectory

use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use memmap2::Mmap;

use super::memory_segment_index_input_provider::{MemorySegmentIndexInputProvider, SharedArenas};
use super::{
    hints_of, DataAccessHint, Directory, FSDirectory, IOContext, IndexInput, IndexOutput, Lock,
    LockFactory, PreloadHint, ReadAdvice, ReadOnceHint,
};
use crate::error::{LuceneError, Result};
use std::collections::HashSet;

/// Decides which files to bring into physical memory when they are opened.
///
/// Equivalent to the `BiPredicate<String, IOContext>` Java passes to
/// `MMapDirectory.setPreload`.
pub type PreloadPredicate = Box<dyn Fn(&str, &dyn IOContext) -> bool + Send + Sync>;

/// Decides the [`ReadAdvice`] a file is opened with, or `None` to take the
/// default.
///
/// Equivalent to the `BiFunction<String, IOContext, Optional<ReadAdvice>>` Java
/// passes to `MMapDirectory.setReadAdvice`.
pub type ReadAdvicePolicy = Box<dyn Fn(&str, &dyn IOContext) -> Option<ReadAdvice> + Send + Sync>;

/// Returns the logical group a file belongs to, so that the files of one index
/// segment share their mapping's lifetime.
///
/// Equivalent to the `Function<String, Optional<String>>` Java passes to
/// `MMapDirectory.setGroupingFunction`.
pub type GroupingFunction = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Maps `len` bytes of `file`, starting at byte `offset`, read-only.
///
/// This is the crate's single memory-mapping primitive: every other module that
/// needs a mapping — including
/// [`memory_segment`](super::memory_segment), the port of Lucene's Java 21
/// `java.lang.foreign` implementation — goes through it, so that the one
/// `unsafe` call the operation requires lives in exactly one place.
///
/// # Safety of the call site
///
/// `memmap2::MmapOptions::map` is `unsafe` because a memory mapping aliases a
/// file that another process may truncate or overwrite while it is mapped, and
/// Rust cannot express that hazard in the type system. This is Lucene's own
/// trust model: `MMapDirectory` assumes the index files it maps are not
/// modified behind its back, and the same assumption applies here. Callers must
/// only map files inside a `Directory` they control.
///
/// `offset` need not be page-aligned: `memmap2` maps from the enclosing page
/// boundary and adjusts the returned pointer.
///
/// # Errors
///
/// Returns the underlying `mmap` failure, including the refusal of `memmap2` to
/// create a zero-length mapping; callers that need an empty region must not
/// call this function.
pub(crate) fn map_read_only(file: &fs::File, offset: u64, len: usize) -> io::Result<Mmap> {
    // SAFETY: see the "Safety of the call site" section above. The caller
    // guarantees the mapped file is not concurrently modified; nothing else can
    // be guaranteed statically for any memory mapping.
    unsafe {
        memmap2::MmapOptions::new()
            .offset(offset)
            .len(len)
            .map(file)
    }
}

/// Default maximum chunk size for memory mapping: 16 GiB on 64-bit targets
/// and 256 MiB on 32-bit ones.
///
/// Equivalent to `MMapDirectory.DEFAULT_MAX_CHUNK_SIZE`, which Java assigns
/// from `PROVIDER.getDefaultMaxChunkSize()` in a static initializer
/// (`MMapDirectory.java:427-430`).
pub static DEFAULT_MAX_CHUNK_SIZE: LazyLock<usize> =
    LazyLock::new(|| MMapDirectory::lookup_provider().get_default_max_chunk_size() as usize);

/// Filesystem-backed [`Directory`] that memory-maps files for reading.
///
/// Equivalent to `org.apache.lucene.store.MMapDirectory`. Writing reuses
/// [`FSIndexOutput`](super::FSIndexOutput); reading goes through
/// [`MemorySegmentIndexInputProvider`], as Java's `openInput` goes through
/// `PROVIDER`.
pub struct MMapDirectory {
    inner: FSDirectory,
    /// Equivalent to `MMapDirectory.chunkSizePower`.
    chunk_size_power: i32,
    /// Equivalent to `MMapDirectory.attachment`, the provider's per-directory
    /// arena map.
    attachment: SharedArenas,
    /// The current read-advice policy. Each input's `toReadAdvice` reads it
    /// when it is called, as Java's lambda reads the `readAdvice` field
    /// (`MMapDirectory.java:317-318`), so a later
    /// [`set_read_advice`](Self::set_read_advice) reaches inputs already open.
    read_advice: Arc<std::sync::RwLock<ReadAdvicePolicy>>,
    preload: PreloadPredicate,
    grouping_function: GroupingFunction,
}

impl MMapDirectory {
    /// Opens a new `MMapDirectory` at `path` using the default lock factory and
    /// chunk size.
    ///
    /// Equivalent to `MMapDirectory(Path)` in Lucene 10.5.0.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::with_options(path, None, *DEFAULT_MAX_CHUNK_SIZE)
    }

    /// Opens a new `MMapDirectory` at `path` with the supplied lock factory.
    pub fn with_lock_factory(
        path: impl AsRef<Path>,
        lock_factory: Option<Box<dyn LockFactory>>,
    ) -> Result<Self> {
        Self::with_options(path, lock_factory, *DEFAULT_MAX_CHUNK_SIZE)
    }

    /// Opens a new `MMapDirectory` with a custom maximum chunk size, which is
    /// rounded down to a power of two.
    ///
    /// Equivalent to `MMapDirectory(Path, LockFactory, long)`
    /// (`MMapDirectory.java:250-258`).
    pub fn with_options(
        path: impl AsRef<Path>,
        lock_factory: Option<Box<dyn LockFactory>>,
        max_chunk_size: usize,
    ) -> Result<Self> {
        if max_chunk_size == 0 {
            return Err(LuceneError::IllegalArgument(
                "Maximum chunk size for mmap must be >0".to_string(),
            ));
        }
        // Java's `long maxChunkSize` cannot exceed `Long.MAX_VALUE`; a `usize`
        // can, and its power would not be one Java can hold.
        if i64::try_from(max_chunk_size).is_err() {
            return Err(LuceneError::IllegalArgument(format!(
                "Maximum chunk size for mmap must be <= {} (a Java long)",
                i64::MAX
            )));
        }
        let inner = FSDirectory::with_lock_factory(path, lock_factory)?;
        // `Long.SIZE - 1 - Long.numberOfLeadingZeros(maxChunkSize)`
        let chunk_size_power = (u64::BITS - 1 - (max_chunk_size as u64).leading_zeros()) as i32;
        Ok(Self {
            inner,
            chunk_size_power,
            attachment: Self::lookup_provider().attachment(),
            read_advice: Arc::new(std::sync::RwLock::new(Box::new(|_filename, _context| None))),
            preload: Self::no_files(),
            grouping_function: Self::group_by_segment(),
        })
    }

    /// Returns the canonical filesystem path backing this directory.
    pub fn directory_path(&self) -> &Path {
        self.inner.directory_path()
    }

    /// Returns how many shared arenas the directory currently keeps, one per
    /// file group with an input still open.
    #[cfg(test)]
    pub(crate) fn shared_arena_count(&self) -> usize {
        self.attachment
            .lock()
            .map(|arenas| arenas.len())
            .unwrap_or(0)
    }

    /// Returns the current mmap chunk size, the configured maximum rounded
    /// down to a power of two.
    ///
    /// Equivalent to `MMapDirectory.getMaxChunkSize()`
    /// (`MMapDirectory.java:307-309`).
    pub fn max_chunk_size(&self) -> usize {
        1usize << self.chunk_size_power
    }

    /// Returns the predicate that preloads every file.
    ///
    /// Equivalent to `MMapDirectory.ALL_FILES`.
    pub fn all_files() -> PreloadPredicate {
        Box::new(|_filename, _context| true)
    }

    /// Returns the predicate that preloads nothing, which is the default.
    ///
    /// Equivalent to `MMapDirectory.NO_FILES`.
    pub fn no_files() -> PreloadPredicate {
        Box::new(|_filename, _context| false)
    }

    /// Returns the predicate that preloads a file when its context asks for it.
    ///
    /// Equivalent to `MMapDirectory.PRELOAD_HINT`.
    pub fn preload_hint() -> PreloadPredicate {
        Box::new(|_filename, context| {
            context
                .hints()
                .iter()
                .any(|hint| hint.as_any().downcast_ref::<PreloadHint>().is_some())
        })
    }

    /// Returns the grouping function that groups nothing.
    ///
    /// Equivalent to `MMapDirectory.NO_GROUPING`.
    pub fn no_grouping() -> GroupingFunction {
        Box::new(|_filename| None)
    }

    /// Returns the grouping function that groups a file by the index segment it
    /// belongs to, which is the default.
    ///
    /// Equivalent to `MMapDirectory.GROUP_BY_SEGMENT`.
    pub fn group_by_segment() -> GroupingFunction {
        Box::new(|filename| {
            if !crate::index::is_codec_file(filename) {
                return None;
            }
            let mut group_key = crate::index::parse_segment_name(filename)
                .get(1..)
                .unwrap_or_default()
                .to_string();
            // keep the original generation (=0) in base group, later
            // generations in extra group; a name that does not conform to the
            // generation syntax, or trash, keeps the base group
            if let Ok(generation) = crate::index::parse_generation(filename) {
                if generation > 0 {
                    group_key.push_str("-g");
                }
            }
            Some(group_key)
        })
    }

    /// Returns the policy that derives the read advice from the context: the
    /// merge and flush contexts read sequentially, an explicit
    /// [`DataAccessHint`] wins otherwise, and everything else takes the default.
    ///
    /// Equivalent to `MMapDirectory.ADVISE_BY_CONTEXT`.
    pub fn advise_by_context() -> ReadAdvicePolicy {
        Box::new(|_filename, context| {
            if context.context() == super::Context::Merge
                || context.context() == super::Context::Flush
            {
                return Some(ReadAdvice::Sequential);
            }
            for hint in context.hints() {
                if let Some(access) = hint.as_any().downcast_ref::<DataAccessHint>() {
                    return match access {
                        DataAccessHint::Random => Some(ReadAdvice::Random),
                        DataAccessHint::Sequential => Some(ReadAdvice::Sequential),
                    };
                }
            }
            Some(crate::util::Constants::default_read_advice())
        })
    }

    /// Configures which files to bring into physical memory when they are
    /// opened. The default preloads nothing. The behaviour is best effort and
    /// operating-system dependent.
    ///
    /// Equivalent to `org.apache.lucene.store.MMapDirectory#setPreload`.
    pub fn set_preload(&mut self, preload: PreloadPredicate) {
        self.preload = preload;
    }

    /// Configures the [`ReadAdvice`] certain files are opened with. The default
    /// policy returns `None` for every file, so every file takes
    /// `Constants::default_read_advice()`.
    ///
    /// Equivalent to `org.apache.lucene.store.MMapDirectory#setReadAdvice`.
    pub fn set_read_advice(&mut self, to_read_advice: ReadAdvicePolicy) {
        *self
            .read_advice
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = to_read_advice;
    }

    /// Configures the grouping of files that belong to the same logical group,
    /// which is a hint that lets their resources be handled together. The
    /// default is [`group_by_segment`](Self::group_by_segment); pass
    /// [`no_grouping`](Self::no_grouping) to disable it.
    ///
    /// The group key is handed to the provider, which picks the shared arena
    /// that unmaps the files of one group together.
    ///
    /// Equivalent to
    /// `org.apache.lucene.store.MMapDirectory#setGroupingFunction`.
    pub fn set_grouping_function(&mut self, grouping_function: GroupingFunction) {
        self.grouping_function = grouping_function;
    }

    /// Returns the provider that opens memory-mapped inputs.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Java resolves `MemorySegmentIndexInputProvider` reflectively, with
    /// `MethodHandles.lookup().findClass(...)` and `findConstructor(...)`, so
    /// that a Java 21 class can be loaded from a multi-release JAR without the
    /// rest of the code depending on it at compile time, and raises
    /// `LinkageError` when it is absent. Rust has no multi-release artefacts
    /// and no reflection: the provider is an ordinary item of this crate, so
    /// the lookup is a direct reference to it and cannot fail. It is built once
    /// and shared, which is what Java's `static final PROVIDER` gives.
    ///
    /// Equivalent to `org.apache.lucene.store.MMapDirectory#lookupProvider`.
    pub fn lookup_provider() -> &'static MemorySegmentIndexInputProvider {
        static PROVIDER: LazyLock<MemorySegmentIndexInputProvider> =
            LazyLock::new(MemorySegmentIndexInputProvider::new);
        &PROVIDER
    }

    /// Returns whether this platform's `madvise` syscall can be used to advise
    /// the kernel how a mapping will be read.
    ///
    /// Equivalent to `org.apache.lucene.store.MMapDirectory#supportsMadvise`.
    pub fn supports_madvise() -> bool {
        Self::lookup_provider().supports_madvise()
    }
}

impl Directory for MMapDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)
    }

    /// Creates an input for the file with the given name, mapped by the
    /// provider.
    ///
    /// Equivalent to `MMapDirectory.openInput(String, IOContext)`
    /// (`MMapDirectory.java:311-328`).
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.ensure_open()?;
        self.inner.ensure_can_read(name)?;
        let path = self.inner.directory_path().join(name);
        let read_advice = Arc::clone(&self.read_advice);
        let file_name = name.to_string();
        // `c -> readAdvice.apply(name, c).orElse(Constants.DEFAULT_READADVICE)`
        let to_read_advice: super::memory_segment_index_input::ToReadAdvice =
            Arc::new(move |context: &dyn IOContext| {
                let policy = read_advice
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (*policy)(&file_name, context)
                    .unwrap_or_else(crate::util::Constants::default_read_advice)
            });
        let input = Self::lookup_provider().open_input(
            &path,
            self.chunk_size_power,
            context,
            to_read_advice,
            // `context.hints().contains(ReadOnceHint.INSTANCE)`
            !hints_of::<ReadOnceHint>(context).is_empty(),
            (self.preload)(name, context),
            (self.grouping_function)(name).as_deref(),
            &self.attachment,
        )?;
        Ok(Box::new(input))
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }

    fn fs_directory_path(&self) -> Option<&Path> {
        self.inner.fs_directory_path()
    }

    fn directory_type_name(&self) -> &'static str {
        "MMapDirectory"
    }

    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

impl std::fmt::Display for MMapDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MMapDirectory@{} maxChunkSize={}",
            self.inner.directory_path().display(),
            self.max_chunk_size()
        )
    }
}

impl std::fmt::Debug for MMapDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MMapDirectory")
            .field("directory", &self.inner.directory_path())
            .field("max_chunk_size", &self.max_chunk_size())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::DEFAULT_IO_CONTEXT;
    use tempfile::TempDir;

    #[test]
    fn mmap_directory_round_trip() {
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;

        {
            let mut out = dir.create_output("data.bin", context).unwrap();
            out.write_int(0x12345678_i32).unwrap();
            out.write_string("mmap round-trip").unwrap();
            out.close().unwrap();
        }

        {
            let mut input = dir.open_input("data.bin", context).unwrap();
            assert_eq!(input.length(), 20);
            assert_eq!(input.read_int().unwrap(), 0x12345678_i32);
            assert_eq!(input.read_string().unwrap(), "mmap round-trip");
        }

        assert_eq!(dir.directory_type_name(), "MMapDirectory");
    }

    #[test]
    fn mmap_index_input_slices_and_clones() {
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;

        {
            let mut out = dir.create_output("data.bin", context).unwrap();
            out.write_long(0x1111_2222_3333_4444_i64).unwrap();
            out.write_long(0x5555_6666_7777_8888_i64).unwrap();
            out.close().unwrap();
        }

        let input = dir.open_input("data.bin", context).unwrap();
        let mut slice = input.slice("slice", 8, 8).unwrap();
        assert_eq!(slice.length(), 8);
        assert_eq!(slice.read_long().unwrap(), 0x5555_6666_7777_8888_i64);

        let mut clone = input.clone_input().unwrap();
        assert_eq!(clone.length(), 16);
        assert_eq!(clone.read_long().unwrap(), 0x1111_2222_3333_4444_i64);
    }

    /// `MMapDirectory.openInput` hands the file to `PROVIDER.openInput` with
    /// the directory's `chunkSizePower` (`MMapDirectory.java:311-328`), so an
    /// input is a memory-segment input mapped in chunks of
    /// `getMaxChunkSize()` bytes, the configured maximum rounded down to a
    /// power of two (`MMapDirectory.java:250-258`).
    #[test]
    fn open_input_maps_through_the_provider_in_chunks() {
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::with_options(&temp, None, 1000).unwrap();
        assert_eq!(dir.max_chunk_size(), 512);
        let context = &*DEFAULT_IO_CONTEXT;
        {
            let mut out = dir.create_output("data.bin", context).unwrap();
            for i in 0..600i64 {
                out.write_long(i).unwrap();
            }
            out.close().unwrap();
        }

        let mut input = dir.open_input("data.bin", context).unwrap();
        assert!(
            input.as_memory_segment_access_input().is_some(),
            "the provider maps the file as a memory-segment input"
        );
        assert_eq!(input.length(), 4800);
        for i in 0..600i64 {
            assert_eq!(input.read_long().unwrap(), i);
        }
    }

    /// Writes `count` bytes `0, 1, 2, ...` (mod 256) into `name`.
    fn write_counting_bytes(dir: &MMapDirectory, name: &str, count: usize) {
        let mut out = dir.create_output(name, &*DEFAULT_IO_CONTEXT).unwrap();
        for i in 0..count {
            out.write_byte(i as u8).unwrap();
        }
        out.close().unwrap();
    }

    fn expected_long(at: usize) -> i64 {
        let bytes: Vec<u8> = (at..at + 8).map(|i| i as u8).collect();
        i64::from_le_bytes(bytes.try_into().unwrap())
    }

    /// With 16-byte chunks, multi-byte reads at positions that are not a
    /// multiple of eight straddle a chunk boundary, `readBytes` spans several
    /// chunks, seeks land in any chunk, and a slice spanning chunks reads the
    /// same bytes (`MemorySegmentIndexInput.java` `MultiSegmentImpl`).
    #[test]
    fn reads_seeks_and_slices_cross_chunk_boundaries() {
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::with_options(&temp, None, 16).unwrap();
        assert_eq!(dir.max_chunk_size(), 16);
        write_counting_bytes(&dir, "data.bin", 200);
        let mut input = dir.open_input("data.bin", &*DEFAULT_IO_CONTEXT).unwrap();

        // A long starting at 13 spans chunks 0 and 1.
        input.seek(13).unwrap();
        assert_eq!(input.read_long().unwrap(), expected_long(13));
        assert_eq!(input.file_pointer(), 21);
        // An int at 30 spans chunks 1 and 2.
        input.seek(30).unwrap();
        assert_eq!(
            input.read_int().unwrap(),
            i32::from_le_bytes([30, 31, 32, 33])
        );

        // A bulk read across five chunks.
        input.seek(7).unwrap();
        let mut bulk = vec![0u8; 75];
        input.read_bytes(&mut bulk, 0, 75).unwrap();
        assert_eq!(bulk, (7..82).map(|i| i as u8).collect::<Vec<_>>());

        // Seeks back and forth across chunks.
        for &pos in &[199i64, 0, 64, 15, 16, 177] {
            input.seek(pos).unwrap();
            assert_eq!(input.read_byte().unwrap(), pos as u8);
        }

        // A slice from 10 to 100 spans six chunks; its own position 3 is the
        // file's 13.
        let mut slice = input.slice("span", 10, 90).unwrap();
        assert_eq!(slice.length(), 90);
        slice.seek(3).unwrap();
        assert_eq!(slice.read_long().unwrap(), expected_long(13));
        let mut rest = vec![0u8; 79];
        slice.read_bytes(&mut rest, 0, 79).unwrap();
        assert_eq!(rest, (21..100).map(|i| i as u8).collect::<Vec<_>>());
        assert!(slice.read_byte().is_err(), "the slice ends at its length");

        // Random access across a boundary.
        let mut random = input.random_access_slice(0, 200).unwrap();
        assert_eq!(random.read_long_at(45).unwrap(), expected_long(45));
    }

    /// `IndexInput.updateIOContext` reaches `MemorySegmentIndexInput`'s
    /// override, which re-derives the read advice from the new context through
    /// the directory's `toReadAdvice` (`MemorySegmentIndexInput.java:384-386`,
    /// `MMapDirectory.java:317-318`); the default would not consult it.
    #[test]
    fn update_io_context_reaches_the_mapped_input() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let temp = TempDir::new().unwrap();
        let mut dir = MMapDirectory::open(&temp).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        dir.set_read_advice(Box::new(move |_name, _context| {
            counted.fetch_add(1, Ordering::SeqCst);
            None
        }));
        let context = &*DEFAULT_IO_CONTEXT;
        {
            let mut out = dir.create_output("data.bin", context).unwrap();
            out.write_long(7).unwrap();
            out.close().unwrap();
        }
        let input = dir.open_input("data.bin", context).unwrap();
        let before = calls.load(Ordering::SeqCst);
        input.update_io_context(context).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), before + 1);
    }

    /// A maximum chunk size is a Java `long`: one above `Long.MAX_VALUE` is
    /// rejected, as is zero.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn a_chunk_size_beyond_a_java_long_is_rejected() {
        let temp = TempDir::new().unwrap();
        assert!(matches!(
            MMapDirectory::with_options(&temp, None, usize::MAX),
            Err(LuceneError::IllegalArgument(_))
        ));
        assert!(matches!(
            MMapDirectory::with_options(&temp, None, 0),
            Err(LuceneError::IllegalArgument(_))
        ));
        assert_eq!(
            MMapDirectory::with_options(&temp, None, i64::MAX as usize)
                .unwrap()
                .max_chunk_size(),
            1usize << 62
        );
    }

    /// `toReadAdvice` reads the directory's policy when it is called, as
    /// Java's lambda reads the `readAdvice` field (`MMapDirectory.java:317-318`):
    /// a policy set after an input was opened is the one its
    /// `updateIOContext` consults.
    #[test]
    fn update_io_context_reads_the_policy_set_after_opening() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let temp = TempDir::new().unwrap();
        let mut dir = MMapDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;
        {
            let mut out = dir.create_output("data.bin", context).unwrap();
            out.write_long(7).unwrap();
            out.close().unwrap();
        }
        let input = dir.open_input("data.bin", context).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        dir.set_read_advice(Box::new(move |_name, _context| {
            counted.fetch_add(1, Ordering::SeqCst);
            None
        }));
        input.update_io_context(context).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// The default maximum chunk size is the provider's
    /// (`MMapDirectory.java:427-430`).
    #[test]
    fn default_max_chunk_size_is_the_providers() {
        assert_eq!(
            *DEFAULT_MAX_CHUNK_SIZE as i64,
            MMapDirectory::lookup_provider().get_default_max_chunk_size()
        );
        // `Constants.JRE_IS_64BIT ? (1L << 34) : (1L << 28)`
        // (`MemorySegmentIndexInputProvider.java:93`).
        #[cfg(target_pointer_width = "64")]
        assert_eq!(*DEFAULT_MAX_CHUNK_SIZE, 1usize << 34);
        #[cfg(not(target_pointer_width = "64"))]
        assert_eq!(*DEFAULT_MAX_CHUNK_SIZE, 1usize << 28);
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::open(&temp).unwrap();
        assert_eq!(dir.max_chunk_size(), *DEFAULT_MAX_CHUNK_SIZE);
    }
}
