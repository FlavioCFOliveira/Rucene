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

//! Port of `org.apache.lucene.codecs.memory.FSTTermsWriter`.

use std::cell::RefCell;
use std::sync::Arc;

use rucene::codecs::codec_util::{write_footer, write_index_header};
use rucene::codecs::{
    BlockTermState, FieldsConsumer, NormsProducer, PostingsWriterBase, SegmentWriteState,
};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{FieldInfo, FieldInfos, Fields, IndexOptions};
use rucene::store::{ByteBuffersDataOutput, DataOutput, IndexOutput};
use rucene::util::fst::{Builder, FSTCompiler, InputType, Util, FST};
use rucene::util::{BytesRef, FixedBitSet, IOUtils, IntsRefBuilder};

use super::fst_term_outputs::{FSTTermOutputs, TermData};

pub(crate) const TERMS_EXTENSION: &str = "tfp";
pub(crate) const TERMS_CODEC_NAME: &str = "FSTTerms";
/// Port of `FSTTermsWriter.TERMS_VERSION_START`.
pub const TERMS_VERSION_START: i32 = 2;
/// Port of `FSTTermsWriter.TERMS_VERSION_CURRENT`.
pub const TERMS_VERSION_CURRENT: i32 = TERMS_VERSION_START;

/// Port of the private static class `FSTTermsWriter.FieldMetaData`.
struct FieldMetaData {
    field_number: i32,
    index_options: IndexOptions,
    num_terms: i64,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    doc_count: i32,
    dict: FST<FSTTermOutputs>,
}

/// FST-based term dict, using metadata as FST output.
///
/// The FST directly holds the mapping between `<term, metadata>`.
///
/// Term metadata consists of three parts: 1. term statistics: docFreq,
/// totalTermFreq; 2. monotonic long[], e.g. the pointer to the postings list
/// for that term; 3. generic byte[], e.g. other information need by
/// postings reader.
///
/// File format (`.tfp`): Header, *PostingsHeader*, FieldSummary, DirOffset;
/// FieldSummary is NumFields, `<FieldNumber, NumTerms, SumTotalTermFreq?,
/// SumDocFreq, DocCount, TermFST>` per field; TermFST is an
/// `FST<TermData>` whose metadata and bytes are both written to the terms
/// file.
///
/// Port of `org.apache.lucene.codecs.memory.FSTTermsWriter`.
pub struct FSTTermsWriter {
    postings_writer: Box<dyn PostingsWriterBase>,
    field_infos: FieldInfos,
    out: Option<Box<dyn IndexOutput>>,
    max_doc: i32,
    fields: Vec<FieldMetaData>,
}

impl FSTTermsWriter {
    /// Port of `FSTTermsWriter(SegmentWriteState, PostingsWriterBase)`.
    ///
    /// The postings writer moves into the constructor, so when it fails it
    /// also closes it, which Java's caller (`FSTPostingsFormat`) does with
    /// `IOUtils.closeWhileHandlingException(postingsWriter)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of creating the file, writing its header and the
    /// postings writer's `init`.
    pub fn new(
        state: &SegmentWriteState<'_>,
        mut postings_writer: Box<dyn PostingsWriterBase>,
    ) -> Result<Self> {
        let terms_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_EXTENSION,
        );
        let opened = (|| -> Result<(Box<dyn IndexOutput>, i32)> {
            let mut out = state
                .directory
                .create_output(&terms_file_name, state.context)?;
            let init = (|| -> Result<i32> {
                let max_doc = state.segment_info.max_doc()?;
                write_index_header(
                    out.as_mut(),
                    TERMS_CODEC_NAME,
                    TERMS_VERSION_CURRENT,
                    &state.segment_info.id(),
                    &state.segment_suffix,
                )?;
                postings_writer.init(out.as_mut(), state)?;
                Ok(max_doc)
            })();
            match init {
                Ok(max_doc) => Ok((out, max_doc)),
                Err(e) => {
                    IOUtils::close_while_handling_exception([IOUtils::closeable(|| out.close())])?;
                    Err(e)
                }
            }
        })();
        match opened {
            Ok((out, max_doc)) => Ok(Self {
                postings_writer,
                field_infos: state.field_infos.clone(),
                out: Some(out),
                max_doc,
                fields: Vec::new(),
            }),
            Err(e) => {
                IOUtils::close_while_handling_exception([IOUtils::closeable(|| {
                    postings_writer.close()
                })])?;
                Err(e)
            }
        }
    }
}

/// One output written through two `DataOutput` handles, for
/// `field.dict.save(out, out)`: the FST's metadata and then its bytes go to
/// the same file, in that order.
struct AliasedOutput<'c, 'o>(&'c RefCell<&'o mut dyn IndexOutput>);

impl DataOutput for AliasedOutput<'_, '_> {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.0.borrow_mut().write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.0.borrow_mut().write_bytes(b, offset, len)
    }
}

