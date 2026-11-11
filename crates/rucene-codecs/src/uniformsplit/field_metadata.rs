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

//! Port of `org.apache.lucene.codecs.uniformsplit.FieldMetadata`.

use rucene::codecs::BlockTermState;
use rucene::error::{LuceneError, Result};
use rucene::index::{FieldInfo, FieldInfos, IndexOptions};
use rucene::store::{DataInput, DataOutput};
use rucene::util::{BytesRef, FixedBitSet};

use crate::blocktreeords::{array_len, java_assert};

/// Metadata and stats for one field in the index.
///
/// There is only one instance of [`FieldMetadata`] per [`FieldInfo`].
///
/// Port of `org.apache.lucene.codecs.uniformsplit.FieldMetadata`
/// (`@lucene.experimental`). Java's `assert isMutable` in the setters are
/// `debug_assert!`s: the setters return nothing to raise an
/// `AssertionError` through, and only the writers, which build mutable
/// instances, call them.
#[derive(Debug, Clone)]
pub struct FieldMetadata {
    pub(crate) field_info: Option<FieldInfo>,
    pub(crate) is_mutable: bool,
    pub(crate) docs_seen: Option<FixedBitSet>,

    pub(crate) sum_doc_freq: i64,
    pub(crate) num_terms: i64,
    pub(crate) sum_total_term_freq: i64,
    pub(crate) doc_count: i32,

    pub(crate) dictionary_start_fp: i64,
    pub(crate) first_block_start_fp: i64,
    pub(crate) last_block_start_fp: i64,

    pub(crate) last_term: Option<BytesRef>,
}

impl FieldMetadata {
    /// Constructs field metadata for writing.
    ///
    /// `max_doc` is the total number of documents in the segment being
    /// written.
    ///
    /// Port of `FieldMetadata(FieldInfo, int)`.
    pub fn new(field_info: FieldInfo, max_doc: i32) -> Self {
        Self::with_mutability(Some(field_info), max_doc, true)
    }

    /// Constructs immutable virtual field metadata for reading.
    ///
    /// Port of `FieldMetadata(long, long, long, BytesRef)`.
    pub fn virtual_field(
        dictionary_start_fp: i64,
        first_block_start_fp: i64,
        last_block_start_fp: i64,
        last_term: BytesRef,
    ) -> Self {
        let mut m = Self::with_mutability(None, 0, false);
        m.dictionary_start_fp = dictionary_start_fp;
        m.first_block_start_fp = first_block_start_fp;
        m.last_block_start_fp = last_block_start_fp;
        m.last_term = Some(last_term);
        m
    }

    /// Constructs field metadata for reading or writing.
    ///
    /// `max_doc` is the total number of documents in the segment being
    /// written; `is_mutable` is true if this `FieldMetadata` is created for
    /// writing the index, false if it is used for reading the index.
    ///
    /// Port of `FieldMetadata(FieldInfo, int, boolean)`.
    pub fn with_mutability(field_info: Option<FieldInfo>, max_doc: i32, is_mutable: bool) -> Self {
        debug_assert!(is_mutable || max_doc == 0);
        Self {
            field_info,
            is_mutable,
            // docsSeen must not be set if this FieldMetadata is immutable, that means it is used
            // for reading the index.
            docs_seen: is_mutable.then(|| FixedBitSet::new(max_doc.max(0) as usize)),
            sum_doc_freq: 0,
            num_terms: 0,
            sum_total_term_freq: 0,
            doc_count: 0,
            dictionary_start_fp: -1,
            first_block_start_fp: -1,
            last_block_start_fp: -1,
            last_term: None,
        }
    }

    /// Updates the field stats with the given [`BlockTermState`] for the
    /// current block line (for one term).
    ///
    /// Port of `FieldMetadata.updateStats(BlockTermState)`.
    pub fn update_stats(&mut self, state: &BlockTermState) {
        debug_assert!(self.is_mutable);
        debug_assert!(state.doc_freq > 0);
        self.sum_doc_freq += i64::from(state.doc_freq);
        if state.total_term_freq > 0 {
            self.sum_total_term_freq += state.total_term_freq;
        }
        self.num_terms += 1;
    }

