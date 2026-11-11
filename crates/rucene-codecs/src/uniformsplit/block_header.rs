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

//! Port of `org.apache.lucene.codecs.uniformsplit.BlockHeader`.

use rucene::error::{LuceneError, Result};
use rucene::store::{DataInput, DataOutput};
use rucene::util::{Accountable, RamUsageEstimator};

use super::uniform_split_terms_writer::MAX_NUM_BLOCK_LINES;

/// Block header containing block metadata.
///
/// Holds the number of lines in the block.
///
/// Holds the base file pointers to apply delta base encoding to all the file
/// pointers in the block with
/// [`DeltaBaseTermStateSerializer`](super::DeltaBaseTermStateSerializer).
///
/// Holds the offset to the details region of the block (the term states).
///
/// Holds the offset to the middle term of the block to divide the number of
/// terms to scan by 2.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.BlockHeader`
/// (`@lucene.experimental`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockHeader {
    pub(crate) lines_count: i32,

    pub(crate) base_docs_fp: i64,
    pub(crate) base_positions_fp: i64,
    pub(crate) base_payloads_fp: i64,

    pub(crate) term_states_base_offset: i32,
    pub(crate) middle_line_index: i32,
    pub(crate) middle_line_offset: i32,
}

impl BlockHeader {
    /// Port of `BlockHeader(int, long, long, long, int, int)`.
    ///
    /// `lines_count` is the number of lines in the block; `base_docs_fp` the
    /// file pointer to the docs of the first term with docs in the block;
    /// `base_positions_fp` the one to the positions of the first term with
    /// positions; `base_payloads_fp` the one to the payloads of the first
    /// term with payloads; `term_states_base_offset` the offset to the
    /// details region of the block (the term states), relative to the block
    /// start; `middle_line_offset` the offset to the middle term of the
    /// block, relative to the block start.
    pub fn new(
        lines_count: i32,
        base_docs_fp: i64,
        base_positions_fp: i64,
        base_payloads_fp: i64,
        term_states_base_offset: i32,
        middle_line_offset: i32,
    ) -> Self {
        let mut header = Self::default();
        header.reset(
            lines_count,
            base_docs_fp,
            base_positions_fp,
            base_payloads_fp,
            term_states_base_offset,
            middle_line_offset,
        );
        header
    }

    /// Port of `BlockHeader.reset(int, long, long, long, int, int)`.
    pub fn reset(
        &mut self,
        lines_count: i32,
        base_docs_fp: i64,
        base_positions_fp: i64,
        base_payloads_fp: i64,
        term_states_base_offset: i32,
        middle_term_offset: i32,
    ) -> &mut Self {
        self.base_docs_fp = base_docs_fp;
        self.base_positions_fp = base_positions_fp;
        self.base_payloads_fp = base_payloads_fp;
        self.lines_count = lines_count;
        self.middle_line_index = lines_count >> 1;
        self.term_states_base_offset = term_states_base_offset;
        self.middle_line_offset = middle_term_offset;
        self
    }

    /// The number of lines in the block.
    pub fn get_lines_count(&self) -> i32 {
        self.lines_count
    }

    /// The index of the middle line of the block.
    pub fn get_middle_line_index(&self) -> i32 {
        self.middle_line_index
    }

    /// The offset to the middle line of the block, relative to the block
    /// start.
    pub fn get_middle_line_offset(&self) -> i32 {
        self.middle_line_offset
    }

    /// The offset to the details region of the block (the term states),
    /// relative to the block start.
    pub fn get_term_states_base_offset(&self) -> i32 {
        self.term_states_base_offset
    }

    /// The file pointer to the docs of the first term with docs in the block.
    pub fn get_base_docs_fp(&self) -> i64 {
        self.base_docs_fp
    }

    /// The file pointer to the positions of the first term with positions in
    /// the block.
    pub fn get_base_positions_fp(&self) -> i64 {
        self.base_positions_fp
    }

    /// The file pointer to the payloads of the first term with payloads in
    /// the block.
    pub fn get_base_payloads_fp(&self) -> i64 {
        self.base_payloads_fp
    }
}

impl Accountable for BlockHeader {
    /// Port of `BlockHeader.ramBytesUsed()`: `RAM_USAGE`, the shallow size of
    /// the instance.
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::shallow_size_of_instance::<BlockHeader>()
    }
}

/// Reads/writes block header.
///
/// Port of the static nested class `BlockHeader.Serializer`, which holds no
/// state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Serializer;

