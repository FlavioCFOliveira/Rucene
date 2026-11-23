//! The JDK behaviours the benchmark module relies on, reproduced where
//! their observable output (iteration order, tokenisation, messages) is part
//! of the module's behaviour.
//!
//! | Rust | JDK 25 |
//! | --- | --- |
//! | [`properties::Properties`] | `java.util.Properties` (its `ConcurrentHashMap` iteration order) |
//! | [`sax::SaxParser`] | the `SAXParser` of `SAXParserFactory.newDefaultInstance()` (streaming, not namespace aware) |
//! | [`stream_tokenizer::StreamTokenizer`] | `java.io.StreamTokenizer` |
//! | the functions below | `System`, `Thread`, `String`, `Integer`, `Double` and `Boolean` members |

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rucene::error::{LuceneError, Result};

pub mod properties;
pub mod sax;
pub mod stream_tokenizer;

/// `System.getProperty("line.separator")`.
pub const NEW_LINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// `System.currentTimeMillis()`.
pub fn current_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The `threadInitNumber` of `java.lang.Thread`: the `N` of the default
/// name `"Thread-N"` of a thread created without a name.
static THREAD_INIT_NUMBER: AtomicU64 = AtomicU64::new(0);

/// `"Thread-" + nextThreadNum()`: the name Java gives a thread created
/// without one.
pub fn next_thread_name() -> String {
    format!("Thread-{}", THREAD_INIT_NUMBER.fetch_add(1, Ordering::Relaxed))
}

/// `Thread.currentThread().getName()`.
pub fn current_thread_name() -> String {
    std::thread::current().name().unwrap_or("").to_string()
}

/// `String.length()`: the number of UTF-16 code units.
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `String.trim()`: removes the leading and trailing code units `<= ' '`.
pub fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `Integer.parseInt(s)`.
///
/// # Errors
///
/// `NumberFormatException` where Java throws it.
pub fn parse_int(s: &str) -> Result<i32> {
    rucene::util::extra::java_parse_int(s).ok_or_else(|| LuceneError::number_format_for_input_string(s, 10))
}

/// `Double.parseDouble(s)` (which first trims `s`).
///
/// # Errors
///
/// `NumberFormatException` where Java throws it.
pub fn parse_double(s: &str) -> Result<f64> {
    rucene::util::java_fmt::parse_double(java_trim(s)).ok_or_else(|| {
        let message = if java_trim(s).is_empty() {
            "empty String".to_string()
        } else {
            format!("For input string: \"{s}\"")
        };
        LuceneError::number_format(message)
    })
}

/// `Float.parseFloat(s)` (which first trims `s`).
///
/// # Errors
///
/// `NumberFormatException` where Java throws it.
pub fn parse_float(s: &str) -> Result<f32> {
    rucene::util::java_fmt::parse_float(java_trim(s)).ok_or_else(|| {
        let message = if java_trim(s).is_empty() {
            "empty String".to_string()
        } else {
            format!("For input string: \"{s}\"")
        };
        LuceneError::number_format(message)
    })
}

/// `Boolean.valueOf(s).booleanValue()` (`Boolean.parseBoolean`): `true`
/// only for `"true"`, ignoring case.
pub fn parse_boolean(s: Option<&str>) -> bool {
    s.is_some_and(|s| s.eq_ignore_ascii_case("true"))
}

/// `Double.toString(d)` (also `"" + d`).
pub fn double_to_string(d: f64) -> String {
    rucene::util::java_fmt::double_to_string(d)
}

/// `Float.toString(f)` (also `"" + f`).
pub fn float_to_string(f: f32) -> String {
    rucene::util::java_fmt::float_to_string(f)
}

/// `new Exception(message)`: a checked `java.lang.Exception`.
pub fn exception(message: impl Into<String>) -> LuceneError {
    LuceneError::unchecked("java.lang.Exception", message)
}

/// `new RuntimeException(message)`.
pub fn runtime_exception(message: impl Into<String>) -> LuceneError {
    LuceneError::unchecked("java.lang.RuntimeException", message)
}

/// `new RuntimeException(cause)`: its message is the cause's `toString()`.
pub fn runtime_exception_from(cause: LuceneError) -> LuceneError {
    use std::sync::Arc;
    LuceneError::from(rucene::error::LuceneErrorKind::Unchecked {
        class: "java.lang.RuntimeException",
        message: Some(cause.to_string()),
        cause: Some(Arc::new(cause)),
    })
}

