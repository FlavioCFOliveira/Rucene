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

//! Port of `org.apache.lucene.codecs.blocktreeords.OrdsBlockTreeTermsReader`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use rucene::codecs::codec_util::{check_index_header, checksum_entire_file, retrieve_checksum};
use rucene::codecs::{FieldsProducer, PostingsReaderBase, SegmentReadState};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{Fields, IndexOptions, Terms};
use rucene::store::IndexInput;
use rucene::util::java_collections::java_compare_to;

use super::java_assert;
use super::ords_block_tree_terms_writer::{
    FST_OUTPUTS, TERMS_CODEC_NAME, TERMS_EXTENSION, TERMS_INDEX_CODEC_NAME, TERMS_INDEX_EXTENSION,
    VERSION_CURRENT, VERSION_START,
};
use super::ords_field_reader::{FieldData, OrdsFieldReader};
use crate::blockterms::fixed_gap_terms_index_reader::seek_dir;

/// What the reader shares with its field readers and enums: Java's `in` and
/// `postingsReader`, which they reach through `fr.parent`.
pub(crate) struct Shared {
    /// Open input to the main terms dict file (`_X.tio`).
    pub(crate) input: Arc<Box<dyn IndexInput>>,
    /// Reads the terms dict entries, to gather state to produce `DocsEnum`
    /// on demand.
    pub(crate) postings_reader: Arc<dyn PostingsReaderBase>,
    /// The number of fields, for `OrdsSegmentTermsEnum.toString()`, which
    /// prints the reader (`seg=` + `fr.parent`).
    pub(crate) description: String,
    /// Whether the reader's `close()` closed `input` and the postings reader
    /// already, so that `Drop` does not close them twice.
    pub(crate) closed: bool,
}

impl Drop for Shared {
    /// `IOUtils.close(in, postingsReader)` (`OrdsBlockTreeTermsReader.java:207-215`),
    /// run by the last holder when the reader's `close()` found
    /// them still shared by a merge instance or an enum (rmp #351). A failure
    /// is dropped: no caller is left to receive it.
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        if let Some(input) = Arc::get_mut(&mut self.input) {
            let _ = input.close();
        }
        if let Some(reader) = Arc::get_mut(&mut self.postings_reader) {
            let _ = reader.close();
        }
    }
}

/// See [`OrdsBlockTreeTermsWriter`](super::OrdsBlockTreeTermsWriter).
///
/// Port of `org.apache.lucene.codecs.blocktreeords.OrdsBlockTreeTermsReader`.
/// Java's `TreeMap<String, OrdsFieldReader>` is a map plus the names in
/// `String.compareTo` order.
///
/// `close()` closes `in` and the postings reader when this reader is their
/// last holder; ones a live enum or merge instance still shares are closed
/// by `Shared`'s `Drop` when the last of them is dropped (see
/// `Lucene103BlockTreeTermsReader`).
pub struct OrdsBlockTreeTermsReader {
    shared: Option<Arc<Shared>>,
    fields: HashMap<String, Arc<FieldData>>,
    field_names: Vec<String>,
}

impl OrdsBlockTreeTermsReader {
    /// Sole constructor.
    ///
    /// Port of `OrdsBlockTreeTermsReader(PostingsReaderBase,
    /// SegmentReadState)`. The postings reader moves into the constructor,
    /// so when it fails it also closes it, which Java does twice: `this.close()`
    /// in the constructor's `finally` and
    /// `IOUtils.closeWhileHandlingException(postingsReader)` in
    /// `BlockTreeOrdsPostingsFormat.fieldsProducer`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a bad header, footer, checksum, field
    /// count or field statistics, or a duplicate field; I/O errors.
    pub fn new(
        mut postings_reader: Box<dyn PostingsReaderBase>,
        state: &SegmentReadState<'_>,
    ) -> Result<Self> {
        let terms_file = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_EXTENSION,
        );
        let mut input = match state.directory.open_input(&terms_file, state.context) {
            Ok(input) => input,
            Err(e) => {
                // The format's `finally`: closeWhileHandlingException(postingsReader).
                let _ = postings_reader.close();
                return Err(e);
            }
        };

