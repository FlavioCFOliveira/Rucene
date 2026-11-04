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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextSkipReader`.

use rucene::codecs::{MultiLevelSkipListReader, SkipDataReader};
use rucene::error::Result;
use rucene::index::{FreqAndNormBuffer, Impacts};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{BufferedChecksumIndexInput, IndexInput};

use super::fields_writer::{END, FIELD, TERM};
use super::skip_writer::{
    BLOCK_SIZE, CHILD_POINTER, FREQ, IMPACT, IMPACTS, IMPACTS_END, LEVEL_LENGTH, MAX_SKIP_LEVELS,
    NORM, SKIP_DOC, SKIP_DOC_FP, SKIP_LIST, SKIP_MULTIPLIER,
};
use super::util::{self, parse_int_at, parse_long, read_line, read_string};

/// Reads skip lists with multiple levels.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextSkipReader`, a
/// `MultiLevelSkipListReader` subclass: the base class is the wrapped
/// [`MultiLevelSkipListReader`], the subclass's `readSkipData`,
/// `readLevelLength` and `readChildPointer` are [`SkipReaderData`]'s.
pub(crate) struct SimpleTextSkipReader {
    inner: MultiLevelSkipListReader<SkipReaderData>,
    num_levels: i32,
    has_skip_list: bool,
}

/// The state `readSkipData` updates: Java's `perLevelImpacts` and
/// `nextSkipDocFP`.
pub(crate) struct SkipReaderData {
    per_level_impacts: Vec<FreqAndNormBuffer>,
    next_skip_doc_fp: i64,
}

impl SimpleTextSkipReader {
    /// Port of `SimpleTextSkipReader(IndexInput)`.
    pub(crate) fn new(skip_stream: Box<dyn IndexInput>) -> Self {
        let data = SkipReaderData {
            per_level_impacts: Vec::new(),
            next_skip_doc_fp: -1,
        };
        let inner = MultiLevelSkipListReader::new(
            skip_stream,
            data,
            MAX_SKIP_LEVELS,
            BLOCK_SIZE,
            SKIP_MULTIPLIER,
        );
        let mut reader = Self {
            inner,
            num_levels: 1,
            has_skip_list: false,
        };
        reader.init();
        reader
    }

    /// Port of `SimpleTextSkipReader.skipTo`.
    pub(crate) fn skip_to(&mut self, target: i32) -> Result<i32> {
        if !self.has_skip_list {
            return Ok(-1);
        }
        let result = self.inner.skip_to(target)?;
        if self.inner.number_of_skip_levels() > 0 {
            self.num_levels = self.inner.number_of_skip_levels();
        } else {
            // End of postings don't have skip data anymore, so we fill with
            // dummy data like SlowImpactsEnum.
            self.num_levels = 1;
            let mut freq_and_norms = FreqAndNormBuffer::new();
            freq_and_norms.add(i32::MAX, 1);
            self.inner
                .skip_data_reader_mut()
                .per_level_impacts
                .insert(0, freq_and_norms);
        }
        Ok(result)
    }

    /// Port of `SimpleTextSkipReader.reset`.
    pub(crate) fn reset(&mut self, skip_pointer: i64, doc_freq: i32) -> Result<()> {
        self.init();
        if skip_pointer > 0 {
            self.inner.init(skip_pointer, doc_freq)?;
            self.has_skip_list = true;
        }
        Ok(())
    }

    /// Port of the private `SimpleTextSkipReader.init`.
    fn init(&mut self) {
        let max_levels = self.inner.max_number_of_skip_levels();
        let data = self.inner.skip_data_reader_mut();
        data.next_skip_doc_fp = -1;
        self.num_levels = 1;
        data.per_level_impacts = (0..max_levels)
            .map(|_| {
                let mut impacts = FreqAndNormBuffer::new();
                impacts.add(i32::MAX, 1);
                impacts
            })
            .collect();
        self.has_skip_list = false;
    }

    /// Port of `SimpleTextSkipReader.getImpacts`.
    ///
    /// **Divergence.** Java returns the one anonymous `Impacts` the
    /// constructor built, which reads the skip reader's `numLevels`,
    /// `skipDoc` and `perLevelImpacts` on every call
    /// (`SimpleTextSkipReader.java:53-71`), so a caller that keeps it across an
    /// `advanceShallow` sees the new state. The port's
    /// `ImpactsSource::get_impacts` returns an owned `Box<dyn Impacts>`, which
    /// cannot borrow the enum that keeps advancing; this is a copy of that
    /// state taken when the caller asks for it. Every caller reads the impacts
    /// right after `advanceShallow`, before advancing again, where the two
    /// agree (the oracle's `imp` lines compare them at every target).
    pub(crate) fn get_impacts(&self) -> SimpleTextImpacts {
        let num_levels = self.num_levels;
        SimpleTextImpacts {
            num_levels,
            doc_id_up_to: (0..num_levels.max(0))
                .map(|level| self.inner.skip_doc(level))
                .collect(),
            per_level_impacts: self.inner.skip_data_reader().per_level_impacts.clone(),
        }
    }

