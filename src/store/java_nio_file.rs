//! The `java.nio.file` surface Lucene's store uses: [`Path`], [`FileSystem`],
//! [`FileSystemProvider`], [`Files`], [`FileChannel`], [`DirectoryStream`],
//! [`FileStore`] and the streams and options they hand out.
//!
//! Lucene selects the filesystem through the path: `FSDirectory.open(path)`
//! works on whatever `path.getFileSystem().provider()` is, and the test
//! framework's `mockfile` package wraps a provider (`FilterFileSystemProvider`)
//! and hands out paths of the wrapping filesystem, so that every file operation
//! the store makes on such a path goes through the wrapper. This module
//! reproduces that model: a [`Path`] carries its [`FileSystem`], every
//! operation in [`Files`] dispatches to `path.get_file_system().provider()`,
//! and [`default_file_system`] (the JDK's `FileSystems.getDefault()`) is a
//! provider over `std::fs`.
//!
//! | Rucene | JDK 25 |
//! | --- | --- |
//! | [`Path`] | `java.nio.file.Path` (`sun.nio.fs.UnixPath` for the default filesystem) |
//! | [`FileSystem`], [`default_file_system`] | `java.nio.file.FileSystem`, `FileSystems.getDefault()` |
//! | [`FileSystemProvider`] | `java.nio.file.spi.FileSystemProvider` |
//! | [`Files`] | `java.nio.file.Files` |
//! | [`FileChannel`], [`SeekableByteChannel`], [`FileLock`] | `java.nio.channels.FileChannel`, `SeekableByteChannel`, `FileLock` |
//! | [`AsynchronousFileChannel`] | `java.nio.channels.AsynchronousFileChannel` |
//! | [`InputStream`], [`OutputStream`] | `java.io.InputStream`, `java.io.OutputStream` |
//! | [`DirectoryStream`], [`PathMatcher`], [`FileVisitor`] | `java.nio.file.DirectoryStream`, `PathMatcher`, `FileVisitor` |
//! | [`FileStore`] | `java.nio.file.FileStore` |
//! | [`BasicFileAttributes`], [`FileKey`], [`AttributeValue`] | `java.nio.file.attribute.BasicFileAttributes`, `sun.nio.fs.UnixFileKey`, the attribute values |
//! | [`OpenOption`], [`CopyOption`], [`LinkOption`], [`AccessMode`] | `StandardOpenOption`, `StandardCopyOption`, `LinkOption`, `AccessMode` |
//! | [`JavaIoException`] | `java.io.IOException` and its `java.nio.file` / `java.nio.channels` subclasses |
//!
//! **Divergence.** This module is an emulation of the JDK, not a port of a
//! Lucene file: the JDK is GPL with the Classpath Exception, so its behaviour is
//! reproduced from its specification (javadoc) and verified against a JDK 25
//! oracle, not copied. Where the emulation differs from the JDK, the item says
//! so at its point. The differences that span the module are:
//!
//! * a checked `IOException` is an [`io::Error`], whose Java class and message
//!   [`JavaIoException`] carries; an unchecked exception (`InvalidPathException`,
//!   `ProviderMismatchException`, `UnsupportedOperationException`,
//!   `IllegalArgumentException`) is a panic whose message starts with the Java
//!   class name, as it propagates through Java callers unchecked;
//! * the attribute-view class hierarchy (`getFileAttributeView`,
//!   `getFileStoreAttributeView`, `supportsFileAttributeView(Class)`), watch
//!   services, user-principal lookups and `newFileSystem` have no counterpart:
//!   Lucene's store calls none of them, and the views are only reachable here
//!   through the string forms ([`FileSystemProvider::read_attributes_by_name`],
//!   [`FileSystemProvider::set_attribute`]);
//! * a `ByteBuffer` argument is a byte slice: a read fills the whole slice as
//!   Java fills `dst.remaining()` bytes, and returns the count;
//! * the default filesystem reproduces `sun.nio.fs.UnixFileSystem` on Unix.
//!   On other targets its paths follow [`std::path`] and its errors are the
//!   platform's [`io::Error`]s unchanged, not `WindowsFileSystem`'s.

#![deny(unsafe_code)]

mod default_file_system;

use std::any::Any;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

pub use default_file_system::{default_file_system, DefaultFileSystem, DefaultFileSystemProvider};

/// `java.nio.file.FileSystems.getDefault()`.
pub struct FileSystems;

impl FileSystems {
    /// `FileSystems.getDefault()`: the filesystem over `std::fs`.
    pub fn get_default() -> Arc<dyn FileSystem> {
        default_file_system()
    }
}

// =============================================================================
// Exceptions
// =============================================================================

/// `java.io.IOException`.
pub const IO_EXCEPTION: &str = "java.io.IOException";
/// `java.io.EOFException`.
pub const EOF_EXCEPTION: &str = "java.io.EOFException";
/// `java.nio.file.FileSystemException`.
pub const FILE_SYSTEM_EXCEPTION: &str = "java.nio.file.FileSystemException";
/// `java.nio.file.NoSuchFileException`.
pub const NO_SUCH_FILE_EXCEPTION: &str = "java.nio.file.NoSuchFileException";
/// `java.nio.file.FileAlreadyExistsException`.
pub const FILE_ALREADY_EXISTS_EXCEPTION: &str = "java.nio.file.FileAlreadyExistsException";
/// `java.nio.file.AccessDeniedException`.
pub const ACCESS_DENIED_EXCEPTION: &str = "java.nio.file.AccessDeniedException";
/// `java.nio.file.DirectoryNotEmptyException`.
pub const DIRECTORY_NOT_EMPTY_EXCEPTION: &str = "java.nio.file.DirectoryNotEmptyException";
/// `java.nio.file.NotDirectoryException`.
pub const NOT_DIRECTORY_EXCEPTION: &str = "java.nio.file.NotDirectoryException";
/// `java.nio.file.NotLinkException`.
pub const NOT_LINK_EXCEPTION: &str = "java.nio.file.NotLinkException";
/// `java.nio.file.AtomicMoveNotSupportedException`.
pub const ATOMIC_MOVE_NOT_SUPPORTED_EXCEPTION: &str =
    "java.nio.file.AtomicMoveNotSupportedException";
/// `java.nio.channels.ClosedChannelException`.
pub const CLOSED_CHANNEL_EXCEPTION: &str = "java.nio.channels.ClosedChannelException";
/// `java.nio.channels.NonReadableChannelException` (unchecked).
pub const NON_READABLE_CHANNEL_EXCEPTION: &str = "java.nio.channels.NonReadableChannelException";
/// `java.nio.channels.NonWritableChannelException` (unchecked).
pub const NON_WRITABLE_CHANNEL_EXCEPTION: &str = "java.nio.channels.NonWritableChannelException";
/// `java.util.concurrent.ExecutionException`.
pub const EXECUTION_EXCEPTION: &str = "java.util.concurrent.ExecutionException";

/// A `java.io.IOException` of a given class, carried as the payload of an
/// [`io::Error`].
///
/// Two shapes exist, as in the JDK: the `java.nio.file.FileSystemException`
/// family, which holds a file, another file and a reason and builds its
/// message from them (`FileSystemException.getMessage()`), and every other
/// `IOException`, which holds a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaIoException {
    class: Cow<'static, str>,
    shape: Shape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Shape {
    Message(Option<String>),
    FileSystem {
        file: Option<String>,
        other: Option<String>,
        reason: Option<String>,
    },
}

impl JavaIoException {
    /// An exception of `class` with `message` (`new X(message)`).
    pub fn new(class: impl Into<Cow<'static, str>>, message: Option<String>) -> JavaIoException {
        JavaIoException {
            class: class.into(),
            shape: Shape::Message(message),
        }
    }

    /// A `FileSystemException` (or subclass) of `class`:
    /// `new X(file, other, reason)`.
    pub fn file_system(
        class: impl Into<Cow<'static, str>>,
        file: Option<String>,
        other: Option<String>,
        reason: Option<String>,
    ) -> JavaIoException {
        JavaIoException {
            class: class.into(),
            shape: Shape::FileSystem {
                file,
                other,
                reason,
            },
        }
    }

    /// `getClass().getName()`.
    pub fn class_name(&self) -> &str {
        &self.class
    }

    /// `getMessage()`. For the `FileSystemException` family: the reason when
    /// neither file is set, and otherwise `file`, then ` -> other` when there
    /// is another file, then `: reason` when there is a reason.
    pub fn get_message(&self) -> Option<String> {
        match &self.shape {
            Shape::Message(message) => message.clone(),
            Shape::FileSystem {
                file,
                other,
                reason,
            } => {
                if file.is_none() && other.is_none() {
                    return reason.clone();
                }
                let mut sb = String::new();
                if let Some(file) = file {
                    sb.push_str(file);
                }
                if let Some(other) = other {
                    sb.push_str(" -> ");
                    sb.push_str(other);
                }
                if let Some(reason) = reason {
                    sb.push_str(": ");
                    sb.push_str(reason);
                }
                Some(sb)
            }
        }
    }

    /// `FileSystemException.getFile()`.
    pub fn get_file(&self) -> Option<&str> {
        match &self.shape {
            Shape::FileSystem { file, .. } => file.as_deref(),
            Shape::Message(_) => None,
        }
    }

    /// `FileSystemException.getOtherFile()`.
    pub fn get_other_file(&self) -> Option<&str> {
        match &self.shape {
            Shape::FileSystem { other, .. } => other.as_deref(),
            Shape::Message(_) => None,
        }
    }

    /// `FileSystemException.getReason()`.
    pub fn get_reason(&self) -> Option<&str> {
        match &self.shape {
            Shape::FileSystem { reason, .. } => reason.as_deref(),
            Shape::Message(_) => None,
        }
    }

    /// The [`io::ErrorKind`] that stands for this class.
    pub fn kind(&self) -> io::ErrorKind {
        match self.class.as_ref() {
            NO_SUCH_FILE_EXCEPTION => io::ErrorKind::NotFound,
            FILE_ALREADY_EXISTS_EXCEPTION => io::ErrorKind::AlreadyExists,
            ACCESS_DENIED_EXCEPTION => io::ErrorKind::PermissionDenied,
            DIRECTORY_NOT_EMPTY_EXCEPTION => io::ErrorKind::DirectoryNotEmpty,
            NOT_DIRECTORY_EXCEPTION => io::ErrorKind::NotADirectory,
            EOF_EXCEPTION => io::ErrorKind::UnexpectedEof,
            _ => io::ErrorKind::Other,
        }
    }

    /// This exception as an [`io::Error`] of [`kind`](Self::kind).
    pub fn into_io_error(self) -> io::Error {
        io::Error::new(self.kind(), self)
    }
}

impl fmt::Display for JavaIoException {
    /// `getMessage()`, or the empty string for a `null` message: the text an
    /// [`io::Error`] carrying this exception displays.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_message().as_deref().unwrap_or(""))
    }
}

impl std::error::Error for JavaIoException {}

/// The Java exception carried by `error`, if it carries one.
pub fn java_exception_of(error: &io::Error) -> Option<&JavaIoException> {
    error.get_ref()?.downcast_ref::<JavaIoException>()
}

