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

//! Port of `org.apache.lucene.codecs.blockterms.BlockTermsWriter`.

use rucene::codecs::codec_util::{write_footer, write_index_header};
use rucene::codecs::{
    BlockTermState, FieldsConsumer, NormsProducer, PostingsWriterBase, SegmentWriteState, TermStats,
};
use rucene::error::{to_int_exact, LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{FieldInfo, FieldInfos, Fields, IndexOptions, TermsEnum};
use rucene::store::{ByteBuffersDataOutput, DataOutput, IndexOutput};
use rucene::util::{BytesRef, FixedBitSet, IOUtils};

use super::terms_index_writer_base::{FieldWriter, TermsIndexWriterBase};

pub(crate) const CODEC_NAME: &str = "BlockTermsWriter";

/// Initial format.
pub const VERSION_START: i32 = 4;
/// Port of `BlockTermsWriter.VERSION_CURRENT`.
pub const VERSION_CURRENT: i32 = VERSION_START;

/// Extension of terms file.
pub(crate) const TERMS_EXTENSION: &str = "tib";

/// Port of the private record `BlockTermsWriter.FieldMetaData`.
struct FieldMetaData {
    field_number: i32,
    index_options: IndexOptions,
    num_terms: i64,
    terms_start_pointer: i64,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    doc_count: i32,
}

/// Writes terms dict, block-encoding (column stride) each term's metadata
/// for each set of terms between two index terms.
///
/// Port of `org.apache.lucene.codecs.blockterms.BlockTermsWriter`.
pub struct BlockTermsWriter {
    out: Option<Box<dyn IndexOutput>>,
    postings_writer: Box<dyn PostingsWriterBase>,
    field_infos: FieldInfos,
    current_field: Option<String>,
    terms_index_writer: Box<dyn TermsIndexWriterBase>,
    max_doc: i32,
    fields: Vec<FieldMetaData>,
}

fn closed() -> LuceneError {
    LuceneError::already_closed("this IndexOutput is closed")
}

impl BlockTermsWriter {
    /// Port of `BlockTermsWriter(TermsIndexWriterBase, SegmentWriteState,
    /// PostingsWriterBase)`.
    ///
    /// The two writers move into the constructor, so when it fails it also
    /// closes them, which Java's callers (`LuceneFixedGap` and the other
    /// blockterms assemblies of `lucene-test-framework`) do in their
    /// `finally`: the postings writer first, then the terms index writer.
    ///
    /// # Errors
    ///
    /// The I/O errors of creating the file, writing its header and the
    /// postings writer's `init`.
    pub fn new(
        mut terms_index_writer: Box<dyn TermsIndexWriterBase>,
        state: &SegmentWriteState<'_>,
        mut postings_writer: Box<dyn PostingsWriterBase>,
    ) -> Result<Self> {
        let terms_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            TERMS_EXTENSION,
        );
        let opened = (|| -> Result<(i32, Box<dyn IndexOutput>)> {
            let max_doc = state.segment_info.max_doc()?;
            let mut out = state
                .directory
                .create_output(&terms_file_name, state.context)?;
            let init = (|| -> Result<()> {
                write_index_header(
                    out.as_mut(),
                    CODEC_NAME,
                    VERSION_CURRENT,
                    &state.segment_info.id(),
                    &state.segment_suffix,
                )?;
                // have consumer write its format/header
                postings_writer.init(out.as_mut(), state)
            })();
            match init {
                Ok(()) => Ok((max_doc, out)),
                Err(e) => {
                    IOUtils::close_while_handling_exception([IOUtils::closeable(|| out.close())])?;
                    Err(e)
                }
            }
        })();
        match opened {
            Ok((max_doc, out)) => Ok(Self {
                out: Some(out),
                postings_writer,
                field_infos: state.field_infos.clone(),
                current_field: None,
                terms_index_writer,
                max_doc,
                fields: Vec::new(),
            }),
            Err(e) => {
                // The callers' `finally { docs.close(); indexWriter.close(); }`.
                let docs = postings_writer.close();
                let index = terms_index_writer.close();
                docs?;
                index?;
                Err(e)
            }
        }
    }

    fn out(&mut self) -> Result<&mut Box<dyn IndexOutput>> {
        self.out.as_mut().ok_or_else(closed)
    }

    /// Port of the private `BlockTermsWriter.addField(FieldInfo)`.
    fn add_field(&mut self, field: &FieldInfo) -> Result<TermsWriter> {
        debug_assert!(self
            .current_field
            .as_deref()
            .is_none_or(|current| current < field.name.as_str()));
        self.current_field = Some(field.name.clone());
        let fp = self.out()?.file_pointer();
        let field_index_writer = self.terms_index_writer.add_field(field, fp)?;
        TermsWriter::new(self, field_index_writer, field)
    }
}

