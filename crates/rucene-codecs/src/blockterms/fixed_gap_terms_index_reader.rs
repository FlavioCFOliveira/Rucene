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

//! Port of `org.apache.lucene.codecs.blockterms.FixedGapTermsIndexReader`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use rucene::codecs::codec_util::{check_index_header, checksum_entire_file, footer_length};
use rucene::codecs::SegmentReadState;
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::FieldInfo;
use rucene::store::IndexInput;
use rucene::util::packed::MonotonicBlockPackedReader;
use rucene::util::paged_bytes::{PagedBytes, PagedBytesReader};
use rucene::util::{BytesRef, IOUtils, LongValues};

use super::fixed_gap_terms_index_writer::{CODEC_NAME, TERMS_INDEX_EXTENSION, VERSION_CURRENT};
use super::terms_index_reader_base::{FieldIndexEnum, TermsIndexReaderBase};

const PAGED_BYTES_BITS: u32 = 15;

/// Port of the private inner class `FixedGapTermsIndexReader.FieldIndexData`.
struct FieldIndexData {
    term_bytes_start: i64,
    term_offsets: MonotonicBlockPackedReader,
    terms_dict_offsets: MonotonicBlockPackedReader,
    num_index_terms: i64,
    terms_start: i64,
}

impl fmt::Display for FieldIndexData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FixedGapTermIndex(indexterms={})", self.num_index_terms)
    }
}

/// What every [`FieldIndexEnum`] of the reader reads: Java's
/// `termBytesReader`, `indexInterval` and `fields`.
struct Shared {
    index_interval: i64,
    term_bytes_reader: PagedBytesReader,
    fields: HashMap<String, Arc<FieldIndexData>>,
}

/// TermsIndexReader for simple every Nth terms indexes.
///
/// Port of `org.apache.lucene.codecs.blockterms.FixedGapTermsIndexReader`.
pub struct FixedGapTermsIndexReader {
    shared: Arc<Shared>,
    #[allow(dead_code)] // read by Java only through the field data's readers
    packed_ints_version: i32,
    #[allow(dead_code)]
    blocksize: i32,
}

impl FixedGapTermsIndexReader {
    /// Port of `FixedGapTermsIndexReader(SegmentReadState)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a bad header, checksum, interval, field
    /// count or index-start pointer, or a duplicate field; I/O errors.
    pub fn new(state: &SegmentReadState<'_>) -> Result<Self> {
        let mut term_bytes = PagedBytes::new(PAGED_BYTES_BITS);
        let file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_INDEX_EXTENSION,
        );
        let mut input = state.directory.open_input(&file_name, state.context)?;
        let read = Self::read(state, input.as_mut(), &mut term_bytes);
        let closed = match &read {
            Ok(_) => IOUtils::close([IOUtils::closeable(|| input.close())]),
            Err(_) => {
                IOUtils::close_while_handling_exception([IOUtils::closeable(|| input.close())])
            }
        };
        // `termBytesReader = termBytes.freeze(true)` runs in Java's `finally`.
        let frozen = term_bytes.freeze(true);
        let (index_interval, packed_ints_version, blocksize, fields) = read?;
        closed?;
        Ok(Self {
            shared: Arc::new(Shared {
                index_interval,
                term_bytes_reader: frozen?,
                fields,
            }),
            packed_ints_version,
            blocksize,
        })
    }

    #[allow(clippy::type_complexity)]
    fn read(
        state: &SegmentReadState<'_>,
        input: &mut dyn IndexInput,
        term_bytes: &mut PagedBytes,
    ) -> Result<(i64, i32, i32, HashMap<String, Arc<FieldIndexData>>)> {
        check_index_header(
            input,
            CODEC_NAME,
            VERSION_CURRENT,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        checksum_entire_file(input)?;

        let index_interval = i64::from(input.read_v_int()?);
        if index_interval < 1 {
            return Err(LuceneError::corrupt_index(
                format!("invalid indexInterval: {index_interval}"),
                input.resource_description(),
            ));
        }
        let packed_ints_version = input.read_v_int()?;
        let blocksize = input.read_v_int()?;

        seek_dir(input)?;

        // Read directory
        let num_fields = input.read_v_int()?;
        if num_fields < 0 {
            return Err(LuceneError::corrupt_index(
                format!("invalid numFields: {num_fields}"),
                input.resource_description(),
            ));
        }
        let mut fields = HashMap::new();
        for _ in 0..num_fields {
            let field = input.read_v_int()?;
            // TODO: change this to a vLong if we fix writer to support > 2B index terms
            let num_index_terms = i64::from(input.read_v_int()?);
            if num_index_terms < 0 {
                return Err(LuceneError::corrupt_index(
                    format!("invalid numIndexTerms: {num_index_terms}"),
                    input.resource_description(),
                ));
            }
            let terms_start = input.read_v_long()?;
            let index_start = input.read_v_long()?;
            let packed_index_start = input.read_v_long()?;
            let packed_offsets_start = input.read_v_long()?;
            if packed_index_start < index_start {
                return Err(LuceneError::corrupt_index(
                    format!(
                        "invalid packedIndexStart: {packed_index_start} indexStart: \
                         {index_start} numIndexTerms: {num_index_terms}"
                    ),
                    input.resource_description(),
                ));
            }
            let field_info = field_info_by_number(state, field)?;
            let data = FieldIndexData::new(
                input,
                term_bytes,
                index_start,
                terms_start,
                packed_index_start,
                packed_offsets_start,
                num_index_terms,
                packed_ints_version,
                blocksize,
            )?;
            let name = field_info.name.clone();
            if fields.insert(name.clone(), Arc::new(data)).is_some() {
                return Err(LuceneError::corrupt_index(
                    format!("duplicate field: {name}"),
                    input.resource_description(),
                ));
            }
        }
        Ok((index_interval, packed_ints_version, blocksize, fields))
    }
}

