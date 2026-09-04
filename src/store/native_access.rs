//! Access to the operating system's `madvise` hooks.
//!
//! Ported from `org.apache.lucene.store.NativeAccess` and
//! `org.apache.lucene.store.PosixNativeAccess` (the Java 21 sources under
//! `lucene/core/src/java21`).

use std::sync::LazyLock;

use super::memory_segment::{MemorySegment, OsReadAdvice};
use super::ReadAdvice;
use crate::error::{LuceneError, Result};

/// Page size assumed when the operating system cannot be asked for it.
///
/// See [`PosixNativeAccess::get_page_size`] for when this is used and why an
/// inaccurate value here cannot make an advice call fail.
const FALLBACK_PAGE_SIZE: usize = 4096;

/// Access to the native calls Lucene uses to advise the kernel about how a
/// mapping will be read.
///
/// Equivalent to the package-private abstract class
/// `org.apache.lucene.store.NativeAccess`. Obtain the implementation for the
/// running platform with [`get_implementation`].
pub trait NativeAccess: std::fmt::Debug + Send + Sync {
    /// Invokes `madvise` for `segment` with the advice matching `read_advice`.
    ///
    /// Equivalent to `NativeAccess.madvise(MemorySegment, ReadAdvice)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] if the call fails.
    fn madvise(&self, segment: &MemorySegment, read_advice: ReadAdvice) -> Result<()>;

    /// Invokes `madvise` for `segment` with `MADV_WILLNEED`.
    ///
    /// Equivalent to `NativeAccess.madviseWillNeed(MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] if the call fails.
    fn madvise_will_need(&self, segment: &MemorySegment) -> Result<()>;

    /// Returns the native page size.
    ///
    /// Equivalent to `NativeAccess.getPageSize()`.
    fn get_page_size(&self) -> usize;
}

/// Returns the [`NativeAccess`] implementation for this platform, if there is
/// one.
///
/// Equivalent to the static `NativeAccess.getImplementation()`. As in Lucene,
/// only Linux and macOS are supported, and only on 64-bit targets; everywhere
/// else this returns `None` and every caller falls back to doing nothing, which
/// is the same path Lucene takes when the native bindings cannot be
/// initialised.
///
/// # Divergence from Lucene 10.5.0
///
/// Java's static method lives on the `NativeAccess` class. A trait with a
/// static method is not object-safe in Rust, and every caller needs
/// `&dyn NativeAccess`, so the lookup is a module-level function instead.
pub fn get_implementation() -> Option<&'static dyn NativeAccess> {
    PosixNativeAccess::get_instance()
}

/// `madvise` and page-size access through the POSIX interface.
///
/// Equivalent to the package-private `org.apache.lucene.store.PosixNativeAccess`.
///
/// # Divergence from Lucene 10.5.0
///
/// Java binds `posix_madvise` and `getpagesize` from libc with the Java 21
/// foreign function API. This crate denies `unsafe` code, so:
///
/// * `posix_madvise` is issued through `memmap2`'s safe `Mmap::advise_range`,
///   which calls `madvise` with the same advice constants — `MADV_NORMAL`,
///   `MADV_RANDOM`, `MADV_SEQUENTIAL` and `MADV_WILLNEED` have the same numeric
///   values as their `POSIX_MADV_*` counterparts on Linux and macOS, which is
///   exactly why Lucene's own constants carry the comment that glibc and macOS
///   agree on them. Because the advice is applied to the mapping rather than to
///   a bare address, `memmap2` re-aligns the range to a page boundary itself,
///   so the call cannot fail with `EINVAL` for misalignment.
/// * `getpagesize` has no safe wrapper. On Linux the true page size is read
///   from the kernel-supplied auxiliary vector in `/proc/self/auxv`
///   (`AT_PAGESZ`), which is plain file I/O; on other platforms
///   [`FALLBACK_PAGE_SIZE`] is assumed. The value is only used to align the
///   range handed to `madvise`, and `memmap2` re-aligns it against the real
///   page size regardless, so an inaccurate fallback cannot turn a working
///   advice call into a failing one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PosixNativeAccess;

impl PosixNativeAccess {
    /// No further special treatment.
    ///
    /// Equivalent to `PosixNativeAccess.POSIX_MADV_NORMAL`.
    pub const POSIX_MADV_NORMAL: i32 = 0;

    /// Expect random page references.
    ///
    /// Equivalent to `PosixNativeAccess.POSIX_MADV_RANDOM`.
    pub const POSIX_MADV_RANDOM: i32 = 1;

    /// Expect sequential page references.
    ///
    /// Equivalent to `PosixNativeAccess.POSIX_MADV_SEQUENTIAL`.
    pub const POSIX_MADV_SEQUENTIAL: i32 = 2;