    /// Provides the [`FixedBitSet`] to keep track of the docs seen when
    /// calling `PostingsWriterBase.writeTerm`.
    ///
    /// The returned [`FixedBitSet`] is created once in this `FieldMetadata`
    /// constructor; `None` if this `FieldMetadata` is created immutable
    /// during segment reading.
    pub fn get_docs_seen(&mut self) -> Option<&mut FixedBitSet> {
        self.docs_seen.as_mut()
    }

    /// Port of `FieldMetadata.getFieldInfo()`.
    pub fn get_field_info(&self) -> Option<&FieldInfo> {
        self.field_info.as_ref()
    }

    /// Port of `FieldMetadata.getSumDocFreq()`.
    pub fn get_sum_doc_freq(&self) -> i64 {
        self.sum_doc_freq
    }

    /// Port of `FieldMetadata.getNumTerms()`.
    pub fn get_num_terms(&self) -> i64 {
        self.num_terms
    }

    /// Port of `FieldMetadata.getSumTotalTermFreq()`.
    pub fn get_sum_total_term_freq(&self) -> i64 {
        self.sum_total_term_freq
    }

    /// Port of `FieldMetadata.getDocCount()`.
    pub fn get_doc_count(&self) -> i32 {
        match (&self.docs_seen, self.is_mutable) {
            (Some(docs_seen), true) => docs_seen.cardinality() as i32,
            _ => self.doc_count,
        }
    }

    /// The file pointer to the start of the first block of the field.
    pub fn get_first_block_start_fp(&self) -> i64 {
        self.first_block_start_fp
    }

    /// Sets the file pointer to the start of the first block of the field.
    pub fn set_first_block_start_fp(&mut self, first_block_start_fp: i64) {
        debug_assert!(self.is_mutable);
        self.first_block_start_fp = first_block_start_fp;
    }

    /// The start file pointer for the last block of the field.
    pub fn get_last_block_start_fp(&self) -> i64 {
        self.last_block_start_fp
    }

    /// Sets the file pointer after the end of the last block of the field.
    pub fn set_last_block_start_fp(&mut self, last_block_start_fp: i64) {
        debug_assert!(self.is_mutable);
        self.last_block_start_fp = last_block_start_fp;
    }

    /// The file pointer to the start of the dictionary of the field.
    pub fn get_dictionary_start_fp(&self) -> i64 {
        self.dictionary_start_fp
    }

    /// Sets the file pointer to the start of the dictionary of the field.
    pub fn set_dictionary_start_fp(&mut self, dictionary_start_fp: i64) {
        debug_assert!(self.is_mutable);
        self.dictionary_start_fp = dictionary_start_fp;
    }

    /// Port of `FieldMetadata.setLastTerm(BytesRef)`.
    pub fn set_last_term(&mut self, last_term: BytesRef) {
        self.last_term = Some(last_term);
    }

    /// Port of `FieldMetadata.getLastTerm()`.
    pub fn get_last_term(&self) -> Option<&BytesRef> {
        self.last_term.as_ref()
    }

    /// The field's name, or Java's `NullPointerException` for the virtual
    /// field metadata (`getFieldInfo().name`).
    pub(crate) fn field_name(&self) -> Result<&str> {
        self.field_info
            .as_ref()
            .map(|fi| fi.name.as_str())
            .ok_or_else(|| {
                super::npe(
                    "Cannot read field \"name\" because the return value of \
                 \"org.apache.lucene.codecs.uniformsplit.FieldMetadata.getFieldInfo()\" is null",
                )
            })
    }

    /// The field's [`FieldInfo`], or Java's `NullPointerException` for the
    /// virtual field metadata.
    pub(crate) fn field_info_or_npe(&self) -> Result<&FieldInfo> {
        self.field_info.as_ref().ok_or_else(|| {
            super::npe(
                "Cannot invoke \"org.apache.lucene.index.FieldInfo.getIndexOptions()\" because \
                 the return value of \
                 \"org.apache.lucene.codecs.uniformsplit.FieldMetadata.getFieldInfo()\" is null",
            )
        })
    }
}

