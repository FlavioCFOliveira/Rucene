//! Command-line helpers ported from
//! `org.apache.lucene.util.CommandLineUtil`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`CommandLineUtil`] | `CommandLineUtil` |
//! | [`FSDirectoryKind`] | the `Class<? extends FSDirectory>` its methods pass around |
//!
//! **Divergence from Lucene 10.5.0.** Java resolves the directory
//! implementation with `Class.forName` and instantiates it through a reflective
//! constructor lookup, so that a command-line tool can name any `FSDirectory`
//! on the classpath. Rust has no runtime class loading: the set of
//! implementations is fixed at compile time, so the lookup becomes the
//! [`FSDirectoryKind`] enum over the `FSDirectory` subclasses this crate ports.
//! The accepted spellings are unchanged — a simple name is qualified with
//! `org.apache.lucene.store.`, exactly as `adjustDirectoryClassName` does — and
//! so are the error messages. The abstract `FSDirectory` resolves, as
//! `Class.forName` resolves it, but cannot be instantiated: its constructor is
//! `protected` (`FSDirectory.java:119`), so `getConstructor(Path.class,
//! LockFactory.class)` finds none.
//!
//! Java tells a name that loads no class (`ClassNotFoundException`) from one
//! that loads a class which is not an `FSDirectory` (`asSubclass` throws
//! `ClassCastException`). This port knows the classes Java can load from the
//! package an unqualified name is qualified with: the 85 classes other than
//! `FSDirectory`, `NIOFSDirectory` and `MMapDirectory` that
//! `lucene-core-10.5.0.jar` holds in `org.apache.lucene.store`, nested classes
//! included. A name of one of them, simple or qualified, is refused as Java
//! refuses it; a qualified name of a class in any other package is reported
//! as not found, where Java would load the class and refuse the cast.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::store::java_nio_file::IntoNioPath;
use crate::store::{Directory, LockFactory, NIOFSDirectory, NativeFSLockFactory};

/// The package Lucene qualifies an unqualified directory name with.
///
/// `Directory.class.getPackage().getName()`.
const DIRECTORY_PACKAGE: &str = "org.apache.lucene.store";

/// The binary names, without the package, of the classes
/// `lucene-core-10.5.0.jar` holds in `org.apache.lucene.store` that are not
/// `FSDirectory` or one of its subclasses: the classes `Class.forName` loads
/// there and `asSubclass(FSDirectory.class)` refuses with a
/// `ClassCastException` (measured on every one of them).
const NON_FS_DIRECTORY_STORE_CLASSES: [&str; 85] = [
    "AlreadyClosedException",
    "BaseDirectory",
    "BufferedChecksum",
    "BufferedChecksumIndexInput",
    "BufferedIndexInput",
    "BufferedIndexInput$1",
    "BufferedIndexInput$SlicedIndexInput",
    "ByteArrayDataInput",
    "ByteArrayDataOutput",
    "ByteBuffersDataInput",
    "ByteBuffersDataOutput",
    "ByteBuffersDataOutput$ByteBufferRecycler",
    "ByteBuffersDirectory",
    "ByteBuffersDirectory$1",
    "ByteBuffersDirectory$FileEntry",
    "ByteBuffersIndexInput",
    "ByteBuffersIndexOutput",
    "ChecksumIndexInput",
    "DataAccessHint",
    "DataInput",
    "DataOutput",
    "DefaultIOContext",
    "Directory",
    "FSDirectory$FSIndexOutput",
    "FSDirectory$FSIndexOutput$1",
    "FSLockFactory",
    "FileDataHint",
    "FileSwitchDirectory",
    "FileTypeHint",
    "FilterDirectory",
    "FilterIndexInput",
    "FilterIndexOutput",
    "FlushInfo",
    "IOContext",
    "IOContext$1",
    "IOContext$2",
    "IOContext$Context",
    "IOContext$FileOpenHint",
    "IndexInput",
    "IndexInput$1",
    "IndexOutput",
    "InputStreamDataInput",
    "Lock",
    "LockFactory",
    "LockObtainFailedException",
    "LockReleaseFailedException",
    "LockStressTest",
    "LockValidatingDirectoryWrapper",
    "LockVerifyServer",
    "LockVerifyServer$1",
    "MMapDirectory$MMapIndexInputProvider",
    "MemorySegmentAccessInput",
    "MemorySegmentIndexInput",
    "MemorySegmentIndexInput$MultiSegmentImpl",
    "MemorySegmentIndexInput$SingleSegmentImpl",
    "MemorySegmentIndexInputProvider",
    "MergeInfo",
    "NIOFSDirectory$NIOFSIndexInput",
    "NRTCachingDirectory",
    "NativeAccess",
    "NativeFSLockFactory",
    "NativeFSLockFactory$NativeFSLock",
    "NoLockFactory",
    "NoLockFactory$NoLock",
    "OutputStreamDataOutput",
    "OutputStreamIndexOutput",
    "OutputStreamIndexOutput$XBufferedOutputStream",
    "PosixNativeAccess",
    "PosixNativeAccess$1",
    "PreloadHint",
    "RandomAccessInput",
    "RateLimitedIndexOutput",
    "RateLimiter",
    "RateLimiter$SimpleRateLimiter",
    "ReadAdvice",
    "ReadOnceHint",
    "RefCountedSharedArena",
    "SimpleFSLockFactory",
    "SimpleFSLockFactory$SimpleFSLock",
    "SingleInstanceLockFactory",
    "SingleInstanceLockFactory$SingleInstanceLock",
    "SleepingLockWrapper",
    "TrackingDirectoryWrapper",
    "VerifyingLockFactory",
    "VerifyingLockFactory$CheckedLock",
];

