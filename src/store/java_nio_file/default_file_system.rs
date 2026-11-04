//! The default filesystem, `FileSystems.getDefault()`: a provider over
//! `std::fs` that reproduces `sun.nio.fs.UnixFileSystemProvider` and
//! `sun.nio.ch.FileChannelImpl` on Unix.
//!
//! Errors of the operating system are translated as `UnixException` translates
//! them: `EACCES` to `AccessDeniedException`, `ENOENT` to `NoSuchFileException`,
//! `EEXIST` to `FileAlreadyExistsException`, `ELOOP` to a
//! `FileSystemException` whose reason is the system's text plus " or unable to
//! access attributes of symbolic link", and every other error to a
//! `FileSystemException` whose reason is the system's text (`strerror`), each
//! naming the file (and the other file of a two-file operation). Verified
//! against OpenJDK 25.

use std::any::Any;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock, Weak};

use super::{
    closed_channel_exception, directory_not_empty_exception, file_already_exists_exception,
    follow_links, not_directory_exception, os_bytes, throw_unchecked, AccessMode,
    AsynchronousFileChannel, AttributeValue, BasicFileAttributes, CompletionHandler, CopyOption,
    DirectoryStream, DirectoryStreamFilter, FileChannel, FileLock, FileStore, FileSystem,
    FileSystemProvider, InputStream, JavaFuture, JavaIoException, LinkOption, OpenOption,
    OutputStream, Path, PathMatcher, RegexPathMatcher, SeekableByteChannel,
    ACCESS_DENIED_EXCEPTION, ATOMIC_MOVE_NOT_SUPPORTED_EXCEPTION, FILE_ALREADY_EXISTS_EXCEPTION,
    FILE_SYSTEM_EXCEPTION, NO_SUCH_FILE_EXCEPTION,
};

// =============================================================================
// Errors
// =============================================================================

/// The system's text for an OS error, `strerror(errno)`: the [`io::Error`]
/// text without its ` (os error N)` suffix.
fn error_string(error: &io::Error) -> String {
    let text = error.to_string();
    match error.raw_os_error() {
        Some(code) => text
            .strip_suffix(&format!(" (os error {code})"))
            .unwrap_or(&text)
            .to_string(),
        None => text,
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "emscripten",
        target_os = "espidf",
        target_os = "fuchsia",
        target_os = "horizon",
        target_os = "redox",
        target_os = "vita",
        target_os = "wasi"
    ))
))]
mod errno {
    use rustix::io::Errno;

    fn is(code: i32, errno: Errno) -> bool {
        Errno::from_raw_os_error(code) == errno
    }

    pub(super) fn is_access(code: i32) -> bool {
        is(code, Errno::ACCESS)
    }
    pub(super) fn is_noent(code: i32) -> bool {
        is(code, Errno::NOENT)
    }
    pub(super) fn is_exist(code: i32) -> bool {
        is(code, Errno::EXIST)
    }
    pub(super) fn is_loop(code: i32) -> bool {
        is(code, Errno::LOOP)
    }
    pub(super) fn is_notempty(code: i32) -> bool {
        is(code, Errno::NOTEMPTY)
    }
    pub(super) fn is_xdev(code: i32) -> bool {
        is(code, Errno::XDEV)
    }
    pub(super) fn is_isdir(code: i32) -> bool {
        is(code, Errno::ISDIR)
    }
    pub(super) fn is_notdir(code: i32) -> bool {
        is(code, Errno::NOTDIR)
    }
}

/// The errno tests on the targets without rustix, through the kinds the
/// standard library assigns.
#[cfg(not(all(
    unix,
    not(any(
        target_os = "emscripten",
        target_os = "espidf",
        target_os = "fuchsia",
        target_os = "horizon",
        target_os = "redox",
        target_os = "vita",
        target_os = "wasi"
    ))
)))]
mod errno {
    use std::io;

    fn kind(code: i32) -> io::ErrorKind {
        io::Error::from_raw_os_error(code).kind()
    }

    pub(super) fn is_access(code: i32) -> bool {
        kind(code) == io::ErrorKind::PermissionDenied
    }
    pub(super) fn is_noent(code: i32) -> bool {
        kind(code) == io::ErrorKind::NotFound
    }
    pub(super) fn is_exist(code: i32) -> bool {
        kind(code) == io::ErrorKind::AlreadyExists
    }
    pub(super) fn is_loop(_code: i32) -> bool {
        false
    }
    pub(super) fn is_notempty(code: i32) -> bool {
        kind(code) == io::ErrorKind::DirectoryNotEmpty
    }
    pub(super) fn is_xdev(code: i32) -> bool {
        kind(code) == io::ErrorKind::CrossesDevices
    }
    pub(super) fn is_isdir(code: i32) -> bool {
        kind(code) == io::ErrorKind::IsADirectory
    }
    pub(super) fn is_notdir(code: i32) -> bool {
        kind(code) == io::ErrorKind::NotADirectory
    }
}

/// `UnixException.translateToIOException(file, other)` of an OS error; any
/// other error is returned unchanged.
///
/// **Divergence.** On targets other than Unix the error is returned unchanged:
/// the JDK's `WindowsException` translation is not reproduced.
pub(crate) fn translate(error: io::Error, file: &Path, other: Option<&Path>) -> io::Error {
    if cfg!(not(unix)) {
        return error;
    }
    let Some(code) = error.raw_os_error() else {
        return error;
    };
    let file = Some(file.to_string());
    let other = other.map(ToString::to_string);
    let (class, reason) = if errno::is_access(code) {
        (ACCESS_DENIED_EXCEPTION, None)
    } else if errno::is_noent(code) {
        (NO_SUCH_FILE_EXCEPTION, None)
    } else if errno::is_exist(code) {
        (FILE_ALREADY_EXISTS_EXCEPTION, None)
    } else if errno::is_loop(code) {
        (
            FILE_SYSTEM_EXCEPTION,
            Some(format!(
                "{} or unable to access attributes of symbolic link",
                error_string(&error)
            )),
        )
    } else {
        (FILE_SYSTEM_EXCEPTION, Some(error_string(&error)))
    };
    JavaIoException::file_system(class, file, other, reason).into_io_error()
}

/// The `java.io.IOException` the JDK raises for a failed channel operation:
/// `IOException(strerror)`.
fn channel_error(error: io::Error) -> io::Error {
    if error.raw_os_error().is_none() {
        return error;
    }
    if cfg!(not(unix)) {
        return error;
    }
    JavaIoException::new(super::IO_EXCEPTION, Some(error_string(&error))).into_io_error()
}

/// `System.identityHashCode`-like hex of an object, for `Object.toString()`.
fn identity_hex<T: ?Sized>(object: &T) -> String {
    let address = std::ptr::from_ref(object).cast::<()>() as usize;
    format!("{:x}", (address >> 3) as u32)
}

// =============================================================================
// FileSystem
// =============================================================================

/// `sun.nio.fs.UnixFileSystem` (`LinuxFileSystem` on Linux): the default
/// filesystem.
pub struct DefaultFileSystem {
    provider: Arc<DefaultFileSystemProvider>,
}

static DEFAULT_FILE_SYSTEM: LazyLock<Arc<DefaultFileSystem>> = LazyLock::new(|| {
    Arc::new(DefaultFileSystem {
        provider: Arc::new(DefaultFileSystemProvider { _private: () }),
    })
});

/// `FileSystems.getDefault()`.
pub fn default_file_system() -> Arc<dyn FileSystem> {
    let fs: Arc<DefaultFileSystem> = Arc::clone(&DEFAULT_FILE_SYSTEM);
    fs
}