/// `state.fieldInfos.fieldInfo(field)`, whose `null` the caller dereferences
/// (`fieldInfo.name`): the `NullPointerException` Java throws.
pub(crate) fn field_info_by_number<'s>(
    state: &'s SegmentReadState<'_>,
    field: i32,
) -> Result<&'s FieldInfo> {
    state
        .field_infos
        .field_info_by_number(field)
        .ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.NullPointerException",
                "Cannot read field \"name\" because \"fieldInfo\" is null",
            )
        })
}

/// Port of `FixedGapTermsIndexReader.seekDir` (and the identical private
/// `seekDir` of the other blockterms and memory readers): the directory
/// offset is the long before the footer.
pub(crate) fn seek_dir(input: &mut dyn IndexInput) -> Result<()> {
    input.seek(input.length() - i64::from(footer_length()) - 8)?;
    let dir_offset = input.read_long()?;
    input.seek(dir_offset)
}

impl FieldIndexData {
    #[allow(clippy::too_many_arguments)]
    fn new(
        input: &dyn IndexInput,
        term_bytes: &mut PagedBytes,
        index_start: i64,
        terms_start: i64,
        packed_index_start: i64,
        _packed_offsets_start: i64,
        num_index_terms: i64,
        packed_ints_version: i32,
        blocksize: i32,
    ) -> Result<Self> {
        let term_bytes_start = term_bytes.get_pointer();
        let mut clone = input.clone_input()?;
        clone.seek(index_start)?;
        debug_assert!(num_index_terms > 0, "numIndexTerms={num_index_terms}");

        // `try (clone)`: closed whatever happens.
        let read = (|| -> Result<(MonotonicBlockPackedReader, MonotonicBlockPackedReader)> {
            // -1 is passed to mean "don't load term index", but
            // if we are then later loaded it's overwritten with
            // a real value
            let num_term_bytes = packed_index_start - index_start;
            term_bytes.copy_from_input(clone.as_mut(), num_term_bytes)?;

            // records offsets into main terms dict file
            let terms_dict_offsets = MonotonicBlockPackedReader::of(
                clone.as_mut(),
                packed_ints_version,
                blocksize as usize,
                num_index_terms,
            )?;

            // records offsets into byte[] term data
            let term_offsets = MonotonicBlockPackedReader::of(
                clone.as_mut(),
                packed_ints_version,
                blocksize as usize,
                1 + num_index_terms,
            )?;
            Ok((terms_dict_offsets, term_offsets))
        })();
        let closed = match &read {
            Ok(_) => clone.close(),
            Err(_) => {
                IOUtils::close_while_handling_exception([IOUtils::closeable(|| clone.close())])
            }
        };
        let (terms_dict_offsets, term_offsets) = read?;
        closed?;
        Ok(Self {
            term_bytes_start,
            term_offsets,
            terms_dict_offsets,
            num_index_terms,
            terms_start,
        })
    }
}

