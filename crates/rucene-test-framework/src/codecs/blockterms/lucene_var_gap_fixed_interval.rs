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

//! Port of `org.apache.lucene.tests.codecs.blockterms.LuceneVarGapFixedInterval`.

use std::fmt;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene_codecs::blockterms::{
    EveryNTermSelector, TermsIndexReaderBase, TermsIndexWriterBase, VariableGapTermsIndexReader,
    VariableGapTermsIndexWriter, DEFAULT_TERM_INDEX_INTERVAL,
};

/// Customized version of `Lucene104PostingsFormat` that uses
/// `VariableGapTermsIndexWriter` with a fixed interval.
///
/// Port of `LuceneVarGapFixedInterval` (`extends PostingsFormat`, named
/// `LuceneVarGapFixedInterval`).
#[derive(Debug, Clone)]
pub struct LuceneVarGapFixedInterval {
    pub(crate) term_index_interval: i32,
}

impl LuceneVarGapFixedInterval {
    /// `new LuceneVarGapFixedInterval()`:
    /// `this(FixedGapTermsIndexWriter.DEFAULT_TERM_INDEX_INTERVAL)`.
    pub fn new() -> Self {
        Self::with_interval(DEFAULT_TERM_INDEX_INTERVAL)
    }

    /// `new LuceneVarGapFixedInterval(int termIndexInterval)`.
    pub fn with_interval(term_index_interval: i32) -> Self {
        LuceneVarGapFixedInterval {
            term_index_interval,
        }
    }
}

impl Default for LuceneVarGapFixedInterval {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for LuceneVarGapFixedInterval {
    /// `PostingsFormat.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PostingsFormat(name={})", self.name())
    }
}

impl PostingsFormat for LuceneVarGapFixedInterval {
    fn name(&self) -> &str {
        "LuceneVarGapFixedInterval"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        // TODO: should we make the terms index more easily
        // pluggable?  Ie so that this codec would record which
        // index impl was used, and switch on loading?
        // Or... you must make a new Codec for this?
        let interval = self.term_index_interval;
        super::fields_consumer(state, |state| {
            VariableGapTermsIndexWriter::new(state, Box::new(EveryNTermSelector::new(interval)))
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
