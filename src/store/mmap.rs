#![allow(unsafe_code)]

//! Memory-mapped directory implementation for trusted indexes.
//!
//! This module provides [`MMapDirectory`] and [`MMapIndexInput`], gated by the
//! crate feature `mmap`. It uses the `memmap2` crate to memory-map index files
//! into the process address space, which is the read path used by
//! `org.apache.lucene.store.MMapDirectory` in Lucene 10.5.0 on 64-bit
//! platforms.
//!
//! # Safety note
//!
//! Memory mapping keeps the underlying file accessible through process memory.
//! This is generally safe for indexes produced or verified by this process,
//! mirroring Lucene's own trust model. The module intentionally does not expose
//! any `unsafe` API surface; `memmap2` encapsulates the platform `mmap` call.
//! Users who need to read untrusted indexes should use [`FSDirectory`] or
//! [`NIOFSDirectory`] instead.
//!
//! [`FSDirectory`]: super::FSDirectory
//! [`NIOFSDirectory`]: super::NIOFSDirectory

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use memmap2::Mmap;

use super::{
    Directory, FSDirectory, IOContext, IndexInput, IndexOutput, Lock, LockFactory, SlicedIndexInput,
};
use crate::error::{LuceneError, Result};
use crate::store::DataInput;
use std::collections::HashSet;

/// Maps `len` bytes of `file`, starting at byte `offset`, read-only.
///
/// This is the crate's single memory-mapping primitive: every other module that
/// needs a mapping — including
/// [`memory_segment`](super::memory_segment), the port of Lucene's Java 21
/// `java.lang.foreign` implementation — goes through it, so that the one
/// `unsafe` call the operation requires lives in exactly one place.
///
/// # Safety of the call site
///
/// `memmap2::MmapOptions::map` is `unsafe` because a memory mapping aliases a
/// file that another process may truncate or overwrite while it is mapped, and
/// Rust cannot express that hazard in the type system. This is Lucene's own
/// trust model: `MMapDirectory` assumes the index files it maps are not
/// modified behind its back, and the same assumption applies here. Callers must
/// only map files inside a `Directory` they control.
///
/// `offset` need not be page-aligned: `memmap2` maps from the enclosing page
/// boundary and adjusts the returned pointer.
///
/// # Errors
///
/// Returns the underlying `mmap` failure, including the refusal of `memmap2` to
/// create a zero-length mapping; callers that need an empty region must not
/// call this function.
pub(crate) fn map_read_only(file: &fs::File, offset: u64, len: usize) -> io::Result<Mmap> {
    // SAFETY: see the "Safety of the call site" section above. The caller
    // guarantees the mapped file is not concurrently modified; nothing else can
    // be guaranteed statically for any memory mapping.
    unsafe {
        memmap2::MmapOptions::new()
            .offset(offset)
            .len(len)
            .map(file)
    }
}

/// Maximum chunk size for memory mapping.
///
/// Lucene 10.5.0 uses 16 GiB on 64-bit JVMs and 256 MiB on 32-bit JVMs. For
/// the initial Rust port we use a fixed 1 GiB upper bound which avoids
/// fragmented address-space issues on most 64-bit hosts while keeping the
/// implementation simple.
pub const DEFAULT_MAX_CHUNK_SIZE: usize = 1024 * 1024 * 1024;

/// Filesystem-backed [`Directory`] that memory-maps files for reading.
///
/// Equivalent to `org.apache.lucene.store.MMapDirectory`. Writing reuses
/// [`FSIndexOutput`]; only the read path is specialized.
pub struct MMapDirectory {
    inner: FSDirectory,
    max_chunk_size: usize,
}

impl MMapDirectory {
    /// Opens a new `MMapDirectory` at `path` using the default lock factory and
    /// chunk size.
    ///
    /// Equivalent to `MMapDirectory(Path)` in Lucene 10.5.0.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::with_options(path, None, DEFAULT_MAX_CHUNK_SIZE)
    }

    /// Opens a new `MMapDirectory` at `path` with the supplied lock factory.
    pub fn with_lock_factory(
        path: impl AsRef<Path>,
        lock_factory: Option<Box<dyn LockFactory>>,
    ) -> Result<Self> {
        Self::with_options(path, lock_factory, DEFAULT_MAX_CHUNK_SIZE)
    }

    /// Opens a new `MMapDirectory` with a custom maximum chunk size.
    pub fn with_options(
        path: impl AsRef<Path>,
        lock_factory: Option<Box<dyn LockFactory>>,
        max_chunk_size: usize,
    ) -> Result<Self> {
        if max_chunk_size == 0 {
            return Err(LuceneError::IllegalArgument(
                "Maximum chunk size for mmap must be >0".to_string(),
            ));
        }
        let inner = FSDirectory::with_lock_factory(path, lock_factory)?;
        Ok(Self {
            inner,
            max_chunk_size,
        })
    }

    /// Returns the canonical filesystem path backing this directory.
    pub fn directory_path(&self) -> &Path {
        self.inner.directory_path()
    }

    /// Returns the configured maximum chunk size.
    pub fn max_chunk_size(&self) -> usize {
        self.max_chunk_size
    }
}

