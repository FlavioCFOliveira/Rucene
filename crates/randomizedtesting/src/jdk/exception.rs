//! Java's unchecked exceptions, as Rust panics.
//!
//! randomizedtesting signals misuse with unchecked exceptions
//! (`IllegalArgumentException`, `IllegalStateException`, `RuntimeException`,
//! `AssertionError`), which fail the test that triggered them. A Rust test
//! fails the same way on a panic, so the port panics where Java throws. The
//! panic message is the exception's `Throwable.toString()`: the binary class
//! name, then `": "` and the message when there is one, so both the error
//! class and its message are observable and can be compared with Java's.
//!
//! **Divergence.** Java exceptions are values that a caller can
//! catch by class; a Rust panic is caught with `std::panic::catch_unwind` and
//! carries the class only in its message text.

use std::collections::HashMap;
use std::fmt::{self, Display};
use std::panic::{RefUnwindSafe, UnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, OnceLock, RwLock};

/// `java.lang.IllegalArgumentException`.
pub const ILLEGAL_ARGUMENT_EXCEPTION: &str = "java.lang.IllegalArgumentException";
/// `java.lang.IllegalStateException`.
pub const ILLEGAL_STATE_EXCEPTION: &str = "java.lang.IllegalStateException";
/// `java.lang.RuntimeException`.
pub const RUNTIME_EXCEPTION: &str = "java.lang.RuntimeException";
/// `java.lang.AssertionError`.
pub const ASSERTION_ERROR: &str = "java.lang.AssertionError";
/// `java.lang.NullPointerException`.
pub const NULL_POINTER_EXCEPTION: &str = "java.lang.NullPointerException";
/// `java.lang.NegativeArraySizeException`.
pub const NEGATIVE_ARRAY_SIZE_EXCEPTION: &str = "java.lang.NegativeArraySizeException";
/// `java.lang.ArithmeticException`.
pub const ARITHMETIC_EXCEPTION: &str = "java.lang.ArithmeticException";
/// `java.lang.IllegalThreadStateException`.
pub const ILLEGAL_THREAD_STATE_EXCEPTION: &str = "java.lang.IllegalThreadStateException";

/// Formats an exception as `Throwable.toString()` does: the class name, then
/// `": "` and the message when the message is not `null`.
pub fn to_string(class: &str, message: Option<&dyn Display>) -> String {
    match message {
        Some(m) => format!("{class}: {m}"),
        None => class.to_string(),
    }
}

/// Throws an exception of `class` with `message`: panics with its
/// `Throwable.toString()`.
#[track_caller]
pub fn throw(class: &str, message: impl Display) -> ! {
    panic!("{}", to_string(class, Some(&message)))
}

/// Throws an exception of `class` whose message is `null`.
#[track_caller]
pub fn throw_without_message(class: &str) -> ! {
    panic!("{}", class)
}

/// `throw new IllegalArgumentException(message)`.
#[track_caller]
pub fn illegal_argument(message: impl Display) -> ! {
    throw(ILLEGAL_ARGUMENT_EXCEPTION, message)
}

/// `throw new IllegalStateException(message)`.
#[track_caller]
pub fn illegal_state(message: impl Display) -> ! {
    throw(ILLEGAL_STATE_EXCEPTION, message)
}

/// `throw new RuntimeException(message)`.
#[track_caller]
pub fn runtime(message: impl Display) -> ! {
    throw(RUNTIME_EXCEPTION, message)
}

/// Java's `assert condition : message;` under `-ea`.
///
/// **Divergence.** Java evaluates `assert` only when assertions are
/// enabled (`-ea`, as Lucene's and randomizedtesting's builds run tests); the
/// port evaluates it when `debug_assertions` are on, the Rust counterpart.
#[track_caller]
#[inline]
pub fn java_assert(condition: bool, message: impl FnOnce() -> String) {
    if cfg!(debug_assertions) && !condition {
        throw(ASSERTION_ERROR, message())
    }
}

/// Java's `assert condition;` (no message) under `-ea`; see [`java_assert`].
#[track_caller]
#[inline]
pub fn java_assert_no_message(condition: bool) {
    if cfg!(debug_assertions) && !condition {
        throw_without_message(ASSERTION_ERROR)
    }
}

