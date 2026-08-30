//! The mapped-memory layer underneath Lucene's Java 21 `MMapDirectory`
//! implementation, plus the reference-counted shared arena built on it.
//!
//! Lucene's `lucene/core/src/java21/org/apache/lucene/store` sources are written
//! against `java.lang.foreign`: a `MemorySegment` is a view over a region of
//! memory, and an `Arena` owns the lifetime of every segment allocated from it.
//! Rust has no equivalent standard library abstraction, so this module provides
//! the two concepts directly over `memmap2` mappings. Only what Lucene actually
//! uses is modelled: mapping a file region, slicing a segment, asking for its
//! size and address, and closing a group of segments together.
//!
//! [`RefCountedSharedArena`] is a port of the Lucene class of the same name.
//!
//! # Divergences from Lucene 10.5.0
//!
//! * **Closing does not invalidate memory in use.** Java's `Arena.ofShared()`
//!   unmaps its segments the moment `close()` is called; other threads still
//!   reading through a clone then get an `IllegalStateException`, which
//!   `MemorySegmentIndexInput` converts into an `AlreadyClosedException`. Rust
//!   cannot free memory that safe references still borrow, so a closed
//!   [`Arena`] instead marks its [`Scope`] dead — every input then reports
//!   `AlreadyClosed`, exactly as in Java — while the mapping itself is released
//!   when the last clone holding it is dropped. The observable contract of
//!   `close()` is preserved; only the instant of unmapping differs, and it
//!   differs in the safe direction.
//! * **`Arena.allocate` has no counterpart.** Lucene never allocates from these
//!   arenas — `RefCountedSharedArena.allocate` throws
//!   `UnsupportedOperationException` — so the [`Arena`] trait models lifetime
//!   only.
//! * **`MemorySegment.isLoaded()` cannot be answered.** It is backed by
//!   `mincore`, which has no safe wrapper here; [`MemorySegment::is_loaded`]
//!   returns `None`, the "no guarantee" answer the `IndexInput` contract
//!   already allows.
//! * **`RefCountedSharedArena`'s `onClose` callback holds a
//!   [`std::sync::Weak`]**, not a strong reference, to the map it removes
//!   itself from. Java relies on the garbage collector to break that cycle;
//!   reference counting cannot.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::ThreadId;

use memmap2::Mmap;

use super::ReadAdvice;
use crate::error::{LuceneError, Result};

/// The lifetime marker shared by every [`MemorySegment`] of one [`Arena`].
///
/// Equivalent to `java.lang.foreign.MemorySegment.Scope`. A scope starts alive
/// and becomes dead when its arena is closed; it never becomes alive again.
#[derive(Debug)]
pub struct Scope {
    alive: AtomicBool,
    /// The thread allowed to access segments of a confined arena, or `None`
    /// for a shared arena.
    owner: Option<ThreadId>,
}

impl Scope {
    /// Creates a live scope confined to `owner`, or unconfined when `owner` is
    /// `None`.
    fn new(owner: Option<ThreadId>) -> Arc<Self> {
        Arc::new(Self {
            alive: AtomicBool::new(true),
            owner,
        })
    }

    /// Returns `true` while the owning arena is open.
    ///
    /// Equivalent to `MemorySegment.Scope.isAlive()`.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Returns `true` if `thread` may access segments governed by this scope.
    ///
    /// Equivalent to `MemorySegment.isAccessibleBy(Thread)`.
    pub fn is_accessible_by(&self, thread: ThreadId) -> bool {
        match self.owner {
            Some(owner) => owner == thread,
            None => true,
        }
    }

    /// Marks the scope dead. Idempotent.
    fn close(&self) {
        self.alive.store(false, Ordering::Release);
    }
}

/// Owns the lifetime of a group of [`MemorySegment`]s.
///
/// Equivalent to `java.lang.foreign.Arena` as Lucene uses it: the segments of
/// one file — or of one whole index segment, when they are grouped — share an
/// arena, and closing it invalidates them all at once.
pub trait Arena: std::fmt::Debug + Send + Sync {
    /// Returns the scope governing every segment of this arena.
    ///
    /// Equivalent to `Arena.scope()`.
    fn scope(&self) -> &Arc<Scope>;