    /// Will need these pages.
    ///
    /// Equivalent to `PosixNativeAccess.POSIX_MADV_WILLNEED`.
    pub const POSIX_MADV_WILLNEED: i32 = 3;

    /// Don't need these pages.
    ///
    /// Equivalent to `PosixNativeAccess.POSIX_MADV_DONTNEED`. Lucene never
    /// issues it; it is kept because Lucene declares it.
    pub const POSIX_MADV_DONTNEED: i32 = 4;

    /// Returns the singleton instance on the platforms Lucene supports.
    ///
    /// Equivalent to `PosixNativeAccess.getInstance()`.
    pub fn get_instance() -> Option<&'static dyn NativeAccess> {
        // Lucene only initialises the bindings on Linux and macOS, and only on
        // 64-bit targets ("we only support 64 bits at the moment").
        #[cfg(all(
            any(target_os = "linux", target_os = "macos"),
            target_pointer_width = "64"
        ))]
        {
            static INSTANCE: PosixNativeAccess = PosixNativeAccess;
            Some(&INSTANCE)
        }
        #[cfg(not(all(
            any(target_os = "linux", target_os = "macos"),
            target_pointer_width = "64"
        )))]
        {
            None
        }
    }

    /// Maps a Lucene [`ReadAdvice`] to the POSIX advice constant.
    ///
    /// Equivalent to `PosixNativeAccess.mapReadAdvice(ReadAdvice)`.
    pub fn map_read_advice(read_advice: ReadAdvice) -> i32 {
        match read_advice {
            ReadAdvice::Normal => Self::POSIX_MADV_NORMAL,
            ReadAdvice::Random => Self::POSIX_MADV_RANDOM,
            ReadAdvice::Sequential => Self::POSIX_MADV_SEQUENTIAL,
        }
    }

    /// Looks up the `posix_madvise` entry point.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] on a platform that has no
    /// such symbol, which is Java's `UnsupportedOperationException`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.store.PosixNativeAccess#lookupMadvise`.
    pub fn lookup_madvise() -> Result<PosixFunction> {
        Self::find_function("posix_madvise")
    }

    /// Looks up the `getpagesize` entry point.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] on a platform that has no
    /// such symbol, which is Java's `UnsupportedOperationException`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.store.PosixNativeAccess#lookupGetPageSize`.
    pub fn lookup_get_page_size() -> Result<PosixFunction> {
        Self::find_function("getpagesize")
    }

    /// Resolves one libc entry point by name.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] with Lucene's own message
    /// when the running platform provides no such symbol.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Java resolves the symbol dynamically:
    /// `SymbolLookup.find(name)` against `Linker.nativeLinker().defaultLookup()`,
    /// then `linker.downcallHandle(symbol, descriptor)`, so any libc function
    /// can be named. Rust has no safe dynamic symbol lookup — `libloading` and
    /// a raw `extern` block both need `unsafe`, which this crate forbids — so
    /// the resolvable names are the two Lucene actually binds, and each
    /// resolves to the safe facility this port calls it through:
    /// `posix_madvise` to `memmap2`'s `advise_range`, and `getpagesize` to the
    /// kernel-supplied auxiliary vector. The failure mode is the one Java
    /// reports for a platform that lacks the symbol, and the set of platforms
    /// that resolve them is the same set `get_instance` accepts.
    ///
    /// Equivalent to
    /// `org.apache.lucene.store.PosixNativeAccess#findFunction`.
    pub fn find_function(name: &str) -> Result<PosixFunction> {
        let symbol = if Self::get_instance().is_some() {
            match name {
                "posix_madvise" => Some(PosixFunction::PosixMadvise),
                "getpagesize" => Some(PosixFunction::GetPageSize),
                _ => None,
            }
        } else {
            None
        };
        symbol.ok_or_else(|| {
            LuceneError::UnsupportedOperation(format!(
                "Platform has no symbol for '{name}' in libc."
            ))
        })
    }

    /// Issues the advice, translating a failure the way Lucene does.
    ///
    /// Equivalent to the private
    /// `PosixNativeAccess.madvise(MemorySegment, int)`.
    fn advise(&self, segment: &MemorySegment, advice: i32) -> Result<()> {
        // Empty segments are excluded, because they may have no address at all.
        if segment.byte_size() == 0 {
            return Ok(());
        }
        Self::lookup_madvise()?
            .invoke_madvise(segment, advice)
            .map_err(|error| {
                LuceneError::Io(std::io::Error::other(format!(
                    "Call to madvise with address=0x{:08X} and byteSize={} failed: {error}",
                    segment.address(),
                    segment.byte_size()
                )))
            })
    }
}

/// A libc entry point [`PosixNativeAccess`] binds, resolved by
/// [`PosixNativeAccess::find_function`].
///
/// Equivalent to the `MethodHandle` Java's `findFunction` returns: a handle
/// that can be invoked, not the result of invoking it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PosixFunction {
    /// `int posix_madvise(void *addr, size_t length, int advice)`.
    PosixMadvise,
    /// `int getpagesize(void)`.
    GetPageSize,
}

