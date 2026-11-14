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

//! An analyzer wrapper that removes a field's most frequent terms, ported
//! from `org.apache.lucene.analysis.query.QueryAutoStopWordAnalyzer`.

use std::sync::Arc;

use rucene::analysis::{
    new_stop_filter, Analyzer, AnalyzerState, AnalyzerWrapper, AnalyzerWrapperLogic, CharArraySet,
    CharReader, SharedTokenStream, TokenStream, TokenStreamComponents,
};
use rucene::error::Result;
use rucene::index::{FieldInfos, IndexReader, MultiTerms, Term};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeFactory;
use rucene::util::java_collections::{JavaHashMap, JavaHashSet};

use crate::analysis::miscellaneous::limit_token_count_analyzer::DelegateReuseStrategy;

/// The default maximum percentage (40%) of index documents which can contain
/// a term, after which the term is considered to be a stop word.
///
/// Equivalent to `QueryAutoStopWordAnalyzer.defaultMaxDocFreqPercent`.
pub const DEFAULT_MAX_DOC_FREQ_PERCENT: f32 = 0.4;

/// What [`QueryAutoStopWordAnalyzer`] overrides of `AnalyzerWrapper`.
#[derive(Debug)]
struct QueryAutoStopWordAnalyzerLogic {
    delegate: Arc<dyn Analyzer>,
    stop_words_per_field: Arc<JavaHashMap<JavaHashSet>>,
}

impl AnalyzerWrapperLogic for QueryAutoStopWordAnalyzerLogic {
    /// Equivalent to `QueryAutoStopWordAnalyzer.getWrappedAnalyzer(String)`.
    fn get_wrapped_analyzer(&self, _field_name: &str) -> &dyn Analyzer {
        self.delegate.as_ref()
    }

    /// Equivalent to `QueryAutoStopWordAnalyzer.wrapComponents(String,
    /// TokenStreamComponents)`: the components as they are for a field
    /// without stop words, else the delegate's source and a `StopFilter` over
    /// its token stream.
    ///
    /// # Panics
    ///
    /// With the exception `StopFilter`'s construction throws (the hook
    /// returns no error, and Java's exceptions there are unchecked), which
    /// only an attribute the stream's factory cannot implement causes.
    fn wrap_components(
        &self,
        field_name: &str,
        components: TokenStreamComponents,
    ) -> TokenStreamComponents {
        let Some(stop_words) = self.stop_words_per_field.get(field_name) else {
            return components;
        };
        let input: Box<dyn TokenStream> =
            Box::new(SharedTokenStream::new(components.token_stream()));
        match new_stop_filter(input, CharArraySet::from_iter(stop_words.iter(), false)) {
            Ok(stop_filter) => components.with_sink(Arc::new(AtomicRefCell::new(stop_filter))),
            Err(error) => rucene::store::java_nio_file::throw_unchecked(
                error.java_simple_name(),
                Some(&error.to_string()),
            ),
        }
    }
}

/// An `Analyzer` used primarily at query time to wrap another analyzer and
/// provide a layer of protection which prevents very common words from being
/// passed into queries.
///
/// For very large indexes the cost of reading TermDocs for a very common
/// word can be high. This analyzer was created after experience with a 38
/// million doc index which had a term in around 50% of docs and was causing
/// TermQueries for this term to take 2 seconds.
///
/// Equivalent to `org.apache.lucene.analysis.query.QueryAutoStopWordAnalyzer`
/// (since 3.1), an `AnalyzerWrapper`. Its `HashMap<String, Set<String>>` of
/// stop words is a [`JavaHashMap`] of [`JavaHashSet`]s, so that
/// [`get_stop_words`](Self::get_stop_words) and
/// [`get_all_stop_words`](Self::get_all_stop_words) list them in Java's
/// iteration order.
#[derive(Debug)]
pub struct QueryAutoStopWordAnalyzer {
    inner: AnalyzerWrapper,
    stop_words_per_field: Arc<JavaHashMap<JavaHashSet>>,
}

/// `FieldInfos.getIndexedFields(indexReader)`, in the iteration order of
/// the `HashSet` Java returns.
///
/// **Divergence from Lucene 10.5.0.** `rucene`'s `getIndexedFields` returns
/// a Rust set; its fields are put into a [`JavaHashSet`] in that set's
/// order. Java's set adds them leaf by leaf, so two fields whose hashes
/// share a bucket may iterate in the other order; the only effect is the
/// order in which the constructor reads them.
fn indexed_fields(index_reader: &Arc<dyn IndexReader>) -> Result<Vec<String>> {
    let mut fields = JavaHashSet::new();
    for field in FieldInfos::get_indexed_fields(Arc::clone(index_reader))? {
        fields.insert(field);
    }
    Ok(fields.iter().cloned().collect())
}