    /// Closes the arena, marking its scope dead.
    ///
    /// Equivalent to `Arena.close()`. Closing twice is harmless.
    fn close(&self);
}

/// An arena whose segments may only be accessed by the thread that created it.
///
/// Equivalent to `Arena.ofConfined()`.
#[derive(Debug)]
pub struct ConfinedArena {
    scope: Arc<Scope>,
}

impl ConfinedArena {
    /// Creates a confined arena owned by the calling thread.
    ///
    /// Equivalent to `Arena.ofConfined()`.
    pub fn new() -> Self {
        Self {
            scope: Scope::new(Some(std::thread::current().id())),
        }
    }
}

impl Default for ConfinedArena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena for ConfinedArena {
    fn scope(&self) -> &Arc<Scope> {
        &self.scope
    }

    fn close(&self) {
        self.scope.close();
    }
}

/// An arena whose segments may be accessed by any thread.
///
/// Equivalent to `Arena.ofShared()`.
#[derive(Debug)]
pub struct SharedArena {
    scope: Arc<Scope>,
}

impl SharedArena {
    /// Creates a shared arena.
    ///
    /// Equivalent to `Arena.ofShared()`.
    pub fn new() -> Self {
        Self {
            scope: Scope::new(None),
        }
    }
}

impl Default for SharedArena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena for SharedArena {
    fn scope(&self) -> &Arc<Scope> {
        &self.scope
    }

    fn close(&self) {
        self.scope.close();
    }
}

/// Default maximum number of acquires one [`RefCountedSharedArena`] will ever
/// hand out.
///
/// Equivalent to `RefCountedSharedArena.DEFAULT_MAX_PERMITS`.
pub const DEFAULT_MAX_PERMITS: i32 = 64;

/// State value meaning the arena is closed.
const CLOSED: i32 = 0;
/// Minimum value beyond which permits are exhausted.
const REMAINING_UNIT: i32 = 1 << 16;
/// Acquire decrement: effectively decrements permits and increments ref count.
const ACQUIRE_DECREMENT: i32 = REMAINING_UNIT - 1;

/// A reference counted shared [`Arena`].
///
/// Equivalent to `org.apache.lucene.store.RefCountedSharedArena`.
///
/// The purpose of this type is to let a number of mapped memory segments share
/// a single underlying arena, so the arena is not closed until all of the
/// segments are. Typically those segments belong to the same logical group —
/// the individual files of one index segment — and grouping them avoids the
/// expensive cost of closing a shared arena once per file.
///
/// The reference count is increased by [`acquire`](Self::acquire) and decreased
/// by [`release`](Self::release). When it reaches zero the underlying arena is
/// closed and the `on_close` callback runs; no more references can be acquired.
///
/// Independently of the reference count, the total number of acquires over the
/// lifetime of one instance is capped at `max_permits`. Once they are exhausted
/// [`acquire`](Self::acquire) returns `false` forever, which is what makes a
/// long-lived group eventually roll over to a fresh arena.
pub struct RefCountedSharedArena {
    segment_name: String,
    /// `on_close` may run more than once if closing the inner arena fails, as
    /// in Java.
    on_close: Box<dyn Fn() + Send + Sync>,
    arena: SharedArena,
    /// High 16 bits hold the total remaining acquires and decrease
    /// monotonically; the low 16 bits hold the current reference count.
    state: AtomicI32,
}

impl RefCountedSharedArena {
    /// Creates an arena for `segment_name` with [`DEFAULT_MAX_PERMITS`].
    ///
    /// Equivalent to `RefCountedSharedArena(String, Runnable)`.
    pub fn new(segment_name: impl Into<String>, on_close: Box<dyn Fn() + Send + Sync>) -> Self {
        Self::with_max_permits(segment_name, on_close, DEFAULT_MAX_PERMITS)
            .expect("DEFAULT_MAX_PERMITS is valid")
    }

