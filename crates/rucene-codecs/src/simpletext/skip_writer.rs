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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextSkipWriter`.

use std::collections::HashMap;

use rucene::codecs::{CompetitiveImpactAccumulator, MultiLevelSkipListWriter, SkipDataWriter};
use rucene::error::Result;
use rucene::store::{DataOutput, IndexOutput};

use super::util;

pub(crate) const SKIP_MULTIPLIER: i32 = 3;
pub(crate) const MAX_SKIP_LEVELS: i32 = 4;

pub(crate) const BLOCK_SIZE: i32 = 8;

pub(crate) const SKIP_LIST: &[u8] = b"    skipList ";
pub(crate) const LEVEL_LENGTH: &[u8] = b"      levelLength ";
pub(crate) const LEVEL: &[u8] = b"      level ";
pub(crate) const SKIP_DOC: &[u8] = b"        skipDoc ";
pub(crate) const SKIP_DOC_FP: &[u8] = b"        skipDocFP ";
pub(crate) const IMPACTS: &[u8] = b"        impacts ";
pub(crate) const IMPACT: &[u8] = b"          impact ";
pub(crate) const FREQ: &[u8] = b"            freq ";
pub(crate) const NORM: &[u8] = b"            norm ";
pub(crate) const IMPACTS_END: &[u8] = b"        impactsEnd ";
pub(crate) const CHILD_POINTER: &[u8] = b"        childPointer ";

/// Plain text skip data.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextSkipWriter`, a
/// `MultiLevelSkipListWriter` subclass: the base class is the wrapped
/// [`MultiLevelSkipListWriter`], the subclass state and overrides are
/// [`SkipData`].
#[derive(Debug)]
pub(crate) struct SimpleTextSkipWriter {
    inner: MultiLevelSkipListWriter<SkipData>,
}

/// The subclass part of `SimpleTextSkipWriter`: its fields and its overrides
/// of `writeSkipData`, `writeLevelLength` and `writeChildPointer`.
#[derive(Debug)]
pub(crate) struct SkipData {
    wrote_header_per_level_map: HashMap<i32, bool>,
    cur_doc: i32,
    cur_doc_file_pointer: i64,
    cur_competitive_freq_norms: Vec<CompetitiveImpactAccumulator>,
    /// The base class's `numberOfSkipLevels`, which `writeSkipData` reads.
    /// The base class sets it once, in its constructor.
    number_of_skip_levels: i32,
}

impl SimpleTextSkipWriter {
    /// Port of `SimpleTextSkipWriter(SegmentWriteState)`.
    pub(crate) fn new(max_doc: i32) -> Result<Self> {
        let data = SkipData {
            wrote_header_per_level_map: HashMap::new(),
            cur_doc: -1,
            cur_doc_file_pointer: -1,
            cur_competitive_freq_norms: (0..MAX_SKIP_LEVELS)
                .map(|_| CompetitiveImpactAccumulator::new())
                .collect(),
            number_of_skip_levels: 0,
        };
        let mut inner = MultiLevelSkipListWriter::new(
            data,
            BLOCK_SIZE,
            SKIP_MULTIPLIER,
            MAX_SKIP_LEVELS,
            max_doc,
        )?;
        inner.skip_data_writer_mut().number_of_skip_levels = inner.number_of_skip_levels();
        let mut writer = Self { inner };
        writer.reset_skip();
        Ok(writer)
    }

    /// Port of `SimpleTextSkipWriter.resetSkip`.
    pub(crate) fn reset_skip(&mut self) {
        self.inner.reset_skip();
        let data = self.inner.skip_data_writer_mut();
        data.wrote_header_per_level_map.clear();
        data.cur_doc = -1;
        data.cur_doc_file_pointer = -1;
        for acc in &mut data.cur_competitive_freq_norms {
            acc.clear();
        }
    }

