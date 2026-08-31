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
//! so are the error messages.

#![deny(unsafe_code)]

use std::path::Path;

use crate::error::{LuceneError, Result};
use crate::store::{Directory, FSDirectory, LockFactory, NIOFSDirectory, NativeFSLockFactory};

/// The package Lucene qualifies an unqualified directory name with.
///
/// `Directory.class.getPackage().getName()`.
const DIRECTORY_PACKAGE: &str = "org.apache.lucene.store";

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

impl FSDirectoryKind {
    /// Returns the fully-qualified name this kind is spelled with.
    pub fn class_name(&self) -> &'static str {
        match self {
            Self::FSDirectory => "org.apache.lucene.store.FSDirectory",
            Self::NIOFSDirectory => "org.apache.lucene.store.NIOFSDirectory",
            #[cfg(feature = "mmap")]
            Self::MMapDirectory => "org.apache.lucene.store.MMapDirectory",
        }
    }
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
    /// Returns [`LuceneError::IllegalArgument`] when the name is unknown, and
    /// [`LuceneError::Io`] when the directory cannot be opened.
    pub fn new_fs_directory(clazz_name: &str, path: &Path) -> Result<Box<dyn Directory>> {
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
    /// As [`CommandLineUtil::new_fs_directory`].
    pub fn new_fs_directory_with_lock(
        clazz_name: &str,
        path: &Path,
        lf: Box<dyn LockFactory>,
    ) -> Result<Box<dyn Directory>> {
        let kind = Self::load_fs_directory_class(clazz_name)?;
        Self::new_fs_directory_of_kind(kind, path, lf)
    }

    /// Creates a directory of the given kind at `path`.
    ///
    /// Equivalent to
    /// `CommandLineUtil.newFSDirectory(Class, Path, LockFactory)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] when the directory cannot be opened.
    pub fn new_fs_directory_of_kind(
        kind: FSDirectoryKind,
        path: &Path,
        lf: Box<dyn LockFactory>,
    ) -> Result<Box<dyn Directory>> {
        match kind {
            FSDirectoryKind::FSDirectory => {
                Ok(Box::new(FSDirectory::with_lock_factory(path, Some(lf))?))
            }
            FSDirectoryKind::NIOFSDirectory => {
                Ok(Box::new(NIOFSDirectory::with_lock_factory(path, Some(lf))?))
            }
            #[cfg(feature = "mmap")]
            FSDirectoryKind::MMapDirectory => Ok(Box::new(
                crate::store::mmap::MMapDirectory::with_lock_factory(path, Some(lf))?,
            )),
        }
    }

    /// Resolves a name to a [`Directory`] implementation.
    ///
    /// Equivalent to `CommandLineUtil.loadDirectoryClass(String)`. Every
    /// implementation this crate can build from a path is an `FSDirectory`, so
    /// it resolves to the same set as
    /// [`CommandLineUtil::load_fs_directory_class`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the name is unknown.
    pub fn load_directory_class(clazz_name: &str) -> Result<FSDirectoryKind> {
        Self::load_fs_directory_class(clazz_name)
    }

    /// Resolves a name to an `FSDirectory` implementation.
    ///
    /// Equivalent to `CommandLineUtil.loadFSDirectoryClass(String)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the name is empty (Java's
    /// "The FSDirectory implementation must not be null or empty") or unknown
    /// (Java's `ClassNotFoundException`, rewrapped as
    /// "FSDirectory implementation not found").
    pub fn load_fs_directory_class(clazz_name: &str) -> Result<FSDirectoryKind> {
        let adjusted = Self::adjust_directory_class_name(clazz_name)?;
        match adjusted.as_str() {
            "org.apache.lucene.store.FSDirectory" => Ok(FSDirectoryKind::FSDirectory),
            "org.apache.lucene.store.NIOFSDirectory" => Ok(FSDirectoryKind::NIOFSDirectory),
            #[cfg(feature = "mmap")]
            "org.apache.lucene.store.MMapDirectory" => Ok(FSDirectoryKind::MMapDirectory),
            _ => Err(LuceneError::IllegalArgument(format!(
                "FSDirectory implementation not found: {clazz_name}"
            ))),
        }
    }

    /// Qualifies an unqualified directory class name with
    /// `org.apache.lucene.store`.
    ///
    /// Equivalent to the private `CommandLineUtil.adjustDirectoryClassName`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the name is empty or blank.
    pub fn adjust_directory_class_name(clazz_name: &str) -> Result<String> {
        if clazz_name.trim().is_empty() {
            return Err(LuceneError::IllegalArgument(
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
