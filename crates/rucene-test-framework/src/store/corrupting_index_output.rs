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

//! Port of `org.apache.lucene.tests.store.CorruptingIndexOutput`: corrupts
//! one bit of a file after close.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::store::{DataOutput, Directory, IndexOutput, DEFAULT_IO_CONTEXT};

use super::mock_directory_wrapper::try_with_resources;

/// Port of `org.apache.lucene.tests.store.CorruptingIndexOutput` (`extends
/// FilterIndexOutput`): once closed, flips bit 0 of byte `byte_to_corrupt`
/// of the file, and its checksum is the delegate's with bit 0 flipped.
pub struct CorruptingIndexOutput {
    resource_description: String,
    out: Box<dyn IndexOutput>,
    dir: Arc<dyn Directory>,
    byte_to_corrupt: i64,
    closed: bool,
    /// A subclass's override of `corruptFile()`, given the file's name
    /// (`out.getName()`).
    corrupt_file_override: Option<CorruptFileOverride>,
}

/// The body of an overriding `corruptFile()`, given `out.getName()`.
pub type CorruptFileOverride = Box<dyn FnMut(&str) -> Result<()> + Send + Sync>;

impl CorruptingIndexOutput {
    /// `CorruptingIndexOutput(dir, byteToCorrupt, out)`: resource description
    /// `CorruptingIndexOutput(<out>)`.
    pub fn new(dir: Arc<dyn Directory>, byte_to_corrupt: i64, out: Box<dyn IndexOutput>) -> Self {
        CorruptingIndexOutput {
            resource_description: format!("CorruptingIndexOutput({})", out.resource_description()),
            out,
            dir,
            byte_to_corrupt,
            closed: false,
            corrupt_file_override: None,
        }
    }

    /// The anonymous subclass `new CorruptingIndexOutput(dir, byteToCorrupt,
    /// out) { protected void corruptFile() { ... } }`: `close()` runs
    /// `corrupt_file` instead of [`corrupt_file`](Self::corrupt_file).
    ///
    /// Java overrides the protected method; Rust has no implementation
    /// inheritance, so the overriding body is passed in.
    pub fn with_corrupt_file(
        dir: Arc<dyn Directory>,
        byte_to_corrupt: i64,
        out: Box<dyn IndexOutput>,
        corrupt_file: CorruptFileOverride,
    ) -> Self {
        let mut output = Self::new(dir, byte_to_corrupt, out);
        output.corrupt_file_override = Some(corrupt_file);
        output
    }

    /// The field `byteToCorrupt`.
    pub fn byte_to_corrupt(&self) -> i64 {
        self.byte_to_corrupt
    }

    /// `corruptFile()`: copies the file to a temporary file with bit 0 of
    /// `byteToCorrupt` flipped, deletes the original and copies the
    /// temporary file back.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the byte is past the end of the file,
    /// and what the directory raises.
    pub fn corrupt_file(&mut self) -> Result<()> {
        let name = self.out.name().to_string();
        // Now corrupt the specfied byte:
        let mut tmp_out = self
            .dir
            .create_temp_output("tmp", "tmp", &*DEFAULT_IO_CONTEXT)?;
        let mut r#in = match self.dir.open_input(&name, &*DEFAULT_IO_CONTEXT) {
            Ok(r#in) => r#in,
            Err(e) => {
                let closed = tmp_out.close();
                return try_with_resources(Err(e), [closed]);
            }
        };
        let new_temp_name = tmp_out.name().to_string();
        let byte_to_corrupt = self.byte_to_corrupt;
        let body = (|| -> Result<()> {
            if byte_to_corrupt >= r#in.length() {
                return Err(LuceneError::illegal_argument(format!(
                    "byteToCorrupt={byte_to_corrupt} but file \"{name}\" is only length={}",
                    r#in.length()
                )));
            }
            tmp_out.copy_bytes(r#in.as_mut(), byte_to_corrupt)?;
            // Flip the 0th bit:
            let b = r#in.read_byte()? ^ 1;
            tmp_out.write_byte(b)?;
            let rest = r#in.length() - byte_to_corrupt - 1;
            tmp_out.copy_bytes(r#in.as_mut(), rest)
        })();
        let closed_in = r#in.close();
        let closed_out = tmp_out.close();
        try_with_resources(body, [closed_in, closed_out])?;

        // Delete original and copy corrupt version back:
        self.dir.delete_file(&name)?;
        self.dir.copy_from(
            self.dir.as_ref(),
            &new_temp_name,
            &name,
            &*DEFAULT_IO_CONTEXT,
        )?;
        self.dir.delete_file(&new_temp_name)
    }
}

impl DataOutput for CorruptingIndexOutput {
    /// `IndexOutput.toString()`: the resource description
    /// (`IndexOutput.java:83-86`).
    fn to_java_string(&self) -> String {
        self.resource_description().to_string()
    }

    /// `FilterIndexOutput.writeByte`: `out.writeByte(b)`.
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.out.write_byte(b)
    }

    /// `writeBytes(b, offset, length)`: one `writeByte` per byte.
    fn write_bytes(&mut self, b: &[u8], offset: usize, length: usize) -> Result<()> {
        for i in 0..length {
            self.write_byte(b[offset + i])?;
        }
        Ok(())
    }
}

impl IndexOutput for CorruptingIndexOutput {
    /// `close()`: the first close closes the delegate, then corrupts the
    /// file. The file must be corrupted after it is closed: corrupting it
    /// "inlined" (as bytes are written) would make the checksum see the
    /// wrong bytes and be "correct".
    fn close(&mut self) -> Result<()> {
        if !self.closed {
            self.out.close()?;
            // NOTE: must corrupt after file is closed, because if we corrupt "inlined" (as bytes are
            // being written) the checksum sees the wrong
            // bytes and is "correct"!!
            match self.corrupt_file_override.as_mut() {
                Some(corrupt_file) => {
                    let name = self.out.name().to_string();
                    corrupt_file(&name)?;
                }
                None => self.corrupt_file()?,
            }
            self.closed = true;
        }
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.out.file_pointer()
    }

    /// `getChecksum()`: the delegate's checksum with bit 0 flipped.
    fn checksum(&self) -> Result<i64> {
        Ok(self.out.checksum()? ^ 1)
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    /// `getName()`: `out.getName()`.
    fn name(&self) -> &str {
        self.out.name()
    }

    fn as_filter_index_output(&self) -> Option<&dyn IndexOutput> {
        Some(self.out.as_ref())
    }
}
