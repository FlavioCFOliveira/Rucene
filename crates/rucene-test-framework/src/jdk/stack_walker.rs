//! `java.lang.StackWalker.getInstance().walk(...)`: the frames of the calling
//! thread's stack, as class and method names.
//!
//! **Divergence.** The JVM walks its own frames, which carry the class and
//! method of each call. Rust has no such frames; this emulation captures a
//! [`std::backtrace::Backtrace`] and reads each frame's symbol,
//! `path::to::Type::method` (or `<path::to::Type as Trait>::method`), as the
//! class `path::to::Type` and the method `method`. The answer depends on the
//! debug information the build keeps:
//!
//! * with `debug = "limited"` or `"full"`, every frame, inlined ones included,
//!   carries its path: both the class and the method are known. `limited`
//!   (`debug = 1`) is the least the walker needs, and the level the workspace
//!   `[profile.test]` sets (rmp #608): it keeps the DWARF names and inline
//!   records the symboliser reads, and drops the variable and type
//!   information it never reads. Against `full`, it cuts the peak RSS of
//!   the tests that walk the stack by about 40 % (the memory std's backtrace
//!   symboliser takes to load the DWARF) and gives the same answers;
//! * with `debug = "line-tables-only"`, the frames of a crate built that way
//!   carry only their bare DWARF name (`flush`, `into<A, B>`): the method is
//!   known and the class is not, so it is reported empty. `callStackContainsAnyOf`
//!   answers as Java does; `callStackContains(Class, …)` never matches such a
//!   frame;
//! * without debug information, only the frames of functions that kept a
//!   symbol are reported;
//! * at every level, a call the optimizer turned into a tail call (a jump)
//!   leaves no frame for its caller.
//!
//! The frames of a call stack are rendered once per process and then served
//! from a cache keyed by the stack's return addresses (see [`frames`]).

use std::backtrace::Backtrace;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

/// One frame: the class (the Rust type path) and the method name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StackFrame {
    /// `getClassName()`: the Rust path of the type (or module) the function
    /// belongs to; empty when the build records only the function's bare
    /// name (see the module docs).
    pub class_name: String,
    /// `getMethodName()`.
    pub method_name: String,
}

/// Strips the `::h0123456789abcdef` hash suffix of a legacy-mangled symbol.
fn strip_hash(symbol: &str) -> &str {
    match symbol.rfind("::h") {
        Some(pos)
            if symbol.len() - pos == 19
                && symbol[pos + 3..].bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            &symbol[..pos]
        }
        _ => symbol,
    }
}

/// The `::`-separated segments of `symbol`, ignoring the `::` inside
/// `<...>` (generic arguments and `<Type as Trait>` qualifications).
fn segments(symbol: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let bytes = symbol.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            b':' if depth == 0 && bytes.get(i + 1) == Some(&b':') => {
                out.push(&symbol[start..i]);
                i += 2;
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&symbol[start..]);
    out
}

/// A function name without its generic arguments: the bare DWARF name
/// `into<A, B>` of a generic function is the method `into`.
fn without_generic_args(name: &str) -> &str {
    match name.find('<') {
        Some(pos) if pos > 0 => &name[..pos],
        _ => name,
    }
}

/// Splits a demangled symbol into its type path and method name. A symbol
/// without a path (a bare name, see the module docs) has an empty class.
fn split(symbol: &str) -> Option<StackFrame> {
    let symbol = strip_hash(symbol.trim());
    let mut segments = segments(symbol);
    // `path::function::<T>`: the turbofish of a generic function is not
    // the method.
    while segments.len() > 1 && segments.last().is_some_and(|s| s.starts_with('<')) {
        segments.pop();
    }
    let method = without_generic_args(segments.pop()?);
    if method.is_empty() {
        return None;
    }
    let owner = segments.join("::");
    // `<path::Type as path::Trait>::method`: the type is the part before
    // ` as `; `<path::Type<T>>::method`: the type is `path::Type<T>`.
    let class_name = match owner
        .strip_prefix('<')
        .and_then(|inner| inner.strip_suffix('>'))
    {
        Some(inner) => inner.split(" as ").next().unwrap_or(inner).to_string(),
        None => owner,
    };
    Some(StackFrame {
        class_name,
        method_name: method.to_string(),
    })
}

/// The frames of a rendered backtrace (`Backtrace`'s `Display`), innermost
/// first, starting with the caller of [`frames`].
fn frames_of(trace: &str) -> Vec<StackFrame> {
    let mut out = Vec::new();
    for line in trace.lines() {
        let line = line.trim_start();
        // `  N: symbol` lines; `at file:line` lines follow them.
        let Some((index, symbol)) = line.split_once(": ") else {
            continue;
        };
        if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Some(frame) = split(symbol) {
            out.push(frame);
        }
    }
    // Drop the capture machinery and `frames` itself (a bare name when the
    // build records no path).
    let me = out.iter().position(|f| {
        f.method_name == "frames"
            && (f.class_name.ends_with("stack_walker") || f.class_name.is_empty())
    });
    match me {
        Some(pos) => out.split_off(pos + 1),
        None => out,
    }
}