/// The message of a panic payload, as the tests and the harness read it: a
/// [`Throwable`]'s `toString()`, or the text of a string payload.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(t) = payload.downcast_ref::<Throwable>() {
        t.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// `java.lang.InterruptedException`.
pub const INTERRUPTED_EXCEPTION: &str = "java.lang.InterruptedException";
/// `java.lang.UnsupportedOperationException`.
pub const UNSUPPORTED_OPERATION_EXCEPTION: &str = "java.lang.UnsupportedOperationException";
/// `java.lang.Exception`.
pub const EXCEPTION: &str = "java.lang.Exception";
/// `java.lang.Error`.
pub const ERROR: &str = "java.lang.Error";
/// `java.lang.Throwable`.
pub const THROWABLE: &str = "java.lang.Throwable";
/// `java.io.IOException`.
pub const IO_EXCEPTION: &str = "java.io.IOException";

/// The superclass of every throwable class the port knows by name
/// (`Class.getSuperclass()`), so that [`Throwable::is_instance`] answers
/// `instanceof` for them. [`register_class`] adds more.
static CLASS_PARENTS: LazyLock<RwLock<HashMap<String, String>>> = LazyLock::new(|| {
    let known: &[(&str, &str)] = &[
        (EXCEPTION, THROWABLE),
        (ERROR, THROWABLE),
        (RUNTIME_EXCEPTION, EXCEPTION),
        (ILLEGAL_ARGUMENT_EXCEPTION, RUNTIME_EXCEPTION),
        (ILLEGAL_STATE_EXCEPTION, RUNTIME_EXCEPTION),
        (NULL_POINTER_EXCEPTION, RUNTIME_EXCEPTION),
        (NEGATIVE_ARRAY_SIZE_EXCEPTION, RUNTIME_EXCEPTION),
        (ARITHMETIC_EXCEPTION, RUNTIME_EXCEPTION),
        (UNSUPPORTED_OPERATION_EXCEPTION, RUNTIME_EXCEPTION),
        ("java.lang.ClassCastException", RUNTIME_EXCEPTION),
        ("java.lang.IndexOutOfBoundsException", RUNTIME_EXCEPTION),
        (
            "java.lang.ArrayIndexOutOfBoundsException",
            "java.lang.IndexOutOfBoundsException",
        ),
        ("java.lang.SecurityException", RUNTIME_EXCEPTION),
        ("java.lang.IllegalCallerException", RUNTIME_EXCEPTION),
        ("java.util.NoSuchElementException", RUNTIME_EXCEPTION),
        (
            "java.util.ConcurrentModificationException",
            RUNTIME_EXCEPTION,
        ),
        (ILLEGAL_THREAD_STATE_EXCEPTION, ILLEGAL_ARGUMENT_EXCEPTION),
        (
            "java.lang.NumberFormatException",
            ILLEGAL_ARGUMENT_EXCEPTION,
        ),
        (
            "java.util.regex.PatternSyntaxException",
            ILLEGAL_ARGUMENT_EXCEPTION,
        ),
        (ASSERTION_ERROR, ERROR),
        ("java.lang.ThreadDeath", ERROR),
        ("java.lang.VirtualMachineError", ERROR),
        (
            "java.lang.OutOfMemoryError",
            "java.lang.VirtualMachineError",
        ),
        (
            "java.lang.StackOverflowError",
            "java.lang.VirtualMachineError",
        ),
        ("java.lang.UnknownError", "java.lang.VirtualMachineError"),
        ("java.lang.InternalError", "java.lang.VirtualMachineError"),
        (
            "java.lang.ExceptionInInitializerError",
            "java.lang.LinkageError",
        ),
        ("java.lang.LinkageError", ERROR),
        (INTERRUPTED_EXCEPTION, EXCEPTION),
        ("java.lang.CloneNotSupportedException", EXCEPTION),
        ("java.lang.ReflectiveOperationException", EXCEPTION),
        (
            "java.lang.InstantiationException",
            "java.lang.ReflectiveOperationException",
        ),
        (
            "java.lang.reflect.InvocationTargetException",
            "java.lang.ReflectiveOperationException",
        ),
        (IO_EXCEPTION, EXCEPTION),
        // The `java.io` and `java.nio` exceptions the port's `java.nio.file`
        // shim raises (JDK 25 superclasses, read with `javap`), so that a
        // `catch (IOException e)` or `expectThrows(IOException.class, ...)`
        // sees them as Java does.
        ("java.io.FileNotFoundException", IO_EXCEPTION),
        ("java.io.EOFException", IO_EXCEPTION),
        ("java.io.IOError", ERROR),
        ("java.nio.file.FileSystemException", IO_EXCEPTION),
        (
            "java.nio.file.AccessDeniedException",
            "java.nio.file.FileSystemException",
        ),
        (
            "java.nio.file.AtomicMoveNotSupportedException",
            "java.nio.file.FileSystemException",
        ),
        (
            "java.nio.file.DirectoryNotEmptyException",
            "java.nio.file.FileSystemException",
        ),
        (
            "java.nio.file.FileAlreadyExistsException",
            "java.nio.file.FileSystemException",
        ),
        (
            "java.nio.file.FileSystemLoopException",
            "java.nio.file.FileSystemException",
        ),
        (
            "java.nio.file.NoSuchFileException",
            "java.nio.file.FileSystemException",
        ),
        (
            "java.nio.file.NotDirectoryException",
            "java.nio.file.FileSystemException",
        ),
        (
            "java.nio.file.NotLinkException",
            "java.nio.file.FileSystemException",
        ),
        (
            "java.nio.file.InvalidPathException",
            ILLEGAL_ARGUMENT_EXCEPTION,
        ),
        (
            "java.nio.file.ProviderMismatchException",
            ILLEGAL_ARGUMENT_EXCEPTION,
        ),
        (
            "java.nio.file.ClosedFileSystemException",
            ILLEGAL_STATE_EXCEPTION,
        ),
        ("java.nio.channels.ClosedChannelException", IO_EXCEPTION),
        (
            "java.nio.channels.NonWritableChannelException",
            ILLEGAL_STATE_EXCEPTION,
        ),
        (
            "java.nio.channels.NonReadableChannelException",
            ILLEGAL_STATE_EXCEPTION,
        ),
        ("java.nio.charset.CharacterCodingException", IO_EXCEPTION),
        (
            "java.nio.charset.MalformedInputException",
            "java.nio.charset.CharacterCodingException",
        ),
        ("java.io.UncheckedIOException", RUNTIME_EXCEPTION),
        ("java.util.concurrent.TimeoutException", EXCEPTION),
        ("java.util.concurrent.ExecutionException", EXCEPTION),
        (
            "org.junit.internal.AssumptionViolatedException",
            RUNTIME_EXCEPTION,
        ),
        (
            "org.junit.AssumptionViolatedException",
            "org.junit.internal.AssumptionViolatedException",
        ),
        ("org.junit.ComparisonFailure", ASSERTION_ERROR),
        (
            "org.junit.runners.model.MultipleFailureException",
            EXCEPTION,
        ),
        (
            "org.junit.internal.runners.model.MultipleFailureException",
            "org.junit.runners.model.MultipleFailureException",
        ),
        ("org.junit.runners.model.InitializationError", EXCEPTION),
        (
            "org.junit.runner.manipulation.NoTestsRemainException",
            EXCEPTION,
        ),
        ("org.junit.runners.model.TestTimedOutException", EXCEPTION),
        (
            "org.junit.runner.notification.StoppedByUserException",
            RUNTIME_EXCEPTION,
        ),
    ];
    RwLock::new(
        known
            .iter()
            .map(|(c, p)| (c.to_string(), p.to_string()))
            .collect(),
    )
});