impl Serializer {
    /// Port of `BlockHeader.Serializer.write(DataOutput, BlockHeader)`.
    ///
    /// # Errors
    ///
    /// `AssertionError` (debug builds) for an uninitialized header; the I/O
    /// errors of the output.
    pub fn write(&self, output: &mut dyn DataOutput, block_header: &BlockHeader) -> Result<()> {
        crate::blocktreeords::java_assert!(
            block_header.lines_count > 0,
            "Block header is not initialized"
        );
        output.write_v_int(block_header.lines_count)?;

        output.write_v_long(block_header.base_docs_fp)?;
        output.write_v_long(block_header.base_positions_fp)?;
        output.write_v_long(block_header.base_payloads_fp)?;

        output.write_v_int(block_header.term_states_base_offset)?;
        output.write_v_int(block_header.middle_line_offset)
    }

    /// Port of `BlockHeader.Serializer.read(DataInput, BlockHeader)`: the
    /// header is read into `reuse` when given (Java's reused instance), into
    /// a new one otherwise.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for an illegal number of lines or a negative
    /// offset; the I/O errors of the input.
    pub fn read(
        &self,
        input: &mut dyn DataInput,
        reuse: Option<BlockHeader>,
    ) -> Result<BlockHeader> {
        let lines_count = input.read_v_int()?;
        if lines_count <= 0 || lines_count > MAX_NUM_BLOCK_LINES {
            return Err(LuceneError::corrupt_index(
                format!("Illegal number of lines in block: {lines_count}"),
                input.to_java_string(),
            ));
        }

        let base_docs_fp = input.read_v_long()?;
        let base_positions_fp = input.read_v_long()?;
        let base_payloads_fp = input.read_v_long()?;

        let term_states_base_offset = input.read_v_int()?;
        if term_states_base_offset < 0 {
            return Err(LuceneError::corrupt_index(
                format!("Illegal termStatesBaseOffset= {term_states_base_offset}"),
                input.to_java_string(),
            ));
        }
        let middle_term_offset = input.read_v_int()?;
        if middle_term_offset < 0 {
            return Err(LuceneError::corrupt_index(
                format!("Illegal middleTermOffset= {middle_term_offset}"),
                input.to_java_string(),
            ));
        }

        let mut block_header = reuse.unwrap_or_default();
        block_header.reset(
            lines_count,
            base_docs_fp,
            base_positions_fp,
            base_payloads_fp,
            term_states_base_offset,
            middle_term_offset,
        );
        Ok(block_header)
    }
}

#[cfg(test)]
mod tests {
    use rucene::store::{ByteArrayDataInput, ByteBuffersDataOutput};

    use super::*;
    use rucene::error::LuceneErrorKind;

    fn encoded(values: &[i64]) -> Vec<u8> {
        let mut out = ByteBuffersDataOutput::new();
        out.write_v_int(values[0] as i32).unwrap();
        for v in &values[1..4] {
            out.write_v_long(*v).unwrap();
        }
        out.write_v_int(values[4] as i32).unwrap();
        out.write_v_int(values[5] as i32).unwrap();
        out.to_array_copy().unwrap()
    }

    fn read(values: &[i64]) -> Result<BlockHeader> {
        let bytes = encoded(values);
        let mut input = ByteArrayDataInput::new(&bytes);
        Serializer.read(&mut input, None)
    }

    fn message(e: LuceneError) -> String {
        assert!(
            matches!(
                &e,
                LuceneError {
                    kind: LuceneErrorKind::CorruptIndex { .. },
                    ..
                }
            ),
            "{e:?}"
        );
        e.to_string()
    }

    /// `BlockHeader.Serializer.write` then `read` give the header back, its
    /// middle line index derived from the line count.
    #[test]
    fn round_trip() {
        let header = BlockHeader::new(7, 10, 20, 30, 40, 5);
        let mut out = ByteBuffersDataOutput::new();
        Serializer.write(&mut out, &header).unwrap();
        let bytes = out.to_array_copy().unwrap();
        let mut input = ByteArrayDataInput::new(&bytes);
        let read = Serializer
            .read(&mut input, Some(BlockHeader::default()))
            .unwrap();
        assert_eq!(read, header);
        assert_eq!(read.get_middle_line_index(), 3);
    }

    /// Every check of `BlockHeader.Serializer.read`, at its bounds.
    #[test]
    fn read_checks() {
        assert!(read(&[1, 0, 0, 0, 0, 0]).is_ok());
        assert!(read(&[MAX_NUM_BLOCK_LINES as i64, 0, 0, 0, 0, 0]).is_ok());
        assert!(message(read(&[0, 0, 0, 0, 0, 0]).unwrap_err())
            .contains("Illegal number of lines in block: 0"));
        assert!(
            message(read(&[MAX_NUM_BLOCK_LINES as i64 + 1, 0, 0, 0, 0, 0]).unwrap_err())
                .contains("Illegal number of lines in block: 1001")
        );
        assert!(message(read(&[1, 0, 0, 0, -1, 0]).unwrap_err())
            .contains("Illegal termStatesBaseOffset= -1"));
        assert!(message(read(&[1, 0, 0, 0, 0, -1]).unwrap_err())
            .contains("Illegal middleTermOffset= -1"));
    }
}
