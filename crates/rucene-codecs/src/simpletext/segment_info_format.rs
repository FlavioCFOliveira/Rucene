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

use super::util::{
    self, hash_map_capacity, java_message, parse_boolean, parse_int, read_line, read_string,
};

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
fn read_str(offset: usize, scratch: &[u8]) -> String {
    read_string(scratch, offset)
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
        let mut scratch = Vec::new();
        let seg_file_name = segment_file_name(segment_name, "", SI_EXTENSION);
        let mut input = directory.open_checksum_input(&seg_file_name)?;
        // `try (ChecksumIndexInput input = ...)` (`SimpleTextSegmentInfoFormat.java:82-226`).
        let result = (|| -> Result<_> {
            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(SI_VERSION));
            let version = Version::parse(&read_str(SI_VERSION.len(), &scratch)).map_err(|pe| {
                LuceneError::corrupt_index(
                    format!("unable to parse version string: {}", java_message(&pe)),
                    input.resource_description(),
                )
            })?;

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(SI_MIN_VERSION));
            let version_string = read_str(SI_MIN_VERSION.len(), &scratch);
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

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(SI_DOCCOUNT));
            let doc_count = parse_int(&read_str(SI_DOCCOUNT.len(), &scratch))?;

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(SI_USECOMPOUND));
            let is_compound_file = parse_boolean(&read_str(SI_USECOMPOUND.len(), &scratch));

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(SI_HAS_BLOCKS));
            let has_blocks = parse_boolean(&read_str(SI_HAS_BLOCKS.len(), &scratch));

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(SI_NUM_DIAG));
            let num_diag = parse_int(&read_str(SI_NUM_DIAG.len(), &scratch))?;
            let mut diagnostics: HashMap<String, String> = HashMap::new();
            for _ in 0..num_diag {
                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(SI_DIAG_KEY));
                let key = read_str(SI_DIAG_KEY.len(), &scratch);

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(SI_DIAG_VALUE));
                let value = read_str(SI_DIAG_VALUE.len(), &scratch);
                diagnostics.insert(key, value);
            }

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(SI_NUM_ATT));
            let num_att = parse_int(&read_str(SI_NUM_ATT.len(), &scratch))?;
            let mut attributes: HashMap<String, String> = HashMap::new();
            for _ in 0..num_att {
                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(SI_ATT_KEY));
                let key = read_str(SI_ATT_KEY.len(), &scratch);

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(SI_ATT_VALUE));
                let value = read_str(SI_ATT_VALUE.len(), &scratch);
                attributes.insert(key, value);
            }

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(SI_NUM_FILES));
            let num_files = parse_int(&read_str(SI_NUM_FILES.len(), &scratch))?;
            // `HashSet.newHashSet(numFiles)`, filled in file order; `setFiles`
            // then walks it in its own iteration order.
            let mut files = JavaHashSet::with_initial_capacity(hash_map_capacity(num_files));
            for _ in 0..num_files {
                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(SI_FILE));
                let file_name = read_str(SI_FILE.len(), &scratch);
                files.insert(file_name);
            }

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(SI_ID));
            let id = util::from_bytes_ref_string(&read_str(SI_ID.len(), &scratch))?
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

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(SI_SORT));
            let num_sort_fields = parse_int(&read_str(SI_SORT.len(), &scratch))?;
            let mut sort_fields = Vec::with_capacity(num_sort_fields.max(0) as usize);
            for _ in 0..num_sort_fields {
                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(SI_SORT_NAME));
                let provider = read_str(SI_SORT_NAME.len(), &scratch);

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(SI_SORT_TYPE));

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(SI_SORT_BYTES));
                let serialized_sort =
                    util::from_bytes_ref_string(&read_str(SI_SORT_BYTES.len(), &scratch))?;
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
                return Err(LuceneError::illegal_argument(format!(
                    "invalid id: {}",
                    StringHelper::id_to_string(Some(&id))
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
                Some(index_sort) if *index_sort != Sort::new() => index_sort.fields(),
                _ => &[],
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