/// Declares `class` a subclass of `superclass`, so that throwables of
/// `class` are `instanceof` every ancestor of `superclass` (the Rust
/// counterpart of loading the class).
pub fn register_class(class: &str, superclass: &str) {
    CLASS_PARENTS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(class.to_string(), superclass.to_string());
}

/// `Class.getSuperclass().getName()` of a known throwable class; `None` for
/// `java.lang.Throwable` and for an unknown class.
pub fn superclass_of(class: &str) -> Option<String> {
    CLASS_PARENTS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(class)
        .cloned()
}

/// `java.lang.StackTraceElement`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackTraceElement {
    /// `getClassName()`.
    pub declaring_class: String,
    /// `getMethodName()`.
    pub method_name: String,
    /// `getFileName()`.
    pub file_name: Option<String>,
    /// `getLineNumber()`.
    pub line_number: i32,
}

impl StackTraceElement {
    /// `new StackTraceElement(declaringClass, methodName, fileName, lineNumber)`.
    pub fn new(
        declaring_class: impl Into<String>,
        method_name: impl Into<String>,
        file_name: Option<String>,
        line_number: i32,
    ) -> StackTraceElement {
        StackTraceElement {
            declaring_class: declaring_class.into(),
            method_name: method_name.into(),
            file_name,
            line_number,
        }
    }
}

