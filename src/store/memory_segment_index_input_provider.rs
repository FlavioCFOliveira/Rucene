//! Opens memory-mapped inputs, chunking a file into memory segments.
//!
//! Ported from `org.apache.lucene.store.MemorySegmentIndexInputProvider` (the
//! Java 21 source under `lucene/core/src/java21`), which implements
//! `MMapDirectory.MMapIndexInputProvider`.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::memory_segment::{
    Arena, ConfinedArena, MemorySegment, RefCountedSharedArena, SharedArena, DEFAULT_MAX_PERMITS,
};
use super::memory_segment_index_input::{MemorySegmentIndexInput, ToReadAdvice};
use super::native_access;
use super::{IOContext, ReadAdvice};
use crate::error::{LuceneError, Result};

/// Name of the setting that caps how many files may share one arena.
///
/// Equivalent to `MMapDirectory.SHARED_ARENA_MAX_PERMITS_SYSPROP`.
///
/// # Divergence from Lucene 10.5.0
///
/// Java reads it as a JVM system property. Rust has no system properties, so
/// this port reads an environment variable of the same name; an unparsable or
/// out-of-range value is ignored with a warning, exactly as in Java.
pub const SHARED_ARENA_MAX_PERMITS_SYSPROP: &str =
    "org.apache.lucene.store.MMapDirectory.sharedArenaMaxPermits";

/// The map of per-group arenas a directory keeps across `open_input` calls.
///
/// Equivalent to the `ConcurrentHashMap<String, RefCountedSharedArena>` Lucene
/// stores as `MMapDirectory.attachment`. Create one with
/// [`MemorySegmentIndexInputProvider::attachment`].
pub type SharedArenas = Arc<Mutex<HashMap<String, Arc<RefCountedSharedArena>>>>;

/// Opens memory-mapped [`MemorySegmentIndexInput`]s.
///
/// Equivalent to `org.apache.lucene.store.MemorySegmentIndexInputProvider`,
/// which is the implementation of `MMapDirectory.MMapIndexInputProvider` used
/// on Java 21 and later. It maps a file as a sequence of chunks of
/// `1 << chunk_size_power` bytes, applies the read advice or preloading the
/// caller asked for, and groups the arenas of files belonging to the same index
/// segment so they are unmapped together.
#[derive(Debug)]
pub struct MemorySegmentIndexInputProvider {
    native_access: Option<&'static dyn native_access::NativeAccess>,
    shared_arena_max_permits: i32,
}

impl Default for MemorySegmentIndexInputProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MemorySegmentIndexInputProvider {
    /// Creates the provider, reading the shared-arena permit budget from the
    /// environment.
    ///
    /// Equivalent to the `MemorySegmentIndexInputProvider()` constructor.
    pub fn new() -> Self {
        Self {
            native_access: native_access::get_implementation(),
            shared_arena_max_permits: Self::check_max_permits(
                Self::shared_arena_max_permits_setting(),
            ),
        }
    }

    /// Returns a fresh, empty arena map for one directory.
    ///
    /// Equivalent to `MemorySegmentIndexInputProvider.attachment()`.
    pub fn attachment(&self) -> SharedArenas {
        Arc::new(Mutex::new(HashMap::new()))
    }

    /// Returns the default maximum chunk size: 16 GiB on 64-bit targets and
    /// 256 MiB on 32-bit ones.
    ///
    /// Equivalent to
    /// `MemorySegmentIndexInputProvider.getDefaultMaxChunkSize()`.
    pub fn get_default_max_chunk_size(&self) -> i64 {
        if usize::BITS >= 64 {
            1i64 << 34
        } else {
            1i64 << 28
        }
    }

    /// Returns whether this platform can advise the kernel about read patterns.
    ///
    /// Equivalent to `MemorySegmentIndexInputProvider.supportsMadvise()`.
    pub fn supports_madvise(&self) -> bool {
        self.native_access.is_some()
    }