impl FileSystem for DefaultFileSystem {
    fn provider(&self) -> Arc<dyn FileSystemProvider> {
        let provider: Arc<DefaultFileSystemProvider> = Arc::clone(&self.provider);
        provider
    }

    /// `UnixFileSystem.close()`: the default filesystem cannot be closed.
    fn close(&self) -> io::Result<()> {
        throw_unchecked("java.lang.UnsupportedOperationException", None)
    }

    fn is_open(&self) -> bool {
        true
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn get_separator(&self) -> String {
        if cfg!(unix) { "/" } else { "\\" }.to_string()
    }

    fn get_root_directories(self: Arc<Self>) -> Vec<Path> {
        vec![Path::new(
            default_file_system(),
            if cfg!(unix) { "/" } else { "\\" },
        )]
    }

    /// `getFileStores()`: one store per mount entry of `/proc/self/mounts`
    /// (none where the table does not exist).
    fn get_file_stores(&self) -> Vec<Arc<dyn FileStore>> {
        mount_entries()
            .into_iter()
            .map(|entry| -> Arc<dyn FileStore> {
                Arc::new(DefaultFileStore {
                    mount_dir: PathBuf::from(&entry.dir),
                    name: entry.name,
                    type_: entry.fstype,
                    dir: entry.dir,
                    read_only: entry.read_only,
                })
            })
            .collect()
    }

    /// `supportedFileAttributeViews()`: `LinuxFileSystem`'s six views on
    /// Linux, `UnixFileSystem`'s four elsewhere.
    fn supported_file_attribute_views(&self) -> Vec<String> {
        let views: &[&str] = if cfg!(target_os = "linux") {
            &["owner", "dos", "basic", "posix", "user", "unix"]
        } else {
            &["owner", "basic", "posix", "unix"]
        };
        views.iter().map(ToString::to_string).collect()
    }

    /// `UnixFileSystem.getPath(first, more...)`: the non-empty parts joined
    /// with the separator.
    fn get_path(self: Arc<Self>, first: &str, more: &[&str]) -> Path {
        let mut joined = String::from(first);
        for segment in more {
            if !segment.is_empty() {
                if !joined.is_empty() {
                    joined.push_str(&self.get_separator());
                }
                joined.push_str(segment);
            }
        }
        Path::new(default_file_system(), joined)
    }

    fn get_path_matcher(&self, syntax_and_pattern: &str) -> Box<dyn PathMatcher> {
        Box::new(RegexPathMatcher::new(syntax_and_pattern))
    }

    fn file_system_to_string(&self) -> String {
        format!("sun.nio.fs.LinuxFileSystem@{}", identity_hex(self))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// =============================================================================
// Provider
// =============================================================================

/// `sun.nio.fs.UnixFileSystemProvider` (`LinuxFileSystemProvider` on Linux).
pub struct DefaultFileSystemProvider {
    _private: (),
}

fn std_path(path: &Path) -> &std::path::Path {
    path.as_std_path()
}

/// `lstat` or `stat` of `path`, translated.
fn metadata(path: &Path, follow: bool) -> io::Result<std::fs::Metadata> {
    let result = if follow {
        std::fs::metadata(std_path(path))
    } else {
        std::fs::symlink_metadata(std_path(path))
    };
    result.map_err(|e| translate(e, path, None))
}

/// `unlink` or `rmdir` of `path` according to `is_dir`, translating
/// `EEXIST`/`ENOTEMPTY` from `rmdir` to `DirectoryNotEmptyException`.
fn remove(path: &Path, is_dir: bool) -> io::Result<()> {
    let result = if is_dir {
        std::fs::remove_dir(std_path(path))
    } else {
        std::fs::remove_file(std_path(path))
    };
    result.map_err(|e| {
        let code = e.raw_os_error().unwrap_or(0);
        if is_dir && (errno::is_exist(code) || errno::is_notempty(code)) {
            directory_not_empty_exception(path.to_string())
        } else {
            translate(e, path, None)
        }
    })
}

impl DefaultFileSystemProvider {
    /// `UnixFileSystemProvider.implDelete(file, failIfNotExists)`.
    fn impl_delete(&self, path: &Path, fail_if_not_exists: bool) -> io::Result<bool> {
        let attrs = match std::fs::symlink_metadata(std_path(path)) {
            Ok(attrs) => attrs,
            Err(e) => {
                if !fail_if_not_exists && e.raw_os_error().is_some_and(errno::is_noent) {
                    return Ok(false);
                }
                return Err(translate(e, path, None));
            }
        };
        remove(path, attrs.is_dir())?;
        Ok(true)
    }

    /// The flags of an option set, as `UnixChannelFactory.Flags.toFlags`, and
    /// their checks.
    fn open_options(options: &[OpenOption]) -> (OpenOptions, Flags) {
        let mut flags = Flags::default();
        for option in options {
            match option {
                OpenOption::Read => flags.read = true,
                OpenOption::Write => flags.write = true,
                OpenOption::Append => flags.append = true,
                OpenOption::TruncateExisting => flags.truncate_existing = true,
                OpenOption::Create => flags.create = true,
                OpenOption::CreateNew => flags.create_new = true,
                OpenOption::DeleteOnClose => flags.delete_on_close = true,
                OpenOption::Sparse => {}
                OpenOption::Sync => flags.sync = true,
                OpenOption::Dsync => flags.dsync = true,
                OpenOption::NoFollowLinks => flags.no_follow_links = true,
            }
        }
        if !flags.read && !flags.write {
            if flags.append {
                flags.write = true;
            } else {
                flags.read = true;
            }
        }
        if flags.read && flags.append {
            throw_unchecked(
                "java.lang.IllegalArgumentException",
                Some("READ + APPEND not allowed"),
            )
        }
        if flags.append && flags.truncate_existing {
            throw_unchecked(
                "java.lang.IllegalArgumentException",
                Some("APPEND + TRUNCATE_EXISTING not allowed"),
            )
        }
        let mut open = OpenOptions::new();
        open.read(flags.read);
        if flags.write {
            if flags.append {
                open.append(true);
            } else {
                open.write(true);
            }
            if flags.truncate_existing {
                open.truncate(true);
            }
            if flags.create_new {
                open.create_new(true);
            } else if flags.create {
                open.create(true);
            }
        }
        custom_flags(&mut open, &flags);
        (open, flags)
    }

    /// `UnixChannelFactory.newFileChannel(path, options)`.
    fn open_channel(&self, path: &Path, options: &[OpenOption]) -> io::Result<DefaultFileChannel> {
        let (open, flags) = Self::open_options(options);
        let file = open
            .open(std_path(path))
            .map_err(|e| translate(e, path, None))?;
        // `DELETE_ON_CLOSE`: Unix unlinks the file once it is open.
        if flags.delete_on_close {
            let _ = std::fs::remove_file(std_path(path));
        }
        Ok(DefaultFileChannel::new(
            file,
            flags.read,
            flags.write,
            flags.append,
        ))
    }
}

#[derive(Default, Clone, Copy)]
struct Flags {
    read: bool,
    write: bool,
    append: bool,
    truncate_existing: bool,
    create: bool,
    create_new: bool,
    delete_on_close: bool,
    sync: bool,
    dsync: bool,
    no_follow_links: bool,
}

#[cfg(all(
    unix,
    not(any(
        target_os = "emscripten",
        target_os = "espidf",
        target_os = "fuchsia",
        target_os = "horizon",
        target_os = "redox",
        target_os = "vita",
        target_os = "wasi"
    ))
))]
fn custom_flags(open: &mut OpenOptions, flags: &Flags) {
    use rustix::fs::OFlags;
    use std::os::unix::fs::OpenOptionsExt;
    let mut bits = OFlags::empty();
    if flags.sync {
        bits |= OFlags::SYNC;
    }
    if flags.dsync {
        bits |= OFlags::DSYNC;
    }
    if flags.no_follow_links {
        bits |= OFlags::NOFOLLOW;
    }
    if !bits.is_empty() {
        open.custom_flags(bits.bits() as i32);
    }
}