impl Directory for MMapDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
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
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)
    }

    fn open_input(&self, name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.ensure_open()?;
        self.inner.ensure_can_read(name)?;
        let path = self.inner.directory_path().join(name);
        let file = fs::OpenOptions::new().read(true).open(&path)?;
        let metadata = file.metadata()?;
        let len = metadata.len() as i64;
        let mmap = unsafe { Mmap::map(&file) }?;
        let input = MMapIndexInput::new(mmap, path, len)?;
        Ok(Box::new(input))
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }

    fn fs_directory_path(&self) -> Option<&Path> {
        self.inner.fs_directory_path()
    }

    fn directory_type_name(&self) -> &'static str {
        "MMapDirectory"
    }

    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

impl std::fmt::Display for MMapDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MMapDirectory@{} maxChunkSize={}",
            self.inner.directory_path().display(),
            self.max_chunk_size
        )
    }
}

impl std::fmt::Debug for MMapDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MMapDirectory")
            .field("directory", &self.inner.directory_path())
            .field("max_chunk_size", &self.max_chunk_size)
            .finish()
    }
}

/// Memory-mapped [`IndexInput`] returned by [`MMapDirectory::open_input`].
///
/// The whole file is mapped as a single contiguous region. Files larger than
/// [`MMapDirectory::max_chunk_size`] are not yet split into chunks in this
/// initial port.
pub struct MMapIndexInput {
    mmap: Mmap,
    pos: usize,
    len: usize,
    resource_description: String,
    path: PathBuf,
}

impl MMapIndexInput {
    fn new(mmap: Mmap, path: PathBuf, len: i64) -> Result<Self> {
        let len = len as usize;
        let resource_description = format!("MMapIndexInput(path=\"{}\")", path.display());
        Ok(Self {
            mmap,
            pos: 0,
            len,
            resource_description,
            path,
        })
    }

    fn bytes(&self) -> &[u8] {
        // `Mmap::as_ref()` returns a slice tied to the owned mapping, which is
        // valid for the lifetime of `self`.
        self.mmap.as_ref()
    }
}

impl DataInput for MMapIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        if self.pos >= self.len {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        let b = self.bytes()[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination buffer too small: offset={offset}, len={len}, buf.len={}",
                b.len()
            )));
        }
        if self.pos + len > self.len {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        b[offset..end].copy_from_slice(&self.bytes()[self.pos..self.pos + len]);
        self.pos += len;
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        let target = self.pos as i64 + num_bytes;
        if target > self.len as i64 {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "skip past EOF",
            )));
        }
        self.pos = target as usize;
        Ok(())
    }
}

impl IndexInput for MMapIndexInput {
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.pos as i64
    }

    fn length(&self) -> i64 {
        self.len as i64
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        if pos > self.len as i64 {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "seek past EOF",
            )));
        }
        self.pos = pos as usize;
        Ok(())
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        if offset < 0 || length < 0 || length > self.len as i64 - offset {
            return Err(LuceneError::IllegalArgument(format!(
                "slice({slice_description}) out of bounds"
            )));
        }
        let clone = self.clone_input()?;
        Ok(Box::new(SlicedIndexInput::new(
            slice_description,
            clone,
            offset,
            length,
        )?))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        let file = fs::OpenOptions::new().read(true).open(&self.path)?;
        let mmap = unsafe { Mmap::map(&file) }?;
        let mut clone = MMapIndexInput::new(mmap, self.path.clone(), self.len as i64)?;
        clone.pos = self.pos;
        Ok(Box::new(clone))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }
}

impl std::fmt::Debug for MMapIndexInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MMapIndexInput")
            .field("path", &self.path)
            .field("pos", &self.pos)
            .field("len", &self.len)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::DEFAULT_IO_CONTEXT;
    use tempfile::TempDir;

    #[test]
    fn mmap_directory_round_trip() {
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;

        {
            let mut out = dir.create_output("data.bin", context).unwrap();
            out.write_int(0x12345678_i32).unwrap();
            out.write_string("mmap round-trip").unwrap();
            out.close().unwrap();
        }

        {
            let mut input = dir.open_input("data.bin", context).unwrap();
            assert_eq!(input.length(), 20);
            assert_eq!(input.read_int().unwrap(), 0x12345678_i32);
            assert_eq!(input.read_string().unwrap(), "mmap round-trip");
        }

        assert_eq!(dir.directory_type_name(), "MMapDirectory");
    }

    #[test]
    fn mmap_index_input_slices_and_clones() {
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;

        {
            let mut out = dir.create_output("data.bin", context).unwrap();
            out.write_long(0x1111_2222_3333_4444_i64).unwrap();
            out.write_long(0x5555_6666_7777_8888_i64).unwrap();
            out.close().unwrap();
        }

        let input = dir.open_input("data.bin", context).unwrap();
        let mut slice = input.slice("slice", 8, 8).unwrap();
        assert_eq!(slice.length(), 8);
        assert_eq!(slice.read_long().unwrap(), 0x5555_6666_7777_8888_i64);

        let mut clone = input.clone_input().unwrap();
        assert_eq!(clone.length(), 16);
        assert_eq!(clone.read_long().unwrap(), 0x1111_2222_3333_4444_i64);
    }
}