/// `getClass().getName()` of the `IOException` `error` stands for:
/// [`JavaIoException::class_name`] when it carries one; otherwise
/// `java.io.EOFException` for an [`io::ErrorKind::UnexpectedEof`] error (the
/// kind the store inputs raise where Lucene throws `EOFException`, e.g.
/// `ByteBuffersDataInput.java:102`, `BufferedIndexInput.java:113`), and
/// `java.io.IOException` for any other.
pub fn java_class_of(error: &io::Error) -> &str {
    match java_exception_of(error) {
        Some(exception) => exception.class_name(),
        None if error.kind() == io::ErrorKind::UnexpectedEof => EOF_EXCEPTION,
        None => IO_EXCEPTION,
    }
}

/// `getMessage()` of the `IOException` `error` stands for:
/// [`JavaIoException::get_message`] when it carries one, the error's text
/// otherwise.
pub fn java_message_of(error: &io::Error) -> Option<String> {
    match java_exception_of(error) {
        Some(exception) => exception.get_message(),
        None => Some(error.to_string()),
    }
}

/// `Throwable.toString()` of the `IOException` `error` stands for: the class
/// name, then `": " + message` when the message is not `null`.
pub fn java_to_string(error: &io::Error) -> String {
    match java_message_of(error) {
        Some(message) => format!("{}: {message}", java_class_of(error)),
        None => java_class_of(error).to_string(),
    }
}

/// `new IOException(message)`.
pub fn io_exception(message: impl Into<String>) -> io::Error {
    JavaIoException::new(IO_EXCEPTION, Some(message.into())).into_io_error()
}

/// `new FileSystemException(file, other, reason)`.
pub fn file_system_exception(
    file: Option<String>,
    other: Option<String>,
    reason: Option<String>,
) -> io::Error {
    JavaIoException::file_system(FILE_SYSTEM_EXCEPTION, file, other, reason).into_io_error()
}

/// `new NoSuchFileException(file)`.
pub fn no_such_file_exception(file: impl Into<String>) -> io::Error {
    JavaIoException::file_system(NO_SUCH_FILE_EXCEPTION, Some(file.into()), None, None)
        .into_io_error()
}

/// `new FileAlreadyExistsException(file)`.
pub fn file_already_exists_exception(file: impl Into<String>) -> io::Error {
    JavaIoException::file_system(FILE_ALREADY_EXISTS_EXCEPTION, Some(file.into()), None, None)
        .into_io_error()
}

/// `new AccessDeniedException(file)`.
pub fn access_denied_exception(file: impl Into<String>) -> io::Error {
    JavaIoException::file_system(ACCESS_DENIED_EXCEPTION, Some(file.into()), None, None)
        .into_io_error()
}

/// `new DirectoryNotEmptyException(dir)`.
pub fn directory_not_empty_exception(dir: impl Into<String>) -> io::Error {
    JavaIoException::file_system(DIRECTORY_NOT_EMPTY_EXCEPTION, Some(dir.into()), None, None)
        .into_io_error()
}

/// `new NotDirectoryException(file)`.
pub fn not_directory_exception(file: impl Into<String>) -> io::Error {
    JavaIoException::file_system(NOT_DIRECTORY_EXCEPTION, Some(file.into()), None, None)
        .into_io_error()
}

/// `new ClosedChannelException()`, whose message is `null`.
pub fn closed_channel_exception() -> io::Error {
    JavaIoException::new(CLOSED_CHANNEL_EXCEPTION, None).into_io_error()
}

/// Throws the unchecked Java exception `class` with `message`: a panic whose
/// text is `class: message` (`class` alone for a `null` message), the form the
/// test framework reads a Java exception back from.
pub fn throw_unchecked(class: &str, message: Option<&str>) -> ! {
    match message {
        Some(message) => panic!("{class}: {message}"),
        None => panic!("{class}"),
    }
}

// =============================================================================
// Options
// =============================================================================

/// `java.nio.file.StandardOpenOption`, plus `LinkOption.NOFOLLOW_LINKS`,
/// the two `OpenOption` implementations of the JDK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpenOption {
    /// `READ`.
    Read,
    /// `WRITE`.
    Write,
    /// `APPEND`.
    Append,
    /// `TRUNCATE_EXISTING`.
    TruncateExisting,
    /// `CREATE`.
    Create,
    /// `CREATE_NEW`.
    CreateNew,
    /// `DELETE_ON_CLOSE`.
    DeleteOnClose,
    /// `SPARSE`.
    Sparse,
    /// `SYNC`.
    Sync,
    /// `DSYNC`.
    Dsync,
    /// `LinkOption.NOFOLLOW_LINKS`.
    NoFollowLinks,
}

impl fmt::Display for OpenOption {
    /// `Enum.toString()`: the constant's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OpenOption::Read => "READ",
            OpenOption::Write => "WRITE",
            OpenOption::Append => "APPEND",
            OpenOption::TruncateExisting => "TRUNCATE_EXISTING",
            OpenOption::Create => "CREATE",
            OpenOption::CreateNew => "CREATE_NEW",
            OpenOption::DeleteOnClose => "DELETE_ON_CLOSE",
            OpenOption::Sparse => "SPARSE",
            OpenOption::Sync => "SYNC",
            OpenOption::Dsync => "DSYNC",
            OpenOption::NoFollowLinks => "NOFOLLOW_LINKS",
        })
    }
}

/// `java.nio.file.StandardCopyOption`, plus `LinkOption.NOFOLLOW_LINKS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CopyOption {
    /// `REPLACE_EXISTING`.
    ReplaceExisting,
    /// `COPY_ATTRIBUTES`.
    CopyAttributes,
    /// `ATOMIC_MOVE`.
    AtomicMove,
    /// `LinkOption.NOFOLLOW_LINKS`.
    NoFollowLinks,
}

impl fmt::Display for CopyOption {
    /// `Enum.toString()`: the constant's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CopyOption::ReplaceExisting => "REPLACE_EXISTING",
            CopyOption::CopyAttributes => "COPY_ATTRIBUTES",
            CopyOption::AtomicMove => "ATOMIC_MOVE",
            CopyOption::NoFollowLinks => "NOFOLLOW_LINKS",
        })
    }
}

/// `java.nio.file.LinkOption`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkOption {
    /// `NOFOLLOW_LINKS`.
    NoFollowLinks,
}

impl fmt::Display for LinkOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NOFOLLOW_LINKS")
    }
}

/// `java.nio.file.AccessMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessMode {
    /// `READ`.
    Read,
    /// `WRITE`.
    Write,
    /// `EXECUTE`.
    Execute,
}

/// `Arrays.toString(Object[])` / `AbstractCollection.toString()` of options:
/// `[A, B]`.
pub fn options_to_string<T: fmt::Display>(options: &[T]) -> String {
    let parts: Vec<String> = options.iter().map(ToString::to_string).collect();
    format!("[{}]", parts.join(", "))
}

/// Whether `options` follow links: `LinkOption.NOFOLLOW_LINKS` is absent.
pub fn follow_links(options: &[LinkOption]) -> bool {
    !options.contains(&LinkOption::NoFollowLinks)
}

// =============================================================================
// Attributes
// =============================================================================

/// `sun.nio.fs.UnixFileKey`: the device and inode of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileKey {
    /// `st_dev`.
    pub dev: u64,
    /// `st_ino`.
    pub ino: u64,
}

impl fmt::Display for FileKey {
    /// `UnixFileKey.toString()`: `(dev=<hex dev>,ino=<ino>)`, measured on
    /// OpenJDK 25 (`(dev=32,ino=…)` for device 50).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(dev={:x},ino={})", self.dev, self.ino)
    }
}

/// `java.nio.file.attribute.BasicFileAttributes`, read at one instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicFileAttributes {
    /// `lastModifiedTime()`.
    pub last_modified_time: SystemTime,
    /// `lastAccessTime()`.
    pub last_access_time: SystemTime,
    /// `creationTime()`: the birth time when the platform reports one, and the
    /// last-modified time otherwise, as `UnixFileAttributes.creationTime()`.
    pub creation_time: SystemTime,
    /// `isRegularFile()`.
    pub is_regular_file: bool,
    /// `isDirectory()`.
    pub is_directory: bool,
    /// `isSymbolicLink()`.
    pub is_symbolic_link: bool,
    /// `isOther()`.
    pub is_other: bool,
    /// `size()`.
    pub size: i64,
    /// `fileKey()`: `None` where the platform has none.
    pub file_key: Option<FileKey>,
}

impl BasicFileAttributes {
    /// The attributes of `metadata`.
    pub fn from_metadata(metadata: &std::fs::Metadata) -> BasicFileAttributes {
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let accessed = metadata.accessed().unwrap_or(modified);
        let created = metadata.created().unwrap_or(modified);
        let file_type = metadata.file_type();
        #[cfg(unix)]
        let file_key = {
            use std::os::unix::fs::MetadataExt;
            Some(FileKey {
                dev: metadata.dev(),
                ino: metadata.ino(),
            })
        };
        #[cfg(not(unix))]
        let file_key = None;
        BasicFileAttributes {
            last_modified_time: modified,
            last_access_time: accessed,
            creation_time: created,
            is_regular_file: file_type.is_file(),
            is_directory: file_type.is_dir(),
            is_symbolic_link: file_type.is_symlink(),
            is_other: !file_type.is_file() && !file_type.is_dir() && !file_type.is_symlink(),
            size: metadata.len() as i64,
            file_key,
        }
    }
}

/// The value of a file or file-store attribute, the `Object` of
/// `Files.getAttribute` / `setAttribute` and `FileStore.getAttribute`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeValue {
    /// A `java.nio.file.attribute.FileTime`.
    FileTime(SystemTime),
    /// A `Boolean`.
    Boolean(bool),
    /// A `Long`.
    Long(i64),
    /// A `String`.
    String(String),
    /// A file key, `null` when absent.
    FileKey(Option<FileKey>),
}

impl fmt::Display for AttributeValue {
    /// `String.valueOf(value)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttributeValue::FileTime(time) => f.write_str(&file_time_to_string(*time)),
            AttributeValue::Boolean(b) => write!(f, "{b}"),
            AttributeValue::Long(l) => write!(f, "{l}"),
            AttributeValue::String(s) => f.write_str(s),
            AttributeValue::FileKey(Some(key)) => write!(f, "{key}"),
            AttributeValue::FileKey(None) => f.write_str("null"),
        }
    }
}

