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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextPointsReader`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rucene::codecs::{PointsReader, SegmentReadState};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{FieldInfos, PointValues};
use rucene::store::{BufferedChecksumIndexInput, IndexInput, DEFAULT_IO_CONTEXT};
use rucene::util::IOUtils;

use super::bkd_reader::{SharedDataIn, SimpleTextBKDReader};
use super::points_format::{POINT_EXTENSION, POINT_INDEX_EXTENSION};
use super::points_writer::{
    BLOCK_FP, BYTES_PER_DIM, DOC_COUNT, FIELD_COUNT, FIELD_FP, FIELD_FP_NAME, INDEX_COUNT,
    MAX_LEAF_POINTS, MAX_VALUE, MIN_VALUE, NUM_DATA_DIMS, NUM_INDEX_DIMS, POINT_COUNT, SPLIT_COUNT,
    SPLIT_DIM, SPLIT_VALUE,
};
use super::util::{self, parse_int, parse_long, read_line, CHECKSUM};

/// Reads the points of a segment written as plain text.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextPointsReader`.
///
/// `getMergeInstance()` returns the reader itself in Java; the port's
/// returns a handle that shares all of this reader's state.
#[derive(Clone)]
pub struct SimpleTextPointsReader {
    data_in: Arc<SharedDataIn>,
    field_infos: Arc<FieldInfos>,
    segment_name: String,
    max_doc: i32,
    readers: Arc<HashMap<String, SimpleTextBKDReader>>,
}

impl std::fmt::Debug for SimpleTextPointsReader {
    /// `SimpleTextPointsReader(segment=... maxDoc=...)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SimpleTextPointsReader(segment={} maxDoc={})",
            self.segment_name, self.max_doc
        )
    }
}

fn strip_prefix(scratch: &[u8], prefix: &[u8]) -> String {
    util::read_string(scratch, prefix.len())
}

fn parse_int_prefix(scratch: &[u8], prefix: &[u8]) -> Result<i32> {
    debug_assert!(scratch.starts_with(prefix));
    parse_int(&strip_prefix(scratch, prefix))
}

fn parse_long_prefix(scratch: &[u8], prefix: &[u8]) -> Result<i64> {
    debug_assert!(scratch.starts_with(prefix));
    parse_long(&strip_prefix(scratch, prefix))
}

impl SimpleTextPointsReader {
    /// Port of `SimpleTextPointsReader(SegmentReadState)`.
    ///
    /// # Errors
    ///
    /// Propagates the directory's and the parsers' errors.
    pub fn new(read_state: &SegmentReadState<'_>) -> Result<Self> {
        // Initialize readers now:

        // Read index:
        let mut field_to_file_offset: HashMap<String, i64> = HashMap::new();

        let index_file_name = segment_file_name(
            &read_state.segment_info.name,
            &read_state.segment_suffix,
            POINT_INDEX_EXTENSION,
        );
        let mut scratch = Vec::new();
        {
            // `try (ChecksumIndexInput in = ...)` (`SimpleTextPointsReader.java:73-84`).
            let mut input = read_state.directory.open_checksum_input(&index_file_name)?;
            let read = (|| -> Result<()> {
                read_line(input.as_mut(), &mut scratch)?;
                let count = parse_int_prefix(&scratch, FIELD_COUNT)?;
                for _ in 0..count {
                    read_line(input.as_mut(), &mut scratch)?;
                    let field_name = strip_prefix(&scratch, FIELD_FP_NAME);
                    read_line(input.as_mut(), &mut scratch)?;
                    let fp = parse_long_prefix(&scratch, FIELD_FP)?;
                    field_to_file_offset.insert(field_name, fp);
                }
                util::check_footer(input.as_mut())?;
                Ok(())
            })();
            rucene::store::close_after(input.as_mut(), read)?;
        }

        let file_name = segment_file_name(
            &read_state.segment_info.name,
            &read_state.segment_suffix,
            POINT_EXTENSION,
        );
        let data_in = read_state
            .directory
            .open_input(&file_name, &*DEFAULT_IO_CONTEXT)?;
        let shared = Arc::new(SharedDataIn {
            input: Mutex::new(Some(data_in)),
        });
        let mut readers = HashMap::new();
        let initialized = (|| -> Result<()> {
            for (name, fp) in &field_to_file_offset {
                readers.insert(name.clone(), Self::init_reader(&shared, *fp, &mut scratch)?);
            }
            Ok(())
        })();
        if let Err(error) = initialized {
            // `IOUtils.closeWhileHandlingException(this)` in the `finally`
            // (`SimpleTextPointsReader.java:93-102`); `close()` is
            // `dataIn.close()`.
            let data_in = shared.input.lock().ok().and_then(|mut input| input.take());
            IOUtils::close_while_handling_exception([data_in.map(|mut i| move || i.close())])?;
            return Err(error);
        }

        Ok(Self {
            data_in: shared,
            field_infos: Arc::new(read_state.field_infos.clone()),
            segment_name: read_state.segment_info.name.clone(),
            max_doc: read_state.segment_info.max_doc()?,
            readers: Arc::new(readers),
        })
    }