/// **Divergence.** Without rustix, `SYNC`, `DSYNC` and `NOFOLLOW_LINKS` are
/// not passed to the operating system.
#[cfg(not(all(
    unix,
    not(any(
        target_os = "emscripten",
        target_os = "espidf",
        target_os = "fuchsia",
        target_os = "horizon",
        target_os = "redox",
        target_os = "vita",
        target_os = "wasi"
    ))
)))]
fn custom_flags(_open: &mut OpenOptions, _flags: &Flags) {}

/// The `basic` attribute names, in `BasicFileAttributeView`'s order.
const BASIC_NAMES: [&str; 9] = [
    "lastModifiedTime",
    "lastAccessTime",
    "creationTime",
    "size",
    "isRegularFile",
    "isDirectory",
    "isSymbolicLink",
    "isOther",
    "fileKey",
];

fn basic_value(attrs: &BasicFileAttributes, name: &str) -> Option<AttributeValue> {
    Some(match name {
        "lastModifiedTime" => AttributeValue::FileTime(attrs.last_modified_time),
        "lastAccessTime" => AttributeValue::FileTime(attrs.last_access_time),
        "creationTime" => AttributeValue::FileTime(attrs.creation_time),
        "size" => AttributeValue::Long(attrs.size),
        "isRegularFile" => AttributeValue::Boolean(attrs.is_regular_file),
        "isDirectory" => AttributeValue::Boolean(attrs.is_directory),
        "isSymbolicLink" => AttributeValue::Boolean(attrs.is_symbolic_link),
        "isOther" => AttributeValue::Boolean(attrs.is_other),
        "fileKey" => AttributeValue::FileKey(attrs.file_key),
        _ => return None,
    })
}

/// Splits `[view:]names` into the view (`basic` by default) and the names.
fn split_attributes(attributes: &str) -> (&str, &str) {
    match attributes.find(':') {
        Some(pos) => (&attributes[..pos], &attributes[pos + 1..]),
        None => ("basic", attributes),
    }
}

impl FileSystemProvider for DefaultFileSystemProvider {
    fn get_scheme(&self) -> String {
        "file".to_string()
    }

    fn get_file_system(&self) -> Arc<dyn FileSystem> {
        default_file_system()
    }

    /// `UnixUriUtils.fromUri(fs, uri)`.
    fn get_path(&self, uri: &str) -> Path {
        from_uri(uri)
    }

    fn resolve(&self, base: &Path, other: &Path) -> Path {
        base.default_resolve(other)
    }

    fn create_directory(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir(std_path(dir)).map_err(|e| translate(e, dir, None))
    }

    fn delete(&self, path: &Path) -> io::Result<()> {
        self.impl_delete(path, true).map(|_| ())
    }

    fn delete_if_exists(&self, path: &Path) -> io::Result<bool> {
        self.impl_delete(path, false)
    }