        let mut index_in: Option<Box<dyn IndexInput>> = None;
        match Self::read(
            postings_reader.as_mut(),
            state,
            input.as_mut(),
            &mut index_in,
        ) {
            Ok(fields) => {
                let mut field_names: Vec<String> = fields.keys().cloned().collect();
                field_names.sort_by(|a, b| java_compare_to(a, b));
                let description = format!("fields={}", fields.len());
                Ok(Self {
                    shared: Some(Arc::new(Shared {
                        input: Arc::new(input),
                        postings_reader: Arc::from(postings_reader),
                        description,
                        closed: false,
                    })),
                    fields,
                    field_names,
                })
            }
            Err(e) => {
                // IOUtils.closeWhileHandlingException(indexIn, this): this.close()
                // closes in and postingsReader.
                if let Some(mut index_in) = index_in {
                    let _ = index_in.close();
                }
                let _ = input.close();
                let _ = postings_reader.close();
                Err(e)
            }
        }
    }

    fn read(
        postings_reader: &mut dyn PostingsReaderBase,
        state: &SegmentReadState<'_>,
        input: &mut dyn IndexInput,
        index_in_slot: &mut Option<Box<dyn IndexInput>>,
    ) -> Result<HashMap<String, Arc<FieldData>>> {
        let version = check_index_header(
            input,
            TERMS_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let index_file = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_INDEX_EXTENSION,
        );
        let index_in =
            index_in_slot.insert(state.directory.open_input(&index_file, state.context)?);
        let index_version = check_index_header(
            index_in.as_mut(),
            TERMS_INDEX_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        if index_version != version {
            return Err(LuceneError::corrupt_index(
                format!(
                    "mixmatched version files: {}={version},{}={index_version}",
                    input.resource_description(),
                    index_in.resource_description()
                ),
                index_in.resource_description(),
            ));
        }

        // verify
        checksum_entire_file(index_in.as_mut())?;

        // Have PostingsReader init itself
        postings_reader.init(input, state)?;

        // NOTE: data file is too costly to verify checksum against all the bytes on open,
        // but for now we at least verify proper structure of the checksum footer: which looks
        // for FOOTER_MAGIC + algorithmID. This is cheap and can detect some forms of corruption
        // such as file truncation.
        retrieve_checksum(input)?;

        // Read per-field details
        seek_dir(input)?;
        seek_dir(index_in.as_mut())?;

        let num_fields = input.read_v_int()?;
        if num_fields < 0 {
            return Err(LuceneError::corrupt_index(
                format!("invalid numFields: {num_fields}"),
                input.resource_description(),
            ));
        }

        let max_doc = state.segment_info.max_doc()?;
        let mut fields: HashMap<String, Arc<FieldData>> = HashMap::new();
        for _ in 0..num_fields {
            let field = input.read_v_int()?;
            let num_terms = input.read_v_long()?;
            java_assert!(num_terms >= 0);
            let num_bytes = super::array_len(input.read_v_int()?)?;
            let mut code = vec![0u8; num_bytes];
            input.read_bytes(Some(&mut code), 0, num_bytes)?;
            let root_code = FST_OUTPUTS.new_output(code, 0, num_terms);
            let field_info = state.field_infos.field_info_by_number(field);
            java_assert!(field_info.is_some(), "field={field}");
            let field_info = field_info.ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.NullPointerException",
                    "Cannot invoke \"org.apache.lucene.index.FieldInfo.getIndexOptions()\" \
                     because \"fieldInfo\" is null",
                )
            })?;
            java_assert!(num_terms <= i64::from(i32::MAX));
            let sum_total_term_freq = input.read_v_long()?;
            // when frequencies are omitted, sumDocFreq=totalTermFreq and we only write one value
            let sum_doc_freq = if field_info.get_index_options() == IndexOptions::DOCS {
                sum_total_term_freq
            } else {
                input.read_v_long()?
            };
            let doc_count = input.read_v_int()?;

            let min_term = read_bytes_ref(input)?;
            let max_term = read_bytes_ref(input)?;
            if doc_count < 0 || doc_count > max_doc {
                // #docs with field must be <= #docs
                return Err(LuceneError::corrupt_index(
                    format!("invalid docCount: {doc_count} maxDoc: {max_doc}"),
                    input.resource_description(),
                ));
            }
            if sum_doc_freq < i64::from(doc_count) {
                // #postings must be >= #docs with field
                return Err(LuceneError::corrupt_index(
                    format!("invalid sumDocFreq: {sum_doc_freq} docCount: {doc_count}"),
                    input.resource_description(),
                ));
            }
            if sum_total_term_freq < sum_doc_freq {
                // #positions must be >= #postings
                return Err(LuceneError::corrupt_index(
                    format!(
                        "invalid sumTotalTermFreq: {sum_total_term_freq} sumDocFreq: {sum_doc_freq}"
                    ),
                    input.resource_description(),
                ));
            }
            let index_start_fp = index_in.read_v_long()?;
            let data = FieldData::new(
                field_info.clone(),
                num_terms,
                root_code,
                sum_total_term_freq,
                sum_doc_freq,
                doc_count,
                index_start_fp,
                Some(index_in.as_ref()),
                min_term,
                max_term,
            )?;
            if fields
                .insert(field_info.name.clone(), Arc::new(data))
                .is_some()
            {
                return Err(LuceneError::corrupt_index(
                    format!("duplicate field: {}", field_info.name),
                    input.resource_description(),
                ));
            }
        }
        if let Some(mut index_in) = index_in_slot.take() {
            index_in.close()?;
        }
        Ok(fields)
    }

    fn shared(&self) -> Result<&Arc<Shared>> {
        self.shared
            .as_ref()
            .ok_or_else(|| LuceneError::already_closed("this OrdsBlockTreeTermsReader is closed"))
    }
}

