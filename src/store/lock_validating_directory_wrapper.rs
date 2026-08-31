//! Best-effort lock validation in front of destructive directory operations.
//!
//! Ported from `org.apache.lucene.store.LockValidatingDirectoryWrapper`.

use std::collections::HashSet;
use std::sync::Arc;

use super::{Directory, FilterDirectory, IOContext, IndexInput, IndexOutput, Lock};
use crate::error::Result;

/// A [`Directory`] that makes a best-effort check that a provided [`Lock`] is
/// still valid before any destructive filesystem operation.
///
/// Equivalent to `org.apache.lucene.store.LockValidatingDirectoryWrapper`. The
/// guarded operations are exactly the ones Lucene guards: `deleteFile`,
/// `createOutput`, `copyFrom`, `rename`, `syncMetaData` and `sync`.
///
/// # Divergence from Lucene 10.5.0
///
/// Java extends `FilterDirectory` and keeps a bare `Lock` reference owned by
/// the caller (`IndexWriter`'s write lock). Rust has no inheritance and no bare
/// references across owners, so this type *contains* a [`FilterDirectory`] and
/// holds the write lock behind an [`Arc`], which is the closest safe model of
/// the shared, caller-owned lock. Only [`Lock::ensure_valid`] is ever called on
/// it, exactly as in Java; the wrapper never releases the lock.
pub struct LockValidatingDirectoryWrapper {
    inner: FilterDirectory,
    write_lock: Arc<dyn Lock>,
}

impl LockValidatingDirectoryWrapper {
    /// Wraps `input` and validates `write_lock` before destructive operations.
    ///
    /// Equivalent to `LockValidatingDirectoryWrapper(Directory, Lock)`.
    pub fn new(input: Box<dyn Directory>, write_lock: Arc<dyn Lock>) -> Self {
        Self {
            inner: FilterDirectory::new(input),
            write_lock,
        }
    }

    /// Returns the wrapped directory.
    ///
    /// Equivalent to `FilterDirectory.getDelegate()`.
    pub fn get_delegate(&self) -> &dyn Directory {
        self.inner.get_delegate()
    }

    /// Returns the write lock validated before each destructive operation.
    pub fn write_lock(&self) -> &Arc<dyn Lock> {
        &self.write_lock
    }
}

impl Directory for LockValidatingDirectoryWrapper {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.write_lock.ensure_valid()?;
        self.inner.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.write_lock.ensure_valid()?;
        self.inner.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.write_lock.ensure_valid()?;
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.write_lock.ensure_valid()?;
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.write_lock.ensure_valid()?;
        self.inner.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn copy_from(
        &self,
        from: &dyn Directory,
        src: &str,
        dest: &str,
        context: &dyn IOContext,
    ) -> Result<()> {
        self.write_lock.ensure_valid()?;
        self.inner.copy_from(from, src, dest, context)
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }

    fn directory_type_name(&self) -> &'static str {
        "LockValidatingDirectoryWrapper"
    }

    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

impl std::fmt::Display for LockValidatingDirectoryWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LockValidatingDirectoryWrapper({})",
            self.inner.get_delegate().directory_type_name()
        )
    }
}

impl std::fmt::Debug for LockValidatingDirectoryWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockValidatingDirectoryWrapper")
            .field("inner", &self.inner.get_delegate().directory_type_name())
            .finish()
    }
}