    /// `UnixCopyFile.copy(source, target, options)`.
    fn copy(&self, source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<()> {
        if options.contains(&CopyOption::AtomicMove) {
            throw_unchecked(
                "java.lang.UnsupportedOperationException",
                Some("Unsupported copy option"),
            )
        }
        let follow = !options.contains(&CopyOption::NoFollowLinks);
        let replace = options.contains(&CopyOption::ReplaceExisting);
        let source_attrs = metadata(source, follow)?;
        let target_attrs = std::fs::symlink_metadata(std_path(target)).ok();
        if let Some(target_attrs) = &target_attrs {
            if same_file(&source_attrs, target_attrs) {
                return Ok(());
            }
            if !replace {
                return Err(file_already_exists_exception(target.to_string()));
            }
            remove(target, target_attrs.is_dir())?;
        }
        if source_attrs.is_dir() {
            std::fs::create_dir(std_path(target)).map_err(|e| translate(e, target, None))?;
        } else if source_attrs.file_type().is_symlink() {
            let link =
                std::fs::read_link(std_path(source)).map_err(|e| translate(e, source, None))?;
            symlink(&link, std_path(target)).map_err(|e| translate(e, target, None))?;
        } else {
            copy_contents(source, target)?;
        }
        if options.contains(&CopyOption::CopyAttributes) {
            let file = File::options()
                .write(!source_attrs.is_dir())
                .read(source_attrs.is_dir())
                .open(std_path(target))
                .map_err(|e| translate(e, target, None))?;
            let mut times = std::fs::FileTimes::new();
            if let Ok(modified) = source_attrs.modified() {
                times = times.set_modified(modified);
            }
            if let Ok(accessed) = source_attrs.accessed() {
                times = times.set_accessed(accessed);
            }
            file.set_times(times)
                .map_err(|e| translate(e, target, None))?;
        }
        Ok(())
    }

    /// `UnixCopyFile.move(source, target, options)`.
    fn r#move(&self, source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<()> {
        if options.contains(&CopyOption::AtomicMove) {
            return std::fs::rename(std_path(source), std_path(target)).map_err(|e| {
                if e.raw_os_error().is_some_and(errno::is_xdev) {
                    JavaIoException::file_system(
                        ATOMIC_MOVE_NOT_SUPPORTED_EXCEPTION,
                        Some(source.to_string()),
                        Some(target.to_string()),
                        Some(error_string(&e)),
                    )
                    .into_io_error()
                } else {
                    translate(e, source, Some(target))
                }
            });
        }
        let replace = options.contains(&CopyOption::ReplaceExisting);
        let source_attrs = metadata(source, false)?;
        let target_attrs = std::fs::symlink_metadata(std_path(target)).ok();
        if let Some(target_attrs) = &target_attrs {
            if same_file(&source_attrs, target_attrs) {
                return Ok(());
            }
            if !replace {
                return Err(file_already_exists_exception(target.to_string()));
            }
            remove(target, target_attrs.is_dir())?;
        }
        match std::fs::rename(std_path(source), std_path(target)) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let code = e.raw_os_error().unwrap_or(0);
                if !errno::is_xdev(code) && !errno::is_isdir(code) {
                    return Err(translate(e, source, Some(target)));
                }
            }
        }
        // A move across file systems: copy, then delete the source.
        if source_attrs.is_dir() {
            std::fs::create_dir(std_path(target)).map_err(|e| translate(e, target, None))?;
        } else {
            copy_contents(source, target)?;
        }
        remove(source, source_attrs.is_dir())
    }

    fn is_same_file(&self, path: &Path, path2: &Path) -> io::Result<bool> {
        if path == path2 {
            return Ok(true);
        }
        if !path.same_file_system(path2) {
            return Ok(false);
        }
        let a = metadata(path, true)?;
        let b = metadata(path2, true)?;
        Ok(same_file(&a, &b))
    }

    /// `UnixFileSystemProvider.isHidden`: the file name starts with `.`.
    fn is_hidden(&self, path: &Path) -> io::Result<bool> {
        let Some(name) = path.get_file_name() else {
            return Ok(false);
        };
        let bytes = os_bytes(name.as_std_path().as_os_str()).into_owned();
        if bytes.is_empty() {
            return Ok(false);
        }
        Ok(bytes[0] == b'.')
    }

    fn get_file_store(&self, path: &Path) -> io::Result<Arc<dyn FileStore>> {
        let real = std::fs::canonicalize(std_path(path)).map_err(|e| translate(e, path, None))?;
        let entries = mount_entries();
        let mut best: Option<MountEntry> = None;
        for entry in entries {
            let dir = std::path::Path::new(&entry.dir);
            if real.starts_with(dir) {
                let better = best.as_ref().is_none_or(|b| {
                    dir.components().count() >= std::path::Path::new(&b.dir).components().count()
                });
                if better {
                    best = Some(entry);
                }
            }
        }
        let store = match best {
            Some(entry) => DefaultFileStore {
                mount_dir: PathBuf::from(&entry.dir),
                name: entry.name,
                type_: entry.fstype,
                dir: entry.dir,
                read_only: entry.read_only,
            },
            None => DefaultFileStore {
                mount_dir: real.clone(),
                name: String::new(),
                type_: String::new(),
                dir: real.to_string_lossy().into_owned(),
                read_only: false,
            },
        };
        Ok(Arc::new(store))
    }

    fn check_access(&self, path: &Path, modes: &[AccessMode]) -> io::Result<()> {
        check_access(path, modes)
    }

    fn read_attributes(
        &self,
        path: &Path,
        options: &[LinkOption],
    ) -> io::Result<BasicFileAttributes> {
        let metadata = metadata(path, follow_links(options))?;
        Ok(BasicFileAttributes::from_metadata(&metadata))
    }

    /// The `basic` view; `UnsupportedOperationException` ("View 'x' not
    /// available") for any other.
    ///
    /// **Divergence.** The JDK's Unix provider also reads the `posix`, `unix`,
    /// `owner`, `dos` and `user` views; this port reads only `basic`, the view
    /// Lucene's store and the test framework read.
    fn read_attributes_by_name(
        &self,
        path: &Path,
        attributes: &str,
        options: &[LinkOption],
    ) -> io::Result<Vec<(String, AttributeValue)>> {
        let (view, names) = split_attributes(attributes);
        if view != "basic" {
            throw_unchecked(
                "java.lang.UnsupportedOperationException",
                Some(&format!("View '{view}' not available")),
            )
        }
        let attrs = self.read_attributes(path, options)?;
        let mut out = Vec::new();
        for name in names.split(',') {
            if name == "*" {
                for n in BASIC_NAMES {
                    out.push((n.to_string(), basic_value(&attrs, n).expect("basic name")));
                }
                continue;
            }
            let Some(value) = basic_value(&attrs, name) else {
                throw_unchecked(
                    "java.lang.IllegalArgumentException",
                    Some(&format!("'{name}' not recognized")),
                )
            };
            out.push((name.to_string(), value));
        }
        Ok(out)
    }

    /// `basic:lastModifiedTime`, `basic:lastAccessTime` and
    /// `basic:creationTime` (which Linux cannot set, and the JDK ignores).
    fn set_attribute(
        &self,
        path: &Path,
        attribute: &str,
        value: &AttributeValue,
        options: &[LinkOption],
    ) -> io::Result<()> {
        let (view, name) = split_attributes(attribute);
        if view != "basic" {
            throw_unchecked(
                "java.lang.UnsupportedOperationException",
                Some(&format!("View '{view}' not available")),
            )
        }
        let time = match (name, value) {
            (
                "lastModifiedTime" | "lastAccessTime" | "creationTime",
                AttributeValue::FileTime(t),
            ) => *t,
            ("lastModifiedTime" | "lastAccessTime" | "creationTime", _) => {
                throw_unchecked("java.lang.ClassCastException", None)
            }
            _ => throw_unchecked(
                "java.lang.IllegalArgumentException",
                Some(&format!("'{view}:{name}' not recognized")),
            ),
        };
        let _ = options;
        let metadata = metadata(path, true)?;
        let file = File::options()
            .read(true)
            .open(std_path(path))
            .or_else(|_| File::options().write(true).open(std_path(path)))
            .map_err(|e| translate(e, path, None))?;
        let mut times = std::fs::FileTimes::new();
        match name {
            "lastModifiedTime" => times = times.set_modified(time),
            "lastAccessTime" => times = times.set_accessed(time),
            _ => return Ok(()),
        }
        let _ = metadata;
        file.set_times(times).map_err(|e| translate(e, path, None))
    }

    /// `Files.newInputStream` on the default provider: the options may not
    /// include `APPEND` or `WRITE`.
    fn new_input_stream(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn InputStream>> {
        for option in options {
            if matches!(option, OpenOption::Append | OpenOption::Write) {
                throw_unchecked(
                    "java.lang.UnsupportedOperationException",
                    Some(&format!("'{option}' not allowed")),
                )
            }
        }
        let channel = self.open_channel(path, options)?;
        Ok(Box::new(ChannelInputStream {
            channel: Arc::new(channel),
        }))
    }

    /// `FileSystemProvider.newOutputStream`: no options means `CREATE`,
    /// `TRUNCATE_EXISTING`, `WRITE`; `READ` is refused; `WRITE` is added.
    fn new_output_stream(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn OutputStream>> {
        let mut opts: Vec<OpenOption> = if options.is_empty() {
            vec![
                OpenOption::Create,
                OpenOption::TruncateExisting,
                OpenOption::Write,
            ]
        } else {
            let mut opts = Vec::with_capacity(options.len() + 1);
            for option in options {
                if *option == OpenOption::Read {
                    throw_unchecked(
                        "java.lang.IllegalArgumentException",
                        Some("READ not allowed"),
                    )
                }
                opts.push(*option);
            }
            opts
        };
        if !opts.contains(&OpenOption::Write) {
            opts.push(OpenOption::Write);
        }
        let channel = self.open_channel(path, &opts)?;
        Ok(Box::new(ChannelOutputStream {
            channel: Arc::new(channel),
        }))
    }

    fn new_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn FileChannel>> {
        Ok(Arc::new(self.open_channel(path, options)?))
    }

    fn new_asynchronous_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn AsynchronousFileChannel>> {
        Ok(Arc::new(DefaultAsynchronousFileChannel {
            channel: self.open_channel(path, options)?,
        }))
    }

    fn new_byte_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn SeekableByteChannel>> {
        Ok(Box::new(self.open_channel(path, options)?))
    }

    /// `UnixFileSystemProvider.newDirectoryStream`: the directory is opened
    /// at once; `ENOTDIR` is `NotDirectoryException`.
    fn new_directory_stream(
        &self,
        dir: &Path,
        filter: DirectoryStreamFilter,
    ) -> io::Result<Box<dyn DirectoryStream>> {
        let entries = std::fs::read_dir(std_path(dir)).map_err(|e| {
            if e.raw_os_error().is_some_and(errno::is_notdir) {
                not_directory_exception(dir.to_string())
            } else {
                translate(e, dir, None)
            }
        })?;
        Ok(Box::new(DefaultDirectoryStream {
            dir: dir.clone(),
            entries: Some(entries),
            filter,
            iterator_obtained: false,
            closed: false,
        }))
    }

    fn create_symbolic_link(&self, link: &Path, target: &Path) -> io::Result<()> {
        symlink(std_path(target), std_path(link)).map_err(|e| translate(e, link, None))
    }

    fn create_link(&self, link: &Path, existing: &Path) -> io::Result<()> {
        std::fs::hard_link(std_path(existing), std_path(link))
            .map_err(|e| translate(e, link, Some(existing)))
    }

    /// `readSymbolicLink`: `NotLinkException` when `link` is not a symbolic
    /// link.
    fn read_symbolic_link(&self, link: &Path) -> io::Result<Path> {
        match std::fs::read_link(std_path(link)) {
            Ok(target) => Ok(Path::new(default_file_system(), target.as_os_str())),
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => Err(JavaIoException::file_system(
                super::NOT_LINK_EXCEPTION,
                Some(link.to_string()),
                None,
                None,
            )
            .into_io_error()),
            Err(e) => Err(translate(e, link, None)),
        }
    }

    /// `UnixFileSystemProvider.exists`: `access(F_OK)` (or `lstat`).
    fn exists(&self, path: &Path, options: &[LinkOption]) -> bool {
        if follow_links(options) {
            std::fs::metadata(std_path(path)).is_ok()
        } else {
            std::fs::symlink_metadata(std_path(path)).is_ok()
        }
    }

    fn provider_class_name(&self) -> String {
        "sun.nio.fs.LinuxFileSystemProvider".to_string()
    }

    fn provider_to_string(&self) -> String {
        format!("sun.nio.fs.LinuxFileSystemProvider@{}", identity_hex(self))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}

#[cfg(unix)]
fn symlink(target: &std::path::Path, link: &std::path::Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &std::path::Path, link: &std::path::Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(not(any(unix, windows)))]
fn symlink(_target: &std::path::Path, _link: &std::path::Path) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// Copies a regular file's contents to a new `target`
/// (`O_WRONLY | O_CREAT | O_EXCL`).
fn copy_contents(source: &Path, target: &Path) -> io::Result<()> {
    let mut input = File::open(std_path(source)).map_err(|e| translate(e, source, None))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(std_path(target))
        .map_err(|e| translate(e, target, None))?;
    io::copy(&mut input, &mut output).map_err(|e| translate(e, source, Some(target)))?;
    if let Ok(permissions) = input.metadata().map(|m| m.permissions()) {
        let _ = output.set_permissions(permissions);
    }
    Ok(())
}

#[cfg(all(
    unix,
    not(any(
        target_os = "emscripten",
        target_os = "espidf",
        target_os = "fuchsia",
        target_os = "horizon",
        target_os = "redox",
        target_os = "vita",
        target_os = "wasi"
    ))
))]
fn check_access(path: &Path, modes: &[AccessMode]) -> io::Result<()> {
    use rustix::fs::Access;
    let mut access = Access::empty();
    for mode in modes {
        access |= match mode {
            AccessMode::Read => Access::READ_OK,
            AccessMode::Write => Access::WRITE_OK,
            AccessMode::Execute => Access::EXEC_OK,
        };
    }
    if access.is_empty() {
        access = Access::EXISTS;
    }
    rustix::fs::access(std_path(path), access)
        .map_err(|errno| translate(io::Error::from(errno), path, None))
}