/// Port of `OrdsBlockTreeTermsReader.readBytesRef(IndexInput)`.
fn read_bytes_ref(input: &mut dyn IndexInput) -> Result<Vec<u8>> {
    let length = super::array_len(input.read_v_int()?)?;
    let mut bytes = vec![0u8; length];
    input.read_bytes(Some(&mut bytes), 0, length)?;
    Ok(bytes)
}

impl fmt::Display for OrdsBlockTreeTermsReader {
    /// Port of `OrdsBlockTreeTermsReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.shared {
            Some(shared) => write!(
                f,
                "OrdsBlockTreeTermsReader(fields={},delegate={})",
                self.fields.len(),
                shared.postings_reader
            ),
            None => write!(
                f,
                "OrdsBlockTreeTermsReader(fields={},delegate=null)",
                self.fields.len()
            ),
        }
    }
}

impl Fields for OrdsBlockTreeTermsReader {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.field_names.iter().cloned())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let Some(data) = self.fields.get(field) else {
            return Ok(None);
        };
        Ok(Some(Box::new(OrdsFieldReader::new(
            Arc::clone(self.shared()?),
            Arc::clone(data),
        ))))
    }

    fn size(&self) -> i32 {
        self.fields.len() as i32
    }
}

impl FieldsProducer for OrdsBlockTreeTermsReader {
    fn check_integrity(&self) -> Result<()> {
        let shared = self.shared()?;
        // term dictionary
        let mut input = shared.input.clone_input()?;
        checksum_entire_file(input.as_mut())?;

        // postings
        shared.postings_reader.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(Self {
            shared: self.shared.clone(),
            fields: self.fields.clone(),
            field_names: self.field_names.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        // finally: Clear so refs to terms index is GCable even if
        // app hangs onto us:
        self.fields.clear();
        self.field_names.clear();
        let Some(mut shared) = self.shared.take() else {
            return Ok(());
        };
        let Some(shared) = Arc::get_mut(&mut shared) else {
            return Ok(());
        };
        shared.closed = true;
        // IOUtils.close(in, postingsReader)
        let input = Arc::get_mut(&mut shared.input).map_or(Ok(()), |i| i.close());
        let postings = Arc::get_mut(&mut shared.postings_reader).map_or(Ok(()), |r| r.close());
        input.and(postings)
    }
}