/// The members of [`NON_FS_DIRECTORY_STORE_CLASSES`] that are interfaces:
/// `Class.toString()`, the message `asSubclass` throws with, spells them
/// `interface <name>` instead of `class <name>` (oracle: `RandomAccessInput`
/// gives `interface org.apache.lucene.store.RandomAccessInput`). Read from
/// `javap` over `lucene-core-10.5.0.jar`.
const STORE_INTERFACES: [&str; 5] = [
    "IOContext",
    "IOContext$FileOpenHint",
    "MMapDirectory$MMapIndexInputProvider",
    "MemorySegmentAccessInput",
    "RandomAccessInput",
];

/// The binary names, without the package, of the `org.apache.lucene.store`
/// classes in `lucene-core-10.5.0.jar` that are `Directory` or extend it, the
/// classes `loadDirectoryClass` returns (read from `javap`; oracle: it loads
/// `ByteBuffersDirectory`, `Directory`, `NRTCachingDirectory`, `FSDirectory`).
const STORE_DIRECTORY_CLASSES: [&str; 12] = [
    "BaseDirectory",
    "ByteBuffersDirectory",
    "Directory",
    "FSDirectory",
    "FileSwitchDirectory",
    "FilterDirectory",
    "LockValidatingDirectoryWrapper",
    "MMapDirectory",
    "NIOFSDirectory",
    "NRTCachingDirectory",
    "SleepingLockWrapper",
    "TrackingDirectoryWrapper",
];

/// Why `Class.forName(adjustDirectoryClassName(clazzName))
/// .asSubclass(FSDirectory.class)`, the body of `loadFSDirectoryClass`, fails.
#[derive(Debug)]
enum LoadFailure {
    /// `adjustDirectoryClassName` refuses an empty or blank name with an
    /// `IllegalArgumentException`.
    IllegalArgument(LuceneError),
    /// `Class.forName` finds no class: `ClassNotFoundException`, whose message
    /// is the adjusted name it looked for.
    ClassNotFound(String),
    /// The class exists but is not an `FSDirectory`: `asSubclass` throws
    /// `ClassCastException`, whose message is `"class " + ` the adjusted name.
    ClassCast(String),
}

impl LoadFailure {
    /// The Java exception the failure stands for, as the cause
    /// `newFSDirectory(String, Path, LockFactory)` attaches to the
    /// `IllegalArgumentException` it rethrows.
    ///
    /// [`LuceneError`] has no reflective exception, so `ClassNotFoundException`
    /// and `ClassCastException` are [`LuceneErrorKind::Other`] carrying Java's
    /// message — `org.apache.lucene.store.Nonexistent` and `class
    /// org.apache.lucene.store.ByteBuffersDirectory` (observed with the 10.5.0
    /// oracle) — as the `NoSuchMethodException` of
    /// [`CommandLineUtil::new_fs_directory_of_kind`] is.
    fn into_cause(self) -> Arc<LuceneError> {
        Arc::new(self.into_error())
    }

    /// The Java exception the failure stands for (see
    /// [`into_cause`](Self::into_cause)).
    fn into_error(self) -> LuceneError {
        match self {
            Self::IllegalArgument(error) => error,
            Self::ClassNotFound(name) => LuceneError::from(LuceneErrorKind::Other(name)),
            Self::ClassCast(name) => {
                LuceneError::from(LuceneErrorKind::Other(class_to_string(&name)))
            }
        }
    }
}

/// `Class.toString()` of a store class: `interface <name>` for an interface,
/// `class <name>` otherwise (`Class.java`), the message of the
/// `ClassCastException` `asSubclass` throws.
fn class_to_string(name: &str) -> String {
    let is_interface = name
        .strip_prefix(DIRECTORY_PACKAGE)
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|simple| STORE_INTERFACES.contains(&simple));
    if is_interface {
        format!("interface {name}")
    } else {
        format!("class {name}")
    }
}

/// Which `FSDirectory` implementation a class name selects.
///
/// Stands in for the `Class<? extends FSDirectory>` that Lucene's
/// `loadFSDirectoryClass` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FSDirectoryKind {
    /// `org.apache.lucene.store.FSDirectory`.
    FSDirectory,
    /// `org.apache.lucene.store.NIOFSDirectory`.
    NIOFSDirectory,
    /// `org.apache.lucene.store.MMapDirectory`.
    #[cfg(feature = "mmap")]
    MMapDirectory,
}