/// Renders a point in time as `java.nio.file.attribute.FileTime.toString()`
/// does.
///
/// The ISO-8601 form in UTC, `yyyy-MM-ddTHH:mm:ss[.fraction]Z`: the year has
/// at least four digits, a year before 1 is written as Java writes it — the
/// proleptic year minus one, signed, so year 0 is `-0001` — and the fraction
/// is the nanoseconds with their trailing zeros removed, omitted when they are
/// zero. The calendar fields come from the same split into 10 000-year cycles
/// `FileTime.toString()` performs before handing a value to
/// `LocalDateTime.ofEpochSecond`, so every value renders as on the JDK.
/// Measured against OpenJDK 25: `1970-01-01T00:00:00Z`,
/// `1970-01-01T00:00:01.12Z`, `-0002-12-31T23:59:59Z` for one second before
/// year 0, and `10000-01-01T00:00:00.000000005Z`.
pub fn file_time_to_string(time: SystemTime) -> String {
    use chrono::{Datelike, Timelike};

    /// `FileTime.SECONDS_PER_10000_YEARS`.
    const SECONDS_PER_10000_YEARS: i64 = 146_097 * 25 * 86_400;
    /// `FileTime.SECONDS_0000_TO_1970`.
    const SECONDS_0000_TO_1970: i64 = ((146_097 * 5) - (30 * 365 + 7)) * 86_400;

    // `toInstant().getEpochSecond()` and `getNano()`: the seconds are floored,
    // and the nanoseconds are never negative.
    let (secs, nanos) = match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => (after.as_secs() as i64, after.subsec_nanos()),
        Err(before) => {
            let before = before.duration();
            let secs = -(before.as_secs() as i64);
            match before.subsec_nanos() {
                0 => (secs, 0),
                nanos => (secs - 1, 1_000_000_000 - nanos),
            }
        }
    };
    let (hi, lo) = if secs >= -SECONDS_0000_TO_1970 {
        // Current era.
        let zero_secs = secs - SECONDS_PER_10000_YEARS + SECONDS_0000_TO_1970;
        (
            zero_secs.div_euclid(SECONDS_PER_10000_YEARS) + 1,
            zero_secs.rem_euclid(SECONDS_PER_10000_YEARS),
        )
    } else {
        // Before the current era.
        let zero_secs = secs + SECONDS_0000_TO_1970;
        (
            zero_secs / SECONDS_PER_10000_YEARS,
            zero_secs % SECONDS_PER_10000_YEARS,
        )
    };
    let ldt = chrono::DateTime::from_timestamp(lo - SECONDS_0000_TO_1970, nanos)
        .expect("INVARIANT: a second within one 10 000-year cycle of year 0 is representable")
        .naive_utc();
    let mut year = i64::from(ldt.year()) + hi * 10_000;
    if year <= 0 {
        year -= 1;
    }
    let mut out = String::with_capacity(32);
    if year < 0 {
        out.push('-');
    }
    let year = year.unsigned_abs();
    if year < 10_000 {
        out.push_str(&format!("{year:04}"));
    } else {
        out.push_str(&year.to_string());
    }
    out.push_str(&format!(
        "-{:02}-{:02}T{:02}:{:02}:{:02}",
        ldt.month(),
        ldt.day(),
        ldt.hour(),
        ldt.minute(),
        ldt.second()
    ));
    if nanos != 0 {
        let fraction = format!("{nanos:09}");
        out.push('.');
        out.push_str(fraction.trim_end_matches('0'));
    }
    out.push('Z');
    out
}

// =============================================================================
// FileSystem and FileSystemProvider
// =============================================================================

/// `java.nio.file.FileSystem`.
pub trait FileSystem: Send + Sync + Any {
    /// `provider()`.
    fn provider(&self) -> Arc<dyn FileSystemProvider>;

    /// `close()`.
    ///
    /// # Errors
    ///
    /// The `IOException` closing raises.
    ///
    /// # Panics
    ///
    /// `UnsupportedOperationException` for the default filesystem, which
    /// cannot be closed.
    fn close(&self) -> io::Result<()>;

    /// `isOpen()`.
    fn is_open(&self) -> bool;

    /// `isReadOnly()`.
    fn is_read_only(&self) -> bool;

    /// `getSeparator()`.
    fn get_separator(&self) -> String;

    /// `getRootDirectories()`.
    fn get_root_directories(self: Arc<Self>) -> Vec<Path>;

    /// `getFileStores()`.
    fn get_file_stores(&self) -> Vec<Arc<dyn FileStore>>;

    /// `supportedFileAttributeViews()`.
    fn supported_file_attribute_views(&self) -> Vec<String>;

    /// `getPath(first, more...)`.
    ///
    /// # Panics
    ///
    /// `InvalidPathException` when the joined string cannot be a path.
    fn get_path(self: Arc<Self>, first: &str, more: &[&str]) -> Path;

    /// `getPathMatcher(syntaxAndPattern)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when the argument has no syntax,
    /// `UnsupportedOperationException` for an unknown syntax, and
    /// `PatternSyntaxException` for an invalid pattern.
    fn get_path_matcher(&self, syntax_and_pattern: &str) -> Box<dyn PathMatcher>;

    /// `Unwrappable.unwrap()` of a path of this filesystem: the path it wraps,
    /// `None` when this filesystem wraps none.
    ///
    /// Lucene unwraps its test-only filesystem layers in
    /// `MemorySegmentIndexInputProvider.openInput` (`Unwrappable.unwrapAll(path)`)
    /// before mapping a file; [`Path::unwrap_all`] repeats this until `None`.
    fn unwrap_path(&self, path: &Path) -> Option<Path> {
        let _ = path;
        None
    }

    /// `toString()`.
    fn file_system_to_string(&self) -> String;

    /// `self` as [`Any`], for `instanceof` tests.
    fn as_any(&self) -> &dyn Any;
}

/// The filter `newDirectoryStream` applies to each entry,
/// `DirectoryStream.Filter<? super Path>`.
pub type DirectoryStreamFilter = Arc<dyn Fn(&Path) -> io::Result<bool> + Send + Sync>;

/// `DirectoryStream.Filter` that accepts every entry, `Files.AcceptAllFilter`.
pub fn accept_all_filter() -> DirectoryStreamFilter {
    Arc::new(|_| Ok(true))
}

/// `java.nio.file.spi.FileSystemProvider`.
///
/// The JDK 20 default methods `exists` and `readAttributesIfExists` keep their
/// default bodies here, written over [`check_access`](Self::check_access) and
/// [`read_attributes`](Self::read_attributes) as the JDK's are, so a provider
/// that does not override them sees the calls the JDK's default makes.
pub trait FileSystemProvider: Send + Sync + Any {
    /// `getScheme()`.
    fn get_scheme(&self) -> String;

    /// `getFileSystem(URI)`: the provider's filesystem.
    fn get_file_system(&self) -> Arc<dyn FileSystem>;

    /// `getPath(URI)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when `uri` is not a URI this provider
    /// accepts.
    fn get_path(&self, uri: &str) -> Path;

    /// `Path.resolve(Path)` of a path of this provider's filesystem.
    ///
    /// Java resolves through the path object; this port resolves through the
    /// path's provider, so that a provider whose paths override `resolve`
    /// (`WindowsFS`'s `WindowsPath`) reaches every resolution.
    ///
    /// # Panics
    ///
    /// `ProviderMismatchException` when `other` is not a path of `base`'s
    /// filesystem, or the unchecked exception an overriding provider throws.
    fn resolve(&self, base: &Path, other: &Path) -> Path;

    /// `createDirectory(dir, attrs...)` (no attributes).
    ///
    /// # Errors
    ///
    /// `FileAlreadyExistsException` and the other failures of the filesystem.
    fn create_directory(&self, dir: &Path) -> io::Result<()>;

    /// `delete(path)`.
    ///
    /// # Errors
    ///
    /// `NoSuchFileException`, `DirectoryNotEmptyException` and the other
    /// failures of the filesystem.
    fn delete(&self, path: &Path) -> io::Result<()>;

    /// `deleteIfExists(path)`.
    ///
    /// # Errors
    ///
    /// As [`delete`](Self::delete), but a missing file answers `false`.
    fn delete_if_exists(&self, path: &Path) -> io::Result<bool>;

    /// `copy(source, target, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn copy(&self, source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<()>;

    /// `move(source, target, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn r#move(&self, source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<()>;

    /// `isSameFile(path, path2)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn is_same_file(&self, path: &Path, path2: &Path) -> io::Result<bool>;

    /// `isHidden(path)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn is_hidden(&self, path: &Path) -> io::Result<bool>;

    /// `getFileStore(path)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn get_file_store(&self, path: &Path) -> io::Result<Arc<dyn FileStore>>;

    /// `checkAccess(path, modes...)`.
    ///
    /// # Errors
    ///
    /// `NoSuchFileException`, `AccessDeniedException` and the other failures
    /// of the filesystem.
    fn check_access(&self, path: &Path, modes: &[AccessMode]) -> io::Result<()>;

