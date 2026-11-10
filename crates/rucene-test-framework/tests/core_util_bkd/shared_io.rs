//! The one `IndexOutput` and the one `IndexInput` the BKD tests pass for
//! every role: Java calls `w.finish(out, out, out)` and `new BKDReader(in,
//! in, in)`, so the metadata, index and data writes land in one stream in
//! call order, and the three reads share one file pointer. Rucene's
//! `BKDWriter.finish`/`BKDFinalizer.run` and `BKDReader::new` take each role
//! as a separate `&mut`, which cannot alias; these handles each stand for
//! the same underlying stream, as Java's one object does.

use std::sync::{Arc, Mutex};

use rucene::error::Result;
use rucene::store::{DataInput, DataOutput, IndexInput, IndexOutput};

/// A handle on a shared `IndexOutput`.
pub struct SharedIndexOutput {
    inner: Arc<Mutex<Box<dyn IndexOutput>>>,
    name: String,
    description: String,
}

impl SharedIndexOutput {
    /// A handle on `out`, and the stream to make more handles from.
    pub fn new(out: Box<dyn IndexOutput>) -> Self {
        let name = out.name().to_string();
        let description = out.resource_description().to_string();
        SharedIndexOutput {
            inner: Arc::new(Mutex::new(out)),
            name,
            description,
        }
    }

    /// Another handle on the same stream.
    pub fn handle(&self) -> SharedIndexOutput {
        SharedIndexOutput {
            inner: Arc::clone(&self.inner),
            name: self.name.clone(),
            description: self.description.clone(),
        }
    }

    /// `out.getFilePointer()`.
    pub fn get_file_pointer(&self) -> i64 {
        self.inner.lock().unwrap().file_pointer()
    }

    /// `out.close()` of the stream itself.
    pub fn close_stream(&self) -> Result<()> {
        self.inner.lock().unwrap().close()
    }
}

impl DataOutput for SharedIndexOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.inner.lock().unwrap().write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.inner.lock().unwrap().write_bytes(b, offset, len)
    }
}

impl IndexOutput for SharedIndexOutput {
    /// Closing a role closes nothing: the test closes the stream.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.inner.lock().unwrap().file_pointer()
    }

    fn checksum(&self) -> Result<i64> {
        self.inner.lock().unwrap().checksum()
    }

    fn resource_description(&self) -> &str {
        &self.description
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// A handle on a shared `IndexInput`.
pub struct SharedIndexInput {
    inner: Arc<Mutex<Box<dyn IndexInput>>>,
    description: String,
}

impl SharedIndexInput {
    /// A handle on `input`.
    pub fn new(input: Box<dyn IndexInput>) -> Self {
        let description = input.resource_description().to_string();
        SharedIndexInput {
            inner: Arc::new(Mutex::new(input)),
            description,
        }
    }

    /// Another handle on the same stream.
    pub fn handle(&self) -> SharedIndexInput {
        SharedIndexInput {
            inner: Arc::clone(&self.inner),
            description: self.description.clone(),
        }
    }

    /// `in.close()` of the stream itself.
    pub fn close_stream(&self) -> Result<()> {
        self.inner.lock().unwrap().close()
    }
}

impl DataInput for SharedIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        self.inner.lock().unwrap().read_byte()
    }

    fn to_java_string(&self) -> String {
        self.inner.lock().unwrap().to_java_string()
    }

    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        self.inner.lock().unwrap().read_bytes(b, offset, len)
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.inner.lock().unwrap().skip_bytes(num_bytes)
    }
}

impl IndexInput for SharedIndexInput {
    /// Closing a role closes nothing: the test closes the stream.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.inner.lock().unwrap().file_pointer()
    }

    fn length(&self) -> i64 {
        self.inner.lock().unwrap().length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.inner.lock().unwrap().seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        self.inner
            .lock()
            .unwrap()
            .slice(slice_description, offset, length)
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        self.inner.lock().unwrap().clone_input()
    }

    fn resource_description(&self) -> &str {
        &self.description
    }
}
