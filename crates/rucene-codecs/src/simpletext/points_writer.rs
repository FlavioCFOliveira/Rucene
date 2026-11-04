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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextPointsWriter`.

use std::sync::Arc;

use rucene::codecs::{PointsReader, PointsWriter, SegmentWriteState};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{FieldInfo, IntersectVisitor, Relation};
use rucene::store::{Directory, IOContext, IndexOutput};
use rucene::util::bkd::BKDConfig;
use rucene::util::java_collections::JavaHashMap;

use super::bkd_writer::{SimpleTextBKDWriter, DEFAULT_MAX_MB_SORT_IN_HEAP};
use super::points_format::{POINT_EXTENSION, POINT_INDEX_EXTENSION};
use super::util;

pub(crate) const NUM_DATA_DIMS: &[u8] = b"num data dims ";
pub(crate) const NUM_INDEX_DIMS: &[u8] = b"num index dims ";
pub(crate) const BYTES_PER_DIM: &[u8] = b"bytes per dim ";
pub(crate) const MAX_LEAF_POINTS: &[u8] = b"max leaf points ";
pub(crate) const INDEX_COUNT: &[u8] = b"index count ";
pub(crate) const BLOCK_COUNT: &[u8] = b"block count ";
pub(crate) const BLOCK_DOC_ID: &[u8] = b"  doc ";
pub(crate) const BLOCK_FP: &[u8] = b"  block fp ";
pub(crate) const BLOCK_VALUE: &[u8] = b"  block value ";
pub(crate) const SPLIT_COUNT: &[u8] = b"split count ";
pub(crate) const SPLIT_DIM: &[u8] = b"  split dim ";
pub(crate) const SPLIT_VALUE: &[u8] = b"  split value ";
pub(crate) const FIELD_COUNT: &[u8] = b"field count ";
pub(crate) const FIELD_FP_NAME: &[u8] = b"  field fp name ";
pub(crate) const FIELD_FP: &[u8] = b"  field fp ";
pub(crate) const MIN_VALUE: &[u8] = b"min value ";
pub(crate) const MAX_VALUE: &[u8] = b"max value ";
pub(crate) const POINT_COUNT: &[u8] = b"point count ";
pub(crate) const DOC_COUNT: &[u8] = b"doc count ";
pub(crate) const END: &[u8] = b"END";

/// Writes the points of a segment as plain text.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextPointsWriter`.
pub struct SimpleTextPointsWriter {
    data_out: Option<Box<dyn IndexOutput>>,
    directory: Arc<dyn Directory>,
    segment_name: String,
    segment_suffix: String,
    max_doc: i32,
    /// `writeState.context`, owned.
    context: Box<dyn IOContext>,
    /// `Map<String, Long> indexFPs = new HashMap<>()`, in Java's iteration
    /// order.
    index_fps: JavaHashMap<i64>,
}

impl std::fmt::Debug for SimpleTextPointsWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleTextPointsWriter")
            .field("segment_name", &self.segment_name)
            .finish_non_exhaustive()
    }
}

impl SimpleTextPointsWriter {
    /// Port of `SimpleTextPointsWriter(SegmentWriteState)`.
    ///
    /// # Errors
    ///
    /// Propagates the directory's error.
    pub fn new(write_state: &SegmentWriteState<'_>) -> Result<Self> {
        let file_name = segment_file_name(
            &write_state.segment_info.name,
            &write_state.segment_suffix,
            POINT_EXTENSION,
        );
        let data_out = write_state
            .directory
            .create_output(&file_name, write_state.context)?;
        Ok(Self {
            data_out: Some(data_out),
            directory: Arc::clone(&write_state.directory),
            segment_name: write_state.segment_info.name.clone(),
            segment_suffix: write_state.segment_suffix.clone(),
            max_doc: write_state.segment_info.max_doc()?,
            context: write_state.context.with_hints(write_state.context.hints()),
            index_fps: JavaHashMap::new(),
        })
    }

