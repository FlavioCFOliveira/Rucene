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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextSegmentInfoFormat`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::codecs::SegmentInfoFormat;
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::index_sorter::{sort_field_provider_for_name, write_sort_field};
use rucene::index::SegmentInfo;
use rucene::search::Sort;
use rucene::store::{ByteArrayDataInput, DataOutput, Directory, IOContext, IndexInput};
use rucene::util::java_collections::JavaHashSet;
use rucene::util::string_helper::StringHelper;
use rucene::util::{BytesRef, Version};

use super::util::{self, hash_map_capacity, java_message, parse_boolean, parse_int, JavaScratch};

pub(crate) const SI_VERSION: &[u8] = b"    version ";
pub(crate) const SI_MIN_VERSION: &[u8] = b"    min version ";
pub(crate) const SI_DOCCOUNT: &[u8] = b"    number of documents ";
pub(crate) const SI_USECOMPOUND: &[u8] = b"    uses compound file ";
pub(crate) const SI_HAS_BLOCKS: &[u8] = b"    has blocks ";
pub(crate) const SI_NUM_DIAG: &[u8] = b"    diagnostics ";
pub(crate) const SI_DIAG_KEY: &[u8] = b"      key ";
pub(crate) const SI_DIAG_VALUE: &[u8] = b"      value ";
pub(crate) const SI_NUM_ATT: &[u8] = b"    attributes ";
pub(crate) const SI_ATT_KEY: &[u8] = b"      key ";
pub(crate) const SI_ATT_VALUE: &[u8] = b"      value ";
pub(crate) const SI_NUM_FILES: &[u8] = b"    files ";
pub(crate) const SI_FILE: &[u8] = b"      file ";
pub(crate) const SI_ID: &[u8] = b"    id ";
pub(crate) const SI_SORT: &[u8] = b"    sort ";
pub(crate) const SI_SORT_TYPE: &[u8] = b"      type ";
pub(crate) const SI_SORT_NAME: &[u8] = b"      name ";
pub(crate) const SI_SORT_BYTES: &[u8] = b"      bytes ";

/// `SimpleTextSegmentInfoFormat.SI_EXTENSION`.
pub const SI_EXTENSION: &str = "si";

/// Plain text segments file format.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextSegmentInfoFormat`.
///
/// **FOR RECREATIONAL USE ONLY**
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextSegmentInfoFormat;

impl SimpleTextSegmentInfoFormat {
    /// Creates the format.
    pub fn new() -> Self {
        Self
    }
}

/// `new String(scratch.bytes(), offset, scratch.length() - offset, UTF_8)`.
/// The negative-size check of `HashMap.newHashMap(numMappings)`.
fn new_hash_map_check(num_mappings: i32) -> Result<()> {
    if num_mappings < 0 {
        return Err(LuceneError::illegal_argument(format!(
            "Negative number of mappings: {num_mappings}"
        )));
    }
    Ok(())
}

impl SegmentInfoFormat for SimpleTextSegmentInfoFormat {
    fn name(&self) -> &str {
        "SimpleTextSegmentInfoFormat"
    }

