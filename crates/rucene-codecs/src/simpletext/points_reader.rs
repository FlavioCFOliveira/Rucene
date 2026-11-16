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

use rucene::util::java_collections::JavaHashMap;
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
use super::util::{self, parse_int, parse_long, read_line, JavaScratch, CHECKSUM};

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

fn strip_prefix(scratch: &JavaScratch, prefix: &[u8]) -> Result<String> {
    scratch.new_string(prefix.len())
}

fn parse_int_prefix(scratch: &JavaScratch, prefix: &[u8]) -> Result<i32> {
    debug_assert!(scratch.starts_with(prefix));
    parse_int(&strip_prefix(scratch, prefix)?)
}

fn parse_long_prefix(scratch: &JavaScratch, prefix: &[u8]) -> Result<i64> {
    debug_assert!(scratch.starts_with(prefix));
    parse_long(&strip_prefix(scratch, prefix)?)
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
        // A `java.util.HashMap`: `initReader` visits the fields in its
        // iteration order (`SimpleTextPointsReader.java:67-97`).
        let mut field_to_file_offset: JavaHashMap<i64> = JavaHashMap::new();

        let index_file_name = segment_file_name(
            &read_state.segment_info.name,
            &read_state.segment_suffix,
            POINT_INDEX_EXTENSION,
        );
        let mut scratch = JavaScratch::new();
        {
            // `try (ChecksumIndexInput in = ...)` (`SimpleTextPointsReader.java:73-84`).
            let mut input = read_state.directory.open_checksum_input(&index_file_name)?;
            let read = (|| -> Result<()> {
                scratch.read_line(input.as_mut())?;
                let count = parse_int_prefix(&scratch, FIELD_COUNT)?;
                for _ in 0..count {
                    scratch.read_line(input.as_mut())?;
                    let field_name = strip_prefix(&scratch, FIELD_FP_NAME)?;
                    scratch.read_line(input.as_mut())?;
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
            for (name, fp) in field_to_file_offset.iter() {
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
        scratch: &mut JavaScratch,
    ) -> Result<SimpleTextBKDReader> {
        let mut guard = shared.input.lock().map_err(|_| {
            LuceneError::illegal_state("points input lock was poisoned".to_string())
        })?;
        let data_in: &mut dyn IndexInput = guard
            .as_deref_mut()
            .ok_or_else(|| LuceneError::already_closed("this IndexInput is closed"))?;
        // NOTE: matches what writeIndex does in SimpleTextPointsWriter
        data_in.seek(fp)?;
        scratch.read_line(data_in)?;
        let num_data_dims = parse_int_prefix(scratch, NUM_DATA_DIMS)?;

        scratch.read_line(data_in)?;
        let num_index_dims = parse_int_prefix(scratch, NUM_INDEX_DIMS)?;

        scratch.read_line(data_in)?;
        let bytes_per_dim = parse_int_prefix(scratch, BYTES_PER_DIM)?;

        scratch.read_line(data_in)?;
        let max_points_in_leaf_node = parse_int_prefix(scratch, MAX_LEAF_POINTS)?;

        scratch.read_line(data_in)?;
        let count = parse_int_prefix(scratch, INDEX_COUNT)?;

        scratch.read_line(data_in)?;
        debug_assert!(scratch.starts_with(MIN_VALUE));
        let min_value = util::from_bytes_ref_string(&strip_prefix(scratch, MIN_VALUE)?)?;
        debug_assert_eq!(
            min_value.length as i32,
            num_index_dims.wrapping_mul(bytes_per_dim)
        );

        scratch.read_line(data_in)?;
        debug_assert!(scratch.starts_with(MAX_VALUE));
        let max_value = util::from_bytes_ref_string(&strip_prefix(scratch, MAX_VALUE)?)?;
        debug_assert_eq!(
            max_value.length as i32,
            num_index_dims.wrapping_mul(bytes_per_dim)
        );

        scratch.read_line(data_in)?;
        debug_assert!(scratch.starts_with(POINT_COUNT));
        let point_count = parse_long_prefix(scratch, POINT_COUNT)?;

        scratch.read_line(data_in)?;
        debug_assert!(scratch.starts_with(DOC_COUNT));
        let doc_count = parse_int_prefix(scratch, DOC_COUNT)?;

        // `new long[count]` (`SimpleTextPointsReader.java:136`).
        if count < 0 {
            return Err(util::negative_array_size(count));
        }
        let mut leaf_block_fps = Vec::with_capacity(count as usize);
        for _ in 0..count {
            scratch.read_line(data_in)?;
            leaf_block_fps.push(parse_long_prefix(scratch, BLOCK_FP)?);
        }
        scratch.read_line(data_in)?;
        let count = parse_int_prefix(scratch, SPLIT_COUNT)?;

        let bytes_per_index_entry: i32 = if num_index_dims == 1 {
            bytes_per_dim
        } else {
            1i32.wrapping_add(bytes_per_dim)
        };
        // `new byte[count * bytesPerIndexEntry]`, an `int` product
        // (`SimpleTextPointsReader.java:155`).
        let split_len = count.wrapping_mul(bytes_per_index_entry);
        if split_len < 0 {
            return Err(util::negative_array_size(split_len));
        }
        let mut split_packed_values = vec![0u8; split_len as usize];
        for i in 0..count {
            scratch.read_line(data_in)?;
            let mut address = bytes_per_index_entry.wrapping_mul(i);
            let split_dim = parse_int_prefix(scratch, SPLIT_DIM)?;
            if num_index_dims != 1 {
                // `splitPackedValues[address++] = (byte) splitDim`.
                let length = split_packed_values.len();
                let slot = usize::try_from(address)
                    .ok()
                    .and_then(|a| split_packed_values.get_mut(a))
                    .ok_or_else(|| util::array_index_out_of_bounds(i64::from(address), length))?;
                *slot = split_dim as u8;
                address = address.wrapping_add(1);
            }
            scratch.read_line(data_in)?;
            debug_assert!(scratch.starts_with(SPLIT_VALUE));
            let br = util::from_bytes_ref_string(&strip_prefix(scratch, SPLIT_VALUE)?)?;
            debug_assert_eq!(br.length as i32, bytes_per_dim);
            util::arraycopy(
                br.slice(),
                0,
                &mut split_packed_values,
                i64::from(address),
                i64::from(bytes_per_dim),
            )?;
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
            if input.file_pointer()? >= footer_start_pos {
                // Make sure we landed at precisely the right location:
                if input.file_pointer()? != footer_start_pos {
                    return Err(LuceneError::corrupt_index(
                        format!(
                            "SimpleText failure: footer does not start at expected position \
                             current={} vs expected={footer_start_pos}",
                            input.file_pointer()?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simpletext::test_support::{directory, segment_info, unchecked, write_file};
    use rucene::error::LuceneErrorKind;
    use rucene::index::FieldInfos;

    fn open(dii: &[&str], dim: &[&str]) -> LuceneError {
        let dir = directory();
        write_file(dir.as_ref(), "_0.dii", dii, true);
        write_file(dir.as_ref(), "_0.dim", dim, false);
        let si = segment_info(&dir, "_0", 1);
        let infos = FieldInfos::new(Vec::new()).unwrap();
        let state = SegmentReadState::new(dir.as_ref(), &si, &infos, &*DEFAULT_IO_CONTEXT);
        SimpleTextPointsReader::new(&state).expect_err("an error")
    }

    /// Regression (rmp #526): the readers are initialised in
    /// `java.util.HashMap` order (`a`..`h` hash to buckets 1..8), so the
    /// first corrupt field reported is `a`; the port used a randomly seeded
    /// `std` map.
    #[test]
    fn fields_are_initialised_in_java_hash_map_order() {
        let names = ["h", "c", "f", "a", "e", "b", "g", "d"];
        let mut dii = vec!["field count 8".to_string()];
        let mut dim = Vec::new();
        for (i, name) in names.iter().enumerate() {
            dii.push(format!("  field fp name {name}"));
            dii.push(format!("  field fp {}", 16 * i));
            dim.push(format!("num data dims {name}"));
        }
        let dii: Vec<&str> = dii.iter().map(String::as_str).collect();
        let dim: Vec<&str> = dim.iter().map(String::as_str).collect();
        let error = open(&dii, &dim);
        match &error.kind {
            LuceneErrorKind::IllegalArgument { message, .. } => {
                assert_eq!(message, "For input string: \"a\"")
            }
            other => panic!("{other:?}"),
        }
    }

    fn header(index_count: &str) -> Vec<String> {
        [
            "num data dims 1",
            "num index dims 1",
            "bytes per dim 2",
            "max leaf points 1",
            &format!("index count {index_count}"),
            "min value [01 02]",
            "max value [01 02]",
            "point count 1",
            "doc count 1",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// Regression (rmp #526): `new long[count]` rejects a negative leaf
    /// count (`SimpleTextPointsReader.java:136`).
    #[test]
    fn negative_leaf_count_is_a_negative_array_size() {
        let dim = header("-1");
        let dim: Vec<&str> = dim.iter().map(String::as_str).collect();
        let error = open(
            &["field count 1", "  field fp name p", "  field fp 0"],
            &dim,
        );
        assert_eq!(
            unchecked(&error),
            ("java.lang.NegativeArraySizeException", "-1".to_string())
        );
    }

    /// Regression (rmp #526), release builds (no `-ea`): a split value
    /// shorter than `bytesPerDim` fails in `System.arraycopy` (the port
    /// panicked on the slice).
    #[cfg(not(debug_assertions))]
    #[test]
    fn short_split_value_fails_in_arraycopy() {
        let mut dim = header("0");
        dim.extend(["split count 1", "  split dim 0", "  split value [01]"].map(String::from));
        let dim: Vec<&str> = dim.iter().map(String::as_str).collect();
        let error = open(
            &["field count 1", "  field fp name p", "  field fp 0"],
            &dim,
        );
        assert_eq!(
            unchecked(&error),
            (
                "java.lang.ArrayIndexOutOfBoundsException",
                "arraycopy: last source index 2 out of bounds for byte[1]".to_string()
            )
        );
    }
}

#[cfg(test)]
mod short_line_tests {
    use super::*;
    use crate::simpletext::test_support::{directory, segment_info, unchecked, write_file};
    use rucene::index::FieldInfos;

    /// Regression (rmp #526): `stripPrefix(FIELD_FP_NAME)` has no prefix
    /// assertion, so a short line fails in `new String(scratch.bytes(), 16,
    /// -15)` with the scratch's capacity: `field count 1` grew it to 16; the
    /// port read an empty field name.
    #[test]
    fn short_field_name_line_is_a_string_index_out_of_bounds() {
        let dir = directory();
        write_file(
            dir.as_ref(),
            "_0.dii",
            &["field count 1", "x", "  field fp 0"],
            true,
        );
        write_file(dir.as_ref(), "_0.dim", &[], false);
        let si = segment_info(&dir, "_0", 1);
        let infos = FieldInfos::new(Vec::new()).unwrap();
        let state = SegmentReadState::new(dir.as_ref(), &si, &infos, &*DEFAULT_IO_CONTEXT);
        let error = SimpleTextPointsReader::new(&state).expect_err("an error");
        assert_eq!(
            unchecked(&error),
            (
                "java.lang.StringIndexOutOfBoundsException",
                "Range [16, 16 + -15) out of bounds for length 16".to_string()
            )
        );
    }
}