/// `Class.getSimpleName()` of the class whose binary name is `binary_name`:
/// the part after the last `.` and `$`, empty for an anonymous class.
pub fn simple_name(binary_name: &str) -> &str {
    let after_dot = binary_name.rsplit('.').next().unwrap_or(binary_name);
    let simple = after_dot.rsplit('$').next().unwrap_or(after_dot);
    if !simple.is_empty() && simple.chars().all(|c| c.is_ascii_digit()) {
        ""
    } else {
        simple
    }
}

/// `Object.toString()` of an object of class `binary_name`:
/// `getClass().getName() + "@" + Integer.toHexString(hashCode())`, with the
/// port's identity stand-in for the identity hash.
pub fn object_to_string<T: ?Sized>(binary_name: &str, identity: &T) -> String {
    rucene::index::index_reader::java_object_to_string(binary_name, identity)
}

/// A `java.lang.ThreadLocal`: one value per thread, per instance.
#[derive(Debug)]
pub struct ThreadLocal<T> {
    values: std::sync::Mutex<std::collections::HashMap<std::thread::ThreadId, std::sync::Arc<std::sync::Mutex<T>>>>,
}

impl<T> Default for ThreadLocal<T> {
    fn default() -> Self {
        ThreadLocal { values: std::sync::Mutex::new(std::collections::HashMap::new()) }
    }
}

impl<T> ThreadLocal<T> {
    /// `new ThreadLocal<>()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `get()`: the current thread's value, `None` for `null`.
    pub fn get(&self) -> Option<std::sync::Arc<std::sync::Mutex<T>>> {
        let values = self.values.lock().unwrap_or_else(|e| e.into_inner());
        values.get(&std::thread::current().id()).cloned()
    }

    /// `set(value)`; `None` is `set(null)`.
    pub fn set(&self, value: Option<T>) -> Option<std::sync::Arc<std::sync::Mutex<T>>> {
        let mut values = self.values.lock().unwrap_or_else(|e| e.into_inner());
        let id = std::thread::current().id();
        match value {
            Some(value) => {
                let cell = std::sync::Arc::new(std::sync::Mutex::new(value));
                values.insert(id, std::sync::Arc::clone(&cell));
                Some(cell)
            }
            None => {
                values.remove(&id);
                None
            }
        }
    }

    /// `get()`, setting `init()` first when the thread has no value.
    pub fn get_or_init(&self, init: impl FnOnce() -> T) -> std::sync::Arc<std::sync::Mutex<T>> {
        match self.get() {
            Some(v) => v,
            None => self.set(Some(init())).expect("just set"),
        }
    }
}

/// Locks `mutex`, ignoring poisoning (a Java monitor has none).
pub fn lock<T: ?Sized>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// `java.lang.Runtime`'s memory figures.
///
/// **Divergence.** Java reports the JVM heap (`Runtime.totalMemory()`, the
/// committed heap, and `freeMemory()`, its unused part), which the
/// benchmark's `TaskStats` records (`TaskStats.java:73-74`, `:79-87`). A
/// Rust process has no managed heap: `total_memory` is the process's
/// resident set size (`VmRSS` of `/proc/self/status`, 0 where it cannot be
/// read) and `free_memory` is 0, so "used" equals "total".
pub struct Runtime;

impl Runtime {
    /// `Runtime.getRuntime().totalMemory()`.
    pub fn total_memory() -> i64 {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status.lines().find_map(|line| {
                    let rest = line.strip_prefix("VmRSS:")?;
                    let kb: i64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
                    Some(kb * 1024)
                })
            })
            .unwrap_or(0)
    }

    /// `Runtime.getRuntime().freeMemory()`.
    pub fn free_memory() -> i64 {
        0
    }
}

/// `String.split(sep)` for a literal separator (no regex metacharacters):
/// the pieces between separators, trailing empty strings removed (an empty
/// input yields `[""]`).
pub fn split(s: &str, sep: &str) -> Vec<String> {
    if s.is_empty() {
        return vec![String::new()];
    }
    let mut parts: Vec<String> = s.split(sep).map(str::to_string).collect();
    while parts.len() > 1 && parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    if parts.len() == 1 && parts[0].is_empty() {
        // "," splits to an empty array in Java.
        return Vec::new();
    }
    parts
}