/// Helpers for command-line tools.
///
/// Port of `org.apache.lucene.util.CommandLineUtil`.
pub struct CommandLineUtil;

impl CommandLineUtil {
    /// Creates a directory of the named implementation at `path`, with the
    /// default lock factory.
    ///
    /// Equivalent to `CommandLineUtil.newFSDirectory(String, Path)`.
    ///
    /// # Errors
    ///
    /// As [`CommandLineUtil::new_fs_directory_with_lock`].
    pub fn new_fs_directory(
        clazz_name: &str,
        path: impl IntoNioPath,
    ) -> Result<Box<dyn Directory>> {
        Self::new_fs_directory_with_lock(clazz_name, path, Box::new(NativeFSLockFactory))
    }

    /// Creates a directory of the named implementation at `path`, with the
    /// supplied lock factory.
    ///
    /// Equivalent to
    /// `CommandLineUtil.newFSDirectory(String, Path, LockFactory)`.
    ///
    /// # Errors
    ///
    /// Returns an `IllegalArgumentException` in every case, as Java's `catch`
    /// clauses rethrow each failure as one (`CommandLineUtil.java:52-69`),
    /// naming the class as it was given:
    ///
    /// - a name that loads no class: `ClassNotFoundException`, rethrown as
    ///   `"FSDirectory implementation not found: " + clazzName` (`:56-58`);
    /// - a class that is not an `FSDirectory`, such as `ByteBuffersDirectory`:
    ///   `ClassCastException`, rethrown as `clazzName + " is not a FSDirectory
    ///   implementation"` (`:59-61`);
    /// - the abstract `FSDirectory`, whose constructor cannot be found:
    ///   `NoSuchMethodException`, rethrown as `clazzName + " constructor with
    ///   Path as parameter not found"` (`:62-65`);
    /// - an empty or blank name, which `adjustDirectoryClassName` refuses with
    ///   an `IllegalArgumentException`, and a constructor that fails — the
    ///   directory cannot be created or canonicalized —, which reflection wraps
    ///   in an `InvocationTargetException`: both reach `catch (Exception e)`,
    ///   rethrown as `"Error creating " + clazzName + " instance"` (`:66-67`).
    ///
    /// Each is [`LuceneErrorKind::IllegalArgument`], carrying the
    /// exception Java attaches as the cause (`CommandLineUtil.java:56-67`):
    /// the `ClassNotFoundException` and `ClassCastException`, as
    /// [`LuceneErrorKind::Other`] with Java's message (the adjusted name, and
    /// `"class "` followed by it); the `NoSuchMethodException` and the
    /// `InvocationTargetException` of
    /// [`new_fs_directory_of_kind`](Self::new_fs_directory_of_kind); and the
    /// `IllegalArgumentException` of
    /// [`adjust_directory_class_name`](Self::adjust_directory_class_name).
    pub fn new_fs_directory_with_lock(
        clazz_name: &str,
        path: impl IntoNioPath,
        lf: Box<dyn LockFactory>,
    ) -> Result<Box<dyn Directory>> {
        // `final Class<? extends FSDirectory> clazz = loadFSDirectoryClass(clazzName);`
        let kind = match Self::for_name_as_fs_directory(clazz_name) {
            Ok(kind) => kind,
            Err(failure @ LoadFailure::ClassNotFound(_)) => {
                return Err(LuceneError::illegal_argument_with_cause(
                    format!("FSDirectory implementation not found: {clazz_name}"),
                    failure.into_cause(),
                ));
            }
            Err(failure @ LoadFailure::ClassCast(_)) => {
                return Err(LuceneError::illegal_argument_with_cause(
                    format!("{clazz_name} is not a FSDirectory implementation"),
                    failure.into_cause(),
                ));
            }
            Err(failure @ LoadFailure::IllegalArgument(_)) => {
                return Err(Self::error_creating_instance(
                    clazz_name,
                    failure.into_cause(),
                ));
            }
        };
        // `return newFSDirectory(clazz, path, lf);`
        if let Err(no_such_method) = Self::get_constructor(kind) {
            return Err(LuceneError::illegal_argument_with_cause(
                format!("{clazz_name} constructor with Path as parameter not found"),
                Arc::new(no_such_method),
            ));
        }
        Self::new_fs_directory_of_kind(kind, path, lf)
            .map_err(|invocation| Self::error_creating_instance(clazz_name, Arc::new(invocation)))
    }

    /// `new IllegalArgumentException("Error creating " + clazzName + "
    /// instance", e)`, what `catch (Exception e)` throws
    /// (`CommandLineUtil.java:66-67`).
    fn error_creating_instance(clazz_name: &str, cause: Arc<LuceneError>) -> LuceneError {
        LuceneError::illegal_argument_with_cause(
            format!("Error creating {clazz_name} instance"),
            cause,
        )
    }

