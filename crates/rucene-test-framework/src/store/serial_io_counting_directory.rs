/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.store.SerialIOCountingDirectory`: a
//! directory wrapper that counts the number of times Lucene may wait for
//! I/O to return serially. Lower counts mean that Lucene better takes
//! advantage of I/O parallelism.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::ThreadId;

use rucene::error::{LuceneError, Result};
use rucene::store::{
    hints_of, BufferedChecksumIndexInput, DataAccessHint, DataInput, Directory, FileTypeHint,
    FilterDirectory, IOContext, IndexInput, IndexOutput, Lock, READONCE_IO_CONTEXT,
};

/// `PAGE_SHIFT`: 4096 bytes per page.
const PAGE_SHIFT: i64 = 12;
/// `PAGE_READAHEAD`: the assumed number of pages that are read ahead.
const PAGE_READAHEAD: i64 = 4;

/// `pendingFetch`, a `CloseableThreadLocal<Boolean>` whose initial value is
/// `false`: one flag per thread, shared by every input of the directory.
#[derive(Default)]
struct PendingFetch(Mutex<HashMap<ThreadId, bool>>);

impl PendingFetch {
    fn map(&self) -> MutexGuard<'_, HashMap<ThreadId, bool>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn get(&self) -> bool {
        *self
            .map()
            .get(&std::thread::current().id())
            .unwrap_or(&false)
    }

    fn set(&self, value: bool) {
        self.map().insert(std::thread::current().id(), value);
    }

    fn close(&self) {
        self.map().clear();
    }
}

/// The state the directory and its inputs share: `counter` (a `LongAdder`)
/// and `pendingFetch`.
#[derive(Default)]
struct Shared {
    counter: AtomicI64,
    pending_fetch: PendingFetch,
}

impl Shared {
    fn increment(&self) {
        self.counter.fetch_add(1, Ordering::SeqCst);
    }
}

/// Port of `org.apache.lucene.tests.store.SerialIOCountingDirectory`
/// (`extends FilterDirectory`).
pub struct SerialIOCountingDirectory {
    r#in: FilterDirectory,
    shared: Arc<Shared>,
}

impl SerialIOCountingDirectory {
    /// `SerialIOCountingDirectory(in)`.
    pub fn new(r#in: Box<dyn Directory>) -> Self {
        SerialIOCountingDirectory {
            r#in: FilterDirectory::new(r#in),
            shared: Arc::new(Shared::default()),
        }
    }

    /// `count()`: the number of I/O requests performed serially.
    pub fn count(&self) -> i64 {
        self.shared.counter.load(Ordering::SeqCst)
    }
}

impl Directory for SerialIOCountingDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.r#in.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.r#in.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.r#in.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.r#in.rename(source, dest)
    }

    /// `openChecksumInput(name)`: sequential access, counts 1 for the whole
    /// file, then `Directory.openChecksumInput` over this directory's
    /// `openInput`.
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        // sequential access, count 1 for the whole file
        self.shared.increment();
        Ok(Box::new(BufferedChecksumIndexInput::new(
            self.open_input(name, &*READONCE_IO_CONTEXT)?,
        )))
    }

    /// `openInput(name, context)`: an `INDEX` file is expected to be small
    /// and fit in the page cache, so it counts 1 at open time; any other
    /// file is wrapped in an input that counts its serial page reads.
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        if hints_of::<FileTypeHint>(context).contains(&&FileTypeHint::Index) {
            // expected to be small and fit in the page cache, only count 1 at open time
            self.shared.increment();
            return self.r#in.open_input(name, context);
        }
        let r#in = self.r#in.open_input(name, context)?;
        Ok(Box::new(SerializedIOCountingIndexInput::new(
            Arc::clone(&self.shared),
            r#in,
            context,
        )))
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }

    /// `close()`: closes `pendingFetch`, then the delegate.
    fn close(&mut self) -> Result<()> {
        self.shared.pending_fetch.close();
        self.r#in.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.r#in.get_pending_deletions()
    }

    /// `FilterDirectory.toString()`: `SerialIOCountingDirectory(<in>)`.
    fn directory_to_string(&self) -> String {
        format!(
            "SerialIOCountingDirectory({})",
            self.r#in.get_delegate().directory_to_string()
        )
    }

    /// A `FilterDirectory`, over its `in`.
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.r#in.get_delegate())
    }
}

/// The context's `DataAccessHint.RANDOM`.
fn is_random(random: bool, current_page: i64) -> i64 {
    if random {
        current_page
    } else {
        // Assume that the next few pages are always free to read thanks to read-ahead.
        current_page + PAGE_READAHEAD
    }
}

/// The private class `SerializedIOCountingIndexInput`: counts a serial I/O
/// for a read of a page that is neither prefetched nor within the
/// read-ahead window after the current page.
struct SerializedIOCountingIndexInput {
    shared: Arc<Shared>,
    resource_description: String,
    r#in: Box<dyn IndexInput>,
    slice_offset: i64,
    slice_length: i64,
    /// `context`, kept as the facts the input reads from it: its hints'
    /// `DataAccessHint.RANDOM`, and the context itself for slices.
    context: Box<dyn IOContext>,
    random: bool,
    pending_pages: Mutex<HashSet<i64>>,
    current_page: AtomicI64,
}

/// A copy of `context` that a slice can keep (Java shares the reference).
fn copy_context(context: &dyn IOContext) -> Box<dyn IOContext> {
    context.with_hints(context.hints())
}

impl SerializedIOCountingIndexInput {
    fn new(shared: Arc<Shared>, r#in: Box<dyn IndexInput>, context: &dyn IOContext) -> Self {
        let length = r#in.length();
        Self::with_slice(shared, r#in, context, 0, length)
    }

    fn with_slice(
        shared: Arc<Shared>,
        r#in: Box<dyn IndexInput>,
        context: &dyn IOContext,
        offset: i64,
        length: i64,
    ) -> Self {
        let random = hints_of::<DataAccessHint>(context).contains(&&DataAccessHint::Random);
        SerializedIOCountingIndexInput {
            shared,
            resource_description: r#in.to_java_string(),
            r#in,
            slice_offset: offset,
            slice_length: length,
            context: copy_context(context),
            random,
            pending_pages: Mutex::new(HashSet::new()),
            current_page: AtomicI64::new(i64::MIN),
        }
    }

    fn pending_pages(&self) -> MutexGuard<'_, HashSet<i64>> {
        self.pending_pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// `onRead(offset, len)`.
    fn on_read(&self, offset: i64, len: i64) {
        if len == 0 {
            return;
        }
        let first_page = (self.slice_offset + offset) >> PAGE_SHIFT;
        let last_page = (self.slice_offset + offset + len - 1) >> PAGE_SHIFT;
        let pending = self.pending_pages();
        for page in first_page..=last_page {
            let current_page = self.current_page.load(Ordering::SeqCst);
            let read_ahead_upto = is_random(self.random, current_page);
            if !pending.contains(&page) && (page < current_page || page > read_ahead_upto) {
                self.shared.increment();
            }
            self.current_page.store(page, Ordering::SeqCst);
        }
        self.shared.pending_fetch.set(false);
    }
}

impl DataInput for SerializedIOCountingIndexInput {
    /// `IndexInput.toString()`: the resource description.
    fn to_java_string(&self) -> String {
        self.resource_description.clone()
    }

    /// `readByte()`.
    fn read_byte(&mut self) -> Result<u8> {
        self.on_read(self.file_pointer(), 1);
        self.r#in.read_byte()
    }

    /// `readBytes(b, offset, len)`.
    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        self.on_read(self.file_pointer(), len as i64);
        self.r#in.read_bytes(b, offset, len)
    }

    /// `IndexInput.skipBytes`: `seek(getFilePointer() + numBytes)`.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let skip_to = self.file_pointer() + num_bytes;
        self.seek(skip_to)
    }
}

impl IndexInput for SerializedIOCountingIndexInput {
    fn close(&mut self) -> Result<()> {
        self.r#in.close()
    }

    fn file_pointer(&self) -> i64 {
        self.r#in.file_pointer() - self.slice_offset
    }

    fn length(&self) -> i64 {
        self.slice_length
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.r#in.seek(self.slice_offset + pos)
    }

    /// `slice(sliceDescription, offset, length)`: with this input's context.
    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        self.slice_with_context(slice_description, offset, length, &*self.context)
    }

    /// `slice(sliceDescription, offset, length, context)`: a clone of the
    /// delegate positioned at the slice, counted with `context`.
    fn slice_with_context(
        &self,
        _slice_description: &str,
        offset: i64,
        length: i64,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexInput>> {
        if (length | offset) < 0 || length > self.slice_length - offset {
            return Err(LuceneError::IllegalArgument(String::new()));
        }
        let mut clone = self.r#in.clone_input()?;
        clone.seek(self.slice_offset + offset)?;
        Ok(Box::new(SerializedIOCountingIndexInput::with_slice(
            Arc::clone(&self.shared),
            clone,
            context,
            self.slice_offset + offset,
            length,
        )))
    }

    /// `clone()`.
    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        let clone = self.r#in.clone_input()?;
        Ok(Box::new(SerializedIOCountingIndexInput::with_slice(
            Arc::clone(&self.shared),
            clone,
            &*self.context,
            self.slice_offset,
            self.slice_length,
        )))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    /// `prefetch(offset, length)`: prefetching outside the current (or, for
    /// non-random access, the read-ahead) window counts 1, once for several
    /// prefetches without a read in between, which can be performed in
    /// parallel.
    fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
        let first_page = (self.slice_offset + offset) >> PAGE_SHIFT;
        let last_page = (self.slice_offset + offset + length - 1) >> PAGE_SHIFT;

        let current_page = self.current_page.load(Ordering::SeqCst);
        let read_ahead_upto = is_random(self.random, current_page);

        let mut pending = self.pending_pages();
        if first_page >= current_page && last_page <= read_ahead_upto {
            // seeking within the current (or next page if ReadAdvice.NORMAL) doesn't increment the
            // counter
        } else if !self.shared.pending_fetch.get() {
            // If multiple prefetch calls are performed without a readXXX() call in-between, count a
            // single increment as these I/O requests can be performed in parallel.
            self.shared.increment();
            pending.clear();
            self.shared.pending_fetch.set(true);
        }

        for page in first_page..=last_page {
            pending.insert(page);
        }
        Ok(())
    }

    fn is_loaded(&self) -> Option<bool> {
        self.r#in.is_loaded()
    }
}