/// Port of the private inner class `FixedGapTermsIndexReader.IndexEnum`.
struct IndexEnum {
    shared: Arc<Shared>,
    field_index: Option<Arc<FieldIndexData>>,
    term: BytesRef,
    ord: i64,
}

/// The `NullPointerException` of an enum over a field without index terms
/// (`fields.get(fieldInfo.name)` returned `null`).
fn no_field_index() -> LuceneError {
    LuceneError::unchecked(
        "java.lang.NullPointerException",
        "Cannot read field \"numIndexTerms\" because \"this.fieldIndex\" is null",
    )
}

impl IndexEnum {
    fn field_index(&self) -> Result<Arc<FieldIndexData>> {
        self.field_index.clone().ok_or_else(no_field_index)
    }

    /// `termBytesReader.fillSlice(term, fieldIndex.termBytesStart + offset,
    /// length)` for index term `idx`.
    fn fill_term(&mut self, field_index: &FieldIndexData, idx: i64) {
        let offset = field_index.term_offsets.get(idx);
        let length = (field_index.term_offsets.get(1 + idx) - offset) as usize;
        let bytes = self
            .shared
            .term_bytes_reader
            .fill_slice(field_index.term_bytes_start + offset, length)
            .into_owned();
        self.term = BytesRef::new(bytes);
    }
}

impl FieldIndexEnum for IndexEnum {
    fn term(&self) -> Option<BytesRef> {
        Some(self.term.clone())
    }

    fn seek(&mut self, target: &BytesRef) -> Result<i64> {
        let field_index = self.field_index()?;
        let index_interval = self.shared.index_interval;
        let mut lo: i64 = 0; // binary search
        let mut hi: i64 = field_index.num_index_terms - 1;

        while hi >= lo {
            let mid = ((lo + hi) as u64 >> 1) as i64;

            self.fill_term(&field_index, mid);
            let delta = target.cmp(&self.term);
            match delta {
                std::cmp::Ordering::Less => hi = mid - 1,
                std::cmp::Ordering::Greater => lo = mid + 1,
                std::cmp::Ordering::Equal => {
                    debug_assert!(mid >= 0);
                    self.ord = mid * index_interval;
                    return Ok(field_index.terms_start + field_index.terms_dict_offsets.get(mid));
                }
            }
        }

        if hi < 0 {
            debug_assert_eq!(hi, -1);
            hi = 0;
        }

        self.fill_term(&field_index, hi);
        self.ord = hi * index_interval;
        Ok(field_index.terms_start + field_index.terms_dict_offsets.get(hi))
    }

    fn next(&mut self) -> Result<i64> {
        let field_index = self.field_index()?;
        let index_interval = self.shared.index_interval;
        let idx = 1 + (self.ord / index_interval);
        if idx >= field_index.num_index_terms {
            return Ok(-1);
        }
        self.ord += index_interval;
        self.fill_term(&field_index, idx);
        Ok(field_index.terms_start + field_index.terms_dict_offsets.get(idx))
    }

    fn ord(&self) -> Result<i64> {
        Ok(self.ord)
    }

    fn seek_ord(&mut self, ord: i64) -> Result<i64> {
        let field_index = self.field_index()?;
        let index_interval = self.shared.index_interval;
        let idx = ord / index_interval;
        // caller must ensure ord is in bounds
        debug_assert!(idx < field_index.num_index_terms);
        self.fill_term(&field_index, idx);
        self.ord = idx * index_interval;
        Ok(field_index.terms_start + field_index.terms_dict_offsets.get(idx))
    }
}

impl TermsIndexReaderBase for FixedGapTermsIndexReader {
    fn supports_ord(&self) -> bool {
        true
    }

    fn get_field_enum(&self, field_info: &FieldInfo) -> Option<Box<dyn FieldIndexEnum>> {
        Some(Box::new(IndexEnum {
            shared: Arc::clone(&self.shared),
            field_index: self.shared.fields.get(&field_info.name).cloned(),
            term: BytesRef::default(),
            ord: 0,
        }))
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

impl fmt::Display for FixedGapTermsIndexReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FixedGapTermsIndexReader(fields={},interval={})",
            self.shared.fields.len(),
            self.shared.index_interval
        )
    }
}