    /// Looks up the public `(Path, LockFactory)` constructor of `kind`.
    ///
    /// Equivalent to `clazz.getConstructor(Path.class, LockFactory.class)`
    /// (`CommandLineUtil.java:141`). `getConstructor` returns only public
    /// constructors, and `FSDirectory`'s is `protected` (`FSDirectory.java:119`),
    /// so for it the lookup fails.
    ///
    /// # Errors
    ///
    /// For [`FSDirectoryKind::FSDirectory`], the `NoSuchMethodException` Java
    /// throws, whose message names the missing constructor; [`LuceneError`]
    /// has no reflective exception, so it is [`LuceneErrorKind::Other`].
    fn get_constructor(kind: FSDirectoryKind) -> Result<()> {
        match kind {
            FSDirectoryKind::FSDirectory => Err(Self::no_fs_directory_constructor()),
            _ => Ok(()),
        }
    }

    /// The `NoSuchMethodException` `getConstructor` throws for `FSDirectory`,
    /// whose message names the constructor it looked for.
    fn no_fs_directory_constructor() -> LuceneError {
        LuceneError::from(LuceneErrorKind::Other(
            "org.apache.lucene.store.FSDirectory.<init>(java.nio.file.Path,\
             org.apache.lucene.store.LockFactory)"
                .to_string(),
        ))
    }

    /// Creates a directory of the given kind at `path`.
    ///
    /// Equivalent to
    /// `CommandLineUtil.newFSDirectory(Class, Path, LockFactory)`
    /// (`CommandLineUtil.java:137-143`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::Other`] in both of Java's failures:
    ///
    /// - for [`FSDirectoryKind::FSDirectory`], which has no public constructor
    ///   (`FSDirectory.java:119`), the message of the `NoSuchMethodException`
    ///   `getConstructor` throws;
    /// - when the constructor fails — the directory cannot be created or
    ///   canonicalized —, the `InvocationTargetException` `newInstance` wraps
    ///   that failure in (`Constructor.newInstance`): its message is the
    ///   failure's text.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `InvocationTargetException`
    /// has no message (`getMessage()` is `null`) and carries the constructor's
    /// exception as its cause (`getCause()`); [`LuceneError`] has no reflective
    /// exception and [`LuceneErrorKind::Other`] holds one message and no cause, so
    /// the cause's text is that message.
    ///
    /// `path` keeps its filesystem (a [`nio::Path`](crate::store::java_nio_file::Path)
    /// of a mock filesystem stays one), as Java's `Path` argument does.
    pub fn new_fs_directory_of_kind(
        kind: FSDirectoryKind,
        path: impl IntoNioPath,
        lf: Box<dyn LockFactory>,
    ) -> Result<Box<dyn Directory>> {
        // `Constructor ctor = clazz.getConstructor(Path.class,
        // LockFactory.class); return ctor.newInstance(path, lf);`
        Self::get_constructor(kind)?;
        let instance: Result<Box<dyn Directory>> = match kind {
            FSDirectoryKind::FSDirectory => return Err(Self::no_fs_directory_constructor()),
            FSDirectoryKind::NIOFSDirectory => NIOFSDirectory::with_lock_factory(path, lf)
                .map(|directory| Box::new(directory) as Box<dyn Directory>),
            #[cfg(feature = "mmap")]
            FSDirectoryKind::MMapDirectory => {
                crate::store::mmap::MMapDirectory::with_lock_factory(path, lf)
                    .map(|directory| Box::new(directory) as Box<dyn Directory>)
            }
        };
        // `newInstance` wraps what the constructor throws in an
        // `InvocationTargetException`.
        instance.map_err(|cause| LuceneError::from(LuceneErrorKind::Other(cause.to_string())))
    }

    /// Resolves a name to a [`Directory`] class.
    ///
    /// Equivalent to `CommandLineUtil.loadDirectoryClass(String)`
    /// (`CommandLineUtil.java:76-79`): `Class.forName(adjustDirectoryClassName(
    /// clazzName)).asSubclass(Directory.class)`. Unlike
    /// [`load_fs_directory_class`](Self::load_fs_directory_class) it accepts
    /// every `Directory`, `ByteBuffersDirectory` and `NRTCachingDirectory`
    /// included.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns a `Class`; Rust has no
    /// class objects, so the class is returned as its binary name,
    /// `Class.getName()`.
    ///
    /// # Errors
    ///
    /// The exceptions Java throws: [`LuceneErrorKind::IllegalArgument`] for an
    /// empty or blank name (`adjustDirectoryClassName`); for an unknown class
    /// the `ClassNotFoundException`, and for a store class that is not a
    /// `Directory` the `ClassCastException`, both [`LuceneErrorKind::Other`]
    /// with Java's message (the adjusted name; `class <name>` or
    /// `interface <name>`), since [`LuceneError`] has no reflective
    /// exception.
    pub fn load_directory_class(clazz_name: &str) -> Result<String> {
        let adjusted = Self::adjust_directory_class_name(clazz_name)?;
        let simple = adjusted
            .strip_prefix(DIRECTORY_PACKAGE)
            .and_then(|rest| rest.strip_prefix('.'));
        match simple {
            Some(simple) if STORE_DIRECTORY_CLASSES.contains(&simple) => Ok(adjusted),
            Some(simple) if NON_FS_DIRECTORY_STORE_CLASSES.contains(&simple) => {
                Err(LoadFailure::ClassCast(adjusted).into_error())
            }
            _ => Err(LoadFailure::ClassNotFound(adjusted).into_error()),
        }
    }

