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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsWriter`.

use rucene::codecs::TermVectorsWriter;
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::FieldInfo;
use rucene::store::{Directory, IOContext, IndexOutput};
use rucene::util::{Accountable, BytesRef};

use super::util::{self, bool_str};

pub(crate) const END: &[u8] = b"END";
pub(crate) const DOC: &[u8] = b"doc ";
pub(crate) const NUMFIELDS: &[u8] = b"  numfields ";
pub(crate) const FIELD: &[u8] = b"  field ";
pub(crate) const FIELDNAME: &[u8] = b"    name ";
pub(crate) const FIELDPOSITIONS: &[u8] = b"    positions ";
pub(crate) const FIELDOFFSETS: &[u8] = b"    offsets   ";
pub(crate) const FIELDPAYLOADS: &[u8] = b"    payloads  ";
pub(crate) const FIELDTERMCOUNT: &[u8] = b"    numterms ";
pub(crate) const TERMTEXT: &[u8] = b"    term ";
pub(crate) const TERMFREQ: &[u8] = b"      freq ";
pub(crate) const POSITION: &[u8] = b"      position ";
pub(crate) const PAYLOAD: &[u8] = b"        payload ";
pub(crate) const STARTOFFSET: &[u8] = b"        startoffset ";
pub(crate) const ENDOFFSET: &[u8] = b"        endoffset ";

pub(crate) const VECTORS_EXTENSION: &str = "tvc";

/// Writes plain-text term vectors.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsWriter`.
///
/// **FOR RECREATIONAL USE ONLY**
pub struct SimpleTextTermVectorsWriter {
    out: Option<Box<dyn IndexOutput>>,
    num_docs_written: i32,
    /// The capacity Java's scratch `BytesRefBuilder` has reached, which
    /// `ramBytesUsed` reports.
    scratch_capacity: usize,
    offsets: bool,
    positions: bool,
    payloads: bool,
}

impl std::fmt::Debug for SimpleTextTermVectorsWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleTextTermVectorsWriter")
            .field("num_docs_written", &self.num_docs_written)
            .finish_non_exhaustive()
    }
}

impl SimpleTextTermVectorsWriter {
    /// Creates the writer of `segment`'s `.tvc` file.
    ///
    /// # Errors
    ///
    /// Propagates the directory's error.
    pub fn new(directory: &dyn Directory, segment: &str, context: &dyn IOContext) -> Result<Self> {
        let out =
            directory.create_output(&segment_file_name(segment, "", VECTORS_EXTENSION), context)?;
        Ok(Self {
            out: Some(out),
            num_docs_written: 0,
            scratch_capacity: 0,
            offsets: false,
            positions: false,
            payloads: false,
        })
    }

    fn out(&mut self) -> Result<&mut dyn IndexOutput> {
        match self.out.as_deref_mut() {
            Some(out) => Ok(out),
            None => Err(LuceneError::already_closed("this writer is closed")),
        }
    }

    /// `write(String)`, through the scratch builder.
    fn write(&mut self, s: &str) -> Result<()> {
        self.grow_scratch(s.encode_utf16().count());
        util::write_str(self.out()?, s)
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        util::write_bytes(self.out()?, bytes)
    }

    fn new_line(&mut self) -> Result<()> {
        util::write_newline(self.out()?)
    }

    /// Tracks `BytesRefBuilder.copyChars`, which grows the scratch array with
    /// `ArrayUtil.grow(bytes, UnicodeUtil.maxUTF8Length(len))` — to
    /// `ArrayUtil.oversize(len * 3, 1)` when it is too small.
    fn grow_scratch(&mut self, utf16_len: usize) {
        let needed = utf16_len * 3;
        if self.scratch_capacity < needed {
            self.scratch_capacity = rucene::util::ArrayUtil::oversize(needed, 1);
        }
    }
}

impl Accountable for SimpleTextTermVectorsWriter {
    /// Port of `ramBytesUsed`: `scratch.get().bytes.length`.
    fn ram_bytes_used(&self) -> i64 {
        self.scratch_capacity as i64
    }
}

impl TermVectorsWriter for SimpleTextTermVectorsWriter {
    fn start_document(&mut self, num_vector_fields: i32) -> Result<()> {
        self.write_bytes(DOC)?;
        let n = self.num_docs_written.to_string();
        self.write(&n)?;
        self.new_line()?;

        self.write_bytes(NUMFIELDS)?;
        self.write(&num_vector_fields.to_string())?;
        self.new_line()?;
        self.num_docs_written += 1;
        Ok(())
    }

    fn start_field(
        &mut self,
        info: &FieldInfo,
        num_terms: i32,
        positions: bool,
        offsets: bool,
        payloads: bool,
    ) -> Result<()> {
        self.write_bytes(FIELD)?;
        self.write(&info.number.to_string())?;
        self.new_line()?;

        self.write_bytes(FIELDNAME)?;
        self.write(&info.name)?;
        self.new_line()?;

        self.write_bytes(FIELDPOSITIONS)?;
        self.write(bool_str(positions))?;
        self.new_line()?;

        self.write_bytes(FIELDOFFSETS)?;
        self.write(bool_str(offsets))?;
        self.new_line()?;

        self.write_bytes(FIELDPAYLOADS)?;
        self.write(bool_str(payloads))?;
        self.new_line()?;

        self.write_bytes(FIELDTERMCOUNT)?;
        self.write(&num_terms.to_string())?;
        self.new_line()?;

        self.positions = positions;
        self.offsets = offsets;
        self.payloads = payloads;
        Ok(())
    }

    fn start_term(&mut self, term: &BytesRef, freq: i32) -> Result<()> {
        self.write_bytes(TERMTEXT)?;
        self.write_bytes(term.slice())?;
        self.new_line()?;

        self.write_bytes(TERMFREQ)?;
        self.write(&freq.to_string())?;
        self.new_line()
    }

    fn add_position(
        &mut self,
        position: i32,
        start_offset: i32,
        end_offset: i32,
        payload: Option<&BytesRef>,
    ) -> Result<()> {
        debug_assert!(self.positions || self.offsets);

        if self.positions {
            self.write_bytes(POSITION)?;
            self.write(&position.to_string())?;
            self.new_line()?;

            if self.payloads {
                self.write_bytes(PAYLOAD)?;
                if let Some(payload) = payload {
                    debug_assert!(payload.length > 0);
                    self.write_bytes(payload.slice())?;
                }
                self.new_line()?;
            }
        }

        if self.offsets {
            self.write_bytes(STARTOFFSET)?;
            self.write(&start_offset.to_string())?;
            self.new_line()?;

            self.write_bytes(ENDOFFSET)?;
            self.write(&end_offset.to_string())?;
            self.new_line()?;
        }
        Ok(())
    }

    fn finish(&mut self, num_docs: i32) -> Result<()> {
        if self.num_docs_written != num_docs {
            let description = self.out.as_deref().map_or_else(
                || "null".to_string(),
                |o| o.resource_description().to_string(),
            );
            return Err(LuceneError::unchecked(
                "java.lang.RuntimeException",
                format!(
                    "mergeVectors produced an invalid result: mergedDocs is {num_docs} but vec \
                 numDocs is {} file={description}; now aborting this merge to prevent index \
                 corruption",
                    self.num_docs_written
                ),
            ));
        }
        self.write_bytes(END)?;
        self.new_line()?;
        util::write_checksum(self.out()?)
    }

    fn close(&mut self) -> Result<()> {
        match self.out.take() {
            Some(mut out) => out.close(),
            None => Ok(()),
        }
    }
}