    /// Creates an arena for `segment_name` with an explicit permit budget.
    ///
    /// Equivalent to `RefCountedSharedArena(String, Runnable, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `max_permits` is not
    /// accepted by [`valid_max_permits`](Self::valid_max_permits).
    pub fn with_max_permits(
        segment_name: impl Into<String>,
        on_close: Box<dyn Fn() + Send + Sync>,
        max_permits: i32,
    ) -> Result<Self> {
        if !Self::valid_max_permits(max_permits) {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid max permits: {max_permits}"
            )));
        }
        Ok(Self {
            segment_name: segment_name.into(),
            on_close,
            arena: SharedArena::new(),
            state: AtomicI32::new(max_permits << 16),
        })
    }

    /// Returns `true` if `value` is a usable permit budget.
    ///
    /// Equivalent to `RefCountedSharedArena.validMaxPermits(int)`.
    pub fn valid_max_permits(value: i32) -> bool {
        value > 0 && value <= 0x7FFF
    }

    /// Returns the group name this arena was created for. For debugging.
    ///
    /// Equivalent to `RefCountedSharedArena.getSegmentName()`.
    pub fn get_segment_name(&self) -> &str {
        &self.segment_name
    }

    /// Increments the reference count.
    ///
    /// Returns `true` if it was increased, or `false` when there are no
    /// remaining acquires.
    ///
    /// Equivalent to `RefCountedSharedArena.acquire()`.
    pub fn acquire(&self) -> bool {
        loop {
            let value = self.state.load(Ordering::SeqCst);
            if value < REMAINING_UNIT {
                return false;
            }
            if self
                .state
                .compare_exchange(
                    value,
                    value - ACQUIRE_DECREMENT,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Decrements the reference count, closing the underlying arena when it
    /// reaches zero.
    ///
    /// Equivalent to `RefCountedSharedArena.release()`.
    pub fn release(&self) {
        loop {
            let value = self.state.load(Ordering::SeqCst);
            let count = value & 0xFFFF;
            let new_value = if count <= 1 { CLOSED } else { value - 1 };
            if self
                .state
                .compare_exchange(value, new_value, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                if new_value == CLOSED {
                    (self.on_close)();
                    self.arena.close();
                }
                return;
            }
        }
    }
}

impl Arena for RefCountedSharedArena {
    fn scope(&self) -> &Arc<Scope> {
        self.arena.scope()
    }

    fn close(&self) {
        self.release();
    }
}

impl std::fmt::Debug for RefCountedSharedArena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for RefCountedSharedArena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RefCountedArena[segmentName={}, value={}, arena={:?}]",
            self.segment_name,
            self.state.load(Ordering::SeqCst),
            self.arena
        )
    }
}

/// A view over a region of mapped memory.
///
/// Equivalent to `java.lang.foreign.MemorySegment` as Lucene uses it: a
/// read-only window on part of a memory-mapped file, governed by the [`Scope`]
/// of the [`Arena`] it was mapped into.
///
/// Cloning is cheap — it shares the underlying mapping — and a segment stays
/// readable while any clone of it lives, even after its scope is closed. Code
/// that must honour the closed state checks [`is_alive`](Self::is_alive); see
/// the module documentation for why the two are separate here and not in Java.
#[derive(Clone)]
pub struct MemorySegment {
    /// `None` for a zero-length segment: `mmap` refuses an empty mapping, while
    /// Lucene deliberately creates one trailing zero-byte segment per file.
    mapping: Option<Arc<Mmap>>,
    offset: usize,
    len: usize,
    scope: Arc<Scope>,
}

impl MemorySegment {
    /// Maps `len` bytes of `file` starting at `file_offset` into `scope`.
    ///
    /// Equivalent to `FileChannel.map(MapMode.READ_ONLY, offset, size, arena)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] if the mapping cannot be created.
    pub fn map(
        file: &std::fs::File,
        file_offset: u64,
        len: usize,
        scope: Arc<Scope>,
    ) -> Result<Self> {
        if len == 0 {
            return Ok(Self::empty(scope));
        }
        let mapping = super::mmap::map_read_only(file, file_offset, len)?;
        Ok(Self {
            mapping: Some(Arc::new(mapping)),
            offset: 0,
            len,
            scope,
        })
    }

