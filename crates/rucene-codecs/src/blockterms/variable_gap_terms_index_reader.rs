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

//! Port of `org.apache.lucene.codecs.blockterms.VariableGapTermsIndexReader`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use rucene::codecs::codec_util::{check_footer, check_footer_with_prior, check_index_header};
use rucene::codecs::SegmentReadState;
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::FieldInfo;
use rucene::store::{ChecksumIndexInput, IndexInput};
use rucene::util::fst::{BytesRefFSTEnum, PositiveIntOutputs, FST};
use rucene::util::BytesRef;

use super::fixed_gap_terms_index_reader::field_info_by_number;
use super::terms_index_reader_base::{FieldIndexEnum, TermsIndexReaderBase};
use super::variable_gap_terms_index_writer::{
    CODEC_NAME, META_CODEC_NAME, TERMS_INDEX_EXTENSION, TERMS_META_EXTENSION, VERSION_CURRENT,
    VERSION_START,
};

/// See [`VariableGapTermsIndexWriter`](super::VariableGapTermsIndexWriter).
///
/// Port of `org.apache.lucene.codecs.blockterms.VariableGapTermsIndexReader`.
pub struct VariableGapTermsIndexReader {
    fields: HashMap<String, Arc<FST<PositiveIntOutputs>>>,
}

impl VariableGapTermsIndexReader {
    /// Port of `VariableGapTermsIndexReader(SegmentReadState)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a bad header or footer, a gap between
    /// two fields' FSTs or a duplicate field; I/O errors.
    pub fn new(state: &SegmentReadState<'_>) -> Result<Self> {
        let meta_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_META_EXTENSION,
        );
        let index_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_INDEX_EXTENSION,
        );
        let mut meta_in = state.directory.open_checksum_input(&meta_file_name)?;
        // The second resource of the try-with-resources: when it cannot be
        // opened, the first one is closed (`VariableGapTermsIndexReader.java:57-58`).
        let mut index_in = match state.directory.open_checksum_input(&index_file_name) {
            Ok(index_in) => index_in,
            Err(error) => {
                // A failure closing `metaIn` is suppressed into `error` in
                // Java; dropped here, as `rucene::store::close_after` does.
                let _ = meta_in.close();
                return Err(error);
            }
        };
        let read = Self::read(state, meta_in.as_mut(), index_in.as_mut());
        let result = match read {
            Ok(fields) => check_footer(meta_in.as_mut())
                .and_then(|_| check_footer(index_in.as_mut()))
                .map(|_| Self { fields }),
            // `checkFooter(metaIn, priorE)` rethrows `priorE`, so the index
            // input's footer is never checked.
            Err(prior) => Err(check_footer_with_prior(meta_in.as_mut(), prior)),
        };
        // try-with-resources closes `indexIn`, then `metaIn`.
        let closed = index_in.close().and(meta_in.close());
        match result {
            Ok(reader) => closed.map(|_| reader),
            Err(e) => Err(e),
        }
    }

    fn read(
        state: &SegmentReadState<'_>,
        meta_in: &mut dyn ChecksumIndexInput,
        index_in: &mut dyn ChecksumIndexInput,
    ) -> Result<HashMap<String, Arc<FST<PositiveIntOutputs>>>> {
        let segment_id = state.segment_info.id();
        check_index_header(
            meta_in,
            META_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &segment_id,
            &state.segment_suffix,
        )?;
        check_index_header(
            index_in,
            CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &segment_id,
            &state.segment_suffix,
        )?;
        let mut fields = HashMap::new();
        let mut field = meta_in.read_int()?;
        while field != -1 {
            let index_start = meta_in.read_v_long()?;
            let field_info = field_info_by_number(state, field)?;
            if index_in.file_pointer() != index_start {
                return Err(LuceneError::corrupt_index(
                    format!(
                        "Gap in FST, expected position {}, got {index_start}",
                        index_in.file_pointer()
                    ),
                    meta_in.resource_description(),
                ));
            }
            let metadata = FST::read_metadata(meta_in, PositiveIntOutputs::get_singleton())?;
            let fst = FST::new(metadata, index_in)?;
            if fields
                .insert(field_info.name.clone(), Arc::new(fst))
                .is_some()
            {
                return Err(LuceneError::corrupt_index(
                    format!("duplicate field: {}", field_info.name),
                    meta_in.resource_description(),
                ));
            }
            field = meta_in.read_int()?;
        }
        Ok(fields)
    }
}

/// Port of the private static inner class `VariableGapTermsIndexReader.IndexEnum`.
struct IndexEnum {
    fst_enum: BytesRefFSTEnum<'static, PositiveIntOutputs>,
    /// `current.input` of the `InputOutput` the enum is positioned on;
    /// `None` is Java's `null` `current`.
    current: Option<BytesRef>,
}

impl FieldIndexEnum for IndexEnum {
    fn term(&self) -> Option<BytesRef> {
        self.current.clone()
    }

    fn seek(&mut self, target: &BytesRef) -> Result<i64> {
        match self.fst_enum.seek_floor(target)? {
            Some(io) => {
                self.current = Some(io.input.clone());
                Ok(*io.output)
            }
            None => {
                self.current = None;
                // `return current.output` on a `null` `current`.
                Err(LuceneError::unchecked(
                    "java.lang.NullPointerException",
                    "Cannot read field \"output\" because \"this.current\" is null",
                ))
            }
        }
    }

    fn next(&mut self) -> Result<i64> {
        match self.fst_enum.next()? {
            Some(io) => {
                self.current = Some(io.input.clone());
                Ok(*io.output)
            }
            None => {
                self.current = None;
                Ok(-1)
            }
        }
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<i64> {
        Err(LuceneError::unsupported_operation(String::new()))
    }
}

impl TermsIndexReaderBase for VariableGapTermsIndexReader {
    fn supports_ord(&self) -> bool {
        false
    }

    fn get_field_enum(&self, field_info: &FieldInfo) -> Option<Box<dyn FieldIndexEnum>> {
        let fst = self.fields.get(&field_info.name)?;
        let fst_enum = BytesRefFSTEnum::new_shared(Arc::clone(fst))
            .expect("INVARIANT: BytesRefFSTEnum::new_shared never fails");
        Some(Box::new(IndexEnum {
            fst_enum,
            current: None,
        }))
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

impl fmt::Display for VariableGapTermsIndexReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "VariableGapTermsIndexReader(fields={})",
            self.fields.len()
        )
    }
}
