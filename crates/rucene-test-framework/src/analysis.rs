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

//! Port of `org.apache.lucene.tests.analysis`: mock tokenizers, filters and
//! analyzers, canned token streams, [`BaseTokenStreamTestCase`], the
//! assertions every analysis test is built on, [`BaseTokenStreamFactoryTestCase`]
//! for the analysis factories and [`CollationTestBase`] for collating
//! analyzers.
//!
//! The mocks hold their attributes by interface through [`attributes::Att`],
//! so that they work over each factory
//! [`BaseTokenStreamTestCase::new_attribute_factory`] can pick.
//!
//! Offsets, lengths and random positions inside a text count UTF-16 code
//! units, as Java's do; the readers of `rucene::analysis` yield Unicode
//! scalar values, and the mocks convert where Java's arithmetic depends on
//! it.

use std::cell::RefCell;
use std::rc::Rc;

use rucene::analysis::{CharReader, SharedTokenStream, TokenStream, TokenStreamComponents};
use rucene::error::Result;

pub mod attributes;
pub mod base_token_stream_factory_test_case;
pub mod base_token_stream_test_case;
pub mod canned_binary_token_stream;
pub mod canned_token_stream;
pub mod collation_test_base;
pub mod cranky_token_filter;
pub mod lookahead_token_filter;
pub mod mock_analyzer;
pub mod mock_bytes_analyzer;
pub mod mock_char_filter;
pub mod mock_fixed_length_payload_filter;
pub mod mock_graph_token_filter;
pub mod mock_hole_injecting_token_filter;
pub mod mock_lower_case_filter;
pub mod mock_payload_analyzer;
pub mod mock_random_lookahead_token_filter;
pub mod mock_reader_wrapper;
pub mod mock_synonym_analyzer;
pub mod mock_synonym_filter;
pub mod mock_token_filter;
pub mod mock_tokenizer;
pub mod mock_utf16_term_attribute_impl;
pub mod mock_variable_length_payload_filter;
pub mod simple_payload_filter;
pub mod standard;
pub mod token;
pub mod token_stream_to_dot;
pub mod validating_token_filter;
pub mod vocabulary_assert;

pub use base_token_stream_factory_test_case::BaseTokenStreamFactoryTestCase;
pub use base_token_stream_test_case::BaseTokenStreamTestCase;
pub use canned_binary_token_stream::{BinaryToken, CannedBinaryTokenStream};
pub use canned_token_stream::CannedTokenStream;
pub use collation_test_base::CollationTestBase;
pub use cranky_token_filter::CrankyTokenFilter;
pub use lookahead_token_filter::{
    LookaheadLogic, LookaheadPosition, LookaheadTokenFilter, Position,
};
pub use mock_analyzer::MockAnalyzer;
pub use mock_bytes_analyzer::MockBytesAnalyzer;
pub use mock_char_filter::MockCharFilter;
pub use mock_fixed_length_payload_filter::MockFixedLengthPayloadFilter;
pub use mock_graph_token_filter::MockGraphTokenFilter;
pub use mock_hole_injecting_token_filter::MockHoleInjectingTokenFilter;
pub use mock_lower_case_filter::MockLowerCaseFilter;
pub use mock_payload_analyzer::MockPayloadAnalyzer;
pub use mock_random_lookahead_token_filter::MockRandomLookaheadTokenFilter;
pub use mock_reader_wrapper::MockReaderWrapper;
pub use mock_synonym_analyzer::MockSynonymAnalyzer;
pub use mock_synonym_filter::MockSynonymFilter;
pub use mock_token_filter::MockTokenFilter;
pub use mock_tokenizer::MockTokenizer;
pub use mock_utf16_term_attribute_impl::MockUTF16TermAttributeImpl;
pub use mock_variable_length_payload_filter::MockVariableLengthPayloadFilter;
pub use simple_payload_filter::SimplePayloadFilter;
pub use token::Token;
pub use token_stream_to_dot::TokenStreamToDot;
pub use validating_token_filter::ValidatingTokenFilter;
pub use vocabulary_assert::VocabularyAssert;

/// A tokenizer: a [`TokenStream`] that accepts a new reader.
///
/// Java's `Tokenizer.setReader(Reader)`, which `TokenStreamComponents`
/// installs as its source.
pub trait ReaderTokenizer: TokenStream {
    /// `setReader(Reader)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when the previous input was not closed.
    fn set_reader(&mut self, reader: Box<dyn CharReader>) -> Result<()>;
}

impl ReaderTokenizer for MockTokenizer {
    fn set_reader(&mut self, reader: Box<dyn CharReader>) -> Result<()> {
        MockTokenizer::set_reader(self, reader)
    }
}

impl<T: rucene::analysis::TokenizerLogic + 'static> ReaderTokenizer
    for rucene::analysis::Tokenizer<T>
{
    fn set_reader(&mut self, reader: Box<dyn CharReader>) -> Result<()> {
        rucene::analysis::Tokenizer::set_reader(self, reader)
    }
}

/// `new TokenStreamComponents(tokenizer, sink)`: `tokenizer::setReader` as
/// the source, `sink` as the stream handed out.
pub fn components<T: ReaderTokenizer + 'static>(
    tokenizer: Rc<RefCell<T>>,
    sink: Rc<RefCell<dyn TokenStream>>,
) -> TokenStreamComponents {
    TokenStreamComponents::new(
        Box::new(move |reader| tokenizer.borrow_mut().set_reader(reader)),
        sink,
    )
}

/// `new TokenStreamComponents(tokenizer)`: the tokenizer is both source and
/// sink.
pub fn tokenizer_components<T: ReaderTokenizer + 'static>(
    tokenizer: Rc<RefCell<T>>,
) -> TokenStreamComponents {
    let sink: Rc<RefCell<dyn TokenStream>> = tokenizer.clone();
    components(tokenizer, sink)
}

/// The tokenizer as the input of a filter chain, while the components keep
/// it as their source.
pub fn shared<T: TokenStream + 'static>(tokenizer: &Rc<RefCell<T>>) -> Box<dyn TokenStream> {
    let inner: Rc<RefCell<dyn TokenStream>> = tokenizer.clone();
    Box::new(SharedTokenStream::new(inner))
}

/// Wraps a stream for [`TokenStreamComponents`] as their sink.
pub fn sink<T: TokenStream + 'static>(stream: T) -> Rc<RefCell<dyn TokenStream>> {
    Rc::new(RefCell::new(stream))
}