    /// Maps `path` and returns an input over it.
    ///
    /// Equivalent to
    /// `MemorySegmentIndexInputProvider.openInput(Path, int, IOContext, Function, boolean, boolean, Optional, ConcurrentHashMap)`.
    ///
    /// * `chunk_size_power` — base-two logarithm of the chunk size.
    /// * `context` — the context the file is being opened for; `to_read_advice`
    ///   turns it into the advice to apply.
    /// * `confined` — restrict the input to the calling thread, which Lucene
    ///   does for read-once contexts.
    /// * `preload` — bring the whole file into physical memory up front.
    /// * `group` — the arena group key, so that files of one index segment
    ///   share an arena; `None` gives the file an arena of its own.
    /// * `arenas` — the directory's arena map, from
    ///   [`attachment`](Self::attachment).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the file needs more chunks
    /// than can be addressed, and [`LuceneError::Io`] if the file cannot be
    /// opened or mapped. A mapping failure is reported with the same diagnostic
    /// advice Lucene attaches in `convertMapFailedIOException`.
    #[allow(clippy::too_many_arguments)]
    pub fn open_input(
        &self,
        path: &Path,
        chunk_size_power: i32,
        context: &dyn IOContext,
        to_read_advice: ToReadAdvice,
        confined: bool,
        preload: bool,
        group: Option<&str>,
        arenas: &SharedArenas,
    ) -> Result<MemorySegmentIndexInput> {
        let resource_description = format!("MemorySegmentIndexInput(path=\"{}\")", path.display());

        let arena: Arc<dyn Arena> = if confined {
            Arc::new(ConfinedArena::new())
        } else {
            self.get_shared_arena(group, arenas)?
        };

        let opened = (|| -> Result<MemorySegmentIndexInput> {
            let file = File::open(path)?;
            let file_size = file.metadata()?.len() as i64;
            let segments = self.map(
                &arena,
                &resource_description,
                &file,
                to_read_advice(context),
                chunk_size_power,
                preload,
                file_size,
            )?;
            MemorySegmentIndexInput::new_instance(
                resource_description.clone(),
                Arc::clone(&arena),
                segments,
                file_size,
                chunk_size_power,
                confined,
                Arc::clone(&to_read_advice),
            )
        })();

        if opened.is_err() {
            arena.close();
        }
        opened
    }

    /// Maps the whole file as a sequence of chunks.
    ///
    /// Equivalent to the private `MemorySegmentIndexInputProvider.map(...)`.
    #[allow(clippy::too_many_arguments)]
    fn map(
        &self,
        arena: &Arc<dyn Arena>,
        resource_description: &str,
        file: &File,
        read_advice: ReadAdvice,
        chunk_size_power: i32,
        preload: bool,
        length: i64,
    ) -> Result<Vec<MemorySegment>> {
        if (length >> chunk_size_power) >= i32::MAX as i64 {
            return Err(LuceneError::IllegalArgument(format!(
                "File too big for chunk size: {resource_description}"
            )));
        }

        let chunk_size = 1i64 << chunk_size_power;
        // One more segment is always allocated; the last one may be zero bytes.
        let nr_segments = (length >> chunk_size_power) + 1;
        let mut segments = Vec::with_capacity(nr_segments as usize);

        let mut start_offset = 0i64;
        for _ in 0..nr_segments {
            let seg_size = if length > start_offset + chunk_size {
                chunk_size
            } else {
                length - start_offset
            };
            let segment = MemorySegment::map(
                file,
                start_offset as u64,
                seg_size as usize,
                Arc::clone(arena.scope()),
            )
            .map_err(|error| {
                Self::convert_map_failed_error(error, resource_description, seg_size)
            })?;

            // If preloading, apply it without madvise. Skip madvise when the
            // address of the segment is not page-aligned, which happens for
            // small segments.
            if preload {
                segment.load()?;
            } else if read_advice != ReadAdvice::Normal {
                // No need to madvise with ReadAdvice::Normal, since it is the
                // OS' default read advice.
                if let Some(native) = self.native_access {
                    if segment.address() % native.get_page_size() == 0 {
                        native.madvise(&segment, read_advice)?;
                    }
                }
            }
            segments.push(segment);
            start_offset += seg_size;
        }
        Ok(segments)
    }

    /// Gets an arena for `group`, aggregating files of the same index segment
    /// into a single reference-counted shared arena, which is added to `arenas`
    /// when it is created.
    ///
    /// Equivalent to the private
    /// `MemorySegmentIndexInputProvider.getSharedArena(Optional, ConcurrentHashMap)`.
    fn get_shared_arena(
        &self,
        group: Option<&str>,
        arenas: &SharedArenas,
    ) -> Result<Arc<dyn Arena>> {
        let Some(key) = group else {
            return Ok(Arc::new(SharedArena::new()));
        };

        let mut map = arenas.lock().map_err(|_| {
            LuceneError::IllegalState("shared arena map mutex poisoned".to_string())
        })?;

        // `computeIfAbsent`.
        let arena = match map.get(key) {
            Some(existing) => Arc::clone(existing),
            None => {
                let created = self.new_ref_counted(key, arenas)?;
                map.insert(key.to_string(), Arc::clone(&created));
                created
            }
        };
        if arena.acquire() {
            return Ok(arena);
        }

        // The permits of that arena are exhausted; `compute` a replacement.
        if let Some(current) = map.get(key) {
            let current = Arc::clone(current);
            if current.acquire() {
                return Ok(current);
            }
        }
        let replacement = self.new_ref_counted(key, arenas)?;
        // Guaranteed to succeed on a fresh arena.
        replacement.acquire();
        map.insert(key.to_string(), Arc::clone(&replacement));
        Ok(replacement)
    }