    /// `readAttributes(path, BasicFileAttributes.class, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn read_attributes(
        &self,
        path: &Path,
        options: &[LinkOption],
    ) -> io::Result<BasicFileAttributes>;

    /// `readAttributes(path, String attributes, options...)`: the
    /// `[view:]name,name…` form, as `(name, value)` pairs.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    ///
    /// # Panics
    ///
    /// `UnsupportedOperationException` for a view the provider has not,
    /// `IllegalArgumentException` for an attribute the view has not.
    fn read_attributes_by_name(
        &self,
        path: &Path,
        attributes: &str,
        options: &[LinkOption],
    ) -> io::Result<Vec<(String, AttributeValue)>>;

    /// `setAttribute(path, attribute, value, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    ///
    /// # Panics
    ///
    /// As [`read_attributes_by_name`](Self::read_attributes_by_name).
    fn set_attribute(
        &self,
        path: &Path,
        attribute: &str,
        value: &AttributeValue,
        options: &[LinkOption],
    ) -> io::Result<()>;

    /// `newInputStream(path, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn new_input_stream(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn InputStream>>;

    /// `newOutputStream(path, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn new_output_stream(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn OutputStream>>;

    /// `newFileChannel(path, options, attrs...)` (no attributes). `options` is
    /// the option set, in the order the caller listed it.
    ///
    /// **Divergence.** Java passes a `Set<OpenOption>`, whose iteration order
    /// (visible in `VerboseFS`'s log) is a `HashSet`'s, which changes between
    /// JVM runs, or an `EnumSet`'s declaration order; this port keeps the
    /// caller's order.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn new_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn FileChannel>>;

    /// `newAsynchronousFileChannel(path, options, executor, attrs...)` (default
    /// executor, no attributes).
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn new_asynchronous_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn AsynchronousFileChannel>>;

    /// `newByteChannel(path, options, attrs...)` (no attributes).
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn new_byte_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn SeekableByteChannel>>;

    /// `newDirectoryStream(dir, filter)`.
    ///
    /// # Errors
    ///
    /// `NotDirectoryException` and the other failures of the filesystem.
    fn new_directory_stream(
        &self,
        dir: &Path,
        filter: DirectoryStreamFilter,
    ) -> io::Result<Box<dyn DirectoryStream>>;

    /// `createSymbolicLink(link, target, attrs...)` (no attributes).
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn create_symbolic_link(&self, link: &Path, target: &Path) -> io::Result<()>;

    /// `createLink(link, existing)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn create_link(&self, link: &Path, existing: &Path) -> io::Result<()>;

    /// `readSymbolicLink(link)`.
    ///
    /// # Errors
    ///
    /// The failures of the filesystem.
    fn read_symbolic_link(&self, link: &Path) -> io::Result<Path>;

    /// `exists(path, options...)`, JDK 20's default: `checkAccess` when
    /// following links, `readAttributes(NOFOLLOW_LINKS)` otherwise, and `false`
    /// on any `IOException`.
    fn exists(&self, path: &Path, options: &[LinkOption]) -> bool {
        let result = if follow_links(options) {
            self.check_access(path, &[])
        } else {
            self.read_attributes(path, &[LinkOption::NoFollowLinks])
                .map(|_| ())
        };
        result.is_ok()
    }

    /// `readAttributesIfExists(path, BasicFileAttributes.class, options...)`,
    /// JDK 20's default: `readAttributes`, `None` on `NoSuchFileException`.
    ///
    /// # Errors
    ///
    /// Any other failure of `readAttributes`.
    fn read_attributes_if_exists(
        &self,
        path: &Path,
        options: &[LinkOption],
    ) -> io::Result<Option<BasicFileAttributes>> {
        match self.read_attributes(path, options) {
            Ok(attributes) => Ok(Some(attributes)),
            Err(error) if java_class_of(&error) == NO_SUCH_FILE_EXCEPTION => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// `getClass().getName()`.
    fn provider_class_name(&self) -> String;

    /// `toString()`.
    fn provider_to_string(&self) -> String;

    /// `self` as [`Any`], for `instanceof` tests.
    fn as_any(&self) -> &dyn Any;
}

// =============================================================================
// Path
// =============================================================================

/// `java.nio.file.Path`: a path string bound to the [`FileSystem`] it belongs
/// to.
///
/// A path of the default filesystem reproduces `sun.nio.fs.UnixPath` on Unix:
/// the string is kept with redundant and trailing slashes removed, a NUL
/// character is refused, names are the `/`-separated elements, `hashCode()` is
/// computed over the bytes and paths compare by their bytes.
///
/// **Divergence.** A `FilterPath` of the test framework wraps the delegate
/// path object; this port keeps one path string and the filesystem it is bound
/// to, and unwrapping a filter path rebinds the same string to the delegate
/// filesystem ([`FileSystem::unwrap_path`]). Every `FilterPath` operation of
/// Lucene 10.5.0 delegates to the wrapped path and wraps the result, so the
/// strings, comparisons and hashes are the same; `resolve`, the one operation
/// a filter path overrides (`WindowsPath`), dispatches through
/// [`FileSystemProvider::resolve`].
#[derive(Clone)]
pub struct Path {
    fs: Arc<dyn FileSystem>,
    path: PathBuf,
}

impl Path {
    /// The path `path` of `fs`, normalized as `UnixPath`'s constructor
    /// normalizes it.
    ///
    /// # Panics
    ///
    /// `InvalidPathException` ("Nul character not allowed") when `path`
    /// contains a NUL character.
    pub fn new(fs: Arc<dyn FileSystem>, path: impl AsRef<OsStr>) -> Path {
        Path {
            fs,
            path: normalize_and_check(path.as_ref()),
        }
    }

    /// The same string bound to `fs`: how a filter filesystem wraps a delegate
    /// path (`wrapPath`) and unwraps its own (`toDelegate`).
    pub fn with_file_system(&self, fs: Arc<dyn FileSystem>) -> Path {
        Path {
            fs,
            path: self.path.clone(),
        }
    }

    /// `getFileSystem()`.
    pub fn get_file_system(&self) -> &Arc<dyn FileSystem> {
        &self.fs
    }

    /// Whether `self` and `other` belong to the same filesystem instance.
    pub fn same_file_system(&self, other: &Path) -> bool {
        same_file_system(&self.fs, &other.fs)
    }

    /// The path string as a standard-library path: the file this path names
    /// on the underlying operating system, `Path.toFile()`.
    pub fn as_std_path(&self) -> &std::path::Path {
        &self.path
    }

    /// `toFile()`: the file this path names, as a standard-library path.
    pub fn to_file(&self) -> PathBuf {
        self.path.clone()
    }

    /// `Unwrappable.unwrapAll(path)`: the innermost path under every filter
    /// filesystem this path is wrapped in.
    pub fn unwrap_all(&self) -> Path {
        let mut path = self.clone();
        while let Some(inner) = path.fs.unwrap_path(&path) {
            path = inner;
        }
        path
    }

    fn bytes(&self) -> Cow<'_, [u8]> {
        os_bytes(self.path.as_os_str())
    }

    fn derive(&self, bytes: &[u8]) -> Path {
        Path {
            fs: Arc::clone(&self.fs),
            path: bytes_to_path(bytes),
        }
    }

    /// `isAbsolute()`.
    pub fn is_absolute(&self) -> bool {
        self.path.is_absolute()
    }

    /// `getRoot()`: `/` for an absolute path, `None` otherwise.
    pub fn get_root(&self) -> Option<Path> {
        if self.is_absolute() {
            Some(self.derive(SEPARATOR_BYTES))
        } else {
            None
        }
    }

    /// The offsets of the names, `UnixPath.initOffsets`.
    fn offsets(&self) -> Vec<(usize, usize)> {
        name_offsets(&self.bytes())
    }

    /// `getFileName()`: the last name, the empty path for the empty path, and
    /// `None` for the root.
    pub fn get_file_name(&self) -> Option<Path> {
        let bytes = self.bytes();
        if bytes.is_empty() {
            return Some(self.derive(b""));
        }
        let offsets = name_offsets(&bytes);
        let (start, end) = *offsets.last()?;
        if start == 0 && end == bytes.len() {
            return Some(self.clone());
        }
        Some(self.derive(&bytes[start..end]))
    }

    /// `getParent()`.
    pub fn get_parent(&self) -> Option<Path> {
        let bytes = self.bytes();
        let offsets = name_offsets(&bytes);
        let count = offsets.len();
        if count == 0 {
            return None;
        }
        let len = offsets[count - 1].0;
        if len == 0 {
            return self.get_root();
        }
        let len = len - 1;
        if len == 0 {
            // `/a`: the parent is the root.
            return self.get_root();
        }
        Some(self.derive(&bytes[..len]))
    }

    /// `getNameCount()`.
    pub fn get_name_count(&self) -> usize {
        let bytes = self.bytes();
        if bytes.is_empty() {
            return 1;
        }
        name_offsets(&bytes).len()
    }

    /// `getName(index)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when `index` is out of range.
    pub fn get_name(&self, index: usize) -> Path {
        let bytes = self.bytes();
        if bytes.is_empty() && index == 0 {
            return self.derive(b"");
        }
        let offsets = name_offsets(&bytes);
        let Some(&(start, end)) = offsets.get(index) else {
            throw_unchecked("java.lang.IllegalArgumentException", None)
        };
        self.derive(&bytes[start..end])
    }

    /// `subpath(beginIndex, endIndex)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when the range is not within the names.
    pub fn subpath(&self, begin_index: usize, end_index: usize) -> Path {
        let bytes = self.bytes();
        let offsets = if bytes.is_empty() {
            vec![(0, 0)]
        } else {
            name_offsets(&bytes)
        };
        if begin_index >= offsets.len() || end_index > offsets.len() || begin_index >= end_index {
            throw_unchecked("java.lang.IllegalArgumentException", None)
        }
        let start = offsets[begin_index].0;
        let end = offsets[end_index - 1].1;
        self.derive(&bytes[start..end])
    }

    /// `iterator()`: the names.
    pub fn iter_names(&self) -> Vec<Path> {
        (0..self.get_name_count())
            .map(|i| self.get_name(i))
            .collect()
    }

    /// `startsWith(Path)`: whether this path starts with `other`'s names
    /// (and its root).
    pub fn starts_with(&self, other: &Path) -> bool {
        if !self.same_file_system(other) {
            return false;
        }
        if self.is_absolute() != other.is_absolute() {
            return false;
        }
        let mine = self.offsets();
        let theirs = other.offsets();
        let (a, b) = (self.bytes(), other.bytes());
        if b.is_empty() {
            return a.is_empty();
        }
        if theirs.len() > mine.len() {
            return false;
        }
        theirs
            .iter()
            .zip(mine.iter())
            .all(|(&(ts, te), &(ms, me))| b[ts..te] == a[ms..me])
    }

    /// `startsWith(String)`: `startsWith(getFileSystem().getPath(other))`.
    pub fn starts_with_str(&self, other: &str) -> bool {
        self.starts_with(&Arc::clone(&self.fs).get_path(other, &[]))
    }

    /// `endsWith(Path)`.
    pub fn ends_with(&self, other: &Path) -> bool {
        if !self.same_file_system(other) {
            return false;
        }
        let (a, b) = (self.bytes(), other.bytes());
        if other.is_absolute() {
            return a == b;
        }
        if b.is_empty() {
            return a.is_empty();
        }
        let mine = self.offsets();
        let theirs = other.offsets();
        if theirs.len() > mine.len() {
            return false;
        }
        theirs
            .iter()
            .rev()
            .zip(mine.iter().rev())
            .all(|(&(ts, te), &(ms, me))| b[ts..te] == a[ms..me])
    }

    /// `endsWith(String)`.
    pub fn ends_with_str(&self, other: &str) -> bool {
        self.ends_with(&Arc::clone(&self.fs).get_path(other, &[]))
    }

    /// `normalize()`: `.` names removed and `name/..` pairs collapsed,
    /// lexically.
    pub fn normalize(&self) -> Path {
        let bytes = self.bytes();
        let absolute = self.is_absolute();
        let mut names: Vec<&[u8]> = Vec::new();
        for (start, end) in name_offsets(&bytes) {
            let name = &bytes[start..end];
            if name == b"." {
                continue;
            }
            if name == b".." {
                match names.last() {
                    Some(last) if *last != b".." => {
                        names.pop();
                        continue;
                    }
                    None if absolute => continue,
                    _ => {}
                }
            }
            names.push(name);
        }
        let mut out: Vec<u8> = Vec::new();
        if absolute {
            out.extend_from_slice(SEPARATOR_BYTES);
        }
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                out.extend_from_slice(SEPARATOR_BYTES);
            }
            out.extend_from_slice(name);
        }
        self.derive(&out)
    }

    /// `resolve(Path)`, through the provider of this path's filesystem.
    ///
    /// # Panics
    ///
    /// `ProviderMismatchException`, or the unchecked exception an overriding
    /// provider throws (`InvalidPathException` from `WindowsFS`).
    pub fn resolve(&self, other: &Path) -> Path {
        self.fs.provider().resolve(self, other)
    }

    /// `resolve(String)`: `resolve(getFileSystem().getPath(other))`.
    ///
    /// # Panics
    ///
    /// As [`resolve`](Self::resolve).
    pub fn resolve_str(&self, other: &str) -> Path {
        self.resolve(&Arc::clone(&self.fs).get_path(other, &[]))
    }

    /// The default `resolve`: `other` when it is absolute, this path when
    /// `other` is empty, and the two joined otherwise. A provider's
    /// [`FileSystemProvider::resolve`] calls this for its own paths.
    ///
    /// # Panics
    ///
    /// `ProviderMismatchException` when `other` belongs to another filesystem.
    pub fn default_resolve(&self, other: &Path) -> Path {
        if !self.same_file_system(other) {
            throw_unchecked("java.nio.file.ProviderMismatchException", None)
        }
        let (a, b) = (self.bytes(), other.bytes());
        if other.is_absolute() {
            return other.clone();
        }
        if b.is_empty() {
            return self.clone();
        }
        if a.is_empty() {
            return other.clone();
        }
        let mut out = a.to_vec();
        if out.as_slice() != SEPARATOR_BYTES {
            out.extend_from_slice(SEPARATOR_BYTES);
        }
        out.extend_from_slice(&b);
        self.derive(&out)
    }

    /// `resolveSibling(Path)`: `getParent().resolve(other)`, or `other` when
    /// there is no parent.
    pub fn resolve_sibling(&self, other: &Path) -> Path {
        match self.get_parent() {
            Some(parent) => parent.default_resolve(other),
            None => {
                if !self.same_file_system(other) {
                    throw_unchecked("java.nio.file.ProviderMismatchException", None)
                }
                other.clone()
            }
        }
    }

    /// `resolveSibling(String)`.
    pub fn resolve_sibling_str(&self, other: &str) -> Path {
        self.resolve_sibling(&Arc::clone(&self.fs).get_path(other, &[]))
    }

    /// `relativize(Path)`: the path from this one to `other`, both absolute or
    /// both relative.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` ("'other' is different type of Path") when
    /// one path is absolute and the other is not, and
    /// `ProviderMismatchException` for a path of another filesystem.
    pub fn relativize(&self, other: &Path) -> Path {
        if !self.same_file_system(other) {
            throw_unchecked("java.nio.file.ProviderMismatchException", None)
        }
        let (a, b) = (self.bytes(), other.bytes());
        if a == b {
            return self.derive(b"");
        }
        if self.is_absolute() != other.is_absolute() {
            throw_unchecked(
                "java.lang.IllegalArgumentException",
                Some("'other' is different type of Path"),
            )
        }
        if a.is_empty() {
            return other.clone();
        }
        let base: Vec<&[u8]> = name_offsets(&a)
            .into_iter()
            .map(|(s, e)| &a[s..e])
            .collect();
        let child: Vec<&[u8]> = name_offsets(&b)
            .into_iter()
            .map(|(s, e)| &b[s..e])
            .collect();
        let mut i = 0;
        while i < base.len() && i < child.len() && base[i] == child[i] {
            i += 1;
        }
        let mut parts: Vec<&[u8]> = vec![b".."; base.len() - i];
        parts.extend_from_slice(&child[i..]);
        let mut out: Vec<u8> = Vec::new();
        for (k, part) in parts.iter().enumerate() {
            if k > 0 {
                out.extend_from_slice(SEPARATOR_BYTES);
            }
            out.extend_from_slice(part);
        }
        self.derive(&out)
    }

    /// `toAbsolutePath()`: this path when absolute, and the current directory
    /// joined with it otherwise.
    ///
    /// # Panics
    ///
    /// `IOError` when the current directory cannot be read.
    pub fn to_absolute_path(&self) -> Path {
        if self.is_absolute() {
            return self.clone();
        }
        let cwd = std::env::current_dir()
            .unwrap_or_else(|e| throw_unchecked("java.io.IOError", Some(&e.to_string())));
        let base = self.derive(&os_bytes(cwd.as_os_str()));
        if self.bytes().is_empty() {
            return base;
        }
        base.default_resolve(self)
    }

    /// `toRealPath(options...)`: the absolute path with symbolic links
    /// resolved (unless `NOFOLLOW_LINKS`), `.` and `..` removed.
    ///
    /// # Errors
    ///
    /// `NoSuchFileException` (the path as given) when the file does not exist,
    /// and the other failures of the filesystem.
    pub fn to_real_path(&self, options: &[LinkOption]) -> io::Result<Path> {
        default_file_system::to_real_path(self, options)
    }

    /// `toUri()`: a `file:` URI of the absolute path, every byte outside the
    /// characters a URI path allows percent-encoded, and a trailing slash for
    /// an existing directory.
    pub fn to_uri(&self) -> String {
        default_file_system::to_uri(self)
    }

    /// `hashCode()`: `UnixPath.hashCode()`, `31 * h + (b & 0xff)` over the
    /// bytes of the path.
    pub fn hash_code(&self) -> i32 {
        let mut h: i32 = 0;
        for &b in self.bytes().iter() {
            h = h.wrapping_mul(31).wrapping_add(i32::from(b));
        }
        h
    }

    /// `compareTo(Path)`: the difference of the first differing bytes, or of
    /// the lengths.
    pub fn compare_to(&self, other: &Path) -> i32 {
        let (a, b) = (self.bytes(), other.bytes());
        for (x, y) in a.iter().zip(b.iter()) {
            if x != y {
                return i32::from(*x) - i32::from(*y);
            }
        }
        a.len() as i32 - b.len() as i32
    }

    /// Whether the path string is empty.
    pub fn is_empty(&self) -> bool {
        self.path.as_os_str().is_empty()
    }
}