/// The frames of the current thread, innermost first, starting with the
/// caller of this function.
///
/// PERF: rendering a [`std::backtrace::Backtrace`] resolves every frame's
/// symbol, file and line and demangles it; done on every
/// `callStackContains` of a failure hook, it was 45 % of
/// `TestIndexFileDeleter` and 32 % of `TestIndexWriterExceptions` (rmp #606,
/// W5). The frames are therefore cached by the call stack's return
/// addresses: the same addresses in the same process are the same stack, so
/// a hit returns exactly the frames a fresh render would. Only a stack seen
/// for the first time is rendered. Each distinct frame is stored once and
/// shared by the stacks that contain it.
pub fn frames() -> Arc<[Arc<StackFrame>]> {
    let Some(key) = return_addresses::walk() else {
        return render().into_iter().map(Arc::new).collect();
    };
    if let Some(cached) = cache().stacks.get(&key) {
        return Arc::clone(cached);
    }
    let rendered = render();
    let mut cache = cache();
    let interned: Arc<[Arc<StackFrame>]> = rendered
        .into_iter()
        .map(|frame| cache.intern(frame))
        .collect();
    cache.stacks.insert(key, Arc::clone(&interned));
    interned
}

/// The frames of the calling thread's stack, read from a rendered
/// backtrace; called from [`frames`] only, whose frame and the frames
/// inside it are dropped.
fn render() -> Vec<StackFrame> {
    frames_of(&Backtrace::force_capture().to_string())
}

/// The frames of every call stack [`frames`] has seen, by its return
/// addresses, and the distinct frames those stacks share.
#[derive(Default)]
struct Cache {
    stacks: HashMap<Box<[usize]>, Arc<[Arc<StackFrame>]>>,
    frames: HashSet<Arc<StackFrame>>,
}

impl Cache {
    /// The shared copy of `frame`.
    fn intern(&mut self, frame: StackFrame) -> Arc<StackFrame> {
        if let Some(shared) = self.frames.get(&frame) {
            return Arc::clone(shared);
        }
        let shared = Arc::new(frame);
        self.frames.insert(Arc::clone(&shared));
        shared
    }
}

fn cache() -> MutexGuard<'static, Cache> {
    static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);
    CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The return addresses of the calling thread's stack, walked with the
/// `_Unwind_Backtrace` of the platform unwinder — the walk the standard
/// library's [`Backtrace`] makes on these targets — without resolving them.
#[cfg(all(unix, not(target_arch = "arm")))]
#[allow(unsafe_code)]
mod return_addresses {
    use std::ffi::{c_int, c_void};

    /// `struct _Unwind_Context`, opaque.
    #[repr(C)]
    struct UnwindContext {
        _opaque: [u8; 0],
    }

    /// `_URC_NO_REASON`: continue the walk.
    const URC_NO_REASON: c_int = 0;

    type TraceFn = extern "C" fn(*mut UnwindContext, *mut c_void) -> c_int;

    // The unwinder `std` links on these targets (`libgcc_s`, or the
    // system/LLVM `libunwind`); its ABI is the Itanium C++ ABI's level 1
    // unwind interface (`_Unwind_GetIP` is a macro only on ARM EHABI, which
    // the `cfg` above excludes).
    extern "C" {
        fn _Unwind_Backtrace(trace: TraceFn, arg: *mut c_void) -> c_int;
        fn _Unwind_GetIP(ctx: *mut UnwindContext) -> usize;
    }

    extern "C" fn push(ctx: *mut UnwindContext, arg: *mut c_void) -> c_int {
        // SAFETY: `arg` is the `&mut Vec<usize>` that `walk` passed to
        // `_Unwind_Backtrace`, which calls this function synchronously
        // during `walk`, on the same thread, while that borrow is live and
        // not otherwise used; `ctx` is the unwinder's live context for the
        // current frame, valid for the duration of this callback.
        let (addresses, ip) = unsafe { (&mut *arg.cast::<Vec<usize>>(), _Unwind_GetIP(ctx)) };
        addresses.push(ip);
        URC_NO_REASON
    }

    /// The return addresses, innermost first; `None` when the walk failed.
    pub(super) fn walk() -> Option<Box<[usize]>> {
        let mut addresses: Vec<usize> = Vec::with_capacity(64);
        // SAFETY: `push` matches `_Unwind_Trace_Fn` and only touches the
        // `Vec` passed as `arg`, which outlives the call; the walk does not
        // retain `arg` or `push` after it returns.
        unsafe {
            _Unwind_Backtrace(push, (&raw mut addresses).cast::<c_void>());
        }
        (!addresses.is_empty()).then(|| addresses.into_boxed_slice())
    }
}

