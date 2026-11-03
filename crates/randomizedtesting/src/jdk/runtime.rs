//! `Runtime.addShutdownHook`.
//!
//! **Divergence.** The JVM runs its shutdown hooks when it exits.
//! A Rust process has no exit hooks without `unsafe` FFI (`atexit`), so the
//! hooks registered here run when [`run_shutdown_hooks`] is called: the test
//! harness calls it before the process exits, which is where the JVM runs
//! them. Java runs hooks concurrently in unspecified order; they run here in
//! registration order on the calling thread.

use std::sync::Mutex;

type Hook = Box<dyn FnOnce() + Send>;

static HOOKS: Mutex<Vec<Hook>> = Mutex::new(Vec::new());

/// Registers `hook` to run at shutdown: `Runtime.getRuntime().addShutdownHook`.
pub fn add_shutdown_hook(hook: impl FnOnce() + Send + 'static) {
    HOOKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Box::new(hook));
}

/// Runs, and forgets, every registered shutdown hook.
pub fn run_shutdown_hooks() {
    let hooks = std::mem::take(&mut *HOOKS.lock().unwrap_or_else(|e| e.into_inner()));
    for hook in hooks {
        hook();
    }
}
