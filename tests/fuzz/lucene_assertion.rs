//! Tells a Lucene `assert` apart from a defect, for the fuzz targets.
//!
//! Lucene runs its tests with `-ea`, so an `assert` its decoder makes about a
//! corrupt file throws `AssertionError`: an exception, which the JVM survives.
//! The port keeps every such `assert` as a `debug_assert!`, so a debug build
//! panics exactly where Java throws. That panic is Java's behaviour and counts
//! as survived.
//!
//! Any other panic — an index out of bounds, an arithmetic overflow, an
//! `unwrap` of `None` — has no Java counterpart and stays a defect of the
//! reader. The two are told apart by the source line the panic was raised at:
//! only a panic raised by a `debug_assert!`, `debug_assert_eq!` or
//! `debug_assert_ne!` invocation in this crate's sources is accepted.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe, PanicHookInfo};
use std::sync::{Mutex, Once};

thread_local! {
    /// Whether [`survive_lucene_assertions`] is running on this thread.
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// Whether the last panic of this thread was raised by a `debug_assert!`.
    static LAST_WAS_ASSERTION: Cell<bool> = const { Cell::new(false) };
    /// The `file:line` of the last panic of this thread, for the report.
    static LAST_LOCATION: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The lines of the source files asked about so far.
static SOURCES: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);

/// Whether line `line` (1-based) of `file`, a path relative to this package's
/// root as the compiler records it, invokes one of the `debug_assert` macros.
fn is_debug_assert_site(file: &str, line: u32) -> bool {
    let mut sources = SOURCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let sources = sources.get_or_insert_with(HashMap::new);
    let lines = sources.entry(file.to_string()).or_insert_with(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
        std::fs::read_to_string(path)
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default()
    });
    line.checked_sub(1)
        .and_then(|index| lines.get(index as usize))
        .is_some_and(|text| text.trim_start().starts_with("debug_assert"))
}

/// Whether `info` is a panic raised by a `debug_assert!` of this crate.
fn is_lucene_assertion(info: &PanicHookInfo<'_>) -> bool {
    info.location()
        .is_some_and(|location| is_debug_assert_site(location.file(), location.line()))
}

/// Installs, once per binary, a hook that classifies every panic raised while
/// [`survive_lucene_assertions`] runs, keeps the accepted ones quiet, and hands
/// every other panic to the hook installed before.
fn install_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if ACTIVE.with(Cell::get) {
                let assertion = is_lucene_assertion(info);
                LAST_WAS_ASSERTION.with(|last| last.set(assertion));
                if let Some(location) = info.location() {
                    LAST_LOCATION.with(|last| *last.borrow_mut() = format!("{location}"));
                }
                if assertion {
                    return;
                }
            }
            previous(info);
        }));
    });
}

/// Runs `body`: `Some` of its value when it returns, `None` when it panicked
/// at a Lucene `assert` (a `debug_assert!` of this crate, Java's
/// `AssertionError` under `-ea`). Any other panic is resumed, so it fails the
/// test.
pub fn survive_lucene_assertions<R>(body: impl FnOnce() -> R) -> Option<R> {
    install_hook();
    let was_active = ACTIVE.with(|active| active.replace(true));
    LAST_WAS_ASSERTION.with(|last| last.set(false));
    let outcome = panic::catch_unwind(AssertUnwindSafe(body));
    ACTIVE.with(|active| active.set(was_active));
    match outcome {
        Ok(value) => Some(value),
        Err(payload) => {
            if LAST_WAS_ASSERTION.with(|last| last.replace(false)) {
                None
            } else {
                let location = LAST_LOCATION.with(|last| last.borrow().clone());
                eprintln!("a panic that is not a Lucene assertion, at {location}");
                panic::resume_unwind(payload)
            }
        }
    }
}
