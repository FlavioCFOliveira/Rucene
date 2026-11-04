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

//! Port of `org.apache.lucene.tests.codecs.blockterms`: the postings formats
//! that assemble `lucene-codecs`' `BlockTermsWriter`/`BlockTermsReader`, a
//! terms index and `Lucene104PostingsWriter`/`Lucene104PostingsReader`.
//!
//! * [`LuceneFixedGap`]: the fixed-gap terms index.
//! * [`LuceneVarGapFixedInterval`]: the variable-gap terms index with a fixed
//!   interval.
//! * [`LuceneVarGapDocFreqInterval`]: the variable-gap terms index with a
//!   fixed interval, forcing high-docFreq terms to be indexed terms.

pub mod lucene_fixed_gap;
pub mod lucene_var_gap_doc_freq_interval;
pub mod lucene_var_gap_fixed_interval;

pub use lucene_fixed_gap::LuceneFixedGap;
pub use lucene_var_gap_doc_freq_interval::LuceneVarGapDocFreqInterval;
pub use lucene_var_gap_fixed_interval::LuceneVarGapFixedInterval;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, Lucene104PostingsReader, Lucene104PostingsWriter,
    PostingsReaderBase, PostingsWriterBase, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene_codecs::blockterms::{
    BlockTermsReader, BlockTermsWriter, TermsIndexReaderBase, TermsIndexWriterBase,
};

/// Extension of freq postings file (`static final String FREQ_EXTENSION`,
/// unused by the formats, as in Lucene).
pub const FREQ_EXTENSION: &str = "frq";

/// Extension of prox postings file (`static final String PROX_EXTENSION`,
/// unused by the formats, as in Lucene).
pub const PROX_EXTENSION: &str = "prx";

/// The body of the three formats' `fieldsConsumer(SegmentWriteState)`, which
/// differ only in the terms index writer they create:
///
/// ```java
/// PostingsWriterBase docs = new Lucene104PostingsWriter(state);
/// TermsIndexWriterBase indexWriter;
/// try { indexWriter = <index>; success = true; }
/// finally { if (!success) docs.close(); }
/// try { return new BlockTermsWriter(indexWriter, state, docs); }
/// finally { if (!success) try { docs.close(); } finally { indexWriter.close(); } }
/// ```
///
/// `docs.close()` runs (and its error, if any, replaces the index writer's,
/// as a Java `finally` that throws does) when the index writer fails;
/// `BlockTermsWriter::new` closes both the postings writer and the index
/// writer when it fails, as the second `finally` does.
pub(crate) fn fields_consumer<'a>(
    state: &SegmentWriteState<'a>,
    index_writer: impl FnOnce(&SegmentWriteState<'a>) -> Result<Box<dyn TermsIndexWriterBase>>,
) -> Result<Box<dyn FieldsConsumer + 'a>> {
    let mut docs: Box<dyn PostingsWriterBase> = Box::new(Lucene104PostingsWriter::new(state)?);
    let index_writer = match index_writer(state) {
        Ok(w) => w,
        Err(e) => {
            docs.close()?;
            return Err(e);
        }
    };
    // Must use BlockTermsWriter (not BlockTree) because
    // BlockTree doens't support ords (yet)...
    Ok(Box::new(BlockTermsWriter::new(index_writer, state, docs)?))
}

/// The body of the three formats' `fieldsProducer(SegmentReadState)`, which
/// differ only in the terms index reader they open; the error paths mirror
/// [`fields_consumer`]'s.
pub(crate) fn fields_producer(
    state: &SegmentReadState<'_>,
    index_reader: impl FnOnce(&SegmentReadState<'_>) -> Result<Box<dyn TermsIndexReaderBase>>,
) -> Result<Box<dyn FieldsProducer>> {
    let mut postings: Box<dyn PostingsReaderBase> = Box::new(Lucene104PostingsReader::new(state)?);
    let index_reader = match index_reader(state) {
        Ok(r) => r,
        Err(e) => {
            postings.close()?;
            return Err(e);
        }
    };
    Ok(Box::new(BlockTermsReader::new(
        index_reader,
        postings,
        state,
    )?))
}
