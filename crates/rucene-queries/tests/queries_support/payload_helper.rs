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

//! Port of `PayloadHelper.java` of Lucene 10.5.0
//! `lucene/queries/src/test/org/apache/lucene/queries/payloads/`.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::analysis::{
    Analyzer, AnalyzerState, PerFieldReuseStrategy, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::similarities::Similarity;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{payload_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE};
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::store::mock_directory_wrapper::MockDirectoryWrapper;
use rucene_test_framework::util::english::English;
use rucene_test_framework::util::LuceneTestCase;

/// `PayloadHelper.NO_PAYLOAD_FIELD`.
pub const NO_PAYLOAD_FIELD: &str = "noPayloadField";
/// `PayloadHelper.MULTI_FIELD`.
pub const MULTI_FIELD: &str = "multiField";
/// `PayloadHelper.FIELD`.
pub const FIELD: &str = "field";

/// `payloadField`.
const PAYLOAD_FIELD: [u8; 1] = [1];
/// `payloadMultiField1`.
const PAYLOAD_MULTI_FIELD1: [u8; 1] = [2];
/// `payloadMultiField2`.
const PAYLOAD_MULTI_FIELD2: [u8; 1] = [4];

/// Equivalent to `org.apache.lucene.queries.payloads.PayloadHelper`.
#[derive(Default)]
pub struct PayloadHelper {
    /// `reader`.
    pub reader: Option<Arc<dyn DirectoryReader>>,
}

/// `PayloadHelper.PayloadAnalyzer`.
#[derive(Debug)]
pub struct PayloadAnalyzer {
    state: AnalyzerState,
}

impl PayloadAnalyzer {
    /// `PayloadAnalyzer()`: `super(PER_FIELD_REUSE_STRATEGY)`.
    pub fn new() -> Self {
        Self {
            state: AnalyzerState::new(Box::new(PerFieldReuseStrategy)),
        }
    }
}

impl Default for PayloadAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for PayloadAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.payloads.PayloadHelper$PayloadAnalyzer"
    }

    fn create_components(&self, field_name: Option<&str>) -> Result<TokenStreamComponents> {
        let result = Arc::new(AtomicRefCell::new(MockTokenizer::new(
            Arc::clone(&SIMPLE),
            true,
        )?));
        let filter = PayloadFilter::new(shared(&result), field_name)?;
        Ok(components(result, sink(filter)))
    }
}

/// The logic of `PayloadHelper.PayloadFilter`.
#[derive(Debug)]
pub struct PayloadFilterLogic {
    /// `fieldName`, `None` for Java's `null`.
    field_name: Option<String>,
    num_seen: i32,
    payload_att: Att,
}

impl TokenFilterLogic for PayloadFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            // `fieldName.equals(FIELD)` dereferences a `null` field name.
            let field_name = self.field_name.as_deref().ok_or_else(|| {
                rucene::error::LuceneError::null_pointer_with_message(
                    "Cannot invoke \"String.equals(Object)\" because \"this.fieldName\" is null",
                )
            })?;
            if field_name == FIELD {
                self.payload_att
                    .payload_mut()
                    .set_payload(Some(BytesRef::new(PAYLOAD_FIELD.to_vec())));
            } else if field_name == MULTI_FIELD {
                if self.num_seen % 2 == 0 {
                    self.payload_att
                        .payload_mut()
                        .set_payload(Some(BytesRef::new(PAYLOAD_MULTI_FIELD1.to_vec())));
                } else {
                    self.payload_att
                        .payload_mut()
                        .set_payload(Some(BytesRef::new(PAYLOAD_MULTI_FIELD2.to_vec())));
                }
                self.num_seen += 1;
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.num_seen = 0;
        Ok(())
    }
}

/// `PayloadHelper.PayloadFilter`.
pub struct PayloadFilter;

impl PayloadFilter {
    /// `PayloadFilter(TokenStream input, String fieldName)`.
    ///
    /// # Errors
    ///
    /// When the stream's factory lacks the payload attribute.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        mut input: Box<dyn TokenStream>,
        field_name: Option<&str>,
    ) -> Result<TokenFilter<PayloadFilterLogic>> {
        let payload_att = Att::add(input.attribute_source_mut(), payload_id())?;
        Ok(TokenFilter::new(
            input,
            PayloadFilterLogic {
                field_name: field_name.map(str::to_string),
                num_seen: 0,
                payload_att,
            },
        ))
    }
}

impl PayloadHelper {
    /// `new PayloadHelper()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets up a RAM-resident Directory, and adds documents (using
    /// English.intToEnglish()) with two fields: field and multiField and
    /// analyzes them using the PayloadAnalyzer.
    ///
    /// `setUp(Random, Similarity, int)`: `similarity` is the Similarity class
    /// to use in the Searcher, `num_docs` the num docs to add; returns an
    /// IndexSearcher.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while indexing or opening the reader.
    pub fn set_up(
        &mut self,
        random: &dyn Random,
        similarity: Arc<dyn Similarity>,
        num_docs: i32,
    ) -> Result<IndexSearcher> {
        let directory: Arc<dyn Directory> = Arc::new(MockDirectoryWrapper::new(
            random,
            Arc::new(ByteBuffersDirectory::default()),
        ));
        let analyzer: Arc<dyn Analyzer> = Arc::new(PayloadAnalyzer::new());

        // TODO randomize this
        let config = IndexWriterConfig::with_analyzer(Some(analyzer));
        config.set_similarity(Some(Arc::clone(&similarity)))?;
        let writer = IndexWriter::new(directory, config)?;
        // writer.infoStream = System.out;
        for i in 0..num_docs {
            let mut doc = Document::new();
            doc.add(Box::new(TextField::new(
                FIELD,
                English::int_to_english(i),
                Store::YES,
            )?));
            doc.add(Box::new(TextField::new(
                MULTI_FIELD,
                format!(
                    "{}  {}",
                    English::int_to_english(i),
                    English::int_to_english(i)
                ),
                Store::YES,
            )?));
            doc.add(Box::new(TextField::new(
                NO_PAYLOAD_FIELD,
                English::int_to_english(i),
                Store::YES,
            )?));
            writer.add_document(&doc)?;
        }
        writer.force_merge(1)?;
        let reader = directory_reader::open_with_writer(&*writer)?;
        self.reader = Some(Arc::clone(&reader));
        writer.close()?;

        let mut searcher = LuceneTestCase::new_searcher(LuceneTestCase::get_only_leaf_reader(
            reader as Arc<dyn IndexReader>,
        )?);
        searcher.set_similarity(similarity);
        Ok(searcher)
    }

    /// `tearDown()`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while closing the reader.
    pub fn tear_down(&mut self) -> Result<()> {
        match self.reader.as_ref() {
            Some(reader) => reader.close(),
            None => Err(rucene::error::LuceneError::null_pointer()),
        }
    }
}