    /// Port of `SimpleTextSkipWriter.writeSkip`.
    pub(crate) fn write_skip(&mut self, output: &mut dyn IndexOutput) -> Result<i64> {
        let skip_offset = output.file_pointer();
        util::write_bytes(output, SKIP_LIST)?;
        util::write_newline(output)?;
        self.inner.write_skip(output)?;
        Ok(skip_offset)
    }

    /// Port of `SimpleTextSkipWriter.bufferSkip(int, long, int,
    /// CompetitiveImpactAccumulator)`.
    pub(crate) fn buffer_skip(
        &mut self,
        doc: i32,
        doc_file_pointer: i64,
        num_docs: i32,
        competitive_impact_accumulator: &CompetitiveImpactAccumulator,
    ) -> Result<()> {
        let data = self.inner.skip_data_writer_mut();
        debug_assert!(doc > data.cur_doc);
        data.cur_doc = doc;
        data.cur_doc_file_pointer = doc_file_pointer;
        data.cur_competitive_freq_norms[0].add_all(competitive_impact_accumulator);
        self.inner.buffer_skip(num_docs)
    }
}

impl SkipDataWriter for SkipData {
    /// Port of `SimpleTextSkipWriter.writeSkipData`.
    fn write_skip_data(&mut self, level: i32, skip_buffer: &mut dyn DataOutput) -> Result<()> {
        let wrote_header = self.wrote_header_per_level_map.get(&level).copied();
        if wrote_header != Some(true) {
            util::write_bytes(skip_buffer, LEVEL)?;
            util::write_str(skip_buffer, &level.to_string())?;
            util::write_newline(skip_buffer)?;

            self.wrote_header_per_level_map.insert(level, true);
        }
        util::write_bytes(skip_buffer, SKIP_DOC)?;
        util::write_str(skip_buffer, &self.cur_doc.to_string())?;
        util::write_newline(skip_buffer)?;

        util::write_bytes(skip_buffer, SKIP_DOC_FP)?;
        util::write_str(skip_buffer, &self.cur_doc_file_pointer.to_string())?;
        util::write_newline(skip_buffer)?;

        let level = level as usize;
        let impacts = self.cur_competitive_freq_norms[level].get_competitive_freq_norm_pairs();
        debug_assert!(!impacts.is_empty());
        if (level as i32) + 1 < self.number_of_skip_levels {
            let (lower, upper) = self.cur_competitive_freq_norms.split_at_mut(level + 1);
            upper[0].add_all(&lower[level]);
        }
        util::write_bytes(skip_buffer, IMPACTS)?;
        util::write_newline(skip_buffer)?;
        for impact in &impacts {
            util::write_bytes(skip_buffer, IMPACT)?;
            util::write_newline(skip_buffer)?;
            util::write_bytes(skip_buffer, FREQ)?;
            util::write_str(skip_buffer, &impact.freq.to_string())?;
            util::write_newline(skip_buffer)?;
            util::write_bytes(skip_buffer, NORM)?;
            util::write_str(skip_buffer, &impact.norm.to_string())?;
            util::write_newline(skip_buffer)?;
        }
        util::write_bytes(skip_buffer, IMPACTS_END)?;
        util::write_newline(skip_buffer)?;
        self.cur_competitive_freq_norms[level].clear();
        Ok(())
    }

    /// Port of `SimpleTextSkipWriter.writeLevelLength`.
    fn write_level_length(
        &mut self,
        level_length: i64,
        output: &mut dyn IndexOutput,
    ) -> Result<()> {
        util::write_bytes(output, LEVEL_LENGTH)?;
        util::write_str(output, &level_length.to_string())?;
        util::write_newline(output)
    }

    /// Port of `SimpleTextSkipWriter.writeChildPointer`.
    fn write_child_pointer(
        &mut self,
        child_pointer: i64,
        skip_buffer: &mut dyn DataOutput,
    ) -> Result<()> {
        util::write_bytes(skip_buffer, CHILD_POINTER)?;
        util::write_str(skip_buffer, &child_pointer.to_string())?;
        util::write_newline(skip_buffer)
    }
}