impl fmt::Display for Path {
    /// `toString()`: the path string, decoded as UTF-8 with malformed input
    /// replaced (`sun.jnu.encoding`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path.to_string_lossy())
    }
}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Path({:?})", self.path)
    }
}

impl PartialEq for Path {
    /// `equals(Object)`: the same filesystem and the same bytes.
    fn eq(&self, other: &Path) -> bool {
        self.same_file_system(other) && self.bytes() == other.bytes()
    }
}

impl Eq for Path {}

impl Hash for Path {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl PartialOrd for Path {
    fn partial_cmp(&self, other: &Path) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Path {
    fn cmp(&self, other: &Path) -> Ordering {
        self.compare_to(other).cmp(&0)
    }
}

/// Whether two filesystem handles are the same instance.
pub fn same_file_system(a: &Arc<dyn FileSystem>, b: &Arc<dyn FileSystem>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// Conversion into a [`Path`]: a [`Path`] as is, and anything the standard
/// library takes as a path, as a path of the default filesystem.
pub trait IntoNioPath {
    /// The path.
    fn into_nio_path(self) -> Path;
}

impl<T: AsRef<std::path::Path>> IntoNioPath for T {
    fn into_nio_path(self) -> Path {
        Path::new(default_file_system(), self.as_ref().as_os_str())
    }
}

impl IntoNioPath for Path {
    fn into_nio_path(self) -> Path {
        self
    }
}

impl IntoNioPath for &Path {
    fn into_nio_path(self) -> Path {
        self.clone()
    }
}

#[cfg(unix)]
const SEPARATOR_BYTES: &[u8] = b"/";
#[cfg(not(unix))]
const SEPARATOR_BYTES: &[u8] = b"\\";

#[cfg(unix)]
fn os_bytes(s: &OsStr) -> Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    Cow::Borrowed(s.as_bytes())
}

#[cfg(not(unix))]
fn os_bytes(s: &OsStr) -> Cow<'_, [u8]> {
    match s.to_string_lossy() {
        Cow::Borrowed(b) => Cow::Borrowed(b.as_bytes()),
        Cow::Owned(o) => Cow::Owned(o.into_bytes()),
    }
}

#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

fn is_separator(b: u8) -> bool {
    b == b'/' || (cfg!(not(unix)) && b == b'\\')
}

/// `UnixPath.normalizeAndCheck`: redundant and trailing separators removed,
/// a NUL refused.
fn normalize_and_check(path: &OsStr) -> PathBuf {
    let bytes = os_bytes(path);
    if bytes.contains(&0) {
        throw_unchecked(
            "java.nio.file.InvalidPathException",
            Some(&format!(
                "Nul character not allowed: {}",
                String::from_utf8_lossy(&bytes)
            )),
        )
    }
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut prev_sep = false;
    for &b in bytes.iter() {
        let sep = is_separator(b);
        if sep && prev_sep {
            continue;
        }
        out.push(if sep { SEPARATOR_BYTES[0] } else { b });
        prev_sep = sep;
    }
    if out.len() > 1 && is_separator(*out.last().expect("non-empty")) {
        out.pop();
    }
    let _ = OsString::new();
    bytes_to_path(&out)
}

/// The `(start, end)` byte offsets of the names of a normalized path.
fn name_offsets(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut offsets = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if is_separator(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && !is_separator(bytes[i]) {
            i += 1;
        }
        offsets.push((start, i));
    }
    offsets
}

// =============================================================================
// Streams and channels
// =============================================================================

/// `java.io.InputStream`.
pub trait InputStream: Send + Sync {
    /// `read()`: the next byte, `-1` at the end of the stream.
    ///
    /// # Errors
    ///
    /// The failure of the read.
    fn read_byte(&mut self) -> io::Result<i32>;

    /// `read(b, off, len)`: the number of bytes read into `b[off..off + len]`,
    /// `-1` at the end of the stream.
    ///
    /// # Errors
    ///
    /// The failure of the read.
    fn read(&mut self, b: &mut [u8], off: usize, len: usize) -> io::Result<i32>;

    /// `skip(n)`.
    ///
    /// # Errors
    ///
    /// The failure of the skip.
    fn skip(&mut self, n: i64) -> io::Result<i64>;

    /// `available()`.
    ///
    /// # Errors
    ///
    /// The failure of the query.
    fn available(&mut self) -> io::Result<i32>;

    /// `close()`.
    ///
    /// # Errors
    ///
    /// The failure of the close.
    fn close(&mut self) -> io::Result<()>;

    /// `mark(readlimit)`.
    fn mark(&mut self, readlimit: i32);

    /// `reset()`.
    ///
    /// # Errors
    ///
    /// `IOException` ("mark/reset not supported") when marks are not
    /// supported.
    fn reset(&mut self) -> io::Result<()>;

    /// `markSupported()`.
    fn mark_supported(&self) -> bool;

    /// `toString()`.
    fn stream_to_string(&self) -> String;
}

/// `java.io.OutputStream`.
pub trait OutputStream: Send + Sync {
    /// `write(int)`: the low eight bits of `b`.
    ///
    /// # Errors
    ///
    /// The failure of the write.
    fn write_byte(&mut self, b: i32) -> io::Result<()>;

    /// `write(b, off, len)`.
    ///
    /// # Errors
    ///
    /// The failure of the write.
    fn write(&mut self, b: &[u8], off: usize, len: usize) -> io::Result<()>;

    /// `flush()`.
    ///
    /// # Errors
    ///
    /// The failure of the flush.
    fn flush(&mut self) -> io::Result<()>;

    /// `close()`.
    ///
    /// # Errors
    ///
    /// The failure of the close.
    fn close(&mut self) -> io::Result<()>;

