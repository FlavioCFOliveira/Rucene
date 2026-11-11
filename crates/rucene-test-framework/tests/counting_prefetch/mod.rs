//! Port of the private nested classes `CountingPrefetchDirectory` and
//! `CountingPrefetchIndexInput` that
//! `lucene/core/src/test/org/apache/lucene/codecs/lucene90/TestLucene90StoredFieldsFormat.java`
//! and `TestLucene90TermVectorsFormat.java` declare, each with an identical
//! copy; the two suites share this one.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::store::{
    BufferedChecksumIndexInput, DataInput, Directory, IOContext, IndexInput, IndexOutput, Lock,
};

/// `CountingPrefetchDirectory extends FilterDirectory`: every input it opens
/// counts its `prefetch` calls on `counter`.
pub struct CountingPrefetchDirectory {
    r#in: Arc<dyn Directory>,
    counter: Arc<AtomicI32>,
}

impl CountingPrefetchDirectory {
    /// `CountingPrefetchDirectory(Directory in, AtomicInteger counter)`.
    pub fn new(r#in: Arc<dyn Directory>, counter: Arc<AtomicI32>) -> Self {
        Self { r#in, counter }
    }
}

impl Directory for CountingPrefetchDirectory {
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
    /// `openInput(name, context)`: `new CountingPrefetchIndexInput(
    /// super.openInput(name, context), counter)`.
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(CountingPrefetchIndexInput::new(
            self.r#in.open_input(name, context)?,
            Arc::clone(&self.counter),
        )))
    }
    /// `Directory.openChecksumInput`, inherited by `FilterDirectory`:
    /// `new BufferedChecksumIndexInput(openInput(name, READONCE))`, over this
    /// directory's `openInput`.
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        Ok(Box::new(BufferedChecksumIndexInput::new(
            self.open_input(name, &*rucene::store::READONCE_IO_CONTEXT)?,
        )))
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }
    /// `FilterDirectory.close()` closes the wrapped directory; the tests never
    /// close this wrapper, only the wrapped directory, through its
    /// try-with-resources.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.r#in.get_pending_deletions()
    }
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.r#in.as_ref())
    }
}

/// `CountingPrefetchIndexInput extends FilterIndexInput`: counts its
/// `prefetch` calls; its clones and slices share the counter.
pub struct CountingPrefetchIndexInput {
    resource_description: String,
    r#in: Box<dyn IndexInput>,
    counter: Arc<AtomicI32>,
}

impl CountingPrefetchIndexInput {
    /// `CountingPrefetchIndexInput(IndexInput input, AtomicInteger counter)`:
    /// `super(input.toString(), input)`.
    pub fn new(input: Box<dyn IndexInput>, counter: Arc<AtomicI32>) -> Self {
        Self {
            resource_description: input.resource_description().to_string(),
            r#in: input,
            counter,
        }
    }
}

impl std::fmt::Debug for CountingPrefetchIndexInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountingPrefetchIndexInput")
            .field("resource_description", &self.resource_description)
            .finish()
    }
}

impl DataInput for CountingPrefetchIndexInput {
    fn to_java_string(&self) -> String {
        self.resource_description.clone()
    }

    /// `FilterIndexInput.readByte()`.
    fn read_byte(&mut self) -> Result<u8> {
        self.r#in.read_byte()
    }

    /// `FilterIndexInput.readBytes(b, offset, len)`.
    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        self.r#in.read_bytes(b, offset, len)
    }

    /// `IndexInput.skipBytes(numBytes)`, which `FilterIndexInput` inherits:
    /// a seek past the bytes.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let skip_to = self.file_pointer() + num_bytes;
        self.seek(skip_to)
    }
}

impl IndexInput for CountingPrefetchIndexInput {
    fn close(&mut self) -> Result<()> {
        self.r#in.close()
    }

    fn file_pointer(&self) -> i64 {
        self.r#in.file_pointer()
    }

    fn length(&self) -> i64 {
        self.r#in.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.r#in.seek(pos)
    }

    /// `slice(sliceDescription, offset, length)`: `new
    /// CountingPrefetchIndexInput(in.slice(...), counter)`.
    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(CountingPrefetchIndexInput::new(
            self.r#in.slice(slice_description, offset, length)?,
            Arc::clone(&self.counter),
        )))
    }

    /// `clone()`: `new CountingPrefetchIndexInput(in.clone(), counter)`.
    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(CountingPrefetchIndexInput::new(
            self.r#in.clone_input()?,
            Arc::clone(&self.counter),
        )))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    /// `prefetch(offset, length)`: `in.prefetch(offset, length)`, then
    /// `counter.incrementAndGet()`.
    fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
        self.r#in.prefetch(offset, length)?;
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// A `FilterIndexInput` of class `CountingPrefetchIndexInput`, over its
    /// `in`.
    fn as_filter_index_input(&self) -> Option<(std::any::TypeId, &dyn IndexInput)> {
        Some((
            std::any::TypeId::of::<CountingPrefetchIndexInput>(),
            self.r#in.as_ref(),
        ))
    }
}