impl FieldsConsumer for BlockTermsWriter {
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        for field in fields.iterator() {
            let Some(terms) = fields.terms(&field)? else {
                continue;
            };
            let mut terms_enum = terms.iterator()?;
            let field_info = self
                .field_infos
                .field_info(&field)
                .cloned()
                .ok_or_else(|| {
                    LuceneError::unchecked(
                        "java.lang.NullPointerException",
                        "Cannot read field \"name\" because \"field\" is null",
                    )
                })?;
            let mut terms_writer = self.add_field(&field_info)?;
            while let Some(term) = terms_enum.next()? {
                terms_writer.write(self, &term, terms_enum.as_mut(), norms)?;
            }
            terms_writer.finish(self)?;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        let Some(mut out) = self.out.take() else {
            return Ok(());
        };
        let written = (|| -> Result<()> {
            let dir_start = out.file_pointer();
            out.write_v_int(self.fields.len() as i32)?;
            for field in &self.fields {
                out.write_v_int(field.field_number)?;
                out.write_v_long(field.num_terms)?;
                out.write_v_long(field.terms_start_pointer)?;
                if field.index_options != IndexOptions::DOCS {
                    out.write_v_long(field.sum_total_term_freq)?;
                }
                out.write_v_long(field.sum_doc_freq)?;
                out.write_v_int(field.doc_count)?;
            }
            // writeTrailer(dirStart)
            out.write_long(dir_start)?;
            write_footer(out.as_mut())
        })();
        // finally: IOUtils.close(out, postingsWriter, termsIndexWriter)
        let postings_writer = &mut self.postings_writer;
        let terms_index_writer = &mut self.terms_index_writer;
        let closed = IOUtils::close([
            IOUtils::closeable(|| out.close()),
            IOUtils::closeable(|| postings_writer.close()),
            IOUtils::closeable(|| terms_index_writer.close()),
        ]);
        // A `finally` that throws replaces the exception of its `try`.
        closed?;
        written
    }
}

/// Port of the private static class `BlockTermsWriter.TermEntry`.
struct TermEntry {
    term: Vec<u8>,
    state: BlockTermState,
}

/// Port of the inner class `BlockTermsWriter.TermsWriter`; the outer
/// writer is the `w` its methods receive.
struct TermsWriter {
    field_info: FieldInfo,
    terms_start_pointer: i64,
    num_terms: i64,
    field_index_writer: Box<dyn FieldWriter>,
    docs_seen: FixedBitSet,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    pending_terms: Vec<TermEntry>,
    last_prev_term: Vec<u8>,
    bytes_writer: ByteBuffersDataOutput,
}

/// Port of `TermsWriter.sharedPrefix(BytesRef, BytesRef)`.
fn shared_prefix(term1: &[u8], term2: &[u8]) -> usize {
    let end = term1.len().min(term2.len());
    (0..end).find(|&i| term1[i] != term2[i]).unwrap_or(end)
}

impl TermsWriter {
    fn new(
        w: &mut BlockTermsWriter,
        field_index_writer: Box<dyn FieldWriter>,
        field_info: &FieldInfo,
    ) -> Result<Self> {
        let docs_seen = FixedBitSet::new(w.max_doc as usize);
        let terms_start_pointer = w.out()?.file_pointer();
        w.postings_writer.set_field(field_info)?;
        Ok(Self {
            field_info: field_info.clone(),
            terms_start_pointer,
            num_terms: 0,
            field_index_writer,
            docs_seen,
            sum_total_term_freq: 0,
            sum_doc_freq: 0,
            pending_terms: Vec::with_capacity(32),
            last_prev_term: Vec::new(),
            bytes_writer: ByteBuffersDataOutput::new_resettable_instance(),
        })
    }