impl Display for StackTraceElement {
    /// `StackTraceElement.toString()` of an element without class loader and
    /// module: `class.method(file:line)`, `(file)` for a negative line,
    /// `(Unknown Source)` without a file, `(Native Method)` for line `-2`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}(", self.declaring_class, self.method_name)?;
        if self.line_number == -2 {
            f.write_str("Native Method")?;
        } else {
            match &self.file_name {
                Some(file) if self.line_number >= 0 => write!(f, "{file}:{}", self.line_number)?,
                Some(file) => f.write_str(file)?,
                None => f.write_str("Unknown Source")?,
            }
        }
        f.write_str(")")
    }
}

/// A Java throwable as a value: `java.lang.Throwable` and its subclasses.
///
/// The port throws a `Throwable` by panicking with it as the payload
/// ([`Throwable::throw`]) and catches it with `std::panic::catch_unwind`
/// ([`Throwable::from_panic`]). A string panic (every
/// [`throw`]-style function of this module, and every Rust `panic!` or
/// `assert!`) converts to a `Throwable` too: `"class: message"` gives that
/// class and message, any other text an `AssertionError` with that text as
/// its message.
///
/// **Divergences.**
/// * A Java throwable is an instance of its class; here the class is its
///   binary name, and `instanceof` follows the superclass names registered
///   with [`register_class`] (the JDK and JUnit throwables the port uses are
///   preregistered). A class that is not registered is a direct subclass of
///   `java.lang.Throwable`.
/// * The JVM fills a throwable's stack trace when it is created; a Rust
///   panic has no Java frames, so the stack trace starts empty and holds
///   only the elements code adds: `setStackTrace` ([`set_stack_trace`]),
///   such as the runner's seed frame, or the frames of a stack captured
///   elsewhere ([`set_lazy_stack_trace`]), which, as the JVM's
///   `getOurStackTrace()` does, are materialised only when first read.
///
/// [`set_stack_trace`]: Throwable::set_stack_trace
/// [`set_lazy_stack_trace`]: Throwable::set_lazy_stack_trace
/// * A Rust panic whose text is not `"class: message"` (a failed `assert!`,
///   say) becomes a `java.lang.AssertionError`, the error Java's `assert`
///   and JUnit's `Assert` throw.
#[derive(Debug, Clone)]
pub struct Throwable(Box<ThrowableData>);

#[derive(Debug, Clone)]
struct ThrowableData {
    class: String,
    message: Option<String>,
    cause: Option<Throwable>,
    stack_trace: StackTrace,
    suppressed: Vec<Throwable>,
    /// `MultipleFailureException.getFailures()`.
    failures: Vec<Throwable>,
}

/// The frames of a throwable's `getStackTrace()`.
///
/// Java's `Throwable` keeps the stack that `fillInStackTrace()` captured as
/// an opaque `backtrace` and builds the `StackTraceElement`s from it only
/// when they are first needed (`getOurStackTrace()`, which `getStackTrace()`
/// and `printStackTrace()` call); `setStackTrace` replaces them
/// (`java.lang.Throwable`, JDK 21 and 25).
#[derive(Clone)]
enum StackTrace {
    /// Elements set as they are: none for a new throwable, or the elements of
    /// `setStackTrace`.
    Elements(Vec<StackTraceElement>),
    /// The elements of a captured stack, built on first read. The clones of
    /// a throwable share them, as they share the stack.
    Captured(Arc<CapturedStack>),
}