/// **Divergence.** Without rustix, access is checked by reading the file's
/// metadata: existence only, and write access by the read-only flag.
#[cfg(not(all(
    unix,
    not(any(
        target_os = "emscripten",
        target_os = "espidf",
        target_os = "fuchsia",
        target_os = "horizon",
        target_os = "redox",
        target_os = "vita",
        target_os = "wasi"
    ))
)))]
fn check_access(path: &Path, modes: &[AccessMode]) -> io::Result<()> {
    let metadata = metadata(path, true)?;
    if modes.contains(&AccessMode::Write) && metadata.permissions().readonly() {
        return Err(super::access_denied_exception(path.to_string()));
    }
    Ok(())
}

// =============================================================================
// Path operations of the default filesystem
// =============================================================================

/// `UnixPath.toRealPath(options...)`.
///
/// **Divergence.** With `NOFOLLOW_LINKS` the JDK resolves each name against
/// the directory entries; this port checks that the file exists (`lstat`) and
/// normalizes the absolute path lexically.
pub(super) fn to_real_path(path: &Path, options: &[LinkOption]) -> io::Result<Path> {
    let unwrapped = path.unwrap_all();
    if follow_links(options) {
        let real =
            std::fs::canonicalize(std_path(&unwrapped)).map_err(|e| translate(e, path, None))?;
        Ok(Path::new(default_file_system(), real.as_os_str())
            .with_file_system(Arc::clone(path.get_file_system())))
    } else {
        let absolute = path.to_absolute_path();
        metadata(&absolute, false).map_err(|_| super::no_such_file_exception(path.to_string()))?;
        Ok(absolute.normalize())
    }
}

/// The characters a URI path keeps unencoded: `unreserved`, `:@&=+$,`, `;`
/// and `/` (RFC 2396, as `UnixUriUtils` encodes).
fn uri_path_allowed(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-_.!~*'():@&=+$,;/".contains(&b)
}

/// `UnixUriUtils.toUri(path)`.
pub(super) fn to_uri(path: &Path) -> String {
    let absolute = path.to_absolute_path();
    let bytes = os_bytes(absolute.as_std_path().as_os_str()).into_owned();
    let mut sb = String::from("file://");
    for &b in &bytes {
        if uri_path_allowed(b) {
            sb.push(b as char);
        } else {
            sb.push_str(&format!("%{b:02X}"));
        }
    }
    if !sb.ends_with('/')
        && std::fs::metadata(absolute.unwrap_all().as_std_path()).is_ok_and(|m| m.is_dir())
    {
        sb.push('/');
    }
    sb
}

fn illegal_argument(message: &str) -> ! {
    throw_unchecked("java.lang.IllegalArgumentException", Some(message))
}

/// `UnixUriUtils.fromUri(fs, uri)`: a `file:` URI without authority, query or
/// fragment, its path percent-decoded into bytes.
pub(super) fn from_uri(uri: &str) -> Path {
    let Some(colon) = uri
        .find(':')
        .filter(|&p| p > 0 && !uri[..p].contains(['/', '?', '#']))
    else {
        illegal_argument("URI is not absolute")
    };
    let scheme = &uri[..colon];
    let rest = &uri[colon + 1..];
    if !rest.starts_with('/') {
        illegal_argument("URI is not hierarchical")
    }
    if !scheme.eq_ignore_ascii_case("file") {
        illegal_argument("URI scheme is not \"file\"")
    }
    let (rest, fragment) = match rest.find('#') {
        Some(pos) => (&rest[..pos], true),
        None => (rest, false),
    };
    let (rest, query) = match rest.find('?') {
        Some(pos) => (&rest[..pos], true),
        None => (rest, false),
    };
    let path = if let Some(after) = rest.strip_prefix("//") {
        let end = after.find('/').unwrap_or(after.len());
        if end > 0 {
            illegal_argument("URI has an authority component")
        }
        &after[end..]
    } else {
        rest
    };
    if fragment {
        illegal_argument("URI has a fragment component")
    }
    if query {
        illegal_argument("URI has a query component")
    }
    if path.is_empty() {
        illegal_argument("URI path component is empty")
    }
    let raw = path.as_bytes();
    let mut bytes: Vec<u8> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            let hex = std::str::from_utf8(&raw[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                bytes.push(b);
                i += 3;
                continue;
            }
        }
        bytes.push(raw[i]);
        i += 1;
    }
    if bytes.contains(&0) {
        illegal_argument("Nul character not allowed")
    }
    if bytes.len() > 1 && bytes.last() == Some(&b'/') {
        bytes.pop();
    }
    Path::new(
        default_file_system(),
        super::bytes_to_path(&bytes).as_os_str(),
    )
}