    /// Creates a zero-length segment in `scope`.
    ///
    /// Equivalent to mapping a zero-byte region, which Lucene does for the
    /// trailing segment of every file.
    pub fn empty(scope: Arc<Scope>) -> Self {
        Self {
            mapping: None,
            offset: 0,
            len: 0,
            scope,
        }
    }

    /// Returns the number of bytes in this segment.
    ///
    /// Equivalent to `MemorySegment.byteSize()`.
    pub fn byte_size(&self) -> i64 {
        self.len as i64
    }

    /// Returns the address of the first byte of this segment, or `0` for an
    /// empty segment.
    ///
    /// Equivalent to `MemorySegment.address()`. Forming the address does not
    /// dereference anything and needs no `unsafe`; it is used only to compute
    /// the offset of the segment within an operating-system page.
    pub fn address(&self) -> usize {
        match &self.mapping {
            Some(mapping) => (mapping.as_ptr() as usize).wrapping_add(self.offset),
            None => 0,
        }
    }

    /// Returns the scope governing this segment.
    ///
    /// Equivalent to `MemorySegment.scope()`.
    pub fn scope(&self) -> &Arc<Scope> {
        &self.scope
    }

    /// Returns `true` while the arena that mapped this segment is open.
    pub fn is_alive(&self) -> bool {
        self.scope.is_alive()
    }

    /// Returns the bytes of this segment.
    ///
    /// The slice stays valid for as long as this segment lives, whether or not
    /// its scope is still alive; callers that must observe the closed state
    /// check [`is_alive`](Self::is_alive) first.
    pub fn bytes(&self) -> &[u8] {
        match &self.mapping {
            Some(mapping) => &mapping[self.offset..self.offset + self.len],
            None => &[],
        }
    }

    /// Returns the sub-segment of `len` bytes starting at `offset`, or `None`
    /// if that range is not contained in this segment.
    ///
    /// Equivalent to `MemorySegment.asSlice(long, long)`, which throws
    /// `IndexOutOfBoundsException` where this returns `None`; callers turn that
    /// into whichever error Lucene raises at their call site.
    pub fn as_slice(&self, offset: i64, len: i64) -> Option<Self> {
        if offset < 0 || len < 0 {
            return None;
        }
        let end = offset.checked_add(len)?;
        if end > self.byte_size() {
            return None;
        }
        Some(Self {
            mapping: self.mapping.clone(),
            offset: self.offset + offset as usize,
            len: len as usize,
            scope: Arc::clone(&self.scope),
        })
    }

    /// Returns whether this segment's contents are resident in physical memory.
    ///
    /// Equivalent to `MemorySegment.isLoaded()`, which is backed by `mincore`.
    /// There is no safe wrapper for it here, so the answer is always `None`,
    /// meaning "unknown".
    pub fn is_loaded(&self) -> Option<bool> {
        None
    }

    /// Asks the operating system to bring this segment into physical memory.
    ///
    /// Equivalent to `MemorySegment.load()`. Implemented with
    /// `madvise(MADV_WILLNEED)`, which is what the JDK issues for `load()`; it
    /// is a hint, so it is a no-op on platforms without `madvise`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] if the advice call fails.
    pub fn load(&self) -> Result<()> {
        self.advise_os(OsReadAdvice::WillNeed)
    }

    /// Issues `madvise` for the whole of this segment.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] if the advice call fails.
    pub(crate) fn advise_os(&self, advice: OsReadAdvice) -> Result<()> {
        // Empty segments are excluded, because they may have no address at all.
        let Some(mapping) = &self.mapping else {
            return Ok(());
        };
        if self.len == 0 {
            return Ok(());
        }
        #[cfg(unix)]
        {
            mapping.advise_range(advice.to_memmap2(), self.offset, self.len)?;
        }
        #[cfg(not(unix))]
        {
            let _ = (mapping, advice);
        }
        Ok(())
    }