/// `Throwable.getMessage()` of `error`: the message of an unchecked Java
/// exception or a throwable class, else the error's display text.
pub fn get_message(error: &LuceneError) -> Option<String> {
    use rucene::error::LuceneErrorKind;
    match error.kind() {
        LuceneErrorKind::Unchecked { message, .. } => message.clone(),
        LuceneErrorKind::Throwable(t) => t.get_message().map(str::to_string),
        LuceneErrorKind::IllegalArgument { message, .. } | LuceneErrorKind::IllegalState { message, .. } => {
            Some(message.clone())
        }
        _ => Some(error.to_string()),
    }
}

/// `new RuntimeException(message, cause)`.
pub fn runtime_exception_with_cause(message: impl Into<String>, cause: LuceneError) -> LuceneError {
    LuceneError::from(rucene::error::LuceneErrorKind::Unchecked {
        class: "java.lang.RuntimeException",
        message: Some(message.into()),
        cause: Some(std::sync::Arc::new(cause)),
    })
}

/// `new IOException(message)`.
pub fn io_exception(message: impl Into<String>) -> LuceneError {
    LuceneError::from(std::io::Error::other(message.into()))
}

/// The reader `Files.newBufferedReader(path, UTF_8)` returns: UTF-8 with
/// `CodingErrorAction.REPORT`.
pub type FileReader = rucene_analysis_common::jdk::io::BufferedReader<
    rucene_analysis_common::jdk::io::InputStreamReader<std::fs::File>,
>;

/// `Files.newBufferedReader(path, StandardCharsets.UTF_8)`.
///
/// # Errors
///
/// The `IOException` of opening the file.
pub fn new_buffered_reader_utf8(path: &std::path::Path) -> Result<FileReader> {
    use rucene_analysis_common::jdk::io::{BufferedReader, CodingErrorAction, InputStreamReader};
    let file = std::fs::File::open(path)?;
    Ok(BufferedReader::new(InputStreamReader::with_action(file, CodingErrorAction::Report)))
}

/// `reader.readLine()` as a `String`.
///
/// # Errors
///
/// The `IOException` (or `MalformedInputException`) of reading.
pub fn read_line<S: rucene_analysis_common::jdk::io::ReadChars>(
    reader: &mut rucene_analysis_common::jdk::io::BufferedReader<S>,
) -> Result<Option<String>> {
    Ok(reader.read_line()?.map(|l| String::from_utf16_lossy(&l)))
}

/// `Path.toRealPath()` as a string.
///
/// # Errors
///
/// The `IOException` of resolving the path.
pub fn real_path_string(path: &std::path::Path) -> Result<String> {
    Ok(std::fs::canonicalize(path)?.to_string_lossy().into_owned())
}

/// A `java.io.PrintWriter` over a byte sink, writing UTF-8: like Java's, it
/// never throws (a write failure is swallowed), and with `auto_flush` it
/// flushes on every `println`.
pub struct PrintWriter {
    out: std::sync::Mutex<Box<dyn std::io::Write + Send>>,
    auto_flush: bool,
}

impl std::fmt::Debug for PrintWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrintWriter")
    }
}

impl PrintWriter {
    /// `new PrintWriter(out)`.
    pub fn new(out: Box<dyn std::io::Write + Send>) -> Self {
        PrintWriter { out: std::sync::Mutex::new(out), auto_flush: false }
    }

    /// `new PrintWriter(out, true)`.
    pub fn with_auto_flush(out: Box<dyn std::io::Write + Send>) -> Self {
        PrintWriter { out: std::sync::Mutex::new(out), auto_flush: true }
    }

    /// `println(String)`.
    pub fn println(&self, s: &str) {
        let mut out = lock(&self.out);
        let _ = out.write_all(s.as_bytes());
        let _ = out.write_all(NEW_LINE.as_bytes());
        if self.auto_flush {
            let _ = out.flush();
        }
    }

    /// `println()`.
    pub fn println_empty(&self) {
        self.println("");
    }

    /// `flush()`.
    pub fn flush(&self) {
        let _ = lock(&self.out).flush();
    }
}

/// The tokens of `new StringTokenizer(s)`: runs between the default
/// delimiters `" \t\n\r\f"`.
pub fn string_tokens(s: &str) -> impl Iterator<Item = &str> {
    s.split([' ', '\t', '\n', '\r', '\u{c}']).filter(|t| !t.is_empty())
}

/// `StringTokenizer.nextToken()` past the last token:
/// `NoSuchElementException`.
pub fn no_such_element() -> LuceneError {
    LuceneError::unchecked_with_message("java.util.NoSuchElementException", None)
}