    /// Port of `SimpleTextSegmentInfoFormat.read`.
    fn read(
        &self,
        directory: Arc<dyn Directory>,
        segment_name: &str,
        segment_id: &[u8],
        _context: &dyn IOContext,
    ) -> Result<SegmentInfo> {
        let mut scratch = JavaScratch::new();
        let seg_file_name = segment_file_name(segment_name, "", SI_EXTENSION);
        let mut input = directory.open_checksum_input(&seg_file_name)?;
        // `try (ChecksumIndexInput input = ...)` (`SimpleTextSegmentInfoFormat.java:82-226`).
        let result = (|| -> Result<_> {
            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(SI_VERSION));
            let version = Version::parse(&scratch.new_string(SI_VERSION.len())?).map_err(|pe| {
                LuceneError::corrupt_index(
                    format!("unable to parse version string: {}", java_message(&pe)),
                    input.resource_description(),
                )
            })?;

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(SI_MIN_VERSION));
            let version_string = scratch.new_string(SI_MIN_VERSION.len())?;
            let min_version = if version_string == "null" {
                None
            } else {
                Some(Version::parse(&version_string).map_err(|pe| {
                    LuceneError::corrupt_index(
                        format!("unable to parse version string: {}", java_message(&pe)),
                        input.resource_description(),
                    )
                })?)
            };

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(SI_DOCCOUNT));
            let doc_count = parse_int(&scratch.new_string(SI_DOCCOUNT.len())?)?;

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(SI_USECOMPOUND));
            let is_compound_file = parse_boolean(&scratch.new_string(SI_USECOMPOUND.len())?);

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(SI_HAS_BLOCKS));
            let has_blocks = parse_boolean(&scratch.new_string(SI_HAS_BLOCKS.len())?);

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(SI_NUM_DIAG));
            let num_diag = parse_int(&scratch.new_string(SI_NUM_DIAG.len())?)?;
            // `HashMap.newHashMap(numDiag)` (`SimpleTextSegmentInfoFormat.java:118`).
            new_hash_map_check(num_diag)?;
            let mut diagnostics: HashMap<String, String> = HashMap::new();
            for _ in 0..num_diag {
                scratch.read_line(input.as_mut())?;
                debug_assert!(scratch.starts_with(SI_DIAG_KEY));
                let key = scratch.new_string(SI_DIAG_KEY.len())?;

                scratch.read_line(input.as_mut())?;
                debug_assert!(scratch.starts_with(SI_DIAG_VALUE));
                let value = scratch.new_string(SI_DIAG_VALUE.len())?;
                diagnostics.insert(key, value);
            }

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(SI_NUM_ATT));
            let num_att = parse_int(&scratch.new_string(SI_NUM_ATT.len())?)?;
            // `HashMap.newHashMap(numAtt)` (`SimpleTextSegmentInfoFormat.java:133`).
            new_hash_map_check(num_att)?;
            let mut attributes: HashMap<String, String> = HashMap::new();
            for _ in 0..num_att {
                scratch.read_line(input.as_mut())?;
                debug_assert!(scratch.starts_with(SI_ATT_KEY));
                let key = scratch.new_string(SI_ATT_KEY.len())?;

                scratch.read_line(input.as_mut())?;
                debug_assert!(scratch.starts_with(SI_ATT_VALUE));
                let value = scratch.new_string(SI_ATT_VALUE.len())?;
                attributes.insert(key, value);
            }

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(SI_NUM_FILES));
            let num_files = parse_int(&scratch.new_string(SI_NUM_FILES.len())?)?;
            // `HashSet.newHashSet(numFiles)` (`SimpleTextSegmentInfoFormat.java:148`).
            if num_files < 0 {
                return Err(LuceneError::illegal_argument(format!(
                    "Negative number of elements: {num_files}"
                )));
            }
            // `HashSet.newHashSet(numFiles)`, filled in file order; `setFiles`
            // then walks it in its own iteration order.
            let mut files = JavaHashSet::with_initial_capacity(hash_map_capacity(num_files));
            for _ in 0..num_files {
                scratch.read_line(input.as_mut())?;
                debug_assert!(scratch.starts_with(SI_FILE));
                let file_name = scratch.new_string(SI_FILE.len())?;
                files.insert(file_name);
            }

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(SI_ID));
            let id = util::from_bytes_ref_string(&scratch.new_string(SI_ID.len())?)?
                .slice()
                .to_vec();

            if segment_id != id.as_slice() {
                return Err(LuceneError::corrupt_index(
                    format!(
                        "file mismatch, expected: {}, got: {}",
                        StringHelper::id_to_string(Some(segment_id)),
                        StringHelper::id_to_string(Some(&id))
                    ),
                    input.resource_description(),
                ));
            }

            scratch.read_line(input.as_mut())?;
            debug_assert!(scratch.starts_with(SI_SORT));
            let num_sort_fields = parse_int(&scratch.new_string(SI_SORT.len())?)?;
            // `new SortField[numSortFields]` (`SimpleTextSegmentInfoFormat.java:173`).
            if num_sort_fields < 0 {
                return Err(util::negative_array_size(num_sort_fields));
            }
            let mut sort_fields = Vec::with_capacity(num_sort_fields.max(0) as usize);
            for _ in 0..num_sort_fields {
                scratch.read_line(input.as_mut())?;
                debug_assert!(scratch.starts_with(SI_SORT_NAME));
                let provider = scratch.new_string(SI_SORT_NAME.len())?;

                scratch.read_line(input.as_mut())?;
                debug_assert!(scratch.starts_with(SI_SORT_TYPE));

                scratch.read_line(input.as_mut())?;
                debug_assert!(scratch.starts_with(SI_SORT_BYTES));
                let serialized_sort =
                    util::from_bytes_ref_string(&scratch.new_string(SI_SORT_BYTES.len())?)?;
                let mut bytes = ByteArrayDataInput::new(serialized_sort.slice());
                sort_fields
                    .push(sort_field_provider_for_name(&provider)?.read_sort_field(&mut bytes)?);
                debug_assert!(bytes.eof());
            }

            // `indexSort = null` when there are no sort fields.
            let index_sort = if sort_fields.is_empty() {
                None
            } else {
                Some(Sort::new_fields(sort_fields)?)
            };

            util::check_footer(input.as_mut())?;

            let mut id_array = [0u8; rucene::util::string_helper::ID_LENGTH];
            if id.len() != id_array.len() {
                // `"invalid id: " + Arrays.toString(id)` (`SegmentInfo.java:139-141`).
                return Err(LuceneError::illegal_argument(format!(
                    "invalid id: [{}]",
                    id.iter()
                        .map(|b| (*b as i8).to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            id_array.copy_from_slice(&id);
            let info = SegmentInfo::new_without_codec(
                directory,
                version,
                min_version,
                segment_name.to_string(),
                doc_count,
                is_compound_file,
                has_blocks,
                diagnostics,
                id_array,
                attributes,
                index_sort,
            )?;
            info.set_files(files.iter().cloned().collect::<Vec<_>>())?;
            Ok(info)
        })();
        rucene::store::close_after(input.as_mut(), result)
    }

    /// Port of `SimpleTextSegmentInfoFormat.write`.
    fn write(
        &self,
        dir: &dyn Directory,
        si: &SegmentInfo,
        io_context: &dyn IOContext,
    ) -> Result<()> {
        let seg_file_name = segment_file_name(&si.name, "", SI_EXTENSION);

        let mut output = dir.create_output(&seg_file_name, io_context)?;
        // Only add the file once we've successfully created it, else IFD
        // assert can trip:
        si.add_file(seg_file_name)?;
        // try-with-resources (`SimpleTextSegmentInfoFormat.java:239`): the output is closed
        // whether or not writing it failed, a failure to close it suppressed
        // by the write's own.
        let result = (|| -> Result<()> {
            let out = output.as_mut();

            util::write_bytes(out, SI_VERSION)?;
            util::write_str(out, &si.version().to_string())?;
            util::write_newline(out)?;

            util::write_bytes(out, SI_MIN_VERSION)?;
            match si.min_version() {
                None => util::write_str(out, "null")?,
                Some(min_version) => util::write_str(out, &min_version.to_string())?,
            }
            util::write_newline(out)?;

            util::write_bytes(out, SI_DOCCOUNT)?;
            util::write_str(out, &si.max_doc()?.to_string())?;
            util::write_newline(out)?;

            util::write_bytes(out, SI_USECOMPOUND)?;
            util::write_str(out, util::bool_str(si.get_use_compound_file()))?;
            util::write_newline(out)?;

            util::write_bytes(out, SI_HAS_BLOCKS)?;
            util::write_str(out, util::bool_str(si.get_has_blocks()))?;
            util::write_newline(out)?;

            let diagnostics = si.get_diagnostics();
            let num_diagnostics = diagnostics.len();
            util::write_bytes(out, SI_NUM_DIAG)?;
            util::write_str(out, &num_diagnostics.to_string())?;
            util::write_newline(out)?;

            for (key, value) in diagnostics.iter() {
                util::write_bytes(out, SI_DIAG_KEY)?;
                util::write_str(out, key)?;
                util::write_newline(out)?;

                util::write_bytes(out, SI_DIAG_VALUE)?;
                util::write_str(out, value)?;
                util::write_newline(out)?;
            }

            let attributes = si.get_attributes_ordered();
            util::write_bytes(out, SI_NUM_ATT)?;
            util::write_str(out, &attributes.len().to_string())?;
            util::write_newline(out)?;

            for (key, value) in attributes.iter() {
                util::write_bytes(out, SI_ATT_KEY)?;
                util::write_str(out, key)?;
                util::write_newline(out)?;

                util::write_bytes(out, SI_ATT_VALUE)?;
                util::write_str(out, value)?;
                util::write_newline(out)?;
            }

            let files = si.files_ordered()?;
            util::write_bytes(out, SI_NUM_FILES)?;
            util::write_str(out, &files.len().to_string())?;
            util::write_newline(out)?;

            for file_name in files.iter() {
                util::write_bytes(out, SI_FILE)?;
                util::write_str(out, file_name)?;
                util::write_newline(out)?;
            }

            util::write_bytes(out, SI_ID)?;
            util::write_str(out, &BytesRef::new(si.id().to_vec()).to_string())?;
            util::write_newline(out)?;

            // Java's `getIndexSort()` is `null` for an unsorted segment.
            let sort_fields: &[rucene::search::SortField] = match si.index_sort() {
                Some(index_sort) => index_sort.fields(),
                None => &[],
            };
            util::write_bytes(out, SI_SORT)?;
            util::write_str(out, &sort_fields.len().to_string())?;
            util::write_newline(out)?;
            for sort_field in sort_fields {
                let Some(sorter) = sort_field.get_index_sorter() else {
                    return Err(LuceneError::illegal_state(format!(
                        "Cannot serialize sort {sort_field}"
                    )));
                };

                util::write_bytes(out, SI_SORT_NAME)?;
                util::write_str(out, sorter.get_provider_name())?;
                util::write_newline(out)?;

                util::write_bytes(out, SI_SORT_TYPE)?;
                util::write_str(out, &sort_field.to_string())?;
                util::write_newline(out)?;

                util::write_bytes(out, SI_SORT_BYTES)?;
                let mut b = BytesRefOutput::default();
                write_sort_field(sort_field, &mut b)?;
                util::write_str(out, &BytesRef::new(b.bytes).to_string())?;
                util::write_newline(out)?;
            }
            util::write_checksum(out)?;
            Ok(())
        })();
        match result {
            Ok(()) => output.close(),
            Err(error) => {
                let _ = output.close();
                Err(error)
            }
        }
    }
}

/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextSegmentInfoFormat.BytesRefOutput`: a `DataOutput` that
/// appends to a byte buffer.
#[derive(Debug, Default)]
pub(crate) struct BytesRefOutput {
    pub(crate) bytes: Vec<u8>,
}

impl DataOutput for BytesRefOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.bytes.push(b);
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.bytes.extend_from_slice(&b[offset..offset + len]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simpletext::test_support::{directory, segment_info, unchecked, write_file};
    use rucene::error::LuceneErrorKind;
    use rucene::store::DEFAULT_IO_CONTEXT;

    /// The `.si` lines up to and including `    diagnostics <diag>`, then
    /// `rest`.
    fn si_file(dir: &dyn Directory, diag: &str, rest: &[&str]) {
        let diag = format!("    diagnostics {diag}");
        let mut lines = vec![
            "    version 10.5.0",
            "    min version null",
            "    number of documents 1",
            "    uses compound file false",
            "    has blocks false",
            diag.as_str(),
        ];
        lines.extend_from_slice(rest);
        write_file(dir, "_0.si", &lines, true);
    }

    fn read_error(dir: &Arc<dyn Directory>, id: &[u8]) -> LuceneError {
        SimpleTextSegmentInfoFormat::new()
            .read(Arc::clone(dir), "_0", id, &*DEFAULT_IO_CONTEXT)
            .unwrap_err()
    }

    fn illegal_argument(error: &LuceneError) -> String {
        match &error.kind {
            LuceneErrorKind::IllegalArgument { message, .. } => message.clone(),
            other => panic!("{other:?}"),
        }
    }

    /// Regression (rmp #526): `HashMap.newHashMap` and `HashSet.newHashSet`
    /// reject negative sizes, and `new SortField[n]` a negative sort count
    /// (`SimpleTextSegmentInfoFormat.java:118-173`).
    #[test]
    fn negative_counts_fail_as_in_java() {
        let dir = directory();
        si_file(dir.as_ref(), "-1", &[]);
        let error = read_error(&dir, &[0; 16]);
        assert_eq!(illegal_argument(&error), "Negative number of mappings: -1");

        // `createOutput` refuses an existing file: one directory per case.
        let dir = directory();
        si_file(dir.as_ref(), "0", &["    attributes -4"]);
        let error = read_error(&dir, &[0; 16]);
        assert_eq!(illegal_argument(&error), "Negative number of mappings: -4");

        let dir = directory();
        si_file(dir.as_ref(), "0", &["    attributes 0", "    files -3"]);
        let error = read_error(&dir, &[0; 16]);
        assert_eq!(illegal_argument(&error), "Negative number of elements: -3");

        let dir = directory();
        let id = "    id [00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00]";
        si_file(
            dir.as_ref(),
            "0",
            &["    attributes 0", "    files 0", id, "    sort -1"],
        );
        let error = read_error(&dir, &[0; 16]);
        assert_eq!(
            unchecked(&error),
            ("java.lang.NegativeArraySizeException", "-1".to_string())
        );
    }

    /// Regression (rmp #526): an id of the wrong length that matches the
    /// caller's reaches `new SegmentInfo`, whose message is
    /// `Arrays.toString(id)` (`SegmentInfo.java:139-141`).
    #[test]
    fn invalid_id_message_is_arrays_to_string() {
        let dir = directory();
        si_file(
            dir.as_ref(),
            "0",
            &[
                "    attributes 0",
                "    files 0",
                "    id [01 fe 03]",
                "    sort 0",
            ],
        );
        let error = read_error(&dir, &[1, 0xfe, 3]);
        assert_eq!(illegal_argument(&error), "invalid id: [1, -2, 3]");
    }

    /// Regression (rmp #526): a relevance index sort is written like any
    /// other, and its `SortField` has no `IndexSorter`
    /// (`SimpleTextSegmentInfoFormat.java:290-296`); the port skipped it.
    #[test]
    fn relevance_index_sort_cannot_be_serialized() {
        let dir = directory();
        let mut si = segment_info(&dir, "_0", 1);
        si.set_index_sort(Some(Sort::new()));
        let error = SimpleTextSegmentInfoFormat::new()
            .write(dir.as_ref(), &si, &*DEFAULT_IO_CONTEXT)
            .unwrap_err();
        match &error.kind {
            LuceneErrorKind::IllegalState { message, .. } => {
                assert_eq!(message, "Cannot serialize sort <score>")
            }
            other => panic!("{other:?}"),
        }
    }
}