    /// `toString()`.
    fn stream_to_string(&self) -> String;
}

/// An [`OutputStream`] as a [`std::io::Write`]: `write` writes the whole
/// buffer with one `write(b, 0, len)`, as a Java caller hands an array.
pub struct OutputStreamWriter(pub Box<dyn OutputStream>);

impl io::Write for OutputStreamWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf, 0, buf.len())?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write(buf, 0, buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// `java.nio.channels.SeekableByteChannel`.
///
/// Channels are shared between threads, as the JDK's are: every method takes
/// `&self`.
pub trait SeekableByteChannel: Send + Sync {
    /// `read(ByteBuffer)`: bytes read into `dst` at the position, which
    /// advances; `-1` at the end of the file.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed, and the failure of the read.
    fn read(&self, dst: &mut [u8]) -> io::Result<i32>;

    /// `write(ByteBuffer)`: bytes of `src` written at the position, which
    /// advances.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed, and the failure of the write.
    fn write(&self, src: &[u8]) -> io::Result<i32>;

    /// `position()`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed.
    fn position(&self) -> io::Result<i64>;

    /// `position(long)`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed.
    fn set_position(&self, new_position: i64) -> io::Result<()>;

    /// `size()`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed, and the failure of the query.
    fn size(&self) -> io::Result<i64>;

    /// `truncate(long)`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed, and the failure of the truncate.
    fn truncate(&self, size: i64) -> io::Result<()>;

    /// `isOpen()`.
    fn is_open(&self) -> bool;

    /// `close()`: closes the channel once; a further close does nothing.
    ///
    /// # Errors
    ///
    /// The failure of the close.
    fn close(&self) -> io::Result<()>;

    /// `toString()`.
    fn channel_to_string(&self) -> String;
}

/// `java.nio.channels.FileChannel`.
pub trait FileChannel: SeekableByteChannel {
    /// `read(ByteBuffer[] dsts, offset, length)`: a scattering read into
    /// `dsts`.
    ///
    /// # Errors
    ///
    /// As [`SeekableByteChannel::read`].
    fn read_scattering(&self, dsts: &mut [&mut [u8]]) -> io::Result<i64>;

    /// `write(ByteBuffer[] srcs, offset, length)`: a gathering write of
    /// `srcs`.
    ///
    /// # Errors
    ///
    /// As [`SeekableByteChannel::write`].
    fn write_gathering(&self, srcs: &[&[u8]]) -> io::Result<i64>;

    /// `force(metaData)`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed, and the failure of the sync.
    fn force(&self, meta_data: bool) -> io::Result<()>;

    /// `transferTo(position, count, target)`.
    ///
    /// # Errors
    ///
    /// The failures of either channel.
    fn transfer_to(
        &self,
        position: i64,
        count: i64,
        target: &dyn SeekableByteChannel,
    ) -> io::Result<i64>;

    /// `transferFrom(src, position, count)`.
    ///
    /// # Errors
    ///
    /// The failures of either channel.
    fn transfer_from(
        &self,
        src: &dyn SeekableByteChannel,
        position: i64,
        count: i64,
    ) -> io::Result<i64>;

    /// `read(ByteBuffer, long position)`: bytes read into `dst` at `position`,
    /// without moving the channel's position; `-1` at the end of the file.
    ///
    /// # Errors
    ///
    /// As [`SeekableByteChannel::read`].
    fn read_at(&self, dst: &mut [u8], position: i64) -> io::Result<i32>;

    /// `write(ByteBuffer, long position)`.
    ///
    /// # Errors
    ///
    /// As [`SeekableByteChannel::write`].
    fn write_at(&self, src: &[u8], position: i64) -> io::Result<i32>;

    /// `map(MapMode.READ_ONLY, position, size, arena)`: a read-only mapping of
    /// `size` bytes from `position`.
    ///
    /// # Errors
    ///
    /// The failure of the mapping.
    ///
    /// # Panics
    ///
    /// `UnsupportedOperationException`, the default of
    /// `FileChannel.map(MapMode, long, long, Arena)`, which a filter channel
    /// does not override.
    #[cfg(feature = "mmap")]
    fn map_read_only(&self, position: i64, size: i64) -> io::Result<memmap2::Mmap> {
        let _ = (position, size);
        throw_unchecked("java.lang.UnsupportedOperationException", None)
    }

    /// `lock(position, size, shared)`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed, and the failure of the lock.
    fn lock(&self, position: i64, size: i64, shared: bool) -> io::Result<Box<dyn FileLock>>;

    /// `tryLock(position, size, shared)`: `None` when another program holds an
    /// overlapping lock.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed, and the failure of the lock.
    fn try_lock(
        &self,
        position: i64,
        size: i64,
        shared: bool,
    ) -> io::Result<Option<Box<dyn FileLock>>>;
}

impl dyn FileChannel {
    /// `FileChannel.open(path, options...)`: the provider of `path`'s
    /// filesystem opens it, `newFileChannel(path, options)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn open(path: &Path, options: &[OpenOption]) -> io::Result<Arc<dyn FileChannel>> {
        path.get_file_system()
            .provider()
            .new_file_channel(path, options)
    }

    /// `tryLock()`: `tryLock(0, Long.MAX_VALUE, false)`.
    ///
    /// # Errors
    ///
    /// As [`FileChannel::try_lock`].
    pub fn try_lock_whole(&self) -> io::Result<Option<Box<dyn FileLock>>> {
        self.try_lock(0, i64::MAX, false)
    }
}

/// `java.nio.channels.FileLock`.
pub trait FileLock: Send + Sync {
    /// `position()`.
    fn position(&self) -> i64;

    /// `size()`.
    fn size(&self) -> i64;

    /// `isShared()`.
    fn is_shared(&self) -> bool;

    /// `overlaps(position, size)`.
    fn overlaps(&self, position: i64, size: i64) -> bool {
        if position.saturating_add(size) <= self.position() {
            return false;
        }
        if self.position().saturating_add(self.size()) <= position {
            return false;
        }
        true
    }

    /// `isValid()`: not released and its channel still open.
    fn is_valid(&self) -> bool;

    /// `release()`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` when the channel is closed, and the failure of
    /// the release.
    fn release(&self) -> io::Result<()>;

    /// `close()`: `release()`.
    ///
    /// # Errors
    ///
    /// As [`release`](Self::release).
    fn close(&self) -> io::Result<()> {
        self.release()
    }

    /// `toString()`: the class name, then `[position:size mode validity]`.
    fn lock_to_string(&self) -> String;
}

/// A `java.util.concurrent.Future` of an asynchronous operation.
///
/// **Divergence.** The JDK completes the operations of an
/// `AsynchronousFileChannel` on a thread pool; this port completes them on the
/// calling thread, before the future is returned, which the specification
/// permits for a channel without an executor of its own. A future is therefore
/// always done.
#[derive(Debug)]
pub struct JavaFuture<T> {
    outcome: io::Result<T>,
}

impl<T> JavaFuture<T> {
    /// A future completed with `outcome`.
    pub fn completed(outcome: io::Result<T>) -> JavaFuture<T> {
        JavaFuture { outcome }
    }

    /// `isDone()`.
    pub fn is_done(&self) -> bool {
        true
    }

    /// `get()`.
    ///
    /// # Errors
    ///
    /// `ExecutionException` whose cause is the operation's exception, carried
    /// as its message (`Throwable.toString()` of the cause).
    pub fn get(self) -> io::Result<T> {
        self.outcome.map_err(|cause| {
            JavaIoException::new(EXECUTION_EXCEPTION, Some(java_to_string(&cause))).into_io_error()
        })
    }

    /// The outcome, without the `ExecutionException` wrapping of `get()`.
    pub fn into_outcome(self) -> io::Result<T> {
        self.outcome
    }
}

/// A `java.nio.channels.CompletionHandler<V, A>`: `completed(result,
/// attachment)` on success, `failed(exc, attachment)` on failure.
///
/// **Divergence.** Java passes an attachment of a type parameter; a Rust
/// closure captures it instead.
pub type CompletionHandler<V> = Box<dyn FnOnce(io::Result<V>) + Send>;

/// `java.nio.channels.AsynchronousFileChannel`.
pub trait AsynchronousFileChannel: Send + Sync {
    /// `size()`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed.
    fn size(&self) -> io::Result<i64>;

    /// `truncate(size)`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed.
    fn truncate(&self, size: i64) -> io::Result<()>;

    /// `force(metaData)`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed.
    fn force(&self, meta_data: bool) -> io::Result<()>;

    /// `lock(position, size, shared)`.
    fn lock(&self, position: i64, size: i64, shared: bool) -> JavaFuture<Box<dyn FileLock>>;

    /// `lock(position, size, shared, attachment, handler)`.
    fn lock_with_handler(
        &self,
        position: i64,
        size: i64,
        shared: bool,
        handler: CompletionHandler<Box<dyn FileLock>>,
    );

    /// `tryLock(position, size, shared)`.
    ///
    /// # Errors
    ///
    /// `ClosedChannelException` once closed.
    fn try_lock(
        &self,
        position: i64,
        size: i64,
        shared: bool,
    ) -> io::Result<Option<Box<dyn FileLock>>>;

    /// `read(dst, position)`.
    fn read(&self, dst: &mut [u8], position: i64) -> JavaFuture<i32>;

    /// `read(dst, position, attachment, handler)`.
    fn read_with_handler(&self, dst: &mut [u8], position: i64, handler: CompletionHandler<i32>);

    /// `write(src, position)`.
    fn write(&self, src: &[u8], position: i64) -> JavaFuture<i32>;

    /// `write(src, position, attachment, handler)`.
    fn write_with_handler(&self, src: &[u8], position: i64, handler: CompletionHandler<i32>);

    /// `isOpen()`.
    fn is_open(&self) -> bool;

    /// `close()`.
    ///
    /// # Errors
    ///
    /// The failure of the close.
    fn close(&self) -> io::Result<()>;

    /// `toString()`.
    fn channel_to_string(&self) -> String;
}

/// `java.nio.file.DirectoryStream<Path>`.
pub trait DirectoryStream: Send + Sync {
    /// `iterator()`: the entries; an entry that cannot be read is an `Err`,
    /// Java's `DirectoryIteratorException`.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` when the iterator was already obtained or the
    /// stream is closed.
    fn iterator(&mut self) -> Box<dyn Iterator<Item = io::Result<Path>> + Send + '_>;

    /// `close()`.
    ///
    /// # Errors
    ///
    /// The failure of the close.
    fn close(&mut self) -> io::Result<()>;

    /// `toString()`.
    fn stream_to_string(&self) -> String;
}

/// `java.nio.file.PathMatcher`.
pub trait PathMatcher: Send + Sync {
    /// `matches(path)`.
    fn matches(&self, path: &Path) -> bool;
}

/// `java.nio.file.FileStore`.
pub trait FileStore: Send + Sync {
    /// `name()`.
    fn name(&self) -> String;

    /// `type()`.
    fn type_(&self) -> String;

    /// `isReadOnly()`.
    fn is_read_only(&self) -> bool;

    /// `getTotalSpace()`.
    ///
    /// # Errors
    ///
    /// The failure of the query.
    fn get_total_space(&self) -> io::Result<i64>;

    /// `getUsableSpace()`.
    ///
    /// # Errors
    ///
    /// The failure of the query.
    fn get_usable_space(&self) -> io::Result<i64>;

    /// `getUnallocatedSpace()`.
    ///
    /// # Errors
    ///
    /// The failure of the query.
    fn get_unallocated_space(&self) -> io::Result<i64>;

    /// `getBlockSize()`.
    ///
    /// # Errors
    ///
    /// The failure of the query.
    fn get_block_size(&self) -> io::Result<i64>;

    /// `supportsFileAttributeView(String name)`.
    fn supports_file_attribute_view(&self, name: &str) -> bool;

    /// `getAttribute(attribute)`.
    ///
    /// # Errors
    ///
    /// The failure of the query.
    ///
    /// # Panics
    ///
    /// `UnsupportedOperationException` for an unknown attribute.
    fn get_attribute(&self, attribute: &str) -> io::Result<AttributeValue>;