impl FieldsConsumer for FSTTermsWriter {
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        for field in fields.iterator() {
            let Some(terms) = fields.terms(&field)? else {
                continue;
            };
            let field_info = self
                .field_infos
                .field_info(&field)
                .cloned()
                .ok_or_else(|| {
                    LuceneError::unchecked(
                        "java.lang.NullPointerException",
                        "Cannot invoke \"org.apache.lucene.index.FieldInfo.getIndexOptions()\" \
                     because \"fieldInfo\" is null",
                    )
                })?;
            let has_freq = field_info
                .get_index_options()
                .subsumes(IndexOptions::DOCS_AND_FREQS);
            let mut terms_enum = terms.iterator()?;
            let mut terms_writer = TermsWriter::new(self, &field_info)?;

            let mut sum_total_term_freq = 0i64;
            let mut sum_doc_freq = 0i64;
            let mut docs_seen = FixedBitSet::new(self.max_doc as usize);

            while let Some(term) = terms_enum.next()? {
                let term_state = self.postings_writer.write_term(
                    &term,
                    terms_enum.as_mut(),
                    &mut docs_seen,
                    norms,
                )?;
                if let Some(term_state) = term_state {
                    terms_writer.finish_term(self, &term, &term_state)?;
                    sum_total_term_freq += term_state.total_term_freq;
                    sum_doc_freq += i64::from(term_state.doc_freq);
                }
            }

            terms_writer.finish(
                self,
                if has_freq { sum_total_term_freq } else { -1 },
                sum_doc_freq,
                docs_seen.cardinality() as i32,
            )?;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        let Some(mut out) = self.out.take() else {
            return Ok(());
        };
        let written = (|| -> Result<()> {
            // write field summary
            let dir_start = out.file_pointer();

            out.write_v_int(self.fields.len() as i32)?;
            for field in &self.fields {
                out.write_v_int(field.field_number)?;
                out.write_v_long(field.num_terms)?;
                if field.index_options != IndexOptions::DOCS {
                    out.write_v_long(field.sum_total_term_freq)?;
                }
                out.write_v_long(field.sum_doc_freq)?;
                out.write_v_int(field.doc_count)?;
                let cell: RefCell<&mut dyn IndexOutput> = RefCell::new(out.as_mut());
                field
                    .dict
                    .save(&mut AliasedOutput(&cell), &mut AliasedOutput(&cell))?;
            }
            // writeTrailer(out, dirStart)
            out.write_long(dir_start)?;
            write_footer(out.as_mut())
        })();
        let postings_writer = &mut self.postings_writer;
        match written {
            Ok(()) => IOUtils::close([
                IOUtils::closeable(|| out.close()),
                IOUtils::closeable(|| postings_writer.close()),
            ]),
            Err(e) => {
                IOUtils::close_while_handling_exception([
                    IOUtils::closeable(|| out.close()),
                    IOUtils::closeable(|| postings_writer.close()),
                ])?;
                Err(e)
            }
        }
    }
}

/// Port of the inner class `FSTTermsWriter.TermsWriter`; the outer writer is
/// the `w` its methods receive.
struct TermsWriter {
    fst_compiler: FSTCompiler<'static, FSTTermOutputs>,
    field_info: FieldInfo,
    num_terms: i64,
    scratch_term: IntsRefBuilder,
    meta_writer: ByteBuffersDataOutput,
}

impl TermsWriter {
    fn new(w: &mut FSTTermsWriter, field_info: &FieldInfo) -> Result<Self> {
        w.postings_writer.set_field(field_info)?;
        let outputs = FSTTermOutputs::new(field_info);
        let fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;
        Ok(Self {
            fst_compiler,
            field_info: field_info.clone(),
            num_terms: 0,
            scratch_term: IntsRefBuilder::new(),
            meta_writer: ByteBuffersDataOutput::new_resettable_instance(),
        })
    }

    /// Port of `TermsWriter.finishTerm(BytesRef, BlockTermState)`.
    fn finish_term(
        &mut self,
        w: &mut FSTTermsWriter,
        text: &BytesRef,
        state: &BlockTermState,
    ) -> Result<()> {
        // write term meta data into fst
        let mut meta = TermData::new(None, state.doc_freq, state.total_term_freq);
        w.postings_writer
            .encode_term(&mut self.meta_writer, &self.field_info, state, true)?;
        if self.meta_writer.size() > 0 {
            meta.bytes = Some(Arc::from(self.meta_writer.to_array_copy()?));
            self.meta_writer.reset()?;
        }
        let input = Util::to_ints_ref(text, &mut self.scratch_term);
        self.fst_compiler.add(&input, meta)?;
        self.num_terms += 1;
        Ok(())
    }

    /// Port of `TermsWriter.finish(long, long, int)`.
    fn finish(
        &mut self,
        w: &mut FSTTermsWriter,
        sum_total_term_freq: i64,
        sum_doc_freq: i64,
        doc_count: i32,
    ) -> Result<()> {
        // save FST dict
        if self.num_terms > 0 {
            let metadata = self.fst_compiler.compile()?;
            let compiler = std::mem::replace(
                &mut self.fst_compiler,
                Builder::new(InputType::Byte1, FSTTermOutputs::new(&self.field_info)).build()?,
            );
            let fst = FST::from_fst_reader(metadata, compiler.into_fst_reader()?).ok_or_else(
                || {
                    LuceneError::unchecked(
                        "java.lang.NullPointerException",
                        "Cannot invoke \"org.apache.lucene.util.fst.FST.save(org.apache.lucene.store.DataOutput, org.apache.lucene.store.DataOutput)\" because \"field.dict\" is null",
                    )
                },
            )?;
            w.fields.push(FieldMetaData {
                field_number: self.field_info.number,
                index_options: self.field_info.get_index_options(),
                num_terms: self.num_terms,
                sum_total_term_freq,
                sum_doc_freq,
                doc_count,
                dict: fst,
            });
        }
        Ok(())
    }
}