// =============================================================================
// FileStore
// =============================================================================

struct MountEntry {
    name: String,
    dir: String,
    fstype: String,
    read_only: bool,
}

/// Decodes the octal escapes (`\040`) of a `/proc/self/mounts` field.
fn decode_mount_field(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            if let Some(oct) = field.get(i + 1..i + 4) {
                if let Ok(b) = u8::from_str_radix(oct, 8) {
                    out.push(b);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The mount table, `/proc/self/mounts`, empty where it does not exist.
fn mount_entries() -> Vec<MountEntry> {
    let Ok(text) = std::fs::read_to_string("/proc/self/mounts") else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let name = decode_mount_field(fields.next()?);
            let dir = decode_mount_field(fields.next()?);
            let fstype = decode_mount_field(fields.next()?);
            let opts = fields.next().unwrap_or("");
            Some(MountEntry {
                name,
                dir,
                fstype,
                read_only: opts.split(',').any(|o| o == "ro"),
            })
        })
        .collect()
}

/// `sun.nio.fs.LinuxFileStore`: the mount entry a file lives on.
///
/// **Divergence.** The JDK finds the entry by walking up the file's parents
/// while their device stays the same; this port takes the deepest mount point
/// that is an ancestor of the file's real path.
struct DefaultFileStore {
    mount_dir: PathBuf,
    name: String,
    type_: String,
    dir: String,
    read_only: bool,
}

#[cfg(all(
    unix,
    not(any(
        target_os = "emscripten",
        target_os = "espidf",
        target_os = "fuchsia",
        target_os = "horizon",
        target_os = "redox",
        target_os = "vita",
        target_os = "wasi"
    ))
))]
fn space(dir: &std::path::Path) -> io::Result<(i64, i64, i64, i64)> {
    let stat = rustix::fs::statvfs(dir).map_err(io::Error::from)?;
    let frsize = stat.f_frsize as i64;
    Ok((
        frsize.saturating_mul(stat.f_blocks as i64),
        frsize.saturating_mul(stat.f_bavail as i64),
        frsize.saturating_mul(stat.f_bfree as i64),
        frsize,
    ))
}

/// **Divergence.** Without rustix, the space of a file store cannot be read.
#[cfg(not(all(
    unix,
    not(any(
        target_os = "emscripten",
        target_os = "espidf",
        target_os = "fuchsia",
        target_os = "horizon",
        target_os = "redox",
        target_os = "vita",
        target_os = "wasi"
    ))
)))]
fn space(_dir: &std::path::Path) -> io::Result<(i64, i64, i64, i64)> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

impl FileStore for DefaultFileStore {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn type_(&self) -> String {
        self.type_.clone()
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn get_total_space(&self) -> io::Result<i64> {
        Ok(space(&self.mount_dir)?.0)
    }

    fn get_usable_space(&self) -> io::Result<i64> {
        Ok(space(&self.mount_dir)?.1)
    }

    fn get_unallocated_space(&self) -> io::Result<i64> {
        Ok(space(&self.mount_dir)?.2)
    }

    fn get_block_size(&self) -> io::Result<i64> {
        Ok(space(&self.mount_dir)?.3)
    }

    fn supports_file_attribute_view(&self, name: &str) -> bool {
        let views: &[&str] = if cfg!(target_os = "linux") {
            &["basic", "posix", "unix", "owner", "dos", "user"]
        } else {
            &["basic", "posix", "unix", "owner"]
        };
        views.contains(&name)
    }

    fn get_attribute(&self, attribute: &str) -> io::Result<AttributeValue> {
        match attribute {
            "totalSpace" => Ok(AttributeValue::Long(self.get_total_space()?)),
            "usableSpace" => Ok(AttributeValue::Long(self.get_usable_space()?)),
            "unallocatedSpace" => Ok(AttributeValue::Long(self.get_unallocated_space()?)),
            _ => throw_unchecked(
                "java.lang.UnsupportedOperationException",
                Some(&format!("'{attribute}' not recognized")),
            ),
        }
    }

    /// `mountPoint + " (" + name + ")"`.
    fn store_to_string(&self) -> String {
        format!("{} ({})", self.dir, self.name)
    }
}

// =============================================================================
// FileChannel
// =============================================================================

/// The state a channel shares with the locks taken on it.
struct ChannelInner {
    /// The open file; `None` once closed.
    file: RwLock<Option<File>>,
    /// The locks taken through this channel, invalidated when it closes.
    locks: Mutex<Vec<Weak<AtomicBool>>>,
}

impl ChannelInner {
    fn is_open(&self) -> bool {
        self.file
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }
}

/// `sun.nio.ch.FileChannelImpl`: the channel of the default provider.
pub(crate) struct DefaultFileChannel {
    inner: Arc<ChannelInner>,
    readable: bool,
    writable: bool,
    append: bool,
    /// `positionLock`: serializes the operations on the channel's position.
    position_lock: Mutex<()>,
}

impl DefaultFileChannel {
    fn new(file: File, readable: bool, writable: bool, append: bool) -> DefaultFileChannel {
        DefaultFileChannel {
            inner: Arc::new(ChannelInner {
                file: RwLock::new(Some(file)),
                locks: Mutex::new(Vec::new()),
            }),
            readable,
            writable,
            append,
            position_lock: Mutex::new(()),
        }
    }

    /// Runs `f` on the open file, `ClosedChannelException` once closed.
    fn with_file<T>(&self, f: impl FnOnce(&File) -> io::Result<T>) -> io::Result<T> {
        let guard = self
            .inner
            .file
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match guard.as_ref() {
            Some(file) => f(file).map_err(channel_error),
            None => Err(closed_channel_exception()),
        }
    }

    fn ensure_readable(&self) {
        if !self.readable {
            throw_unchecked(super::NON_READABLE_CHANNEL_EXCEPTION, None)
        }
    }

    fn ensure_writable(&self) {
        if !self.writable {
            throw_unchecked(super::NON_WRITABLE_CHANNEL_EXCEPTION, None)
        }
    }

    fn new_lock(&self, position: i64, size: i64, shared: bool) -> Box<dyn FileLock> {
        let released = Arc::new(AtomicBool::new(false));
        self.inner
            .locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Arc::downgrade(&released));
        Box::new(DefaultFileLock {
            channel: Arc::clone(&self.inner),
            released,
            position,
            size,
            shared,
        })
    }

    fn check_lock_range(position: i64, size: i64, shared: bool) {
        if position < 0 {
            throw_unchecked(
                "java.lang.IllegalArgumentException",
                Some("Negative position"),
            )
        }
        if size < 0 {
            throw_unchecked("java.lang.IllegalArgumentException", Some("Negative size"))
        }
        // **Divergence.** The JDK locks any byte range, shared or exclusive,
        // and refuses a lock overlapping one this JVM holds
        // (`OverlappingFileLockException`); this port locks the whole file
        // only (`position` 0, `size` `Long.MAX_VALUE`), exclusively, the lock
        // `tryLock()` and `lock()` take, and keeps no table of the locks held.
        if position != 0 || size != i64::MAX || shared {
            throw_unchecked(
                "java.lang.UnsupportedOperationException",
                Some("only the whole-file exclusive lock is supported"),
            )
        }
    }
}

impl SeekableByteChannel for DefaultFileChannel {
    fn read(&self, dst: &mut [u8]) -> io::Result<i32> {
        self.ensure_readable();
        let _position = self
            .position_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.with_file(|mut file| {
            if dst.is_empty() {
                return Ok(0);
            }
            let n = file.read(dst)?;
            Ok(if n == 0 { -1 } else { n as i32 })
        })
    }