impl PosixFunction {
    /// Returns the libc name this handle was resolved under.
    pub fn symbol_name(self) -> &'static str {
        match self {
            Self::PosixMadvise => "posix_madvise",
            Self::GetPageSize => "getpagesize",
        }
    }

    /// Invokes `posix_madvise` over the whole of `segment` with one of the
    /// `POSIX_MADV_*` constants.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when this handle is not
    /// `posix_madvise` or `advice` is not an advice Lucene issues, and
    /// [`LuceneError::Io`] when the call fails.
    pub fn invoke_madvise(self, segment: &MemorySegment, advice: i32) -> Result<()> {
        if self != Self::PosixMadvise {
            return Err(LuceneError::IllegalArgument(format!(
                "'{}' is not posix_madvise",
                self.symbol_name()
            )));
        }
        let os_advice = match advice {
            PosixNativeAccess::POSIX_MADV_NORMAL => {
                OsReadAdvice::from_read_advice(ReadAdvice::Normal)
            }
            PosixNativeAccess::POSIX_MADV_RANDOM => {
                OsReadAdvice::from_read_advice(ReadAdvice::Random)
            }
            PosixNativeAccess::POSIX_MADV_SEQUENTIAL => {
                OsReadAdvice::from_read_advice(ReadAdvice::Sequential)
            }
            PosixNativeAccess::POSIX_MADV_WILLNEED => OsReadAdvice::WillNeed,
            other => {
                return Err(LuceneError::IllegalArgument(format!(
                    "unsupported posix_madvise advice: {other}"
                )))
            }
        };
        segment.advise_os(os_advice)
    }

    /// Invokes `getpagesize`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when this handle is not
    /// `getpagesize`.
    pub fn invoke_get_page_size(self) -> Result<usize> {
        if self != Self::GetPageSize {
            return Err(LuceneError::IllegalArgument(format!(
                "'{}' is not getpagesize",
                self.symbol_name()
            )));
        }
        Ok(detect_page_size())
    }
}

impl NativeAccess for PosixNativeAccess {
    fn madvise(&self, segment: &MemorySegment, read_advice: ReadAdvice) -> Result<()> {
        self.advise(segment, Self::map_read_advice(read_advice))
    }

    fn madvise_will_need(&self, segment: &MemorySegment) -> Result<()> {
        self.advise(segment, Self::POSIX_MADV_WILLNEED)
    }

    fn get_page_size(&self) -> usize {
        // Java resolves `getpagesize` once, in the class initialiser, and
        // caches what it returned; so does this.
        static PAGE_SIZE: LazyLock<usize> = LazyLock::new(|| {
            PosixNativeAccess::lookup_get_page_size()
                .and_then(PosixFunction::invoke_get_page_size)
                .unwrap_or(FALLBACK_PAGE_SIZE)
        });
        *PAGE_SIZE
    }
}

/// Determines the operating system's page size without leaving safe Rust.
fn detect_page_size() -> usize {
    #[cfg(target_os = "linux")]
    {
        if let Some(page_size) = page_size_from_auxv() {
            return page_size;
        }
    }
    FALLBACK_PAGE_SIZE
}

/// Reads `AT_PAGESZ` from the kernel-supplied auxiliary vector.
///
/// `/proc/self/auxv` is a sequence of `(key, value)` pairs of native-width
/// words terminated by a `AT_NULL` key. This is the authoritative page size,
/// obtained without a `sysconf` call.
#[cfg(target_os = "linux")]
fn page_size_from_auxv() -> Option<usize> {
    /// Auxiliary vector key for the system page size, from `<elf.h>`.
    const AT_PAGESZ: usize = 6;
    /// Auxiliary vector terminator key, from `<elf.h>`.
    const AT_NULL: usize = 0;
    const WORD: usize = std::mem::size_of::<usize>();

    let auxv = std::fs::read("/proc/self/auxv").ok()?;
    let mut cursor = 0;
    while cursor + 2 * WORD <= auxv.len() {
        let key = read_native_word(&auxv[cursor..cursor + WORD]);
        let value = read_native_word(&auxv[cursor + WORD..cursor + 2 * WORD]);
        if key == AT_NULL {
            break;
        }
        if key == AT_PAGESZ && value.is_power_of_two() {
            return Some(value);
        }
        cursor += 2 * WORD;
    }
    None
}

/// Decodes one native-endian, native-width word.
#[cfg(target_os = "linux")]
fn read_native_word(bytes: &[u8]) -> usize {
    let mut word = [0u8; std::mem::size_of::<usize>()];
    word.copy_from_slice(bytes);
    usize::from_ne_bytes(word)
}