    /// Creates a reference-counted arena that removes itself from `arenas` when
    /// its last reference is released.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Java's `onClose` runnable captures the map strongly and relies on the
    /// garbage collector to break the resulting cycle. Reference counting
    /// cannot, so the callback holds a [`std::sync::Weak`] and does nothing if
    /// the directory that owns the map is already gone.
    fn new_ref_counted(
        &self,
        key: &str,
        arenas: &SharedArenas,
    ) -> Result<Arc<RefCountedSharedArena>> {
        let weak = Arc::downgrade(arenas);
        let owned_key = key.to_string();
        let on_close = Box::new(move || {
            if let Some(map) = weak.upgrade() {
                if let Ok(mut guard) = map.lock() {
                    guard.remove(&owned_key);
                }
            }
        });
        Ok(Arc::new(RefCountedSharedArena::with_max_permits(
            key,
            on_close,
            self.shared_arena_max_permits,
        )?))
    }

    /// Reads the shared-arena permit budget from the environment.
    ///
    /// Equivalent to
    /// `MemorySegmentIndexInputProvider.getSharedArenaMaxPermitsSysprop()`.
    fn shared_arena_max_permits_setting() -> i32 {
        match std::env::var(SHARED_ARENA_MAX_PERMITS_SYSPROP) {
            Ok(value) => match value.parse::<i32>() {
                Ok(parsed) => parsed,
                Err(_) => {
                    log::warn!(
                        "Cannot read setting {SHARED_ARENA_MAX_PERMITS_SYSPROP}, so the default value will be used."
                    );
                    DEFAULT_MAX_PERMITS
                }
            },
            Err(std::env::VarError::NotPresent) => DEFAULT_MAX_PERMITS,
            Err(_) => {
                log::warn!(
                    "Cannot read setting {SHARED_ARENA_MAX_PERMITS_SYSPROP}, so the default value will be used."
                );
                DEFAULT_MAX_PERMITS
            }
        }
    }

    /// Validates the permit budget, falling back to the default with a warning.
    ///
    /// Equivalent to `MemorySegmentIndexInputProvider.checkMaxPermits(int)`.
    fn check_max_permits(max_permits: i32) -> i32 {
        if RefCountedSharedArena::valid_max_permits(max_permits) {
            return max_permits;
        }
        log::warn!(
            "Invalid value for setting {SHARED_ARENA_MAX_PERMITS_SYSPROP}, must be positive and <= 0x07FF. The default value will be used."
        );
        DEFAULT_MAX_PERMITS
    }

    /// Rewrites a mapping failure with the diagnostic advice Lucene attaches.
    ///
    /// Equivalent to
    /// `MMapDirectory.MMapIndexInputProvider.convertMapFailedIOException(IOException, String, long)`.
    fn convert_map_failed_error(
        error: LuceneError,
        resource_description: &str,
        buf_size: i64,
    ) -> LuceneError {
        let more_info = if usize::BITS < 64 {
            "MMapDirectory should only be used on 64bit platforms, because the address space on 32bit operating systems is too small. "
        } else if cfg!(target_os = "windows") {
            "Windows is unfortunately very limited on virtual address space. If your index size is several hundred Gigabytes, consider changing to Linux. "
        } else if cfg!(target_os = "linux") {
            "Please review 'ulimit -v', 'ulimit -m' (both should return 'unlimited'), and 'sysctl vm.max_map_count'. "
        } else {
            "Please review 'ulimit -v', 'ulimit -m' (both should return 'unlimited'). "
        };
        LuceneError::Io(std::io::Error::other(format!(
            "{error}: {resource_description} [this may be caused by lack of enough unfragmented \
             virtual address space or too restrictive virtual memory limits enforced by the \
             operating system, preventing us to map a chunk of {buf_size} bytes. {more_info}More \
             information: https://blog.thetaphi.de/2012/07/use-lucenes-mmapdirectory-on-64bit.html]"
        )))
    }
}