/// A captured stack whose elements are built on first read.
struct CapturedStack {
    /// The elements, once built.
    elements: OnceLock<Vec<StackTraceElement>>,
    /// Builds the elements from the captured stack.
    resolve: Box<dyn Fn() -> Vec<StackTraceElement> + Send + Sync>,
}

// A panic in `resolve` leaves `elements` unset (`OnceLock::get_or_init`
// stores only a value that `resolve` returned), and `resolve` is reached
// only through a shared reference, so no state half-updated by an unwind can
// be observed afterwards: the stack keeps the unwind safety a `Throwable` of
// plain elements has.
impl UnwindSafe for CapturedStack {}
impl RefUnwindSafe for CapturedStack {}

impl StackTrace {
    /// The elements, built from the captured stack on first read.
    fn elements(&self) -> &[StackTraceElement] {
        match self {
            StackTrace::Elements(elements) => elements,
            StackTrace::Captured(captured) => {
                captured.elements.get_or_init(|| (captured.resolve)())
            }
        }
    }
}

impl fmt::Debug for StackTrace {
    /// The elements' debug form, as a `Vec<StackTraceElement>` prints it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.elements(), f)
    }
}

impl Throwable {
    /// `new <class>(message)`.
    pub fn new(class: impl Into<String>, message: impl Into<String>) -> Throwable {
        Throwable::with_message(class, Some(message.into()))
    }

    /// `new <class>(message)` with a possibly `null` message.
    pub fn with_message(class: impl Into<String>, message: Option<String>) -> Throwable {
        Throwable(Box::new(ThrowableData {
            class: class.into(),
            message,
            cause: None,
            stack_trace: StackTrace::Elements(Vec::new()),
            suppressed: Vec::new(),
            failures: Vec::new(),
        }))
    }

    /// `new <class>(message, cause)`.
    pub fn with_cause(
        class: impl Into<String>,
        message: Option<String>,
        cause: Throwable,
    ) -> Throwable {
        let mut t = Throwable::with_message(class, message);
        t.0.cause = Some(cause);
        t
    }

    /// `new <class>(cause)`: the message is `cause.toString()`.
    pub fn from_cause(class: impl Into<String>, cause: Throwable) -> Throwable {
        let message = Some(cause.to_string());
        Throwable::with_cause(class, message, cause)
    }

    /// The throwable a panic payload carries: the payload itself, or the
    /// conversion of a string panic described on [`Throwable`].
    pub fn from_panic(payload: Box<dyn std::any::Any + Send>) -> Throwable {
        match payload.downcast::<Throwable>() {
            Ok(t) => *t,
            Err(payload) => Throwable::from_text(&panic_message(&*payload)),
        }
    }

    /// The throwable a string panic stands for.
    pub fn from_text(text: &str) -> Throwable {
        match text.split_once(": ") {
            Some((class, message)) if is_class_name(class) => {
                Throwable::new(class, message.to_string())
            }
            _ if is_class_name(text) => Throwable::with_message(text, None),
            _ => Throwable::new(ASSERTION_ERROR, text.to_string()),
        }
    }

    /// `throw this`: panics with this throwable as the payload.
    pub fn throw(self) -> ! {
        std::panic::panic_any(self)
    }

    /// `getClass().getName()`.
    pub fn class_name(&self) -> &str {
        &self.0.class
    }

    /// `getMessage()`.
    pub fn message(&self) -> Option<&str> {
        self.0.message.as_deref()
    }

    /// `getCause()`.
    pub fn cause(&self) -> Option<&Throwable> {
        self.0.cause.as_ref()
    }

    /// `initCause(cause)`.
    pub fn set_cause(&mut self, cause: Throwable) {
        self.0.cause = Some(cause);
    }