    /// `toString()`.
    fn store_to_string(&self) -> String;
}

// =============================================================================
// Glob
// =============================================================================

const REGEX_META_CHARS: &str = ".^$+{[]|()";
const GLOB_META_CHARS: &str = "\\*?[{";

fn pattern_syntax(desc: &str, glob: &str, index: usize) -> ! {
    throw_unchecked(
        "java.util.regex.PatternSyntaxException",
        Some(&format!("{desc} near index {index}\n{glob}")),
    )
}

/// The regular expression of a glob, as `getPathMatcher("glob:…")` compiles
/// it on the Unix filesystem: `*` matches within a name, `**` across names,
/// `?` one character of a name, `[…]` a character class that never matches
/// the separator, `{a,b}` alternatives, and `\` escapes.
///
/// # Panics
///
/// `PatternSyntaxException` for an invalid glob.
pub fn glob_to_regex(glob: &str) -> String {
    let chars: Vec<char> = glob.chars().collect();
    let mut regex = String::from("^");
    let mut in_group = false;
    let mut i = 0;
    let next = |i: usize| chars.get(i).copied();
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '\\' => {
                let Some(n) = next(i) else {
                    pattern_syntax("No character to escape", glob, i - 1)
                };
                i += 1;
                if GLOB_META_CHARS.contains(n) || REGEX_META_CHARS.contains(n) {
                    regex.push('\\');
                }
                regex.push(n);
            }
            '/' => regex.push('/'),
            '[' => {
                regex.push_str("[[^/]&&");
                if next(i) == Some('^') {
                    regex.push_str("\\^");
                    i += 1;
                } else {
                    if next(i) == Some('!') {
                        regex.push('^');
                        i += 1;
                    }
                    if next(i) == Some('-') {
                        regex.push('-');
                        i += 1;
                    }
                }
                let mut has_range_start = false;
                let mut last = '\0';
                let mut closed = false;
                while i < chars.len() {
                    let c = chars[i];
                    i += 1;
                    if c == ']' {
                        closed = true;
                        break;
                    }
                    if c == '/' {
                        pattern_syntax("Explicit 'name separator' in class", glob, i - 1);
                    }
                    if c == '\\' || c == '[' || (c == '&' && next(i) == Some('&')) {
                        regex.push('\\');
                    }
                    regex.push(c);
                    if c == '-' {
                        if !has_range_start {
                            pattern_syntax("Invalid range", glob, i - 1);
                        }
                        let Some(c2) = next(i) else { break };
                        i += 1;
                        if c2 == ']' {
                            closed = true;
                            break;
                        }
                        if c2 < last {
                            pattern_syntax("Invalid range", glob, i - 3);
                        }
                        regex.push(c2);
                        has_range_start = false;
                    } else {
                        has_range_start = true;
                        last = c;
                    }
                }
                if !closed {
                    pattern_syntax("Missing ']", glob, i - 1);
                }
                regex.push_str("]]");
            }
            '{' => {
                if in_group {
                    pattern_syntax("Cannot nest groups", glob, i - 1);
                }
                regex.push_str("(?:(?:");
                in_group = true;
            }
            '}' => {
                if in_group {
                    regex.push_str("))");
                    in_group = false;
                } else {
                    regex.push('}');
                }
            }
            ',' => {
                if in_group {
                    regex.push_str(")|(?:");
                } else {
                    regex.push(',');
                }
            }
            '*' => {
                if next(i) == Some('*') {
                    regex.push_str(".*");
                    i += 1;
                } else {
                    regex.push_str("[^/]*");
                }
            }
            '?' => regex.push_str("[^/]"),
            c => {
                if REGEX_META_CHARS.contains(c) {
                    regex.push('\\');
                }
                regex.push(c);
            }
        }
    }
    if in_group {
        pattern_syntax("Missing '}", glob, i.saturating_sub(1));
    }
    regex.push('$');
    regex
}

/// A [`PathMatcher`] over a regular expression, matched against the whole
/// path string.
pub struct RegexPathMatcher {
    regex: fancy_regex::Regex,
}

impl RegexPathMatcher {
    /// The matcher of `getPathMatcher(syntaxAndPattern)` for the `glob` and
    /// `regex` syntaxes.
    ///
    /// # Panics
    ///
    /// As [`FileSystem::get_path_matcher`].
    pub fn new(syntax_and_pattern: &str) -> RegexPathMatcher {
        let Some(pos) = syntax_and_pattern.find(':').filter(|&p| p > 0) else {
            throw_unchecked("java.lang.IllegalArgumentException", None)
        };
        let syntax = &syntax_and_pattern[..pos];
        let input = &syntax_and_pattern[pos + 1..];
        let expr = if syntax.eq_ignore_ascii_case("glob") {
            glob_to_regex(input)
        } else if syntax.eq_ignore_ascii_case("regex") {
            format!("^(?:{input})$")
        } else {
            throw_unchecked(
                "java.lang.UnsupportedOperationException",
                Some(&format!("Syntax '{syntax}' not recognized")),
            )
        };
        // The Unix glob's character classes are written in Java's
        // intersection syntax, `[[^/]&&…]`; rewrite them as a look-ahead.
        let expr = java_class_intersections(&expr);
        let regex = fancy_regex::Regex::new(&expr).unwrap_or_else(|e| {
            throw_unchecked(
                "java.util.regex.PatternSyntaxException",
                Some(&e.to_string()),
            )
        });
        RegexPathMatcher { regex }
    }
}