/// Reads/writes field metadata.
///
/// Port of the static nested class `FieldMetadata.Serializer`, a stateless
/// singleton (`Serializer.INSTANCE`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Serializer;

impl Serializer {
    /// Stateless singleton.
    ///
    /// Port of `FieldMetadata.Serializer.INSTANCE`.
    pub const INSTANCE: Serializer = Serializer;

    /// Port of `FieldMetadata.Serializer.write(DataOutput, FieldMetadata)`.
    ///
    /// # Errors
    ///
    /// `AssertionError` (debug builds) for unset file pointers, no term or
    /// inconsistent stats; the I/O errors of the output.
    pub fn write(&self, output: &mut dyn DataOutput, field_metadata: &FieldMetadata) -> Result<()> {
        java_assert!(field_metadata.dictionary_start_fp >= 0);
        java_assert!(field_metadata.first_block_start_fp >= 0);
        java_assert!(field_metadata.last_block_start_fp >= 0);
        java_assert!(
            field_metadata.num_terms > 0,
            "There should be at least one term for field {}: {}",
            field_metadata.field_name()?,
            field_metadata.num_terms
        );
        java_assert!(
            field_metadata.first_block_start_fp <= field_metadata.last_block_start_fp,
            "start: {} end: {}",
            field_metadata.first_block_start_fp,
            field_metadata.last_block_start_fp
        );
        java_assert!(
            field_metadata.last_term.is_some(),
            "you must set the last term"
        );
        let field_info = field_metadata.field_info_or_npe()?;

        output.write_v_int(field_info.number)?;

        output.write_v_long(field_metadata.num_terms)?;
        output.write_v_long(field_metadata.sum_doc_freq)?;

        if field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS)
        {
            java_assert!(
                field_metadata.sum_total_term_freq >= field_metadata.sum_doc_freq,
                "sumTotalFQ: {} sumDocFQ: {}",
                field_metadata.sum_total_term_freq,
                field_metadata.sum_doc_freq
            );
            output
                .write_v_long(field_metadata.sum_total_term_freq - field_metadata.sum_doc_freq)?;
        }

        output.write_v_int(field_metadata.get_doc_count())?;

        output.write_v_long(field_metadata.dictionary_start_fp)?;
        output.write_v_long(field_metadata.first_block_start_fp)?;
        output.write_v_long(field_metadata.last_block_start_fp)?;