    /// Port of `TermsWriter.write(BytesRef, TermsEnum, NormsProducer)`.
    fn write(
        &mut self,
        w: &mut BlockTermsWriter,
        text: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        norms: Option<&dyn NormsProducer>,
    ) -> Result<()> {
        let Some(state) =
            w.postings_writer
                .write_term(text, terms_enum, &mut self.docs_seen, norms)?
        else {
            // No docs for this term:
            return Ok(());
        };

        self.sum_doc_freq += i64::from(state.doc_freq);
        self.sum_total_term_freq += state.total_term_freq;
        debug_assert!(state.doc_freq > 0);

        let stats = TermStats::new(state.doc_freq, state.total_term_freq);
        let is_index_term = self.field_index_writer.check_index_term(text, &stats)?;

        if is_index_term {
            if !self.pending_terms.is_empty() {
                // Instead of writing each term, live, we gather terms
                // in RAM in a pending buffer, and then write the
                // entire block in between index terms:
                self.flush_block(w)?;
            }
            let fp = w.out()?.file_pointer();
            self.field_index_writer.add(text, &stats, fp)?;
        }

        self.pending_terms.push(TermEntry {
            term: text.slice().to_vec(),
            state,
        });
        self.num_terms += 1;
        Ok(())
    }

    /// Port of `TermsWriter.finish()`: finishes all terms in this field.
    fn finish(&mut self, w: &mut BlockTermsWriter) -> Result<()> {
        if !self.pending_terms.is_empty() {
            self.flush_block(w)?;
        }
        // EOF marker:
        let out = w.out()?;
        out.write_v_int(0)?;
        let fp = out.file_pointer();
        self.field_index_writer.finish(fp)?;
        if self.num_terms > 0 {
            let index_options = self.field_info.get_index_options();
            w.fields.push(FieldMetaData {
                field_number: self.field_info.number,
                index_options,
                num_terms: self.num_terms,
                terms_start_pointer: self.terms_start_pointer,
                sum_total_term_freq: if index_options.subsumes(IndexOptions::DOCS_AND_FREQS) {
                    self.sum_total_term_freq
                } else {
                    -1
                },
                sum_doc_freq: self.sum_doc_freq,
                doc_count: self.docs_seen.cardinality() as i32,
            });
        }
        Ok(())
    }

    /// Port of `TermsWriter.flushBlock()`.
    fn flush_block(&mut self, w: &mut BlockTermsWriter) -> Result<()> {
        // First pass: compute common prefix for all terms
        // in the block, against term before first term in
        // this block:
        let mut common_prefix = shared_prefix(&self.last_prev_term, &self.pending_terms[0].term);
        for entry in &self.pending_terms[1..] {
            common_prefix = common_prefix.min(shared_prefix(&self.last_prev_term, &entry.term));
        }

        let BlockTermsWriter {
            out,
            postings_writer,
            ..
        } = w;
        let out = out.as_mut().ok_or_else(closed)?;
        out.write_v_int(self.pending_terms.len() as i32)?;
        out.write_v_int(common_prefix as i32)?;

        // 2nd pass: write suffixes, as separate byte[] blob
        for entry in &self.pending_terms {
            let suffix = entry.term.len() - common_prefix;
            // TODO: cutover to better intblock codec, instead
            // of interleaving here:
            self.bytes_writer.write_v_int(suffix as i32)?;
            self.bytes_writer
                .write_bytes(&entry.term, common_prefix, suffix)?;
        }
        out.write_v_int(to_int_exact(self.bytes_writer.size() as i64)?)?;
        self.bytes_writer.copy_to(out.as_mut())?;
        self.bytes_writer.reset();

        // 3rd pass: write the freqs as byte[] blob
        // TODO: cutover to better intblock codec.  simple64?
        // write prefix, suffix first:
        let index_options = self.field_info.get_index_options();
        for entry in &self.pending_terms {
            let state = &entry.state;
            self.bytes_writer.write_v_int(state.doc_freq)?;
            if index_options != IndexOptions::DOCS {
                self.bytes_writer
                    .write_v_long(state.total_term_freq - i64::from(state.doc_freq))?;
            }
        }
        out.write_v_int(to_int_exact(self.bytes_writer.size() as i64)?)?;
        self.bytes_writer.copy_to(out.as_mut())?;
        self.bytes_writer.reset();

        // 4th pass: write the metadata
        let mut absolute = true;
        for entry in &self.pending_terms {
            postings_writer.encode_term(
                &mut self.bytes_writer,
                &self.field_info,
                &entry.state,
                absolute,
            )?;
            absolute = false;
        }
        out.write_v_int(to_int_exact(self.bytes_writer.size() as i64)?)?;
        self.bytes_writer.copy_to(out.as_mut())?;
        self.bytes_writer.reset();

        let last = self
            .pending_terms
            .pop()
            .expect("INVARIANT: flushBlock runs with pendingCount > 0");
        self.last_prev_term = last.term;
        self.pending_terms.clear();
        Ok(())
    }
}