    /// `getStackTrace()`: the elements set, or those of the captured stack
    /// ([`set_lazy_stack_trace`](Self::set_lazy_stack_trace)), built on the
    /// first read.
    pub fn stack_trace(&self) -> &[StackTraceElement] {
        self.0.stack_trace.elements()
    }

    /// `setStackTrace(stackTrace)`.
    pub fn set_stack_trace(&mut self, stack_trace: Vec<StackTraceElement>) {
        self.0.stack_trace = StackTrace::Elements(stack_trace);
    }

    /// The frames of a stack captured elsewhere, as the JVM's
    /// `fillInStackTrace()` leaves them: `resolve` builds the elements and
    /// runs only when they are first read ([`stack_trace`](Self::stack_trace),
    /// the debug form), once, its result shared by the clones of this
    /// throwable. Java's `Throwable.getOurStackTrace()` builds the
    /// `StackTraceElement`s of its `backtrace` the same way, on first use, so
    /// a throwable whose frames nobody reads never has them resolved.
    pub fn set_lazy_stack_trace(
        &mut self,
        resolve: impl Fn() -> Vec<StackTraceElement> + Send + Sync + 'static,
    ) {
        self.0.stack_trace = StackTrace::Captured(Arc::new(CapturedStack {
            elements: OnceLock::new(),
            resolve: Box::new(resolve),
        }));
    }

    /// `getSuppressed()`.
    pub fn suppressed(&self) -> &[Throwable] {
        &self.0.suppressed
    }

    /// `addSuppressed(exception)`.
    pub fn add_suppressed(&mut self, exception: Throwable) {
        self.0.suppressed.push(exception);
    }

    /// `instanceof <class>`: whether this throwable's class is `class` or a
    /// subclass of it.
    pub fn is_instance(&self, class: &str) -> bool {
        if class == THROWABLE {
            return true;
        }
        let mut current = Some(self.0.class.clone());
        let mut steps = 0;
        while let Some(c) = current {
            if c == class {
                return true;
            }
            steps += 1;
            if steps > 64 {
                break;
            }
            current = superclass_of(&c);
        }
        false
    }

    /// `new org.junit.internal.runners.model.MultipleFailureException(errors)`
    /// (what `MultipleFailureException.assertEmpty` throws for two or more
    /// errors): its message lists each error as `class(message)`.
    pub fn multiple_failures(errors: Vec<Throwable>) -> Throwable {
        Throwable::multiple_failures_of_class(
            "org.junit.internal.runners.model.MultipleFailureException",
            errors,
        )
    }

    /// A `MultipleFailureException` of the given class (the public
    /// `org.junit.runners.model.MultipleFailureException` or its internal
    /// subclass).
    pub fn multiple_failures_of_class(class: &str, errors: Vec<Throwable>) -> Throwable {
        let mut message = format!("There were {} errors:", errors.len());
        for e in &errors {
            message.push_str(&format!(
                "\n  {}({})",
                e.0.class,
                e.0.message.as_deref().unwrap_or("null")
            ));
        }
        let mut t = Throwable::new(class, message);
        t.0.failures = errors;
        t
    }

    /// `MultipleFailureException.getFailures()`; empty for any other
    /// throwable.
    pub fn failures(&self) -> &[Throwable] {
        &self.0.failures
    }

    /// Whether this is a `MultipleFailureException`.
    pub fn is_multiple_failure(&self) -> bool {
        self.is_instance("org.junit.runners.model.MultipleFailureException")
    }
}

impl Display for Throwable {
    /// `Throwable.toString()`: the class name, then `": "` and the message
    /// when it is not `null`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0.message {
            Some(m) => write!(f, "{}: {}", self.0.class, m),
            None => f.write_str(&self.0.class),
        }
    }
}

/// Whether `s` reads as a Java binary class name: identifiers joined by
/// `.`, at least two of them.
fn is_class_name(s: &str) -> bool {
    let mut parts = 0;
    for part in s.split('.') {
        let mut chars = part.chars();
        match chars.next() {
            Some(c) if c.is_alphabetic() || c == '_' || c == '$' => {}
            _ => return false,
        }
        if !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$') {
            return false;
        }
        parts += 1;
    }
    parts >= 2
}

