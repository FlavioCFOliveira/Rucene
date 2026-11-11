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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextFieldsWriter`.

use rucene::codecs::{
    CompetitiveImpactAccumulator, FieldsConsumer, NormsProducer, SegmentWriteState,
};
use rucene::error::{LuceneError, Result};
use rucene::index::{
    FieldInfos, Fields, NumericDocValues, PostingsEnum, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_OFFSETS,
    POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};
use rucene::search::NO_MORE_DOCS;
use rucene::store::IndexOutput;

use super::postings_format::get_postings_file_name;
use super::skip_writer::{SimpleTextSkipWriter, BLOCK_SIZE};
use super::util;

pub(crate) const END: &[u8] = b"END";
pub(crate) const FIELD: &[u8] = b"field ";
pub(crate) const TERM: &[u8] = b"  term ";
pub(crate) const DOC: &[u8] = b"    doc ";
pub(crate) const FREQ: &[u8] = b"      freq ";
pub(crate) const POS: &[u8] = b"      pos ";
pub(crate) const START_OFFSET: &[u8] = b"      startOffset ";
pub(crate) const END_OFFSET: &[u8] = b"      endOffset ";
pub(crate) const PAYLOAD: &[u8] = b"        payload ";

/// Writes the postings of a segment as plain text.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextFieldsWriter`.
pub struct SimpleTextFieldsWriter<'a> {
    out: Option<Box<dyn IndexOutput>>,
    field_infos: &'a FieldInfos,
    /// The segment name, Java's `segment`.
    pub segment: String,

    /// for write skip data.
    doc_count: i32,

    skip_writer: SimpleTextSkipWriter,
    competitive_impact_accumulator: CompetitiveImpactAccumulator,
    last_doc_file_pointer: i64,
}

impl std::fmt::Debug for SimpleTextFieldsWriter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleTextFieldsWriter")
            .field("segment", &self.segment)
            .finish_non_exhaustive()
    }
}

impl<'a> SimpleTextFieldsWriter<'a> {
    /// Port of `SimpleTextFieldsWriter(SegmentWriteState)`.
    ///
    /// # Errors
    ///
    /// Propagates the directory's error.
    pub fn new(write_state: &SegmentWriteState<'a>) -> Result<Self> {
        let file_name =
            get_postings_file_name(&write_state.segment_info.name, &write_state.segment_suffix);
        let segment = write_state.segment_info.name.clone();
        let out = write_state
            .directory
            .create_output(&file_name, write_state.context)?;
        let skip_writer = SimpleTextSkipWriter::new(write_state.segment_info.max_doc()?)?;
        Ok(Self {
            out: Some(out),
            field_infos: write_state.field_infos,
            segment,
            doc_count: 0,
            skip_writer,
            competitive_impact_accumulator: CompetitiveImpactAccumulator::new(),
            last_doc_file_pointer: -1,
        })
    }

    fn out(&mut self) -> Result<&mut dyn IndexOutput> {
        match self.out.as_deref_mut() {
            Some(out) => Ok(out),
            None => Err(LuceneError::already_closed("this writer is closed")),
        }
    }

    fn write_str(&mut self, s: &str) -> Result<()> {
        util::write_str(self.out()?, s)
    }

    fn write_bytes(&mut self, b: &[u8]) -> Result<()> {
        util::write_bytes(self.out()?, b)
    }

    fn newline(&mut self) -> Result<()> {
        util::write_newline(self.out()?)
    }