    /// Resolves a name to an `FSDirectory` implementation.
    ///
    /// Equivalent to `CommandLineUtil.loadFSDirectoryClass(String)`
    /// (`CommandLineUtil.java:87-93`).
    ///
    /// # Errors
    ///
    /// The exceptions Java throws: [`LuceneErrorKind::IllegalArgument`] for an
    /// empty or blank name (`adjustDirectoryClassName`); for an unknown class
    /// the `ClassNotFoundException`, and for a store class that is not an
    /// `FSDirectory` the `ClassCastException`, both [`LuceneErrorKind::Other`]
    /// with Java's message, as for
    /// [`load_directory_class`](Self::load_directory_class). The
    /// "FSDirectory implementation not found" text is
    /// `newFSDirectory(String, Path, LockFactory)`'s wrapping, not this
    /// method's.
    pub fn load_fs_directory_class(clazz_name: &str) -> Result<FSDirectoryKind> {
        Self::for_name_as_fs_directory(clazz_name).map_err(LoadFailure::into_error)
    }

    /// `Class.forName(adjustDirectoryClassName(clazzName))
    /// .asSubclass(FSDirectory.class)` (`CommandLineUtil.java:92`), telling
    /// its failures apart as the `catch` clauses of `newFSDirectory(String,
    /// Path, LockFactory)` do.
    fn for_name_as_fs_directory(
        clazz_name: &str,
    ) -> std::result::Result<FSDirectoryKind, LoadFailure> {
        let adjusted =
            Self::adjust_directory_class_name(clazz_name).map_err(LoadFailure::IllegalArgument)?;
        match adjusted.as_str() {
            "org.apache.lucene.store.FSDirectory" => Ok(FSDirectoryKind::FSDirectory),
            "org.apache.lucene.store.NIOFSDirectory" => Ok(FSDirectoryKind::NIOFSDirectory),
            #[cfg(feature = "mmap")]
            "org.apache.lucene.store.MMapDirectory" => Ok(FSDirectoryKind::MMapDirectory),
            other => {
                let in_store_package = other
                    .strip_prefix(DIRECTORY_PACKAGE)
                    .and_then(|rest| rest.strip_prefix('.'))
                    .is_some_and(|simple| NON_FS_DIRECTORY_STORE_CLASSES.contains(&simple));
                Err(if in_store_package {
                    LoadFailure::ClassCast(other.to_string())
                } else {
                    LoadFailure::ClassNotFound(other.to_string())
                })
            }
        }
    }