/// The number of active scopes that silence the panic hook
/// ([`QuietPanics`]).
static QUIET: AtomicUsize = AtomicUsize::new(0);

/// While alive, panics print nothing: Java does not print an exception it
/// throws and catches, and the runner reports every failure itself.
///
/// The first guard installs a panic hook that defers to the previous hook
/// whenever no guard is alive.
pub struct QuietPanics(());

impl QuietPanics {
    /// Silences the panic hook until the guard drops.
    pub fn new() -> QuietPanics {
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if QUIET.load(Ordering::Acquire) == 0 {
                    previous(info);
                }
            }));
        });
        QUIET.fetch_add(1, Ordering::AcqRel);
        QuietPanics(())
    }
}

impl Default for QuietPanics {
    fn default() -> Self {
        QuietPanics::new()
    }
}

impl Drop for QuietPanics {
    fn drop(&mut self) {
        QUIET.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::{QuietPanics, StackTraceElement, Throwable};

    fn elements() -> Vec<StackTraceElement> {
        vec![
            StackTraceElement::new("foo.Bar", "baz", Some("Bar.java".into()), 7),
            StackTraceElement::new("foo.Qux", "quux", None, -1),
        ]
    }

    /// A throwable with lazy frames whose resolutions are counted.
    fn lazy(calls: &Arc<AtomicUsize>) -> Throwable {
        let mut t = Throwable::new("java.lang.IllegalStateException", "m");
        let calls = Arc::clone(calls);
        t.set_lazy_stack_trace(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            elements()
        });
        t
    }

    /// The frames are built only when read, once, and the clones of the
    /// throwable share them, as `getOurStackTrace()` builds them on first use.
    #[test]
    fn lazy_frames_resolve_once_on_first_read() {
        let calls = Arc::new(AtomicUsize::new(0));
        let t = lazy(&calls);
        assert!(t.is_instance("java.lang.RuntimeException"));
        assert_eq!(t.to_string(), "java.lang.IllegalStateException: m");
        let copy = t.clone();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(t.stack_trace(), elements().as_slice());
        assert_eq!(copy.stack_trace(), elements().as_slice());
        assert_eq!(t.stack_trace(), elements().as_slice());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Lazy frames read, print and format as the same frames set eagerly.
    #[test]
    fn lazy_frames_read_as_eager_frames() {
        let calls = Arc::new(AtomicUsize::new(0));
        let lazy = lazy(&calls);
        let mut eager = Throwable::new("java.lang.IllegalStateException", "m");
        eager.set_stack_trace(elements());
        assert_eq!(format!("{lazy:?}"), format!("{eager:?}"));
        assert_eq!(format!("{lazy:#?}"), format!("{eager:#?}"));
        assert_eq!(
            crate::jdk::system::format_stack_trace(&lazy),
            crate::jdk::system::format_stack_trace(&eager)
        );
    }

    /// `setStackTrace` replaces lazy frames without resolving them.
    #[test]
    fn set_stack_trace_replaces_lazy_frames() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut t = lazy(&calls);
        t.set_stack_trace(Vec::new());
        assert!(t.stack_trace().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// A resolver that panics leaves the frames unbuilt; the next read runs
    /// it again.
    #[test]
    fn a_panicking_resolver_leaves_the_frames_unbuilt() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut t = Throwable::new("java.lang.RuntimeException", "m");
        let counter = Arc::clone(&calls);
        t.set_lazy_stack_trace(move || {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("first resolution fails");
            }
            elements()
        });
        let quiet = QuietPanics::new();
        assert!(std::panic::catch_unwind(|| t.stack_trace().len()).is_err());
        drop(quiet);
        assert_eq!(t.stack_trace(), elements().as_slice());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// A throwable stays sendable, shareable and unwind safe, so it can be a
    /// panic payload and be caught by reference.
    #[test]
    fn a_throwable_keeps_its_auto_traits() {
        fn check<T>()
        where
            T: Send + Sync + std::panic::UnwindSafe + std::panic::RefUnwindSafe + 'static,
        {
        }
        check::<Throwable>();
    }
}