    fn write(&self, src: &[u8]) -> io::Result<i32> {
        self.ensure_writable();
        let _position = self
            .position_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.with_file(|mut file| {
            file.write_all(src)?;
            Ok(src.len() as i32)
        })
    }

    fn position(&self) -> io::Result<i64> {
        let _position = self
            .position_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.with_file(|mut file| {
            if self.append {
                Ok(file.metadata()?.len() as i64)
            } else {
                Ok(file.stream_position()? as i64)
            }
        })
    }

    fn set_position(&self, new_position: i64) -> io::Result<()> {
        if new_position < 0 {
            throw_unchecked("java.lang.IllegalArgumentException", None)
        }
        let _position = self
            .position_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.with_file(|mut file| {
            file.seek(SeekFrom::Start(new_position as u64))?;
            Ok(())
        })
    }

    fn size(&self) -> io::Result<i64> {
        self.with_file(|file| Ok(file.metadata()?.len() as i64))
    }

    /// `truncate(size)`: shrinks the file to `size` when larger, and moves the
    /// position back to `size` when it is beyond.
    fn truncate(&self, size: i64) -> io::Result<()> {
        if size < 0 {
            throw_unchecked("java.lang.IllegalArgumentException", Some("Negative size"))
        }
        self.ensure_writable();
        let _position = self
            .position_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.with_file(|mut file| {
            let current = file.metadata()?.len() as i64;
            let position = file.stream_position()? as i64;
            if size < current {
                file.set_len(size as u64)?;
            }
            if position > size {
                file.seek(SeekFrom::Start(size as u64))?;
            }
            Ok(())
        })
    }

    fn is_open(&self) -> bool {
        self.inner.is_open()
    }

    /// `AbstractInterruptibleChannel.close()` → `implCloseChannel()`: the
    /// locks taken through the channel are invalidated, and the file closed.
    fn close(&self) -> io::Result<()> {
        let file = self
            .inner
            .file
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if file.is_some() {
            for lock in self
                .inner
                .locks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .drain(..)
            {
                if let Some(released) = lock.upgrade() {
                    released.store(true, Ordering::SeqCst);
                }
            }
        }
        drop(file);
        Ok(())
    }

    fn channel_to_string(&self) -> String {
        format!("sun.nio.ch.FileChannelImpl@{}", identity_hex(self))
    }
}

/// Reads at `position` without moving the file's own position: one `pread`.
fn pread(file: &File, dst: &mut [u8], position: u64) -> io::Result<usize> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_at(file, dst, position)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::FileExt::seek_read(file, dst, position)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, dst, position);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

fn pwrite(file: &File, src: &[u8], position: u64) -> io::Result<usize> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::write_at(file, src, position)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::FileExt::seek_write(file, src, position)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, src, position);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

impl FileChannel for DefaultFileChannel {
    fn read_scattering(&self, dsts: &mut [&mut [u8]]) -> io::Result<i64> {
        let mut total: i64 = 0;
        for dst in dsts.iter_mut() {
            if dst.is_empty() {
                continue;
            }
            let n = SeekableByteChannel::read(self, dst)?;
            if n < 0 {
                return Ok(if total == 0 { -1 } else { total });
            }
            total += i64::from(n);
            if (n as usize) < dst.len() {
                break;
            }
        }
        Ok(total)
    }

    fn write_gathering(&self, srcs: &[&[u8]]) -> io::Result<i64> {
        let mut total: i64 = 0;
        for src in srcs {
            total += i64::from(SeekableByteChannel::write(self, src)?);
        }
        Ok(total)
    }

    /// `force(metaData)`: `fsync` (`fdatasync` without metadata).
    fn force(&self, meta_data: bool) -> io::Result<()> {
        self.with_file(|file| {
            if meta_data {
                file.sync_all()
            } else {
                file.sync_data()
            }
        })
    }

    fn transfer_to(
        &self,
        position: i64,
        count: i64,
        target: &dyn SeekableByteChannel,
    ) -> io::Result<i64> {
        self.ensure_readable();
        let mut done: i64 = 0;
        let mut buf = vec![0u8; 8192];
        while done < count {
            let want = (count - done).min(buf.len() as i64) as usize;
            let n = self.read_at(&mut buf[..want], position + done)?;
            if n <= 0 {
                break;
            }
            let written = target.write(&buf[..n as usize])?;
            done += i64::from(written);
        }
        Ok(done)
    }

    fn transfer_from(
        &self,
        src: &dyn SeekableByteChannel,
        position: i64,
        count: i64,
    ) -> io::Result<i64> {
        self.ensure_writable();
        let mut done: i64 = 0;
        let mut buf = vec![0u8; 8192];
        while done < count {
            let want = (count - done).min(buf.len() as i64) as usize;
            let n = src.read(&mut buf[..want])?;
            if n <= 0 {
                break;
            }
            self.write_at(&buf[..n as usize], position + done)?;
            done += i64::from(n);
        }
        Ok(done)
    }

    fn read_at(&self, dst: &mut [u8], position: i64) -> io::Result<i32> {
        if position < 0 {
            throw_unchecked(
                "java.lang.IllegalArgumentException",
                Some("Negative position"),
            )
        }
        self.ensure_readable();
        self.with_file(|file| {
            if dst.is_empty() {
                return Ok(0);
            }
            let n = pread(file, dst, position as u64)?;
            Ok(if n == 0 { -1 } else { n as i32 })
        })
    }

    fn write_at(&self, src: &[u8], position: i64) -> io::Result<i32> {
        if position < 0 {
            throw_unchecked(
                "java.lang.IllegalArgumentException",
                Some("Negative position"),
            )
        }
        self.ensure_writable();
        self.with_file(|file| Ok(pwrite(file, src, position as u64)? as i32))
    }

    #[cfg(feature = "mmap")]
    fn map_read_only(&self, position: i64, size: i64) -> io::Result<memmap2::Mmap> {
        self.ensure_readable();
        let guard = self
            .inner
            .file
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match guard.as_ref() {
            Some(file) => crate::store::mmap::map_read_only(file, position as u64, size as usize),
            None => Err(closed_channel_exception()),
        }
    }

    fn lock(&self, position: i64, size: i64, shared: bool) -> io::Result<Box<dyn FileLock>> {
        Self::check_lock_range(position, size, shared);
        self.ensure_writable();
        self.with_file(crate::store::lock_channel)?;
        Ok(self.new_lock(position, size, shared))
    }

    fn try_lock(
        &self,
        position: i64,
        size: i64,
        shared: bool,
    ) -> io::Result<Option<Box<dyn FileLock>>> {
        Self::check_lock_range(position, size, shared);
        self.ensure_writable();
        let locked = {
            let guard = self
                .inner
                .file
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match guard.as_ref() {
                Some(file) => crate::store::try_lock_channel(file)?,
                None => return Err(closed_channel_exception()),
            }
        };
        Ok(locked.then(|| self.new_lock(position, size, shared)))
    }
}

/// `sun.nio.ch.FileLockImpl`.
struct DefaultFileLock {
    channel: Arc<ChannelInner>,
    released: Arc<AtomicBool>,
    position: i64,
    size: i64,
    shared: bool,
}

impl FileLock for DefaultFileLock {
    fn position(&self) -> i64 {
        self.position
    }

    fn size(&self) -> i64 {
        self.size
    }

    fn is_shared(&self) -> bool {
        self.shared
    }

