//! `System.getProperty`, `System.setProperty` and `System.clearProperty`.
//!
//! **Divergence.** A JVM's system properties come from `-D` options
//! and the platform; a Rust process has neither. The port keeps a
//! process-wide property table that [`set_property`] and [`clear_property`]
//! write (the test harness writes the `-D` options it is given), and a
//! property that the table does not hold is read from the process
//! environment variable of the same name, so `env tests.seed=DEADBEEF cargo
//! test` sets `tests.seed`. The one platform property randomizedtesting reads,
//! `java.io.tmpdir`, defaults to `std::env::temp_dir()`.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// `Some(value)` for a set property, `None` for a cleared one.
static PROPERTIES: LazyLock<RwLock<HashMap<String, Option<String>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// `System.getProperty(key)`.
pub fn get_property(key: &str) -> Option<String> {
    if let Some(entry) = PROPERTIES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
    {
        return entry.clone();
    }
    if let Some(v) = std::env::var_os(key) {
        return Some(v.to_string_lossy().into_owned());
    }
    if key == "java.io.tmpdir" {
        return Some(std::env::temp_dir().to_string_lossy().into_owned());
    }
    None
}

/// `System.setProperty(key, value)`; returns the previous value.
pub fn set_property(key: &str, value: &str) -> Option<String> {
    let previous = get_property(key);
    PROPERTIES
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), Some(value.to_string()));
    previous
}

/// `System.clearProperty(key)`; returns the previous value.
pub fn clear_property(key: &str) -> Option<String> {
    let previous = get_property(key);
    PROPERTIES
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), None);
    previous
}

/// `System.out` / `System.err`: a replaceable sink (`System.setOut`,
/// `System.setErr`); `None` writes to the process's own stream.
pub type PrintSink = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

static OUT: RwLock<Option<PrintSink>> = RwLock::new(None);
static ERR: RwLock<Option<PrintSink>> = RwLock::new(None);

/// `System.setOut(stream)`; `None` restores the process's standard output.
/// Returns the previous sink.
pub fn set_out(sink: Option<PrintSink>) -> Option<PrintSink> {
    std::mem::replace(&mut *OUT.write().unwrap_or_else(|e| e.into_inner()), sink)
}

/// `System.setErr(stream)`; `None` restores the process's standard error.
/// Returns the previous sink.
pub fn set_err(sink: Option<PrintSink>) -> Option<PrintSink> {
    std::mem::replace(&mut *ERR.write().unwrap_or_else(|e| e.into_inner()), sink)
}

/// `System.out.print(text)`.
pub fn out_print(text: &str) {
    let sink = OUT.read().unwrap_or_else(|e| e.into_inner()).clone();
    match sink {
        Some(s) => s(text),
        None => {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }
    }
}

/// `System.err.print(text)`.
pub fn err_print(text: &str) {
    let sink = ERR.read().unwrap_or_else(|e| e.into_inner()).clone();
    match sink {
        Some(s) => s(text),
        None => {
            use std::io::Write;
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(text.as_bytes());
            let _ = err.flush();
        }
    }
}

/// `System.out.println(text)`.
pub fn out_println(text: &str) {
    out_print(&format!("{text}\n"));
}

/// `System.err.println(text)`.
pub fn err_println(text: &str) {
    err_print(&format!("{text}\n"));
}

/// `System.nanoTime()`: nanoseconds from an arbitrary, fixed origin.
pub fn nano_time() -> i64 {
    static ORIGIN: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
    ORIGIN.elapsed().as_nanos() as i64
}

/// `System.currentTimeMillis()`.
pub fn current_time_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `Throwable.printStackTrace()` text, without the trailing newline: the
/// throwable, its stack (`\tat frame`), its suppressed throwables
/// (`\tSuppressed: `) and its causes (`Caused by: `).
pub fn format_stack_trace(t: &super::exception::Throwable) -> String {
    let mut b = String::new();
    append_trace(&mut b, t, "", "");
    if b.ends_with('\n') {
        b.pop();
    }
    b
}

fn append_trace(b: &mut String, t: &super::exception::Throwable, caption: &str, prefix: &str) {
    b.push_str(&format!("{prefix}{caption}{t}\n"));
    for e in t.stack_trace() {
        b.push_str(&format!("{prefix}\tat {e}\n"));
    }
    for s in t.suppressed() {
        append_trace(b, s, "Suppressed: ", &format!("{prefix}\t"));
    }
    if let Some(c) = t.cause() {
        append_trace(b, c, "Caused by: ", prefix);
    }
}

/// A snapshot of every property set or cleared through this module
/// (`System.getProperties()`, as far as the port's table holds them): the
/// keys with a value, sorted.
pub fn properties_snapshot() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = PROPERTIES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter_map(|(k, v)| v.clone().map(|v| (k.clone(), v)))
        .collect();
    v.sort();
    v
}