impl QueryAutoStopWordAnalyzer {
    /// Creates a new QueryAutoStopWordAnalyzer with stopwords calculated for
    /// all indexed fields from terms with a document frequency percentage
    /// greater than [`DEFAULT_MAX_DOC_FREQ_PERCENT`]
    ///
    /// * `delegate` Analyzer whose TokenStream will be filtered
    /// * `index_reader` IndexReader to identify the stopwords from
    ///
    /// Equivalent to `QueryAutoStopWordAnalyzer(Analyzer, IndexReader)`.
    ///
    /// # Errors
    ///
    /// Can be thrown while reading from the IndexReader
    pub fn new(delegate: Arc<dyn Analyzer>, index_reader: &Arc<dyn IndexReader>) -> Result<Self> {
        Self::with_max_percent_docs(delegate, index_reader, DEFAULT_MAX_DOC_FREQ_PERCENT)
    }

    /// Creates a new QueryAutoStopWordAnalyzer with stopwords calculated for
    /// all indexed fields from terms with a document frequency greater than
    /// the given maxDocFreq
    ///
    /// * `delegate` Analyzer whose TokenStream will be filtered
    /// * `index_reader` IndexReader to identify the stopwords from
    /// * `max_doc_freq` Document frequency terms should be above in order to
    ///   be stopwords
    ///
    /// Equivalent to `QueryAutoStopWordAnalyzer(Analyzer, IndexReader, int)`.
    ///
    /// # Errors
    ///
    /// Can be thrown while reading from the IndexReader
    pub fn with_max_doc_freq(
        delegate: Arc<dyn Analyzer>,
        index_reader: &Arc<dyn IndexReader>,
        max_doc_freq: i32,
    ) -> Result<Self> {
        let fields = indexed_fields(index_reader)?;
        Self::with_fields_max_doc_freq(delegate, index_reader, &fields, max_doc_freq)
    }

    /// Creates a new QueryAutoStopWordAnalyzer with stopwords calculated for
    /// all indexed fields from terms with a document frequency percentage
    /// greater than the given maxPercentDocs
    ///
    /// * `delegate` Analyzer whose TokenStream will be filtered
    /// * `index_reader` IndexReader to identify the stopwords from
    /// * `max_percent_docs` The maximum percentage (between 0.0 and 1.0) of
    ///   index documents which contain a term, after which the word is
    ///   considered to be a stop word
    ///
    /// Equivalent to `QueryAutoStopWordAnalyzer(Analyzer, IndexReader,
    /// float)`.
    ///
    /// # Errors
    ///
    /// Can be thrown while reading from the IndexReader
    pub fn with_max_percent_docs(
        delegate: Arc<dyn Analyzer>,
        index_reader: &Arc<dyn IndexReader>,
        max_percent_docs: f32,
    ) -> Result<Self> {
        let fields = indexed_fields(index_reader)?;
        Self::with_fields_max_percent_docs(delegate, index_reader, &fields, max_percent_docs)
    }

    /// Creates a new QueryAutoStopWordAnalyzer with stopwords calculated for
    /// the given selection of fields from terms with a document frequency
    /// percentage greater than the given maxPercentDocs
    ///
    /// * `delegate` Analyzer whose TokenStream will be filtered
    /// * `index_reader` IndexReader to identify the stopwords from
    /// * `fields` Selection of fields to calculate stopwords for
    /// * `max_percent_docs` The maximum percentage (between 0.0 and 1.0) of
    ///   index documents which contain a term, after which the word is
    ///   considered to be a stop word
    ///
    /// Equivalent to `QueryAutoStopWordAnalyzer(Analyzer, IndexReader,
    /// Collection<String>, float)`: the threshold is `(int)
    /// (indexReader.numDocs() * maxPercentDocs)`.
    ///
    /// # Errors
    ///
    /// Can be thrown while reading from the IndexReader
    pub fn with_fields_max_percent_docs(
        delegate: Arc<dyn Analyzer>,
        index_reader: &Arc<dyn IndexReader>,
        fields: &[String],
        max_percent_docs: f32,
    ) -> Result<Self> {
        let max_doc_freq = (index_reader.num_docs() as f32 * max_percent_docs) as i32;
        Self::with_fields_max_doc_freq(delegate, index_reader, fields, max_doc_freq)
    }

