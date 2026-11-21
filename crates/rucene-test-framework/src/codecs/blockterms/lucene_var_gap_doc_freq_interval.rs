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

//! Port of `org.apache.lucene.tests.codecs.blockterms.LuceneVarGapDocFreqInterval`.

use std::fmt;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene_codecs::blockterms::{
    EveryNOrDocFreqTermSelector, TermsIndexReaderBase, TermsIndexWriterBase,
    VariableGapTermsIndexReader, VariableGapTermsIndexWriter, DEFAULT_TERM_INDEX_INTERVAL,
};

/// Customized version of `Lucene104PostingsFormat` that uses
/// `VariableGapTermsIndexWriter` with a fixed interval, but forcing high
/// docfreq terms to be indexed terms.
///
/// Port of `LuceneVarGapDocFreqInterval` (`extends PostingsFormat`, named
/// `LuceneVarGapDocFreqInterval`).
#[derive(Debug, Clone)]
pub struct LuceneVarGapDocFreqInterval {
    pub(crate) term_index_interval: i32,
    pub(crate) doc_freq_threshold: i32,
}

impl LuceneVarGapDocFreqInterval {
    /// `new LuceneVarGapDocFreqInterval()`: `this(1000000,
    /// FixedGapTermsIndexWriter.DEFAULT_TERM_INDEX_INTERVAL)`.
    pub fn new() -> Self {
        Self::with_settings(1_000_000, DEFAULT_TERM_INDEX_INTERVAL)
    }

    /// `new LuceneVarGapDocFreqInterval(int docFreqThreshold, int termIndexInterval)`.
    pub fn with_settings(doc_freq_threshold: i32, term_index_interval: i32) -> Self {
        LuceneVarGapDocFreqInterval {
            term_index_interval,
            doc_freq_threshold,
        }
    }
}

impl Default for LuceneVarGapDocFreqInterval {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for LuceneVarGapDocFreqInterval {
    /// `PostingsFormat.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PostingsFormat(name={})", self.name())
    }
}

impl PostingsFormat for LuceneVarGapDocFreqInterval {
    fn name(&self) -> &str {
        "LuceneVarGapDocFreqInterval"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        // TODO: should we make the terms index more easily
        // pluggable?  Ie so that this codec would record which
        // index impl was used, and switch on loading?
        // Or... you must make a new Codec for this?
        let (threshold, interval) = (self.doc_freq_threshold, self.term_index_interval);
        super::fields_consumer(state, |state| {
            VariableGapTermsIndexWriter::new(
                state,
                Box::new(EveryNOrDocFreqTermSelector::new(threshold, interval)),
            )
            .map(|w| Box::new(w) as Box<dyn TermsIndexWriterBase>)
        })
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        super::fields_producer(state, |state| {
            VariableGapTermsIndexReader::new(state)
                .map(|r| Box::new(r) as Box<dyn TermsIndexReaderBase>)
        })
    }
}