        match field_metadata.last_term.as_ref() {
            Some(last_term) if last_term.length > 0 => {
                output.write_v_int(last_term.length as i32)?;
                output.write_bytes(&last_term.bytes, last_term.offset, last_term.length)
            }
            Some(_) => output.write_v_int(0),
            None => Err(super::npe(
                "Cannot read field \"length\" because \"fieldMetadata.lastTerm\" is null",
            )),
        }
    }

    /// Port of `FieldMetadata.Serializer.read(DataInput, FieldInfos, int)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for an unknown field or inconsistent stats;
    /// `IllegalArgumentException` for a negative field number (Java's
    /// `FieldInfos.fieldInfo(int)`); `NegativeArraySizeException` for a
    /// negative last term length (Java's `new BytesRef(int)` raises it before
    /// the length check); the I/O errors of the input.
    pub fn read(
        &self,
        input: &mut dyn DataInput,
        field_infos: &FieldInfos,
        max_num_docs: i32,
    ) -> Result<FieldMetadata> {
        let field_id = input.read_v_int()?;
        let field_info = field_info_by_number(field_infos, field_id)?;
        let Some(field_info) = field_info else {
            return Err(LuceneError::corrupt_index(
                format!("Illegal field id= {field_id}"),
                input.to_java_string(),
            ));
        };
        let mut field_metadata = FieldMetadata::with_mutability(Some(field_info.clone()), 0, false);

        field_metadata.num_terms = input.read_v_long()?;
        if field_metadata.num_terms <= 0 {
            return Err(LuceneError::corrupt_index(
                format!(
                    "Illegal number of terms= {} for field= {field_id}",
                    field_metadata.num_terms
                ),
                input.to_java_string(),
            ));
        }

        field_metadata.sum_doc_freq = input.read_v_long()?;
        field_metadata.sum_total_term_freq = field_metadata.sum_doc_freq;
        if field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS)
        {
            field_metadata.sum_total_term_freq = field_metadata
                .sum_total_term_freq
                .wrapping_add(input.read_v_long()?);
            if field_metadata.sum_total_term_freq < field_metadata.sum_doc_freq {
                // #positions must be >= #postings.
                return Err(LuceneError::corrupt_index(
                    format!(
                        "Illegal sumTotalTermFreq= {} sumDocFreq= {} for field= {field_id}",
                        field_metadata.sum_total_term_freq, field_metadata.sum_doc_freq
                    ),
                    input.to_java_string(),
                ));
            }
        }

        field_metadata.doc_count = input.read_v_int()?;
        if field_metadata.doc_count < 0 || field_metadata.doc_count > max_num_docs {
            // #docs with field must be <= #docs.
            return Err(LuceneError::corrupt_index(
                format!(
                    "Illegal number of docs= {} maxNumDocs= {max_num_docs} for field={field_id}",
                    field_metadata.doc_count
                ),
                input.to_java_string(),
            ));
        }
        if field_metadata.sum_doc_freq < i64::from(field_metadata.doc_count) {
            // #postings must be >= #docs with field.
            return Err(LuceneError::corrupt_index(
                format!(
                    "Illegal sumDocFreq= {} docCount= {} for field= {field_id}",
                    field_metadata.sum_doc_freq, field_metadata.doc_count
                ),
                input.to_java_string(),
            ));
        }

        field_metadata.dictionary_start_fp = input.read_v_long()?;
        field_metadata.first_block_start_fp = input.read_v_long()?;
        field_metadata.last_block_start_fp = input.read_v_long()?;

        let last_term_length = input.read_v_int()?;
        let mut last_term = BytesRef::new(vec![0u8; array_len(last_term_length)?]);
        last_term.length = 0;
        if last_term_length > 0 {
            input.read_bytes(Some(&mut last_term.bytes), 0, last_term_length as usize)?;
            last_term.length = last_term_length as usize;
        }
        field_metadata.set_last_term(last_term);

        Ok(field_metadata)
    }
}

/// Java's `FieldInfos.fieldInfo(int)`: `IllegalArgumentException("Illegal
/// field number: " + n)` for a negative number, `null` for an unknown one.
pub(crate) fn field_info_by_number(
    field_infos: &FieldInfos,
    number: i32,
) -> Result<Option<&FieldInfo>> {
    if number < 0 {
        return Err(LuceneError::illegal_argument(format!(
            "Illegal field number: {number}"
        )));
    }
    Ok(field_infos.field_info_by_number(number))
}

#[cfg(test)]
mod tests {
    use rucene::error::LuceneErrorKind;
    use rucene::store::{ByteArrayDataInput, ByteBuffersDataOutput};

    use super::*;

    fn read(bytes: &[u8]) -> Result<FieldMetadata> {
        let mut input = ByteArrayDataInput::new(bytes);
        Serializer::INSTANCE.read(&mut input, &FieldInfos::empty(), 10)
    }

    /// `FieldMetadata.Serializer.read`'s field check: an unknown field id is
    /// a `CorruptIndexException`, a negative one the `IllegalArgumentException`
    /// of `FieldInfos.fieldInfo(int)`.
    #[test]
    fn read_rejects_unknown_and_negative_field_ids() {
        let e = read(&[3]).unwrap_err();
        assert!(
            matches!(
                &e,
                LuceneError {
                    kind: LuceneErrorKind::CorruptIndex { .. },
                    ..
                }
            ),
            "{e:?}"
        );
        assert!(e.to_string().contains("Illegal field id= 3"), "{e}");

        let mut out = ByteBuffersDataOutput::new();
        out.write_v_int(-2).unwrap();
        let e = read(&out.to_array_copy().unwrap()).unwrap_err();
        assert!(
            matches!(&e, LuceneError { kind: LuceneErrorKind::IllegalArgument { message: m, .. }, .. } if m == "Illegal field number: -2"),
            "{e:?}"
        );
    }
}