    /// Port of the private `initReader`.
    fn init_reader(
        shared: &Arc<SharedDataIn>,
        fp: i64,
        scratch: &mut Vec<u8>,
    ) -> Result<SimpleTextBKDReader> {
        let mut guard = shared.input.lock().map_err(|_| {
            LuceneError::illegal_state("points input lock was poisoned".to_string())
        })?;
        let data_in: &mut dyn IndexInput = guard
            .as_deref_mut()
            .ok_or_else(|| LuceneError::already_closed("this IndexInput is closed"))?;
        // NOTE: matches what writeIndex does in SimpleTextPointsWriter
        data_in.seek(fp)?;
        read_line(data_in, scratch)?;
        let num_data_dims = parse_int_prefix(scratch, NUM_DATA_DIMS)?;

        read_line(data_in, scratch)?;
        let num_index_dims = parse_int_prefix(scratch, NUM_INDEX_DIMS)?;

        read_line(data_in, scratch)?;
        let bytes_per_dim = parse_int_prefix(scratch, BYTES_PER_DIM)?;

        read_line(data_in, scratch)?;
        let max_points_in_leaf_node = parse_int_prefix(scratch, MAX_LEAF_POINTS)?;

        read_line(data_in, scratch)?;
        let count = parse_int_prefix(scratch, INDEX_COUNT)?;

        read_line(data_in, scratch)?;
        debug_assert!(scratch.starts_with(MIN_VALUE));
        let min_value = util::from_bytes_ref_string(&strip_prefix(scratch, MIN_VALUE))?;
        debug_assert_eq!(min_value.length as i32, num_index_dims * bytes_per_dim);

        read_line(data_in, scratch)?;
        debug_assert!(scratch.starts_with(MAX_VALUE));
        let max_value = util::from_bytes_ref_string(&strip_prefix(scratch, MAX_VALUE))?;
        debug_assert_eq!(max_value.length as i32, num_index_dims * bytes_per_dim);

        read_line(data_in, scratch)?;
        debug_assert!(scratch.starts_with(POINT_COUNT));
        let point_count = parse_long_prefix(scratch, POINT_COUNT)?;

        read_line(data_in, scratch)?;
        debug_assert!(scratch.starts_with(DOC_COUNT));
        let doc_count = parse_int_prefix(scratch, DOC_COUNT)?;

        let mut leaf_block_fps = Vec::with_capacity(count.max(0) as usize);
        for _ in 0..count {
            read_line(data_in, scratch)?;
            leaf_block_fps.push(parse_long_prefix(scratch, BLOCK_FP)?);
        }
        read_line(data_in, scratch)?;
        let count = parse_int_prefix(scratch, SPLIT_COUNT)?;

        let bytes_per_index_entry = if num_index_dims == 1 {
            bytes_per_dim
        } else {
            1 + bytes_per_dim
        } as usize;
        let mut split_packed_values = vec![0u8; count.max(0) as usize * bytes_per_index_entry];
        for i in 0..count.max(0) as usize {
            read_line(data_in, scratch)?;
            let mut address = bytes_per_index_entry * i;
            let split_dim = parse_int_prefix(scratch, SPLIT_DIM)?;
            if num_index_dims != 1 {
                split_packed_values[address] = split_dim as u8;
                address += 1;
            }
            read_line(data_in, scratch)?;
            debug_assert!(scratch.starts_with(SPLIT_VALUE));
            let br = util::from_bytes_ref_string(&strip_prefix(scratch, SPLIT_VALUE))?;
            debug_assert_eq!(br.length as i32, bytes_per_dim);
            split_packed_values[address..address + bytes_per_dim as usize]
                .copy_from_slice(&br.slice()[..bytes_per_dim as usize]);
        }
        drop(guard);

        SimpleTextBKDReader::new(
            Arc::clone(shared),
            num_data_dims,
            num_index_dims,
            max_points_in_leaf_node,
            bytes_per_dim,
            leaf_block_fps,
            split_packed_values,
            min_value.slice().to_vec(),
            max_value.slice().to_vec(),
            point_count,
            doc_count,
        )
    }
}

impl PointsReader for SimpleTextPointsReader {
    /// Port of `SimpleTextPointsReader.checkIntegrity`.
    fn check_integrity(&self) -> Result<()> {
        let mut scratch = Vec::new();
        let mut clone = self.data_in.clone_input_pub()?;
        clone.seek(0)?;

        // checksum is fixed-width encoded with 20 bytes, plus 1 byte for
        // newline (the space is included in SimpleTextUtil.CHECKSUM):
        let footer_start_pos = clone.length() - (CHECKSUM.len() as i64 + 21);
        let mut input = BufferedChecksumIndexInput::new(clone);
        loop {
            read_line(&mut input, &mut scratch)?;
            if input.file_pointer() >= footer_start_pos {
                // Make sure we landed at precisely the right location:
                if input.file_pointer() != footer_start_pos {
                    return Err(LuceneError::corrupt_index(
                        format!(
                            "SimpleText failure: footer does not start at expected position \
                             current={} vs expected={footer_start_pos}",
                            input.file_pointer()
                        ),
                        IndexInput::resource_description(&input),
                    ));
                }
                return util::check_footer(&mut input);
            }
        }
    }

    /// Port of `SimpleTextPointsReader.getValues`.
    fn get_values(&self, field_name: &str) -> Result<Option<Box<dyn PointValues>>> {
        let Some(field_info) = self.field_infos.field_info(field_name) else {
            return Err(LuceneError::illegal_argument(format!(
                "field=\"{field_name}\" is unrecognized"
            )));
        };
        if field_info.get_point_dimension_count() == 0 {
            return Err(LuceneError::illegal_argument(format!(
                "field=\"{field_name}\" did not index points"
            )));
        }
        Ok(self
            .readers
            .get(field_name)
            .map(|r| Box::new(r.clone()) as Box<dyn PointValues>))
    }

    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&self) -> Result<()> {
        let data_in = self
            .data_in
            .input
            .lock()
            .map_err(|_| LuceneError::illegal_state("points input lock was poisoned".to_string()))?
            .take();
        match data_in {
            Some(mut data_in) => data_in.close(),
            None => Ok(()),
        }
    }
}