    /// Port of `SimpleTextFieldsWriter.write(FieldInfos, Fields, NormsProducer)`.
    ///
    /// # Errors
    ///
    /// Propagates the reading and writing errors.
    pub fn write_with_field_infos(
        &mut self,
        field_infos: &FieldInfos,
        fields: &dyn Fields,
        norms_producer: Option<&dyn NormsProducer>,
    ) -> Result<()> {
        // for each field
        let field_names: Vec<String> = fields.iterator().collect();
        for field in &field_names {
            let Some(terms) = fields.terms(field)? else {
                // Annoyingly, this can happen!
                continue;
            };
            let field_info = field_infos.field_info(field).ok_or_else(|| {
                LuceneError::null_pointer_with_message(
                    "Cannot invoke \"org.apache.lucene.index.FieldInfo.hasPayloads()\" because \
                     \"fieldInfo\" is null",
                )
            })?;

            let mut wrote_field = false;

            let has_positions = terms.has_positions();
            let has_freqs = terms.has_freqs();
            let has_payloads = field_info.has_payloads();
            let has_offsets = terms.has_offsets();
            let field_has_norms = field_info.has_norms();

            let mut norms: Option<Box<dyn NumericDocValues>> = None;
            if field_has_norms {
                if let Some(norms_producer) = norms_producer {
                    norms = norms_producer.get_norms(field_info)?;
                }
            }

            let mut flags = 0;
            if has_positions {
                flags = POSTINGS_ENUM_POSITIONS;
                if has_payloads {
                    flags |= POSTINGS_ENUM_PAYLOADS;
                }
                if has_offsets {
                    flags |= POSTINGS_ENUM_OFFSETS;
                }
            } else if has_freqs {
                flags |= POSTINGS_ENUM_FREQS;
            }

            let mut terms_enum = terms.iterator()?;
            let mut postings_enum: Option<Box<dyn PostingsEnum>> = None;

            // for each term in field
            while let Some(term) = terms_enum.next()? {
                self.doc_count = 0;
                self.skip_writer.reset_skip();
                self.competitive_impact_accumulator.clear();
                self.last_doc_file_pointer = -1;

                let postings = terms_enum.postings(postings_enum.take(), flags)?;

                debug_assert!(
                    postings.is_some(),
                    "termsEnum={terms_enum} hasPos={has_positions} flags={flags}"
                );
                let mut postings = postings.ok_or_else(LuceneError::null_pointer)?;

                let mut wrote_term = false;

                // for each doc in field+term
                loop {
                    let doc = postings.next_doc()?;
                    if doc == NO_MORE_DOCS {
                        break;
                    }

                    if !wrote_term {
                        if !wrote_field {
                            // we lazily do this, in case the field had no terms
                            self.write_bytes(FIELD)?;
                            self.write_str(field)?;
                            self.newline()?;
                            wrote_field = true;
                        }

                        // we lazily do this, in case the term had zero docs
                        self.write_bytes(TERM)?;
                        self.write_bytes(term.slice())?;
                        self.newline()?;
                        wrote_term = true;
                    }
                    if self.last_doc_file_pointer == -1 {
                        self.last_doc_file_pointer = self.out()?.file_pointer();
                    }
                    self.write_bytes(DOC)?;
                    self.write_str(&doc.to_string())?;
                    self.newline()?;
                    if has_freqs {
                        let freq = postings.freq()?;
                        self.write_bytes(FREQ)?;
                        self.write_str(&freq.to_string())?;
                        self.newline()?;

                        if has_positions {
                            // for assert:
                            let mut last_start_offset = 0;

                            // for each pos in field+term+doc
                            for _ in 0..freq {
                                let position = postings.next_position()?;

                                self.write_bytes(POS)?;
                                self.write_str(&position.to_string())?;
                                self.newline()?;

                                if has_offsets {
                                    let start_offset = postings.start_offset();
                                    let end_offset = postings.end_offset();
                                    debug_assert!(end_offset >= start_offset);
                                    debug_assert!(
                                        start_offset >= last_start_offset,
                                        "startOffset={start_offset} lastStartOffset={last_start_offset}"
                                    );
                                    last_start_offset = start_offset;
                                    self.write_bytes(START_OFFSET)?;
                                    self.write_str(&start_offset.to_string())?;
                                    self.newline()?;
                                    self.write_bytes(END_OFFSET)?;
                                    self.write_str(&end_offset.to_string())?;
                                    self.newline()?;
                                }

                                let payload = postings.get_payload()?.map(<[u8]>::to_vec);

                                if let Some(payload) = payload.filter(|p| !p.is_empty()) {
                                    self.write_bytes(PAYLOAD)?;
                                    self.write_bytes(&payload)?;
                                    self.newline()?;
                                }
                            }
                        }
                        let norm = get_norm(doc, norms.as_deref_mut())?;
                        self.competitive_impact_accumulator.add(freq, norm);
                    } else {
                        let norm = get_norm(doc, norms.as_deref_mut())?;
                        self.competitive_impact_accumulator.add(1, norm);
                    }
                    self.doc_count += 1;
                    if self.doc_count != 0 && self.doc_count % BLOCK_SIZE == 0 {
                        self.skip_writer.buffer_skip(
                            doc,
                            self.last_doc_file_pointer,
                            self.doc_count,
                            &self.competitive_impact_accumulator,
                        )?;
                        self.competitive_impact_accumulator.clear();
                        self.last_doc_file_pointer = -1;
                    }
                }
                if self.doc_count >= BLOCK_SIZE {
                    let out = self
                        .out
                        .as_deref_mut()
                        .ok_or_else(|| LuceneError::already_closed("this writer is closed"))?;
                    self.skip_writer.write_skip(out)?;
                }
                postings_enum = Some(postings);
            }
        }
        Ok(())
    }
}

/// Port of the private `SimpleTextFieldsWriter.getNorm`.
fn get_norm(doc: i32, norms: Option<&mut (dyn NumericDocValues + '_)>) -> Result<i64> {
    let Some(norms) = norms else {
        return Ok(1);
    };
    let found = norms.advance_exact(doc)?;
    if !found {
        return Ok(1);
    }
    norms.long_value()
}

impl FieldsConsumer for SimpleTextFieldsWriter<'_> {
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        let field_infos = self.field_infos;
        self.write_with_field_infos(field_infos, fields, norms)
    }

    /// Port of `SimpleTextFieldsWriter.close`.
    fn close(&mut self) -> Result<()> {
        if self.out.is_some() {
            let result = (|| {
                self.write_bytes(END)?;
                self.newline()?;
                util::write_checksum(self.out()?)
            })();
            let mut out = self.out.take().expect("INVARIANT: checked above");
            let closed = out.close();
            result?;
            closed?;
        }
        Ok(())
    }
}