    /// Port of `SimpleTextSkipReader.getNextSkipDocFP`.
    pub(crate) fn get_next_skip_doc_fp(&self) -> i64 {
        self.inner.skip_data_reader().next_skip_doc_fp
    }

    /// Port of `SimpleTextSkipReader.getNextSkipDoc`.
    pub(crate) fn get_next_skip_doc(&self) -> i32 {
        if !self.has_skip_list {
            return NO_MORE_DOCS;
        }
        self.inner.skip_doc(0)
    }

    /// Port of `SimpleTextSkipReader.hasSkipList`.
    #[allow(dead_code)]
    pub(crate) fn has_skip_list(&self) -> bool {
        self.has_skip_list
    }
}

/// The anonymous `Impacts` of `SimpleTextSkipReader`, at the moment
/// [`SimpleTextSkipReader::get_impacts`] was called.
#[derive(Debug, Clone)]
pub(crate) struct SimpleTextImpacts {
    num_levels: i32,
    doc_id_up_to: Vec<i32>,
    per_level_impacts: Vec<FreqAndNormBuffer>,
}

impl Impacts for SimpleTextImpacts {
    fn num_levels(&self) -> i32 {
        self.num_levels
    }

    fn doc_id_up_to(&self, level: i32) -> i32 {
        self.doc_id_up_to[level as usize]
    }

    fn get_impacts(&self, level: i32) -> FreqAndNormBuffer {
        debug_assert!(level < self.num_levels);
        self.per_level_impacts[level as usize].clone()
    }
}

/// `scratchUTF16.copyUTF8Bytes(...)` then `ArrayUtil.parseInt`.
fn parse_int_after(scratch: &[u8], prefix: &[u8]) -> Result<i32> {
    parse_int_at(scratch, prefix.len())
}

impl SkipDataReader for SkipReaderData {
    /// Port of `SimpleTextSkipReader.readSkipData`.
    ///
    /// Java reads through `new BufferedChecksumIndexInput(skipStream)`, which
    /// advances `skipStream` itself. The port reads through a checksum input
    /// over a clone of `skipStream` at the same position, then moves
    /// `skipStream` to where the clone stopped: the same bytes are read and
    /// checksummed, and `skipStream` ends where Java's does.
    fn read_skip_data(
        &mut self,
        level: i32,
        skip_stream: &mut dyn IndexInput,
        skip_doc_array: &[i32],
    ) -> Result<i32> {
        self.per_level_impacts[level as usize].size = 0;
        let mut skip_doc = NO_MORE_DOCS;
        let mut input = BufferedChecksumIndexInput::new(skip_stream.clone_input()?);
        let mut scratch = Vec::new();
        let mut freq = 1;
        let result = (|| -> Result<()> {
            loop {
                read_line(&mut input, &mut scratch)?;
                if scratch.as_slice() == END {
                    util::check_footer(&mut input)?;
                    break;
                } else if scratch.as_slice() == IMPACTS_END
                    || scratch.as_slice() == TERM
                    || scratch.as_slice() == FIELD
                {
                    break;
                } else if scratch.starts_with(SKIP_LIST) {
                    // continue
                } else if scratch.starts_with(SKIP_DOC) {
                    skip_doc = parse_int_after(&scratch, SKIP_DOC)?;
                    // Because the MultiLevelSkipListReader stores doc id
                    // delta, but simple text codec stores doc id
                    skip_doc = skip_doc.wrapping_sub(skip_doc_array[level as usize]);
                } else if scratch.starts_with(SKIP_DOC_FP) {
                    self.next_skip_doc_fp = i64::from(parse_int_after(&scratch, SKIP_DOC_FP)?);
                } else if scratch.starts_with(IMPACTS) || scratch.starts_with(IMPACT) {
                    // continue;
                } else if scratch.starts_with(FREQ) {
                    freq = parse_int_after(&scratch, FREQ)?;
                } else if scratch.starts_with(NORM) {
                    let norm = parse_long(&read_string(&scratch, NORM.len()))?;
                    self.per_level_impacts[level as usize].add(freq, norm);
                }
            }
            Ok(())
        })();
        skip_stream.seek(input.file_pointer())?;
        result?;
        Ok(skip_doc)
    }

    /// Port of `SimpleTextSkipReader.readLevelLength`.
    fn read_level_length(&self, skip_stream: &mut dyn IndexInput) -> Result<i64> {
        let mut scratch = Vec::new();
        read_line(skip_stream, &mut scratch)?;
        parse_long(&read_string(&scratch, LEVEL_LENGTH.len()))
    }

    /// Port of `SimpleTextSkipReader.readChildPointer`.
    fn read_child_pointer(&self, skip_stream: &mut dyn IndexInput) -> Result<i64> {
        let mut scratch = Vec::new();
        read_line(skip_stream, &mut scratch)?;
        parse_long(&read_string(&scratch, CHILD_POINTER.len()))
    }
}