    fn is_valid(&self) -> bool {
        !self.released.load(Ordering::SeqCst) && self.channel.is_open()
    }

    /// `release()`: `ClosedChannelException` when the channel is closed.
    fn release(&self) -> io::Result<()> {
        let guard = self
            .channel
            .file
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(file) = guard.as_ref() else {
            return Err(closed_channel_exception());
        };
        if self.released.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        crate::store::release_channel_lock(file)
    }

    /// `FileLock.toString()`: measured against OpenJDK 25,
    /// `sun.nio.ch.FileLockImpl[0:9223372036854775807 exclusive valid]`.
    fn lock_to_string(&self) -> String {
        format!(
            "sun.nio.ch.FileLockImpl[{}:{} {} {}]",
            self.position,
            self.size,
            if self.shared { "shared" } else { "exclusive" },
            if self.is_valid() { "valid" } else { "invalid" }
        )
    }
}

// =============================================================================
// Streams
// =============================================================================

/// `sun.nio.ch.ChannelInputStream` over a default channel.
struct ChannelInputStream {
    channel: Arc<DefaultFileChannel>,
}

impl InputStream for ChannelInputStream {
    fn read_byte(&mut self) -> io::Result<i32> {
        let mut b = [0u8; 1];
        let n = SeekableByteChannel::read(&*self.channel, &mut b)?;
        Ok(if n <= 0 { -1 } else { i32::from(b[0]) })
    }

    fn read(&mut self, b: &mut [u8], off: usize, len: usize) -> io::Result<i32> {
        if len == 0 {
            return Ok(0);
        }
        SeekableByteChannel::read(&*self.channel, &mut b[off..off + len])
    }

    /// `ChannelInputStream.skip(n)` over a seekable channel: the position
    /// moves by `n`, clamped to the file.
    fn skip(&mut self, n: i64) -> io::Result<i64> {
        let pos = self.channel.position()?;
        let new_pos = if n > 0 {
            let size = self.channel.size()?;
            let target = pos.saturating_add(n);
            if target < 0 || target > size {
                size
            } else {
                target
            }
        } else {
            (pos + n).max(0)
        };
        self.channel.set_position(new_pos)?;
        Ok(new_pos - pos)
    }

    fn available(&mut self) -> io::Result<i32> {
        let rem = (self.channel.size()? - self.channel.position()?).max(0);
        Ok(rem.min(i64::from(i32::MAX)) as i32)
    }

    fn close(&mut self) -> io::Result<()> {
        self.channel.close()
    }

    fn mark(&mut self, _readlimit: i32) {}

    fn reset(&mut self) -> io::Result<()> {
        Err(super::io_exception("mark/reset not supported"))
    }

    fn mark_supported(&self) -> bool {
        false
    }

    fn stream_to_string(&self) -> String {
        format!("sun.nio.ch.ChannelInputStream@{}", identity_hex(self))
    }
}

/// `sun.nio.ch.ChannelOutputStream` over a default channel.
struct ChannelOutputStream {
    channel: Arc<DefaultFileChannel>,
}

impl OutputStream for ChannelOutputStream {
    fn write_byte(&mut self, b: i32) -> io::Result<()> {
        SeekableByteChannel::write(&*self.channel, &[b as u8]).map(|_| ())
    }

    fn write(&mut self, b: &[u8], off: usize, len: usize) -> io::Result<()> {
        SeekableByteChannel::write(&*self.channel, &b[off..off + len]).map(|_| ())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        self.channel.close()
    }

    fn stream_to_string(&self) -> String {
        format!("sun.nio.ch.ChannelOutputStream@{}", identity_hex(self))
    }
}

// =============================================================================
// AsynchronousFileChannel
// =============================================================================

/// `sun.nio.ch.SimpleAsynchronousFileChannelImpl`, completing on the calling
/// thread (see [`JavaFuture`]).
struct DefaultAsynchronousFileChannel {
    channel: DefaultFileChannel,
}

impl AsynchronousFileChannel for DefaultAsynchronousFileChannel {
    fn size(&self) -> io::Result<i64> {
        self.channel.size()
    }

    fn truncate(&self, size: i64) -> io::Result<()> {
        self.channel.truncate(size)
    }

    fn force(&self, meta_data: bool) -> io::Result<()> {
        self.channel.force(meta_data)
    }

    fn lock(&self, position: i64, size: i64, shared: bool) -> JavaFuture<Box<dyn FileLock>> {
        JavaFuture::completed(FileChannel::lock(&self.channel, position, size, shared))
    }

    fn lock_with_handler(
        &self,
        position: i64,
        size: i64,
        shared: bool,
        handler: CompletionHandler<Box<dyn FileLock>>,
    ) {
        handler(FileChannel::lock(&self.channel, position, size, shared));
    }

    fn try_lock(
        &self,
        position: i64,
        size: i64,
        shared: bool,
    ) -> io::Result<Option<Box<dyn FileLock>>> {
        FileChannel::try_lock(&self.channel, position, size, shared)
    }

    fn read(&self, dst: &mut [u8], position: i64) -> JavaFuture<i32> {
        JavaFuture::completed(self.channel.read_at(dst, position))
    }

    fn read_with_handler(&self, dst: &mut [u8], position: i64, handler: CompletionHandler<i32>) {
        handler(self.channel.read_at(dst, position));
    }

    fn write(&self, src: &[u8], position: i64) -> JavaFuture<i32> {
        JavaFuture::completed(self.channel.write_at(src, position))
    }

    fn write_with_handler(&self, src: &[u8], position: i64, handler: CompletionHandler<i32>) {
        handler(self.channel.write_at(src, position));
    }

    fn is_open(&self) -> bool {
        self.channel.is_open()
    }

    fn close(&self) -> io::Result<()> {
        self.channel.close()
    }

    fn channel_to_string(&self) -> String {
        format!(
            "sun.nio.ch.SimpleAsynchronousFileChannelImpl@{}",
            identity_hex(self)
        )
    }
}

// =============================================================================
// DirectoryStream
// =============================================================================

/// `sun.nio.fs.UnixDirectoryStream`.
struct DefaultDirectoryStream {
    dir: Path,
    entries: Option<std::fs::ReadDir>,
    filter: DirectoryStreamFilter,
    iterator_obtained: bool,
    closed: bool,
}

impl DirectoryStream for DefaultDirectoryStream {
    fn iterator(&mut self) -> Box<dyn Iterator<Item = io::Result<Path>> + Send + '_> {
        if self.closed {
            throw_unchecked(
                "java.lang.IllegalStateException",
                Some("Directory stream is closed"),
            )
        }
        if self.iterator_obtained {
            throw_unchecked(
                "java.lang.IllegalStateException",
                Some("Iterator already obtained"),
            )
        }
        self.iterator_obtained = true;
        let dir = self.dir.clone();
        let filter = Arc::clone(&self.filter);
        let entries = self.entries.take();
        Box::new(entries.into_iter().flatten().filter_map(move |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => return Some(Err(translate(e, &dir, None))),
            };
            let name = entry.file_name();
            let path = dir.resolve(&Path::new(Arc::clone(dir.get_file_system()), &name));
            match filter(&path) {
                Ok(true) => Some(Ok(path)),
                Ok(false) => None,
                Err(e) => Some(Err(e)),
            }
        }))
    }

    fn close(&mut self) -> io::Result<()> {
        self.closed = true;
        self.entries = None;
        Ok(())
    }

    fn stream_to_string(&self) -> String {
        format!("sun.nio.fs.UnixDirectoryStream@{}", identity_hex(self))
    }
}