    /// Qualifies an unqualified directory class name with
    /// `org.apache.lucene.store`.
    ///
    /// Equivalent to the private `CommandLineUtil.adjustDirectoryClassName`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the name is empty or blank
    /// as Java's `String.trim()` sees it: only characters up to U+0020 are
    /// trimmed, so a name of non-breaking or em spaces is not blank (oracle:
    /// `"\u{a0}"` gives `ClassNotFoundException: org.apache.lucene.store.\u{a0}`).
    pub fn adjust_directory_class_name(clazz_name: &str) -> Result<String> {
        if clazz_name.trim_matches(|c: char| c <= '\u{20}').is_empty() {
            return Err(LuceneError::illegal_argument(
                "The FSDirectory implementation must not be null or empty".to_string(),
            ));
        }
        if !clazz_name.contains('.') {
            // Not fully qualified: assume the store package.
            return Ok(format!("{DIRECTORY_PACKAGE}.{clazz_name}"));
        }
        Ok(clazz_name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `newFSDirectory("FSDirectory", path)` resolves the abstract class, as
    /// `Class.forName` does, but cannot instantiate it: its constructor is
    /// `protected`, `getConstructor` throws `NoSuchMethodException`, and the
    /// `String` overload rethrows it as `IllegalArgumentException(clazzName +
    /// " constructor with Path as parameter not found")`, naming the class as
    /// given; nothing is created. Java oracle (`oracle9/Probe23.java cli`):
    /// "new FSDirectory: IllegalArgumentException: FSDirectory constructor
    /// with Path as parameter not found", "new qualified FSDirectory: ...
    /// org.apache.lucene.store.FSDirectory constructor with Path as parameter
    /// not found", "load FSDirectory: ok org.apache.lucene.store.FSDirectory",
    /// "new class FSDirectory: NoSuchMethodException:
    /// org.apache.lucene.store.FSDirectory.<init>(java.nio.file.Path,org.apache.lucene.store.LockFactory)",
    /// "c1 exists: false", "new NIOFSDirectory: ok NIOFSDirectory".
    #[test]
    fn the_abstract_fs_directory_loads_but_cannot_be_instantiated() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["FSDirectory", "org.apache.lucene.store.FSDirectory"] {
            match CommandLineUtil::new_fs_directory(name, &tmp.path().join("c1")) {
                // The cause is the `NoSuchMethodException` (`PassAOracle`,
                // section 5).
                Err(LuceneError {
                    kind:
                        LuceneErrorKind::IllegalArgument {
                            message,
                            cause: Some(cause),
                        },
                    ..
                }) => {
                    assert_eq!(
                        message,
                        format!("{name} constructor with Path as parameter not found")
                    );
                    assert!(
                        matches!(&*cause, LuceneError { kind: LuceneErrorKind::Other(m), .. } if m ==
                            "org.apache.lucene.store.FSDirectory.<init>(java.nio.file.Path,\
                             org.apache.lucene.store.LockFactory)"),
                        "{cause:?}"
                    );
                }
                other => panic!(
                    "expected IllegalArgumentException for {name}, got {:?}",
                    other.err()
                ),
            }
        }
        assert!(!tmp.path().join("c1").exists());

        assert_eq!(
            CommandLineUtil::load_fs_directory_class("FSDirectory").unwrap(),
            FSDirectoryKind::FSDirectory
        );
        match CommandLineUtil::new_fs_directory_of_kind(
            FSDirectoryKind::FSDirectory,
            &tmp.path().join("c3"),
            Box::new(NativeFSLockFactory),
        ) {
            Err(LuceneError {
                kind: LuceneErrorKind::Other(message),
                ..
            }) => assert_eq!(
                message,
                "org.apache.lucene.store.FSDirectory.<init>(java.nio.file.Path,\
                 org.apache.lucene.store.LockFactory)"
            ),
            other => panic!("expected NoSuchMethodException, got {:?}", other.err()),
        }

        let nio = CommandLineUtil::new_fs_directory("NIOFSDirectory", &tmp.path().join("c4"))
            .expect("NIOFSDirectory has a public (Path, LockFactory) constructor");
        assert_eq!(nio.directory_type_name(), "NIOFSDirectory");
    }

    /// A constructor that fails — here because the path is a regular file —
    /// reaches `newFSDirectory(String, Path, LockFactory)` as an
    /// `InvocationTargetException`, which `catch (Exception e)` rethrows as
    /// `IllegalArgumentException("Error creating " + clazzName + " instance",
    /// e)` (`CommandLineUtil.java:66-67`), naming the class as given. Java
    /// oracle (`aud/cli/Cli.java`, `fp/FixOracle.java`): "cli ctor fails:
    /// IllegalArgumentException: Error creating NIOFSDirectory instance |
    /// cause=InvocationTargetException", "cli ctor fails qualified: ...
    /// Error creating org.apache.lucene.store.NIOFSDirectory instance".
    #[test]
    fn a_failing_constructor_is_rethrown_as_an_illegal_argument() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("afile");
        std::fs::write(&file, [1u8]).unwrap();
        let invocation =
            match NIOFSDirectory::with_lock_factory(&file, Box::new(NativeFSLockFactory)) {
                Ok(_) => panic!("a regular file cannot be opened as a directory"),
                Err(error) => error.to_string(),
            };
        for name in ["NIOFSDirectory", "org.apache.lucene.store.NIOFSDirectory"] {
            match CommandLineUtil::new_fs_directory(name, &file) {
                // The cause is the `InvocationTargetException`, which this
                // port renders as `Other` carrying the constructor's failure.
                Err(LuceneError {
                    kind:
                        LuceneErrorKind::IllegalArgument {
                            message,
                            cause: Some(cause),
                        },
                    ..
                }) => {
                    assert_eq!(message, format!("Error creating {name} instance"));
                    assert!(
                        matches!(&*cause, LuceneError { kind: LuceneErrorKind::Other(m), .. } if *m == invocation),
                        "{cause:?}"
                    );
                }
                other => panic!(
                    "expected IllegalArgumentException for {name}, got {:?}",
                    other.err()
                ),
            }
        }
    }