/// Rewrites the `[[^/]&&X]` classes [`glob_to_regex`] emits — Java's class
/// intersection, which `fancy_regex` has not — as `(?!/)[X]`.
fn java_class_intersections(expr: &str) -> String {
    const OPEN: &str = "[[^/]&&";
    let mut out = String::with_capacity(expr.len());
    let mut rest = expr;
    while let Some(pos) = rest.find(OPEN) {
        out.push_str(&rest[..pos]);
        rest = &rest[pos + OPEN.len()..];
        // The class body ends at the first `]]` not escaped.
        let bytes = rest.as_bytes();
        let mut j = 0;
        let mut end = rest.len();
        while j + 1 < bytes.len() {
            if bytes[j] == b'\\' {
                j += 2;
                continue;
            }
            if bytes[j] == b']' && bytes[j + 1] == b']' {
                end = j;
                break;
            }
            j += 1;
        }
        out.push_str("(?!/)[");
        out.push_str(&rest[..end]);
        out.push(']');
        rest = rest.get(end + 2..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

impl PathMatcher for RegexPathMatcher {
    fn matches(&self, path: &Path) -> bool {
        self.regex.is_match(&path.to_string()).unwrap_or(false)
    }
}

// =============================================================================
// Files
// =============================================================================

/// `java.nio.file.FileVisitResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileVisitResult {
    /// `CONTINUE`.
    Continue,
    /// `TERMINATE`.
    Terminate,
    /// `SKIP_SUBTREE`.
    SkipSubtree,
    /// `SKIP_SIBLINGS`.
    SkipSiblings,
}

/// `java.nio.file.FileVisitor<Path>`.
pub trait FileVisitor {
    /// `preVisitDirectory(dir, attrs)`.
    ///
    /// # Errors
    ///
    /// An `IOException` that ends the walk.
    fn pre_visit_directory(
        &mut self,
        dir: &Path,
        attrs: &BasicFileAttributes,
    ) -> io::Result<FileVisitResult>;

    /// `visitFile(file, attrs)`.
    ///
    /// # Errors
    ///
    /// An `IOException` that ends the walk.
    fn visit_file(
        &mut self,
        file: &Path,
        attrs: &BasicFileAttributes,
    ) -> io::Result<FileVisitResult>;

    /// `visitFileFailed(file, exc)`.
    ///
    /// # Errors
    ///
    /// An `IOException` that ends the walk.
    fn visit_file_failed(&mut self, file: &Path, exc: io::Error) -> io::Result<FileVisitResult>;

    /// `postVisitDirectory(dir, exc)`.
    ///
    /// # Errors
    ///
    /// An `IOException` that ends the walk.
    fn post_visit_directory(
        &mut self,
        dir: &Path,
        exc: Option<io::Error>,
    ) -> io::Result<FileVisitResult>;
}

/// `java.nio.file.Files`: every operation dispatches to the provider of the
/// path's filesystem.
pub struct Files;

fn provider(path: &Path) -> Arc<dyn FileSystemProvider> {
    path.get_file_system().provider()
}

impl Files {
    /// `newInputStream(path, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn new_input_stream(
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn InputStream>> {
        provider(path).new_input_stream(path, options)
    }

    /// `newOutputStream(path, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn new_output_stream(
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn OutputStream>> {
        provider(path).new_output_stream(path, options)
    }

    /// `newByteChannel(path, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn new_byte_channel(
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn SeekableByteChannel>> {
        provider(path).new_byte_channel(path, options)
    }

    /// `newDirectoryStream(dir)`: every entry (`AcceptAllFilter`).
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn new_directory_stream(dir: &Path) -> io::Result<Box<dyn DirectoryStream>> {
        provider(dir).new_directory_stream(dir, accept_all_filter())
    }

    /// `newDirectoryStream(dir, glob)`: `"*"` accepts every entry; any other
    /// glob is matched against each entry's file name.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn new_directory_stream_glob(
        dir: &Path,
        glob: &str,
    ) -> io::Result<Box<dyn DirectoryStream>> {
        if glob == "*" {
            return Self::new_directory_stream(dir);
        }
        let fs = dir.get_file_system();
        let matcher: Arc<dyn PathMatcher> = Arc::from(fs.get_path_matcher(&format!("glob:{glob}")));
        let filter: DirectoryStreamFilter = Arc::new(move |entry: &Path| {
            Ok(entry
                .get_file_name()
                .is_some_and(|name| matcher.matches(&name)))
        });
        provider(dir).new_directory_stream(dir, filter)
    }

    /// `newDirectoryStream(dir, filter)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn new_directory_stream_filter(
        dir: &Path,
        filter: DirectoryStreamFilter,
    ) -> io::Result<Box<dyn DirectoryStream>> {
        provider(dir).new_directory_stream(dir, filter)
    }

    /// `createFile(path)`: `newByteChannel(path, {CREATE_NEW, WRITE})`, closed
    /// at once.
    ///
    /// # Errors
    ///
    /// `FileAlreadyExistsException` and the other failures of the provider.
    pub fn create_file(path: &Path) -> io::Result<Path> {
        let channel = Self::new_byte_channel(path, &[OpenOption::CreateNew, OpenOption::Write])?;
        channel.close()?;
        Ok(path.clone())
    }

    /// `createDirectory(dir)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn create_directory(dir: &Path) -> io::Result<Path> {
        provider(dir).create_directory(dir)?;
        Ok(dir.clone())
    }

    fn create_and_check_is_directory(dir: &Path) -> io::Result<()> {
        match Self::create_directory(dir) {
            Ok(_) => Ok(()),
            Err(error) if java_class_of(&error) == FILE_ALREADY_EXISTS_EXCEPTION => {
                // `isDirectory(dir)`, following links (measured on OpenJDK 25:
                // `readAttributes(dir)` without `NOFOLLOW_LINKS`).
                if !Self::is_directory(dir, &[]) {
                    return Err(error);
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// `createDirectories(dir)`: the directory and every missing parent.
    ///
    /// The first attempt creates `dir` itself; when that fails for another
    /// reason than an existing file, the path is made absolute, the nearest
    /// existing ancestor is found with `checkAccess`, and each name below it is
    /// created in turn.
    ///
    /// # Errors
    ///
    /// `FileAlreadyExistsException` when `dir` exists but is not a directory,
    /// `FileSystemException` ("Unable to determine if root directory exists")
    /// when no ancestor exists, and the failures of the provider.
    pub fn create_directories(dir: &Path) -> io::Result<Path> {
        match Self::create_and_check_is_directory(dir) {
            Ok(()) => return Ok(dir.clone()),
            Err(error) if java_class_of(&error) == FILE_ALREADY_EXISTS_EXCEPTION => {
                return Err(error)
            }
            Err(_) => {}
        }
        let dir = dir.to_absolute_path();
        let mut parent = dir.get_parent();
        while let Some(p) = parent.clone() {
            match provider(&p).check_access(&p, &[]) {
                Ok(()) => break,
                Err(error) if java_class_of(&error) == NO_SUCH_FILE_EXCEPTION => {}
                Err(error) => return Err(error),
            }
            parent = p.get_parent();
        }
        let Some(parent) = parent else {
            return Err(file_system_exception(
                Some(dir.to_string()),
                None,
                Some("Unable to determine if root directory exists".to_string()),
            ));
        };
        let mut child = parent.clone();
        for name in parent.relativize(&dir).iter_names() {
            child = child.resolve(&name);
            Self::create_and_check_is_directory(&child)?;
        }
        Ok(dir)
    }

    /// `createSymbolicLink(link, target)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn create_symbolic_link(link: &Path, target: &Path) -> io::Result<Path> {
        provider(link).create_symbolic_link(link, target)?;
        Ok(link.clone())
    }

    /// `createLink(link, existing)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn create_link(link: &Path, existing: &Path) -> io::Result<Path> {
        provider(link).create_link(link, existing)?;
        Ok(link.clone())
    }

    /// `delete(path)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn delete(path: &Path) -> io::Result<()> {
        provider(path).delete(path)
    }

    /// `deleteIfExists(path)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn delete_if_exists(path: &Path) -> io::Result<bool> {
        provider(path).delete_if_exists(path)
    }

    /// `copy(source, target, options...)` between paths of one provider.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn copy(source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<Path> {
        provider(source).copy(source, target, options)?;
        Ok(target.clone())
    }

    /// `move(source, target, options...)` between paths of one provider.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn r#move(source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<Path> {
        provider(source).r#move(source, target, options)?;
        Ok(target.clone())
    }

    /// `readSymbolicLink(link)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn read_symbolic_link(link: &Path) -> io::Result<Path> {
        provider(link).read_symbolic_link(link)
    }

    /// `getFileStore(path)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn get_file_store(path: &Path) -> io::Result<Arc<dyn FileStore>> {
        provider(path).get_file_store(path)
    }

    /// `isSameFile(path, path2)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn is_same_file(path: &Path, path2: &Path) -> io::Result<bool> {
        provider(path).is_same_file(path, path2)
    }

    /// `isHidden(path)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn is_hidden(path: &Path) -> io::Result<bool> {
        provider(path).is_hidden(path)
    }

    /// `readAttributes(path, BasicFileAttributes.class, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn read_attributes(path: &Path, options: &[LinkOption]) -> io::Result<BasicFileAttributes> {
        provider(path).read_attributes(path, options)
    }

    /// `readAttributes(path, String attributes, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn read_attributes_by_name(
        path: &Path,
        attributes: &str,
        options: &[LinkOption],
    ) -> io::Result<Vec<(String, AttributeValue)>> {
        provider(path).read_attributes_by_name(path, attributes, options)
    }

    /// `setAttribute(path, attribute, value, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn set_attribute(
        path: &Path,
        attribute: &str,
        value: &AttributeValue,
        options: &[LinkOption],
    ) -> io::Result<Path> {
        provider(path).set_attribute(path, attribute, value, options)?;
        Ok(path.clone())
    }

    /// `getLastModifiedTime(path, options...)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn get_last_modified_time(path: &Path, options: &[LinkOption]) -> io::Result<SystemTime> {
        Ok(Self::read_attributes(path, options)?.last_modified_time)
    }

    /// `size(path)`: `readAttributes(path, BasicFileAttributes.class).size()`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn size(path: &Path) -> io::Result<i64> {
        Ok(Self::read_attributes(path, &[])?.size)
    }

    /// `exists(path, options...)`: the provider's `exists`.
    pub fn exists(path: &Path, options: &[LinkOption]) -> bool {
        provider(path).exists(path, options)
    }

    /// `notExists(path, options...)`: `true` only when the provider reports
    /// `NoSuchFileException`.
    pub fn not_exists(path: &Path, options: &[LinkOption]) -> bool {
        let result = if follow_links(options) {
            provider(path).check_access(path, &[])
        } else {
            Self::read_attributes(path, &[LinkOption::NoFollowLinks]).map(|_| ())
        };
        matches!(result, Err(error) if java_class_of(&error) == NO_SUCH_FILE_EXCEPTION)
    }

    /// `isDirectory(path, options...)`: `readAttributesIfExists`, `false` when
    /// missing or on any `IOException`.
    pub fn is_directory(path: &Path, options: &[LinkOption]) -> bool {
        matches!(
            provider(path).read_attributes_if_exists(path, options),
            Ok(Some(attributes)) if attributes.is_directory
        )
    }

    /// `isRegularFile(path, options...)`.
    pub fn is_regular_file(path: &Path, options: &[LinkOption]) -> bool {
        matches!(
            provider(path).read_attributes_if_exists(path, options),
            Ok(Some(attributes)) if attributes.is_regular_file
        )
    }

    /// `isSymbolicLink(path)`.
    pub fn is_symbolic_link(path: &Path) -> bool {
        matches!(
            provider(path).read_attributes_if_exists(path, &[LinkOption::NoFollowLinks]),
            Ok(Some(attributes)) if attributes.is_symbolic_link
        )
    }

    fn is_accessible(path: &Path, mode: AccessMode) -> bool {
        provider(path).check_access(path, &[mode]).is_ok()
    }

    /// `isReadable(path)`.
    pub fn is_readable(path: &Path) -> bool {
        Self::is_accessible(path, AccessMode::Read)
    }

    /// `isWritable(path)`.
    pub fn is_writable(path: &Path) -> bool {
        Self::is_accessible(path, AccessMode::Write)
    }

    /// `isExecutable(path)`.
    pub fn is_executable(path: &Path) -> bool {
        Self::is_accessible(path, AccessMode::Execute)
    }

    /// `readAllBytes(path)`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn read_all_bytes(path: &Path) -> io::Result<Vec<u8>> {
        let mut input = Self::new_input_stream(path, &[])?;
        let mut out = Vec::new();
        let mut buf = vec![0u8; 8192];
        let result = loop {
            let len = buf.len();
            match input.read(&mut buf, 0, len) {
                Ok(n) if n < 0 => break Ok(()),
                Ok(n) => out.extend_from_slice(&buf[..n as usize]),
                Err(error) => break Err(error),
            }
        };
        let closed = input.close();
        result?;
        closed?;
        Ok(out)
    }

    /// `readAllLines(path, UTF_8)`: the lines, split at `\n`, `\r` or `\r\n`.
    ///
    /// # Errors
    ///
    /// `MalformedInputException` ("Input length = 1") for bytes that are not
    /// UTF-8, and the failures of the provider.
    pub fn read_all_lines(path: &Path) -> io::Result<Vec<String>> {
        let bytes = Self::read_all_bytes(path)?;
        let text = String::from_utf8(bytes).map_err(|_| {
            JavaIoException::new(
                "java.nio.charset.MalformedInputException",
                Some("Input length = 1".to_string()),
            )
            .into_io_error()
        })?;
        let mut lines = Vec::new();
        let mut current = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\n' => lines.push(std::mem::take(&mut current)),
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    lines.push(std::mem::take(&mut current));
                }
                c => current.push(c),
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        Ok(lines)
    }

    /// `write(path, bytes, options...)`: through `newOutputStream`.
    ///
    /// # Errors
    ///
    /// The failures of the provider.
    pub fn write(path: &Path, bytes: &[u8], options: &[OpenOption]) -> io::Result<Path> {
        let mut out = Self::new_output_stream(path, options)?;
        let written = out.write(bytes, 0, bytes.len());
        let closed = out.close();
        written?;
        closed?;
        Ok(path.clone())
    }

    /// `walkFileTree(start, visitor)`: a depth-first walk that does not
    /// follow symbolic links, visiting each directory's entries in the order
    /// its directory stream returns them.
    ///
    /// # Errors
    ///
    /// The `IOException` a visitor method returns.
    pub fn walk_file_tree(start: &Path, visitor: &mut dyn FileVisitor) -> io::Result<Path> {
        Self::walk(start, visitor)?;
        Ok(start.clone())
    }

    fn walk(file: &Path, visitor: &mut dyn FileVisitor) -> io::Result<FileVisitResult> {
        let attrs = match Self::read_attributes(file, &[LinkOption::NoFollowLinks]) {
            Ok(attrs) => attrs,
            Err(error) => return visitor.visit_file_failed(file, error),
        };
        if !attrs.is_directory {
            return visitor.visit_file(file, &attrs);
        }
        let mut stream = match Self::new_directory_stream(file) {
            Ok(stream) => stream,
            Err(error) => return visitor.visit_file_failed(file, error),
        };
        match visitor.pre_visit_directory(file, &attrs) {
            Ok(FileVisitResult::Continue) => {}
            Ok(FileVisitResult::SkipSubtree) | Ok(FileVisitResult::SkipSiblings) => {
                let _ = stream.close();
                return Ok(FileVisitResult::Continue);
            }
            Ok(FileVisitResult::Terminate) => {
                let _ = stream.close();
                return Ok(FileVisitResult::Terminate);
            }
            Err(error) => {
                let _ = stream.close();
                return Err(error);
            }
        }
        // The entries are visited while the stream is open, in its order,
        // and the stream is closed before `postVisitDirectory`.
        let mut iteration_error: Option<io::Error> = None;
        let mut outcome: io::Result<Option<FileVisitResult>> = Ok(None);
        for entry in stream.iterator() {
            match entry {
                Ok(entry) => match Self::walk(&entry, visitor) {
                    Ok(FileVisitResult::Terminate) => {
                        outcome = Ok(Some(FileVisitResult::Terminate));
                        break;
                    }
                    Ok(FileVisitResult::SkipSiblings) => break,
                    Ok(_) => {}
                    Err(error) => {
                        outcome = Err(error);
                        break;
                    }
                },
                Err(error) => {
                    iteration_error = Some(error);
                    break;
                }
            }
        }
        let close_error = stream.close().err();
        if let Some(early) = outcome? {
            return Ok(early);
        }
        let exc = iteration_error.or(close_error);
        visitor.post_visit_directory(file, exc)
    }
}

#[cfg(test)]
mod tests;
