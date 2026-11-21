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

//! Port of `org.apache.lucene.codecs.blockterms`: pluggable term index /
//! block terms dictionary implementations.
//!
//! The package registers no service of its own in Lucene 10.5.0; its users
//! are `lucene-test-framework`'s `LuceneFixedGap`,
//! `LuceneVarGapFixedInterval` and `LuceneVarGapDocFreqInterval`, which
//! assemble a [`BlockTermsWriter`] (or [`BlockTermsReader`]), a terms index
//! writer (or reader) and `Lucene104PostingsWriter` (or reader).
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`BlockTermsReader`] | `BlockTermsReader` |
//! | [`BlockTermsWriter`] | `BlockTermsWriter` |
//! | [`FixedGapTermsIndexReader`] | `FixedGapTermsIndexReader` |
//! | [`FixedGapTermsIndexWriter`] | `FixedGapTermsIndexWriter` |
//! | [`TermsIndexReaderBase`], [`FieldIndexEnum`] | `TermsIndexReaderBase`, `TermsIndexReaderBase.FieldIndexEnum` |
//! | [`TermsIndexWriterBase`], [`FieldWriter`] | `TermsIndexWriterBase`, `TermsIndexWriterBase.FieldWriter` |
//! | [`VariableGapTermsIndexReader`] | `VariableGapTermsIndexReader` |
//! | [`VariableGapTermsIndexWriter`], [`IndexTermSelector`], [`EveryNTermSelector`], [`EveryNOrDocFreqTermSelector`] | `VariableGapTermsIndexWriter` and its selectors |

pub mod block_terms_reader;
pub mod block_terms_writer;
pub mod fixed_gap_terms_index_reader;
pub mod fixed_gap_terms_index_writer;
pub mod terms_index_reader_base;
pub mod terms_index_writer_base;
pub mod variable_gap_terms_index_reader;
pub mod variable_gap_terms_index_writer;

pub use block_terms_reader::BlockTermsReader;
pub use block_terms_writer::BlockTermsWriter;
pub use fixed_gap_terms_index_reader::FixedGapTermsIndexReader;
pub use fixed_gap_terms_index_writer::{FixedGapTermsIndexWriter, DEFAULT_TERM_INDEX_INTERVAL};
pub use terms_index_reader_base::{FieldIndexEnum, TermsIndexReaderBase};
pub use terms_index_writer_base::{FieldWriter, TermsIndexWriterBase};
pub use variable_gap_terms_index_reader::VariableGapTermsIndexReader;
pub use variable_gap_terms_index_writer::{
    EveryNOrDocFreqTermSelector, EveryNTermSelector, IndexTermSelector, VariableGapTermsIndexWriter,
};
