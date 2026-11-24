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
//! | [`run_main`] | the `java` launcher running a class's `main(String[])` |

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rucene::error::{LuceneError, Result};

pub mod properties;
pub mod sax;
pub mod stream_tokenizer;

/// The `java` launcher running `main(String[] args)` of a class: the
/// command-line arguments after the program name are passed to `main`; an
/// exception thrown out of `main` is reported by the default uncaught
/// exception handler of thread `main` ([`uncaught_in_main_text`] on
/// `System.err`, [`system::err`]) and the JVM exits with status 1. A
/// `System.exit(n)` inside `main` ends the process with its own status, as in
/// Java. An argument that is not valid UTF-8 is decoded with replacement
/// characters, as the launcher decodes it with the UTF-8 platform encoding.
pub fn run_main(main: impl FnOnce(&[String]) -> Result<()>) -> std::process::ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    match main(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            use std::io::Write;
            let _ = system::out().flush();
            let mut err = system::err();
            let _ = err.write_all(uncaught_in_main_text(&e).as_bytes());
            let _ = err.flush();
            std::process::ExitCode::from(1)
        }
    }
}

/// What `ThreadGroup.uncaughtException` prints for an exception thrown out
/// of `main`: `Exception in thread "main" ` followed by the
/// `printStackTrace()` text of `error` — its `toString()`, one `\tat` line
/// per stack frame, its suppressed exceptions (`\tSuppressed: `) and its
/// causes (`Caused by: `).
///
/// The frames are those of [`LuceneError::get_stack_trace`] (Rust paths in
/// place of Java classes), captured only when `RUST_BACKTRACE` or
/// `RUST_LIB_BACKTRACE` enables backtraces; without them no `\tat` line is
/// printed, as a Rust error carries no JVM frames.
pub fn uncaught_in_main_text(error: &LuceneError) -> String {
    let mut out = String::from("Exception in thread \"main\" ");
    write_stack_trace(&mut out, error, "");
    out
}

/// `Throwable.printStackTrace` of `error` into `out`, each line prefixed
/// with `indent` (the enclosing trace's indentation for a suppressed
/// exception).
fn write_stack_trace(out: &mut String, error: &LuceneError, indent: &str) {
    out.push_str(&rucene::util::concurrent::throwable_to_string(error));
    out.push('\n');
    write_frames(out, error, indent);
    for suppressed in error.get_suppressed() {
        out.push_str(indent);
        out.push_str("\tSuppressed: ");
        write_stack_trace(out, suppressed, &format!("{indent}\t"));
    }
    let mut cause = std::error::Error::source(error);
    while let Some(current) = cause {
        out.push_str(indent);
        out.push_str("Caused by: ");
        match current.downcast_ref::<LuceneError>() {
            Some(lucene) => {
                out.push_str(&rucene::util::concurrent::throwable_to_string(lucene));
                out.push('\n');
                write_frames(out, lucene, indent);
            }
            None => {
                out.push_str(&current.to_string());
                out.push('\n');
            }
        }
        cause = current.source();
    }
}

/// The `\tat` lines of the frames of `error`.
fn write_frames(out: &mut String, error: &LuceneError, indent: &str) {
    for frame in error.get_stack_trace() {
        let location = match (&frame.file, frame.line) {
            (Some(file), line) if line >= 0 => format!("{file}:{line}"),
            (Some(file), _) => file.clone(),
            (None, _) => "Unknown Source".to_string(),
        };
        out.push_str(&format!(
            "{indent}\tat {}::{}({location})\n",
            frame.path, frame.function
        ));
    }
}

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
    format!(
        "Thread-{}",
        THREAD_INIT_NUMBER.fetch_add(1, Ordering::Relaxed)
    )
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

/// `String.trim()` of a Java String held as UTF-16 code units: removes the
/// leading and trailing code units `<= ' '`.
pub fn java_trim_utf16(s: &[u16]) -> &[u16] {
    let start = s.iter().position(|&c| c > 0x20).unwrap_or(s.len());
    let end = s.iter().rposition(|&c| c > 0x20).map_or(start, |i| i + 1);
    &s[start..end]
}