    /// Reads the byte at `pos`, or `None` if `pos` is out of bounds.
    pub fn get_u8(&self, pos: i64) -> Option<u8> {
        let bytes = self.bytes();
        let pos = usize::try_from(pos).ok()?;
        bytes.get(pos).copied()
    }

    /// Reads a little-endian `i16` at `pos`, or `None` if the two bytes are not
    /// fully contained in this segment.
    pub fn get_i16_le(&self, pos: i64) -> Option<i16> {
        self.window::<2>(pos).map(i16::from_le_bytes)
    }

    /// Reads a little-endian `i32` at `pos`, or `None` if the four bytes are
    /// not fully contained in this segment.
    pub fn get_i32_le(&self, pos: i64) -> Option<i32> {
        self.window::<4>(pos).map(i32::from_le_bytes)
    }

    /// Reads a little-endian `i64` at `pos`, or `None` if the eight bytes are
    /// not fully contained in this segment.
    pub fn get_i64_le(&self, pos: i64) -> Option<i64> {
        self.window::<8>(pos).map(i64::from_le_bytes)
    }

    /// Reads a little-endian `f32` at `pos`, or `None` if the four bytes are
    /// not fully contained in this segment.
    pub fn get_f32_le(&self, pos: i64) -> Option<f32> {
        self.window::<4>(pos).map(f32::from_le_bytes)
    }

    /// Copies `dst.len()` bytes starting at `pos` into `dst`.
    ///
    /// Returns `false` — leaving `dst` untouched — if the range is not fully
    /// contained in this segment. Equivalent to `MemorySegment.copy` with a
    /// byte layout.
    pub fn copy_to(&self, pos: i64, dst: &mut [u8]) -> bool {
        let bytes = self.bytes();
        let Ok(pos) = usize::try_from(pos) else {
            return false;
        };
        let Some(end) = pos.checked_add(dst.len()) else {
            return false;
        };
        if end > bytes.len() {
            return false;
        }
        dst.copy_from_slice(&bytes[pos..end]);
        true
    }

    /// Returns the `N` bytes starting at `pos`, if they fit in this segment.
    fn window<const N: usize>(&self, pos: i64) -> Option<[u8; N]> {
        let bytes = self.bytes();
        let pos = usize::try_from(pos).ok()?;
        let end = pos.checked_add(N)?;
        if end > bytes.len() {
            return None;
        }
        let mut buffer = [0u8; N];
        buffer.copy_from_slice(&bytes[pos..end]);
        Some(buffer)
    }
}

impl std::fmt::Debug for MemorySegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySegment")
            .field("address", &self.address())
            .field("byte_size", &self.len)
            .field("alive", &self.is_alive())
            .finish()
    }
}

/// The subset of `madvise` advice values Lucene issues.
///
/// Equivalent to the `POSIX_MADV_*` constants
/// [`PosixNativeAccess`](super::PosixNativeAccess) binds. The values are
/// identical between `posix_madvise` and `madvise` for all four.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OsReadAdvice {
    /// No further special treatment. `POSIX_MADV_NORMAL`.
    Normal,
    /// Expect random page references. `POSIX_MADV_RANDOM`.
    Random,
    /// Expect sequential page references. `POSIX_MADV_SEQUENTIAL`.
    Sequential,
    /// Will need these pages. `POSIX_MADV_WILLNEED`.
    WillNeed,
}

impl OsReadAdvice {
    /// Maps a Lucene [`ReadAdvice`] onto the operating system's advice value.
    ///
    /// Equivalent to `PosixNativeAccess.mapReadAdvice(ReadAdvice)`.
    pub(crate) fn from_read_advice(read_advice: ReadAdvice) -> Self {
        match read_advice {
            ReadAdvice::Normal => Self::Normal,
            ReadAdvice::Random => Self::Random,
            ReadAdvice::Sequential => Self::Sequential,
        }
    }

    #[cfg(unix)]
    fn to_memmap2(self) -> memmap2::Advice {
        match self {
            Self::Normal => memmap2::Advice::Normal,
            Self::Random => memmap2::Advice::Random,
            Self::Sequential => memmap2::Advice::Sequential,
            Self::WillNeed => memmap2::Advice::WillNeed,
        }
    }
}
