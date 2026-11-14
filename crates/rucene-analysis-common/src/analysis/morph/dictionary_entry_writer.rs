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

//! The type declared in `DictionaryEntryWriter.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use std::io::BufWriter;

use rucene::codecs::codec_util::write_header;
use rucene::error::{LuceneError, Result};
use rucene::store::java_nio_file::{Files, OutputStreamWriter, Path};
use rucene::store::{DataOutput, OutputStreamDataOutput};

use crate::jdk::byte_buffer::ByteBuffer;

/// `BufferedOutputStream.DEFAULT_BUFFER_SIZE` of the JDK, 8192 bytes.
const BUFFERED_OUTPUT_STREAM_SIZE: usize = 8192;

/// The data output of a dictionary file being written:
/// `new OutputStreamDataOutput(new BufferedOutputStream(Files.newOutputStream(path)))`.
pub type DictionaryFileOutput = OutputStreamDataOutput<BufWriter<OutputStreamWriter>>;

/// Writes the file `path` as the morph writers do: `Files.newOutputStream(path)`
/// wrapped in a `BufferedOutputStream` and an `OutputStreamDataOutput`,
/// `body` writing through it, then the streams closed (flushed first), as
/// the writers' try-with-resources closes them.
pub(crate) fn write_file(
    path: &Path,
    body: impl FnOnce(&mut DictionaryFileOutput) -> Result<()>,
) -> Result<()> {
    let os = Files::new_output_stream(path, &[])?;
    let bos = BufWriter::with_capacity(BUFFERED_OUTPUT_STREAM_SIZE, OutputStreamWriter(os));
    let mut out = OutputStreamDataOutput::new(bos);
    body(&mut out)?;
    let bos = out.into_inner();
    let OutputStreamWriter(mut os) = bos
        .into_inner()
        .map_err(|e| LuceneError::from(e.into_error()))?;
    os.close()?;
    Ok(())
}

/// `Files.createDirectories(path.getParent())`.
pub(crate) fn create_parent_directories(path: &Path) -> Result<()> {
    let parent = path
        .get_parent()
        .expect("INVARIANT: a resolved dictionary file name has a parent");
    Files::create_directories(&parent)?;
    Ok(())
}

/// Abstract writer class to write dictionary entries.
///
/// Equivalent to `org.apache.lucene.analysis.morph.DictionaryEntryWriter`,
/// an abstract class: this struct holds its fields, and a concrete writer
/// holds a `DictionaryEntryWriter` and implements
/// [`DictionaryEntryWriterSubclass`].
#[derive(Debug, Clone)]
pub struct DictionaryEntryWriter {
    /// Equivalent to the protected `DictionaryEntryWriter.buffer`.
    pub buffer: ByteBuffer,
    /// Equivalent to the protected `DictionaryEntryWriter.posDict` (`None`
    /// for a `null` element).
    pub pos_dict: Vec<Option<String>>,
}

impl DictionaryEntryWriter {
    /// Equivalent to the protected `DictionaryEntryWriter(int size)`:
    /// `ByteBuffer.allocate(size)`.
    pub fn new(size: usize) -> Self {
        Self {
            buffer: ByteBuffer::allocate(size),
            pos_dict: Vec::new(),
        }
    }

    /// Returns current word id.
    ///
    /// Equivalent to `DictionaryEntryWriter.currentPosition()`.
    pub fn current_position(&self) -> i32 {
        self.buffer.position() as i32
    }

    /// Equivalent to the package-private `DictionaryEntryWriter.writeDictionary(Path,
    /// String, int)`: the header, the buffer's position as a VInt, then the
    /// buffer's bytes up to that position (`buffer.flip()`, then a channel
    /// over the same stream writes them).
    ///
    /// # Errors
    ///
    /// What creating and writing the file raise.
    pub(crate) fn write_dictionary(
        &mut self,
        path: &Path,
        dict_codec_header: &str,
        dict_codec_version: i32,
    ) -> Result<()> {
        create_parent_directories(path)?;
        write_file(path, |out| {
            write_header(out, dict_codec_header, dict_codec_version)?;
            out.write_v_int(self.buffer.position() as i32)?;
            // Write Buffer
            self.buffer.flip(); // set position to 0, set limit to current position
            let bytes = self.buffer.remaining_bytes().to_vec();
            out.write_bytes(&bytes, 0, bytes.len())?;
            let limit = self.buffer.limit();
            self.buffer.set_position(limit)?;
            Ok(())
        })
    }
}

/// A concrete dictionary entry writer: a subclass of `DictionaryEntryWriter`.
pub trait DictionaryEntryWriterSubclass: Send + Sync {
    /// The `DictionaryEntryWriter` this writer extends.
    fn base(&self) -> &DictionaryEntryWriter;

    /// The `DictionaryEntryWriter` this writer extends, for writing.
    fn base_mut(&mut self) -> &mut DictionaryEntryWriter;

    /// Writes an entry.
    ///
    /// Equivalent to the protected abstract `DictionaryEntryWriter.putEntry(String[])`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the concrete writer.
    fn put_entry(&mut self, entry: &[String]) -> Result<i32>;

    /// Flush POS dictionary data.
    ///
    /// Equivalent to the protected abstract `DictionaryEntryWriter.writePosDict(OutputStream
    /// bos, DataOutput out)`. **Divergence:** Java passes both the buffered
    /// stream and the `OutputStreamDataOutput` over it; Rust cannot lend both
    /// at once, so the writer receives the data output, whose `writeBytes`
    /// writes the same bytes to the same stream.
    ///
    /// # Errors
    ///
    /// What writing raises.
    fn write_pos_dict_to(&mut self, out: &mut DictionaryFileOutput) -> Result<()>;

    /// Returns current word id.
    ///
    /// Equivalent to `DictionaryEntryWriter.currentPosition()`.
    fn current_position(&self) -> i32 {
        self.base().current_position()
    }
}

/// Equivalent to the package-private `DictionaryEntryWriter.writePosDict(Path,
/// String, int)`: the header, then the concrete writer's POS dictionary.
///
/// # Errors
///
/// What creating and writing the file raise.
pub(crate) fn write_pos_dict(
    writer: &mut dyn DictionaryEntryWriterSubclass,
    path: &Path,
    pos_dict_codec_header: &str,
    dict_codec_version: i32,
) -> Result<()> {
    create_parent_directories(path)?;
    write_file(path, |out| {
        write_header(out, pos_dict_codec_header, dict_codec_version)?;
        writer.write_pos_dict_to(out)
    })
}