/// No unwinder interface to walk with: every call renders.
#[cfg(not(all(unix, not(target_arch = "arm"))))]
mod return_addresses {
    pub(super) fn walk() -> Option<Box<[usize]>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(class_name: &str, method_name: &str) -> Option<StackFrame> {
        Some(StackFrame {
            class_name: class_name.to_string(),
            method_name: method_name.to_string(),
        })
    }

    #[test]
    fn splits_symbols() {
        assert_eq!(
            split("rucene::index::index_writer::IndexWriter::flush::h0123456789abcdef"),
            frame("rucene::index::index_writer::IndexWriter", "flush")
        );
        assert_eq!(
            split("<rucene::store::FSDirectory as rucene::store::Directory>::sync"),
            frame("rucene::store::FSDirectory", "sync")
        );
    }

    /// The bare names of a `line-tables-only` build keep their method
    /// (without generic arguments) and an empty class (rmp #576: they were
    /// dropped, so `callStackContainsAnyOf` never saw them).
    #[test]
    fn keeps_the_method_of_a_bare_name() {
        assert_eq!(split("create_output"), frame("", "create_output"));
        assert_eq!(
            split(
                "into<rucene::search::index_searcher::TooManyClauses, rucene::error::LuceneError>"
            ),
            frame("", "into")
        );
        assert_eq!(
            split("call_once<core_internal_hppc_tests::test_max_sized_int_array_list::{closure_env#0}, ()>"),
            frame("", "call_once")
        );
    }

    /// A generic type's path keeps its arguments, as `type_name` writes it,
    /// and a generic function's turbofish is not its method.
    #[test]
    fn splits_generic_owners_and_functions() {
        assert_eq!(
            split("<bt::store::Dir<u32>>::flush::<u16>"),
            frame("bt::store::Dir<u32>", "flush")
        );
        assert_eq!(
            split("<a::Dir<u32> as a::Directory<u8>>::sync"),
            frame("a::Dir<u32>", "sync")
        );
    }

    #[inline(never)]
    fn probe_inner() -> Arc<[Arc<StackFrame>]> {
        let frames = frames();
        std::hint::black_box(&frames);
        frames
    }

    #[inline(never)]
    fn probe_outer() -> Arc<[Arc<StackFrame>]> {
        let frames = probe_inner();
        std::hint::black_box(&frames);
        frames
    }

    /// The cache (rmp #606, W5) serves a stack seen before without
    /// rendering it again, and what it serves is what the render found: the
    /// caller's frames, by class and method, innermost first.
    #[test]
    fn caches_the_frames_of_a_stack_by_its_return_addresses() {
        // One call site walked three times. The counter passes through
        // `black_box`, so the trip count is opaque and no opt level can
        // unroll the loop into several call sites, each its own stack.
        let mut seen = Vec::new();
        let mut walks = 0;
        while std::hint::black_box(walks) < 3 {
            seen.push(probe_outer());
            walks += 1;
        }
        // The class is empty in a `line-tables-only` build (module docs).
        let class = if seen[0][0].class_name.is_empty() {
            ""
        } else {
            module_path!()
        };
        let methods: Vec<(&str, &str)> = seen[0]
            .iter()
            .take(2)
            .map(|f| (f.class_name.as_str(), f.method_name.as_str()))
            .collect();
        assert_eq!(
            methods,
            [(class, "probe_inner"), (class, "probe_outer")],
            "{:?}",
            seen[0]
        );
        // Same call site, same stack: the second and third walks hit.
        if cfg!(all(unix, not(target_arch = "arm"))) {
            assert!(Arc::ptr_eq(&seen[0], &seen[1]));
            assert!(Arc::ptr_eq(&seen[0], &seen[2]));
        }
        // A different stack (another call site) is rendered on its own, and
        // finds the same frames above it.
        let other = probe_outer();
        assert!(!Arc::ptr_eq(&seen[0], &other));
        assert_eq!(other[..2], seen[0][..2]);
    }

    /// The frames of a `line-tables-only` capture: the machinery and the
    /// bare `frames` are dropped, the bare frames above are kept.
    #[test]
    fn walks_a_capture_of_bare_names() {
        let trace = "   0: std::backtrace_rs::backtrace::libunwind::trace\n\
                     \x20            at /rustc/x/library/std/src/../../backtrace/src/backtrace/libunwind.rs:117:9\n\
                     \x20  1: std::backtrace::Backtrace::create\n\
                     \x20  2: frames\n\
                     \x20            at ./crates/rucene-test-framework/src/jdk/stack_walker.rs:150:5\n\
                     \x20  3: call_stack_contains_any_of\n\
                     \x20  4: eval\n\
                     \x20  5: maybe_throw_deterministic_exception\n\
                     \x20  6: create_output\n";
        let methods: Vec<String> = frames_of(trace)
            .into_iter()
            .map(|f| f.method_name)
            .collect();
        assert_eq!(
            methods,
            [
                "call_stack_contains_any_of",
                "eval",
                "maybe_throw_deterministic_exception",
                "create_output"
            ]
        );
    }
}
