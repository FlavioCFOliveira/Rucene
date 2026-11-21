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

//! Port of `org.apache.lucene.codecs.uniformsplit`: pluggable term index /
//! block terms dictionary implementations.
//!
//! Structure similar to `org.apache.lucene.codecs.blockterms.VariableGapTermsIndexWriter`
//! with additional optimizations.
//!
//! * Designed to be extensible
//! * Reduced on-heap memory usage.
//! * Efficient to seek terms (`TermQuery`, `PhraseQuery`)
//! * Quite efficient for `PrefixQuery`
//! * Not efficient for spell-check and `FuzzyQuery`, in this case prefer
//!   `Lucene104PostingsFormat`
//!
//! (`package-info.java`.)
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`BlockDecoder`] | `BlockDecoder` |
//! | [`BlockEncoder`], [`WritableBytes`] | `BlockEncoder`, `BlockEncoder.WritableBytes` |
//! | [`BlockHeader`] | `BlockHeader` (and its `Serializer`) |
//! | [`BlockLine`] | `BlockLine` (and its `Serializer`) |
//! | [`BlockReader`] | `BlockReader` |
//! | [`BlockWriter`] | `BlockWriter` |
//! | [`DeltaBaseTermStateSerializer`] | `DeltaBaseTermStateSerializer` |
//! | [`FSTDictionary`] | `FSTDictionary` (and its `Browser`, `BrowserSupplier`, `Builder`) |
//! | [`FieldMetadata`] | `FieldMetadata` (and its `Serializer`) |
//! | [`IndexDictionary`] | `IndexDictionary` (and its `Builder`, `Browser`, `BrowserSupplier`) |
//! | [`IntersectBlockReader`] | `IntersectBlockReader` |
//! | [`ram_usage_util`] | `RamUsageUtil` |
//! | [`TermBytes`] | `TermBytes` |
//! | [`UniformSplitPostingsFormat`] | `UniformSplitPostingsFormat` |
//! | [`UniformSplitTerms`] | `UniformSplitTerms` |
//! | [`UniformSplitTermsReader`] | `UniformSplitTermsReader` |
//! | [`UniformSplitTermsWriter`] | `UniformSplitTermsWriter` |
//! | [`sharedterms`] | `org.apache.lucene.codecs.uniformsplit.sharedterms` |
//!
//! Java's subclasses in `sharedterms` override protected methods of the base
//! classes (`BlockReader.readTermState`, `BlockWriter.writeBlockLine`,
//! `UniformSplitTermsReader.fillFieldMap`, ...). Rust has no implementation
//! inheritance: each base type here carries the hook its `sharedterms`
//! subclass overrides (documented on the item), and the `sharedterms` types
//! wrap the base types.

pub mod block_decoder;
pub mod block_encoder;
pub mod block_header;
pub mod block_line;
pub mod block_reader;
pub mod block_writer;
pub mod delta_base_term_state_serializer;
pub mod field_metadata;
pub mod fst_dictionary;
pub mod index_dictionary;
pub mod intersect_block_reader;
pub mod ram_usage_util;
pub mod sharedterms;
pub mod term_bytes;
pub mod uniform_split_postings_format;
pub mod uniform_split_terms;
pub mod uniform_split_terms_reader;
pub mod uniform_split_terms_writer;

pub use block_decoder::BlockDecoder;
pub use block_encoder::{BlockEncoder, WritableBytes};
pub use block_header::BlockHeader;
pub use block_line::BlockLine;
pub use block_reader::{BlockReader, BlockReaderOverrides};
pub use block_writer::BlockWriter;
pub use delta_base_term_state_serializer::DeltaBaseTermStateSerializer;
pub use field_metadata::FieldMetadata;
pub use fst_dictionary::FSTDictionary;
pub use index_dictionary::IndexDictionary;
pub use intersect_block_reader::IntersectBlockReader;
pub use term_bytes::TermBytes;
pub use uniform_split_postings_format::UniformSplitPostingsFormat;
pub use uniform_split_terms::UniformSplitTerms;
pub use uniform_split_terms_reader::UniformSplitTermsReader;
pub use uniform_split_terms_writer::{TermsWriterOverrides, UniformSplitTermsWriter};

use rucene::error::LuceneError;
use rucene::store::DataOutput;

/// Java's `NullPointerException` with the JDK's helpful message.
pub(crate) fn npe(message: &str) -> LuceneError {
    LuceneError::unchecked("java.lang.NullPointerException", message)
}

/// One output written through two `DataOutput` handles, for
/// `fst.save(output, output)`: the FST's metadata and then its bytes go to
/// the same output, in that order.
pub(crate) struct AliasedOutput<'c, 'o>(pub(crate) &'c std::cell::RefCell<&'o mut dyn DataOutput>);

impl DataOutput for AliasedOutput<'_, '_> {
    fn write_byte(&mut self, b: u8) -> rucene::error::Result<()> {
        self.0.borrow_mut().write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> rucene::error::Result<()> {
        self.0.borrow_mut().write_bytes(b, offset, len)
    }
}