/// The UTF-8 text Lucene indexes for a Java String held as UTF-16 code
/// units: `UnicodeUtil.UTF16toUTF8`, which writes U+FFFD for every unpaired
/// surrogate (core [`UnicodeUtil::utf16_to_utf8`](rucene::util::unicode_util::UnicodeUtil::utf16_to_utf8)).
pub fn utf16_to_string(s: &[u16]) -> String {
    use rucene::util::unicode_util::{UnicodeUtil, MAX_UTF8_BYTES_PER_CHAR};
    let mut out = vec![0u8; s.len() * MAX_UTF8_BYTES_PER_CHAR];
    let n = UnicodeUtil::utf16_to_utf8(s, 0, s.len(), &mut out);
    out.truncate(n);
    // `utf16_to_utf8` only writes well-formed UTF-8 (unpaired surrogates
    // become U+FFFD), so the conversion cannot fail.
    String::from_utf8(out).expect("UnicodeUtil.UTF16toUTF8 writes well-formed UTF-8")
}

/// `String.getBytes(StandardCharsets.UTF_8)` of a Java String held as UTF-16
/// code units: a valid surrogate pair becomes its 4-byte sequence, an
/// unpaired surrogate becomes `'?'` (JDK 25 `String.encodeUTF8_UTF16`).
pub fn string_get_bytes_utf8(s: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for c in char::decode_utf16(s.iter().copied()) {
        match c {
            Ok(c) => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            Err(_) => out.push(b'?'),
        }
    }
    out
}

/// `Integer.parseInt(s)`.
///
/// # Errors
///
/// `NumberFormatException` where Java throws it.
pub fn parse_int(s: &str) -> Result<i32> {
    rucene::util::extra::java_parse_int(s)
        .ok_or_else(|| LuceneError::number_format_for_input_string(s, 10))
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
    values: std::sync::Mutex<
        std::collections::HashMap<std::thread::ThreadId, std::sync::Arc<std::sync::Mutex<T>>>,
    >,
}

impl<T> Default for ThreadLocal<T> {
    fn default() -> Self {
        ThreadLocal {
            values: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
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
        LuceneErrorKind::IllegalArgument { message, .. }
        | LuceneErrorKind::IllegalState { message, .. } => Some(message.clone()),
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
    Ok(BufferedReader::new(InputStreamReader::with_action(
        file,
        CodingErrorAction::Report,
    )))
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
        PrintWriter {
            out: std::sync::Mutex::new(out),
            auto_flush: false,
        }
    }

    /// `new PrintWriter(out, true)`.
    pub fn with_auto_flush(out: Box<dyn std::io::Write + Send>) -> Self {
        PrintWriter {
            out: std::sync::Mutex::new(out),
            auto_flush: true,
        }
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
    s.split([' ', '\t', '\n', '\r', '\u{c}'])
        .filter(|t| !t.is_empty())
}

/// `StringTokenizer.nextToken()` past the last token:
/// `NoSuchElementException`.
pub fn no_such_element() -> LuceneError {
    LuceneError::unchecked_with_message("java.util.NoSuchElementException", None)
}

/// `System.out` and `System.err` as the benchmark's tasks write to them
/// (`writer.info.stream=SystemOut|SystemErr` of `CreateIndexTask`): a
/// replaceable sink, as `System.setOut`/`System.setErr` replace the JVM's
/// streams; `None` writes to the process's own stream.
pub mod system {
    use std::io::{self, Write};
    use std::sync::{Arc, RwLock};

    /// A replacement of `System.out`/`System.err`: receives every write.
    pub type PrintSink = Arc<dyn Fn(&[u8]) + Send + Sync>;

    static OUT: RwLock<Option<PrintSink>> = RwLock::new(None);
    static ERR: RwLock<Option<PrintSink>> = RwLock::new(None);

    /// `System.setOut(stream)`; returns the previous sink.
    pub fn set_out(sink: Option<PrintSink>) -> Option<PrintSink> {
        std::mem::replace(&mut *OUT.write().unwrap_or_else(|e| e.into_inner()), sink)
    }

    /// `System.setErr(stream)`; returns the previous sink.
    pub fn set_err(sink: Option<PrintSink>) -> Option<PrintSink> {
        std::mem::replace(&mut *ERR.write().unwrap_or_else(|e| e.into_inner()), sink)
    }

    /// The stream `System.out` is when this is called (Java captures the
    /// `PrintStream` object, not the field).
    pub fn out() -> Box<dyn Write + Send> {
        match OUT.read().unwrap_or_else(|e| e.into_inner()).clone() {
            Some(sink) => Box::new(SinkWriter(sink)),
            None => Box::new(io::stdout()),
        }
    }

    /// The stream `System.err` is when this is called.
    pub fn err() -> Box<dyn Write + Send> {
        match ERR.read().unwrap_or_else(|e| e.into_inner()).clone() {
            Some(sink) => Box::new(SinkWriter(sink)),
            None => Box::new(io::stderr()),
        }
    }

    struct SinkWriter(PrintSink);

    impl Write for SinkWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (self.0)(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