    /// `newFSDirectory(Class, Path, LockFactory)` lets the
    /// `InvocationTargetException` of a failing constructor through; its
    /// cause is the constructor's own exception. Java oracle
    /// (`rp/oracle/ResidualOracle.java`): "class NIOFSDirectory over a file
    /// => java.lang.reflect.InvocationTargetException | message=null |
    /// cause=java.nio.file.FileAlreadyExistsException: <path>", and the same
    /// for `MMapDirectory`. Here it is [`LuceneErrorKind::Other`] with the
    /// constructor's error as its text, not that error itself.
    #[test]
    fn a_failing_constructor_of_a_kind_is_an_invocation_target_exception() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("afile");
        std::fs::write(&file, [1u8]).unwrap();
        let cause = |error: Option<LuceneError>| {
            error
                .expect("a regular file cannot be opened as a directory")
                .to_string()
        };
        let kinds = [
            (
                FSDirectoryKind::NIOFSDirectory,
                cause(
                    NIOFSDirectory::with_lock_factory(&file, Box::new(NativeFSLockFactory)).err(),
                ),
            ),
            #[cfg(feature = "mmap")]
            (
                FSDirectoryKind::MMapDirectory,
                cause(
                    crate::store::mmap::MMapDirectory::with_lock_factory(
                        &file,
                        Box::new(NativeFSLockFactory),
                    )
                    .err(),
                ),
            ),
        ];
        for (kind, cause) in kinds {
            match CommandLineUtil::new_fs_directory_of_kind(
                kind,
                &file,
                Box::new(NativeFSLockFactory),
            ) {
                Err(LuceneError {
                    kind: LuceneErrorKind::Other(message),
                    ..
                }) => assert_eq!(message, cause, "{kind:?}"),
                other => panic!(
                    "expected InvocationTargetException for {kind:?}, got {:?}",
                    other.err()
                ),
            }
        }
    }

    /// An empty or blank name makes `adjustDirectoryClassName` throw an
    /// `IllegalArgumentException`, which is no `ClassNotFoundException`,
    /// `ClassCastException` or `NoSuchMethodException`, so `catch (Exception
    /// e)` rethrows it as `"Error creating " + clazzName + " instance"`
    /// (`CommandLineUtil.java:66-67`), the name kept as given; nothing is
    /// created. `loadFSDirectoryClass` itself throws the inner exception.
    /// Java oracle (`rp/oracle/ResidualOracle.java`): `new ""` =>
    /// "IllegalArgumentException | message=Error creating  instance |
    /// cause=java.lang.IllegalArgumentException: The FSDirectory
    /// implementation must not be null or empty"; `new "   "` => "Error
    /// creating     instance".
    #[test]
    fn an_empty_or_blank_name_is_an_error_creating_the_instance() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["", "   "] {
            match CommandLineUtil::new_fs_directory(name, &tmp.path().join("e")) {
                Err(LuceneError {
                    kind:
                        LuceneErrorKind::IllegalArgument {
                            message,
                            cause: Some(cause),
                        },
                    ..
                }) => {
                    assert_eq!(message, format!("Error creating {name} instance"));
                    assert!(
                        matches!(&*cause, LuceneError { kind: LuceneErrorKind::IllegalArgument { message: m, .. }, .. } if m ==
                            "The FSDirectory implementation must not be null or empty"),
                        "{cause:?}"
                    );
                }
                other => panic!(
                    "expected IllegalArgumentException for {name:?}, got {:?}",
                    other.err()
                ),
            }
            match CommandLineUtil::load_fs_directory_class(name) {
                Err(LuceneError {
                    kind: LuceneErrorKind::IllegalArgument { message, .. },
                    ..
                }) => assert_eq!(
                    message,
                    "The FSDirectory implementation must not be null or empty"
                ),
                other => panic!("expected IllegalArgumentException for {name:?}, got {other:?}"),
            }
        }
        assert_eq!(
            CommandLineUtil::new_fs_directory("", &tmp.path().join("e"))
                .err()
                .map(|error| error.to_string()),
            Some("illegal argument: Error creating  instance".to_string())
        );
        assert!(!tmp.path().join("e").exists());
    }

    /// A name that loads a class which is not an `FSDirectory` makes
    /// `asSubclass(FSDirectory.class)` throw a `ClassCastException`, rethrown
    /// as `clazzName + " is not a FSDirectory implementation"`
    /// (`CommandLineUtil.java:59-61`); a name that loads nothing stays
    /// "FSDirectory implementation not found: " + clazzName (`:56-58`). Java
    /// oracle (`rp/oracle/ResidualOracle.java`): "ByteBuffersDirectory is not
    /// a FSDirectory implementation | cause=java.lang.ClassCastException",
    /// the qualified name likewise, "FSDirectory implementation not found:
    /// Foo | cause=java.lang.ClassNotFoundException", and the
    /// `ClassCastException` text for each of the 85 non-`FSDirectory` classes
    /// of `org.apache.lucene.store` in `lucene-core-10.5.0.jar`.
    /// `loadDirectoryClass` and `loadFSDirectoryClass` throw what
    /// `Class.forName(...).asSubclass(...)` throws — `ClassNotFoundException`
    /// (the adjusted name) and `ClassCastException` (`class <name>`, or
    /// `interface <name>` for an interface) — and `loadDirectoryClass` accepts
    /// every `Directory`; `adjustDirectoryClassName` trims only characters up
    /// to U+0020, as `String.trim()`. Every expectation is the Java oracle's
    /// (`oracle/b3/Cli.java`).
    #[test]
    fn the_load_methods_throw_what_class_for_name_throws() {
        for name in [
            "ByteBuffersDirectory",
            "Directory",
            "NRTCachingDirectory",
            "FSDirectory",
        ] {
            assert_eq!(
                CommandLineUtil::load_directory_class(name).unwrap(),
                format!("org.apache.lucene.store.{name}")
            );
        }
        let other = |result: Result<String>| match result {
            Err(LuceneError {
                kind: LuceneErrorKind::Other(message),
                ..
            }) => message,
            other => panic!("{other:?}"),
        };
        let other_fs = |result: Result<FSDirectoryKind>| match result {
            Err(LuceneError {
                kind: LuceneErrorKind::Other(message),
                ..
            }) => message,
            other => panic!("{other:?}"),
        };
        for (name, message) in [
            (
                "ByteBuffersDirectory",
                "class org.apache.lucene.store.ByteBuffersDirectory",
            ),
            ("Directory", "class org.apache.lucene.store.Directory"),
            ("Lock", "class org.apache.lucene.store.Lock"),
            (
                "RandomAccessInput",
                "interface org.apache.lucene.store.RandomAccessInput",
            ),
            ("Nonexistent", "org.apache.lucene.store.Nonexistent"),
            ("\u{a0}", "org.apache.lucene.store.\u{a0}"),
            ("\u{2003} ", "org.apache.lucene.store.\u{2003} "),
        ] {
            assert_eq!(
                other_fs(CommandLineUtil::load_fs_directory_class(name)),
                message,
                "{name:?}"
            );
        }
        for (name, message) in [
            ("Lock", "class org.apache.lucene.store.Lock"),
            (
                "RandomAccessInput",
                "interface org.apache.lucene.store.RandomAccessInput",
            ),
            ("ReadAdvice", "class org.apache.lucene.store.ReadAdvice"),
            ("Nonexistent", "org.apache.lucene.store.Nonexistent"),
        ] {
            assert_eq!(
                other(CommandLineUtil::load_directory_class(name)),
                message,
                "{name:?}"
            );
        }
        assert!(matches!(
            CommandLineUtil::load_directory_class(" "),
            Err(LuceneError { kind: LuceneErrorKind::IllegalArgument { message: m, .. }, .. })
                if m == "The FSDirectory implementation must not be null or empty"
        ));
    }

    #[test]
    fn a_class_that_is_not_an_fs_directory_is_refused_as_lucene() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("n");
        // The cause: `ClassCastException("class " + name)` or
        // `ClassNotFoundException(name)`, the name adjusted (`PassAOracle`,
        // section 5).
        let refusal = |name: &str| match CommandLineUtil::new_fs_directory(name, &target) {
            Err(LuceneError {
                kind:
                    LuceneErrorKind::IllegalArgument {
                        message,
                        cause: Some(cause),
                    },
                ..
            }) => {
                let adjusted = CommandLineUtil::adjust_directory_class_name(name).unwrap();
                match &*cause {
                    LuceneError {
                        kind: LuceneErrorKind::Other(m),
                        ..
                    } if *m == adjusted
                        || *m == format!("class {adjusted}")
                        || *m == format!("interface {adjusted}") => {}
                    other => panic!("unexpected cause for {name}: {other:?}"),
                }
                message
            }
            other => panic!(
                "expected IllegalArgumentException for {name}, got {:?}",
                other.err()
            ),
        };
        let cause = |name: &str| match CommandLineUtil::new_fs_directory(name, &target) {
            Err(LuceneError {
                kind:
                    LuceneErrorKind::IllegalArgument {
                        cause: Some(cause), ..
                    },
                ..
            }) => cause.to_string(),
            other => panic!("expected a cause for {name}, got {:?}", other.err()),
        };
        assert_eq!(
            cause("ByteBuffersDirectory"),
            "class org.apache.lucene.store.ByteBuffersDirectory"
        );
        assert_eq!(cause("Nonexistent"), "org.apache.lucene.store.Nonexistent");
        assert_eq!(
            refusal("ByteBuffersDirectory"),
            "ByteBuffersDirectory is not a FSDirectory implementation"
        );
        assert_eq!(
            refusal("org.apache.lucene.store.ByteBuffersDirectory"),
            "org.apache.lucene.store.ByteBuffersDirectory is not a FSDirectory implementation"
        );
        for simple in NON_FS_DIRECTORY_STORE_CLASSES {
            let qualified = format!("{DIRECTORY_PACKAGE}.{simple}");
            for name in [simple, qualified.as_str()] {
                assert_eq!(
                    refusal(name),
                    format!("{name} is not a FSDirectory implementation")
                );
            }
        }
        for name in [
            "Foo",
            "org.apache.lucene.store.Foo",
            "org.apache.lucene.storeByteBuffersDirectory",
        ] {
            assert_eq!(
                refusal(name),
                format!("FSDirectory implementation not found: {name}")
            );
        }
        assert!(!target.exists());
    }
}
