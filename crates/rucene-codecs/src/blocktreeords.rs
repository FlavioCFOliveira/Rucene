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

//! Port of `org.apache.lucene.codecs.blocktreeords`: same postings format as
//! Lucene50, except the terms dictionary also supports ords, i.e. returning
//! which ord the enum is seeked to, and seeking by ord.
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`BlockTreeOrdsPostingsFormat`] | `BlockTreeOrdsPostingsFormat` |
//! | [`FSTOrdsOutputs`], [`Output`] | `FSTOrdsOutputs`, `FSTOrdsOutputs.Output` |
//! | [`OrdsBlockTreeTermsReader`] | `OrdsBlockTreeTermsReader` |
//! | [`OrdsBlockTreeTermsWriter`] | `OrdsBlockTreeTermsWriter` |
//! | [`OrdsFieldReader`] | `OrdsFieldReader` |
//! | [`OrdsIntersectTermsEnum`] | `OrdsIntersectTermsEnum` |
//! | `ords_intersect_terms_enum_frame` | `OrdsIntersectTermsEnumFrame` |
//! | [`OrdsSegmentTermsEnum`] | `OrdsSegmentTermsEnum` |
//! | `ords_segment_terms_enum_frame` | `OrdsSegmentTermsEnumFrame` |

pub mod block_tree_ords_postings_format;
pub mod fst_ords_outputs;
pub mod ords_block_tree_terms_reader;
pub mod ords_block_tree_terms_writer;
pub mod ords_field_reader;
pub mod ords_intersect_terms_enum;
mod ords_intersect_terms_enum_frame;
pub mod ords_segment_terms_enum;
mod ords_segment_terms_enum_frame;

pub use block_tree_ords_postings_format::BlockTreeOrdsPostingsFormat;
pub use fst_ords_outputs::{FSTOrdsOutputs, Output};
pub use ords_block_tree_terms_reader::OrdsBlockTreeTermsReader;
pub use ords_block_tree_terms_writer::OrdsBlockTreeTermsWriter;
pub use ords_field_reader::OrdsFieldReader;
pub use ords_intersect_terms_enum::OrdsIntersectTermsEnum;
pub use ords_segment_terms_enum::OrdsSegmentTermsEnum;

use rucene::error::{LuceneError, Result};
use rucene::store::{ByteArrayDataInput, DataInput, IndexInput};

/// Java's `assert cond;` (`assert cond : message;`) under `-ea`: an
/// `AssertionError`, raised in builds with `debug_assertions` (see
/// [`LuceneError::AssertionError`]).
macro_rules! java_assert {
    ($cond:expr) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(rucene::error::LuceneError::AssertionError(String::new()));
        }
    };
    ($cond:expr, $($msg:tt)+) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(rucene::error::LuceneError::AssertionError(format!($($msg)+)));
        }
    };
}
pub(crate) use java_assert;

/// A `ByteArrayDataInput` over a `byte[]` its frame owns (Java's
/// `suffixBytes` + `suffixesReader` pairs): the buffer, the window's limit
/// and the read position.
#[derive(Default)]
pub(crate) struct OwnedArrayInput {
    pub(crate) bytes: Vec<u8>,
    limit: usize,
    pos: usize,
}

impl OwnedArrayInput {
    /// `if (buf.length < len) buf = new byte[oversize(len)];
    /// in.readBytes(buf, 0, len); reader.reset(buf, 0, len)`.
    pub(crate) fn fill(&mut self, input: &mut dyn IndexInput, len: i32) -> Result<()> {
        let len = array_len(len)?;
        if self.bytes.len() < len {
            self.bytes.resize(len, 0);
        }
        input.read_bytes(&mut self.bytes, 0, len)?;
        self.limit = len;
        self.pos = 0;
        Ok(())
    }

    /// `System.arraycopy(src, ...); reader.reset(buf, 0, src.length)`.
    pub(crate) fn fill_from(&mut self, src: &[u8]) {
        if self.bytes.len() < src.len() {
            self.bytes.resize(src.len(), 0);
        }
        self.bytes[..src.len()].copy_from_slice(src);
        self.limit = src.len();
        self.pos = 0;
    }

    /// Runs `f` on a `ByteArrayDataInput` over the window, keeping the
    /// position it leaves.
    pub(crate) fn with<R>(
        &mut self,
        f: impl FnOnce(&mut ByteArrayDataInput<'_>) -> Result<R>,
    ) -> Result<R> {
        let mut input = ByteArrayDataInput::with_range(&self.bytes, 0, self.limit);
        input.set_position(self.pos);
        let result = f(&mut input);
        self.pos = input.position();
        result
    }

    pub(crate) fn read_v_int(&mut self) -> Result<i32> {
        self.with(|i| i.read_v_int())
    }

    pub(crate) fn read_v_long(&mut self) -> Result<i64> {
        self.with(|i| i.read_v_long())
    }

    pub(crate) fn read_byte(&mut self) -> Result<u8> {
        self.with(|i| i.read_byte())
    }

    pub(crate) fn skip_bytes(&mut self, n: i32) -> Result<()> {
        self.with(|i| i.skip_bytes(i64::from(n)))
    }

    /// `readBytes(dest, offset, len)`.
    pub(crate) fn read_into(&mut self, dest: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.with(|i| i.read_bytes(dest, offset, len))
    }

    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    pub(crate) fn set_position(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub(crate) fn rewind(&mut self) {
        self.pos = 0;
    }
}

/// `new byte[len]` for a length read from a file: a negative one raises
/// `NegativeArraySizeException`.
pub(crate) fn array_len(len: i32) -> Result<usize> {
    usize::try_from(len).map_err(|_| {
        LuceneError::unchecked("java.lang.NegativeArraySizeException", len.to_string())
    })
}

/// Java's `ArrayIndexOutOfBoundsException` for `index` into an array of
/// `length`.
pub(crate) fn out_of_bounds(index: usize, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// Java's `BytesRefBuilder term`: the backing array plus the length;
/// `setLength` keeps the bytes of the array, and `grow` zero-fills the new
/// ones.
#[derive(Default)]
pub(crate) struct TermBuilder {
    pub(crate) bytes: Vec<u8>,
    pub(crate) length: usize,
}

impl TermBuilder {
    /// `grow(capacity)`.
    pub(crate) fn grow(&mut self, capacity: usize) {
        if self.bytes.len() < capacity {
            self.bytes.resize(capacity, 0);
        }
    }

    /// `setLength(length)`; the port's array grows with it, where Java's
    /// callers `grow` right after.
    pub(crate) fn set_length(&mut self, length: usize) {
        self.grow(length);
        self.length = length;
    }

    /// `copyBytes(src)`.
    pub(crate) fn copy_bytes(&mut self, src: &[u8]) {
        self.set_length(src.len());
        self.bytes[..src.len()].copy_from_slice(src);
    }

    /// `get()`, the term's bytes.
    pub(crate) fn get(&self) -> &[u8] {
        &self.bytes[..self.length]
    }

    /// `byteAt(i)`, which reads the backing array.
    pub(crate) fn byte_at(&self, i: usize) -> Result<u8> {
        self.bytes
            .get(i)
            .copied()
            .ok_or_else(|| out_of_bounds(i, self.bytes.len()))
    }

    /// `setByteAt(i, b)`.
    pub(crate) fn set_byte_at(&mut self, i: usize, b: u8) {
        self.grow(i + 1);
        self.bytes[i] = b;
    }
}