    fn data_out(&mut self) -> Result<&mut dyn IndexOutput> {
        match self.data_out.as_deref_mut() {
            Some(out) => Ok(out),
            None => Err(LuceneError::already_closed("this writer is closed")),
        }
    }
}

impl PointsWriter for SimpleTextPointsWriter {
    /// Port of `SimpleTextPointsWriter.writeField`.
    fn write_field(&mut self, field_info: &FieldInfo, reader: &dyn PointsReader) -> Result<()> {
        let point_values = reader.get_values(&field_info.name)?.ok_or_else(|| {
            LuceneError::illegal_state(format!(
                "Cannot invoke \"org.apache.lucene.index.PointValues.getPointTree()\" because the \
                 return value of \"org.apache.lucene.codecs.PointsReader.getValues(String)\" is \
                 null (field={})",
                field_info.name
            ))
        })?;
        let mut values = point_values.point_tree()?;

        let config = BKDConfig::new(
            field_info.get_point_dimension_count(),
            field_info.get_point_index_dimension_count(),
            field_info.get_point_num_bytes(),
            BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE,
        )?;

        // We use our own fork of the BKDWriter to customize how it writes the
        // index and blocks to disk:
        let mut writer = SimpleTextBKDWriter::new(
            self.max_doc,
            Arc::clone(&self.directory),
            &self.segment_name,
            config,
            f64::from(DEFAULT_MAX_MB_SORT_IN_HEAP),
            values.size()?,
        )?;

        let result = (|| -> Result<()> {
            let mut visitor = AddVisitor {
                writer: &mut writer,
            };
            values.visit_doc_values(&mut visitor)?;

            // We could have 0 points on merge since all docs with points may
            // be deleted:
            if writer.get_point_count() > 0 {
                let data_out = self.data_out()?;
                let fp = writer.finish(data_out)?;
                self.index_fps.insert(field_info.name.clone(), fp);
            }
            Ok(())
        })();
        let closed = writer.close();
        result?;
        closed
    }

    /// Port of `SimpleTextPointsWriter.finish`.
    fn finish(&mut self) -> Result<()> {
        let data_out = self.data_out()?;
        util::write_bytes(data_out, END)?;
        util::write_newline(data_out)?;
        util::write_checksum(data_out)
    }

    /// Port of `SimpleTextPointsWriter.close`.
    fn close(&mut self) -> Result<()> {
        if let Some(mut data_out) = self.data_out.take() {
            data_out.close()?;

            // Write index file
            let file_name = segment_file_name(
                &self.segment_name,
                &self.segment_suffix,
                POINT_INDEX_EXTENSION,
            );
            let mut index_out = self
                .directory
                .create_output(&file_name, self.context.as_ref())?;
            let out = index_out.as_mut();
            let count = self.index_fps.len();
            util::write_bytes(out, FIELD_COUNT)?;
            util::write_str(out, &count.to_string())?;
            util::write_newline(out)?;
            for (key, value) in self.index_fps.iter() {
                util::write_bytes(out, FIELD_FP_NAME)?;
                util::write_str(out, key)?;
                util::write_newline(out)?;
                util::write_bytes(out, FIELD_FP)?;
                util::write_str(out, &value.to_string())?;
                util::write_newline(out)?;
            }
            util::write_checksum(out)?;
            index_out.close()?;
        }
        Ok(())
    }
}

/// The anonymous `IntersectVisitor` of `writeField`, which adds every point
/// to the BKD writer.
struct AddVisitor<'a> {
    writer: &'a mut SimpleTextBKDWriter,
}

impl IntersectVisitor for AddVisitor<'_> {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Err(LuceneError::illegal_state(String::new()))
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.writer.add(packed_value, doc_id)
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Result<Relation> {
        Ok(Relation::CellCrossesQuery)
    }
}