    /// Creates a new QueryAutoStopWordAnalyzer with stopwords calculated for
    /// the given selection of fields from terms with a document frequency
    /// greater than the given maxDocFreq
    ///
    /// * `delegate` Analyzer whose TokenStream will be filtered
    /// * `index_reader` IndexReader to identify the stopwords from
    /// * `fields` Selection of fields to calculate stopwords for
    /// * `max_doc_freq` Document frequency terms should be above in order to
    ///   be stopwords
    ///
    /// Equivalent to `QueryAutoStopWordAnalyzer(Analyzer, IndexReader,
    /// Collection<String>, int)`, whose `super(delegate.getReuseStrategy())`
    /// shares the delegate's strategy.
    ///
    /// # Errors
    ///
    /// Can be thrown while reading from the IndexReader
    pub fn with_fields_max_doc_freq(
        delegate: Arc<dyn Analyzer>,
        index_reader: &Arc<dyn IndexReader>,
        fields: &[String],
        max_doc_freq: i32,
    ) -> Result<Self> {
        let mut stop_words_per_field = JavaHashMap::new();
        for field in fields {
            let mut stop_words = JavaHashSet::new();
            let terms = MultiTerms::get_terms(index_reader, field)?;
            if let Some(terms) = terms {
                let mut te = terms.iterator()?;
                while let Some(text) = te.next()? {
                    if te.doc_freq()? > max_doc_freq {
                        // `spare.copyUTF8Bytes(text); spare.toString()`
                        stop_words.insert(text.utf8_to_string()?);
                    }
                }
            }
            stop_words_per_field.insert(field.clone(), stop_words);
        }
        let stop_words_per_field = Arc::new(stop_words_per_field);
        let inner = AnalyzerWrapper::new(
            Box::new(QueryAutoStopWordAnalyzerLogic {
                delegate: Arc::clone(&delegate),
                stop_words_per_field: Arc::clone(&stop_words_per_field),
            }),
            Box::new(DelegateReuseStrategy(delegate)),
        );
        Ok(Self {
            inner,
            stop_words_per_field,
        })
    }

    /// Provides information on which stop words have been identified for a
    /// field
    ///
    /// * `field_name` The field for which stop words identified in
    ///   "addStopWords" method calls will be returned
    ///
    /// Returns the stop words identified for a field.
    ///
    /// Equivalent to `QueryAutoStopWordAnalyzer.getStopWords(String)`.
    pub fn get_stop_words(&self, field_name: &str) -> Vec<String> {
        match self.stop_words_per_field.get(field_name) {
            Some(stop_words) => stop_words.iter().cloned().collect(),
            None => Vec::new(),
        }
    }

    /// Provides information on which stop words have been identified for all
    /// fields
    ///
    /// Returns the stop words (as terms).
    ///
    /// Equivalent to `QueryAutoStopWordAnalyzer.getStopWords()`.
    pub fn get_all_stop_words(&self) -> Vec<Term> {
        let mut all_stop_words = Vec::new();
        for (field, stop_words) in self.stop_words_per_field.iter() {
            for text in stop_words.iter() {
                all_stop_words.push(Term::from_text(field.clone(), text));
            }
        }
        all_stop_words
    }
}

impl Analyzer for QueryAutoStopWordAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        self.inner.analyzer_state()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.query.QueryAutoStopWordAnalyzer"
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        self.inner.create_components(field_name)
    }

    fn normalize_token_stream(
        &self,
        field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        self.inner.normalize_token_stream(field_name, input)
    }

    fn init_reader(&self, field_name: &str, reader: Box<dyn CharReader>) -> Box<dyn CharReader> {
        self.inner.init_reader(field_name, reader)
    }

    fn init_reader_for_normalization(
        &self,
        field_name: &str,
        reader: Box<dyn CharReader>,
    ) -> Box<dyn CharReader> {
        self.inner.init_reader_for_normalization(field_name, reader)
    }

    fn attribute_factory(&self, field_name: &str) -> Arc<dyn AttributeFactory> {
        self.inner.attribute_factory(field_name)
    }

    fn get_position_increment_gap(&self, field_name: &str) -> i32 {
        self.inner.get_position_increment_gap(field_name)
    }

    fn get_offset_gap(&self, field_name: &str) -> i32 {
        self.inner.get_offset_gap(field_name)
    }
}
