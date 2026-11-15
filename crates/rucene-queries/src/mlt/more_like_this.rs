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

//! Port of `MoreLikeThis.java`, of the Java package
//! `org.apache.lucene.queries.mlt`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rucene::analysis::tokenattributes::{CharTermAttribute, TermFrequencyAttribute};
use rucene::analysis::{Analyzer, CharReader, StringCharReader};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::{FieldInfos, IndexReader, Term, Terms};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::query::Query;
use rucene::search::similarities::tfidf_similarity::{ClassicSimilarity, TFIDFSimilarity};
use rucene::search::term_query::TermQuery;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};
use rucene::util::java_collections::JavaHashMap;
use rucene::util::refs::CharsRefBuilder;

/// Generates "more like this" similarity queries.
///
/// Equivalent to the final `org.apache.lucene.queries.mlt.MoreLikeThis`.
///
/// Based on a document (by doc number, by field values, or by text read
/// through readers), it extracts the "interesting" terms — those that occur
/// often in the document but rarely in the index, scored by tf × idf with the
/// [`TFIDFSimilarity`] of this instance — and builds a disjunction of term
/// queries over the best of them.
///
/// **Divergences from Lucene 10.5.0.**
///
/// * Java's stop words are a `Set<?>` probed with `contains(String)`
///   (`MoreLikeThis.java:852`); this port takes a `HashSet<String>`, the set
///   such a probe can answer `true` for. A `CharArraySet` probed by a `String`
///   matches the same terms when it is case-sensitive.
/// * Java's `like(Map<String, Collection<Object>>)` renders each non-`null`
///   value with `String.valueOf` (`MoreLikeThis.java:753`); the values are
///   passed here already rendered, `None` standing for `null`.
pub struct MoreLikeThis {
    stop_words: Option<Arc<HashSet<String>>>,
    analyzer: Option<Arc<dyn Analyzer>>,
    min_term_freq: i32,
    min_doc_freq: i32,
    max_doc_freq: i32,
    boost: bool,
    field_names: Option<Vec<String>>,
    max_num_tokens_parsed: i32,
    min_word_len: i32,
    max_word_len: i32,
    max_query_terms: i32,
    similarity: Arc<dyn TFIDFSimilarity>,
    ir: Arc<dyn IndexReader>,
    boost_factor: f32,
}

impl std::fmt::Debug for MoreLikeThis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MoreLikeThis")
            .field("field_names", &self.field_names)
            .field("min_term_freq", &self.min_term_freq)
            .field("min_doc_freq", &self.min_doc_freq)
            .field("max_doc_freq", &self.max_doc_freq)
            .field("max_query_terms", &self.max_query_terms)
            .field("boost", &self.boost)
            .field("boost_factor", &self.boost_factor)
            .finish_non_exhaustive()
    }
}

/// `try (TokenStream ts = ...) { ... }`: the body's result, with a failure to
/// close suppressed into a body failure.
fn close_after<T>(result: Result<T>, closed: Result<()>) -> Result<T> {
    match (result, closed) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(close_error)) => Err(close_error),
        (Err(e), Ok(())) => Err(e),
        (Err(mut e), Err(close_error)) => {
            e.add_suppressed(close_error);
            Err(e)
        }
    }
}

impl MoreLikeThis {
    /// Default maximum number of tokens to parse in each example doc field
    /// that is not stored with term vector support.
    ///
    /// Equivalent to `MoreLikeThis.DEFAULT_MAX_NUM_TOKENS_PARSED`.
    pub const DEFAULT_MAX_NUM_TOKENS_PARSED: i32 = 5000;

    /// Ignore terms with less than this frequency in the source doc.
    ///
    /// Equivalent to `MoreLikeThis.DEFAULT_MIN_TERM_FREQ`.
    pub const DEFAULT_MIN_TERM_FREQ: i32 = 2;

    /// Ignore words which do not occur in at least this many docs.
    ///
    /// Equivalent to `MoreLikeThis.DEFAULT_MIN_DOC_FREQ`.
    pub const DEFAULT_MIN_DOC_FREQ: i32 = 5;

    /// Ignore words which occur in more than this many docs.
    ///
    /// Equivalent to `MoreLikeThis.DEFAULT_MAX_DOC_FREQ`.
    pub const DEFAULT_MAX_DOC_FREQ: i32 = i32::MAX;

    /// Boost terms in query based on score.
    ///
    /// Equivalent to `MoreLikeThis.DEFAULT_BOOST`.
    pub const DEFAULT_BOOST: bool = false;

    /// Default field names. Null is used to specify that the field names
    /// should be looked up at runtime from the provided reader.
    ///
    /// Equivalent to `MoreLikeThis.DEFAULT_FIELD_NAMES`.
    pub const DEFAULT_FIELD_NAMES: &'static [&'static str] = &["contents"];

    /// Ignore words less than this length or if 0 then this has no effect.
    ///
    /// Equivalent to `MoreLikeThis.DEFAULT_MIN_WORD_LENGTH`.
    pub const DEFAULT_MIN_WORD_LENGTH: i32 = 0;

    /// Ignore words greater than this length or if 0 then this has no effect.
    ///
    /// Equivalent to `MoreLikeThis.DEFAULT_MAX_WORD_LENGTH`.
    pub const DEFAULT_MAX_WORD_LENGTH: i32 = 0;

    /// Default set of stopwords. If null means to allow stop words.
    ///
    /// Equivalent to `MoreLikeThis.DEFAULT_STOP_WORDS`, which is `null`.
    pub const DEFAULT_STOP_WORDS: Option<Arc<HashSet<String>>> = None;

    /// Return a query with no more than this many terms.
    ///
    /// Equivalent to `MoreLikeThis.DEFAULT_MAX_QUERY_TERMS`.
    pub const DEFAULT_MAX_QUERY_TERMS: i32 = 25;

    /// Constructor requiring an index reader, with a [`ClassicSimilarity`].
    ///
    /// Equivalent to `MoreLikeThis(IndexReader)`.
    pub fn new(ir: Arc<dyn IndexReader>) -> Self {
        Self::with_similarity(ir, Arc::new(ClassicSimilarity::new()))
    }

    /// Equivalent to `MoreLikeThis(IndexReader, TFIDFSimilarity)`.
    pub fn with_similarity(ir: Arc<dyn IndexReader>, sim: Arc<dyn TFIDFSimilarity>) -> Self {
        Self {
            stop_words: Self::DEFAULT_STOP_WORDS,
            analyzer: None,
            min_term_freq: Self::DEFAULT_MIN_TERM_FREQ,
            min_doc_freq: Self::DEFAULT_MIN_DOC_FREQ,
            max_doc_freq: Self::DEFAULT_MAX_DOC_FREQ,
            boost: Self::DEFAULT_BOOST,
            field_names: Some(
                Self::DEFAULT_FIELD_NAMES
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ),
            max_num_tokens_parsed: Self::DEFAULT_MAX_NUM_TOKENS_PARSED,
            min_word_len: Self::DEFAULT_MIN_WORD_LENGTH,
            max_word_len: Self::DEFAULT_MAX_WORD_LENGTH,
            max_query_terms: Self::DEFAULT_MAX_QUERY_TERMS,
            similarity: sim,
            ir,
            boost_factor: 1.0,
        }
    }

    /// Returns the boost factor used when boosting terms.
    ///
    /// Equivalent to `MoreLikeThis.getBoostFactor()`.
    pub fn get_boost_factor(&self) -> f32 {
        self.boost_factor
    }

    /// Sets the boost factor to use when boosting terms.
    ///
    /// Equivalent to `MoreLikeThis.setBoostFactor(float)`.
    pub fn set_boost_factor(&mut self, boost_factor: f32) {
        self.boost_factor = boost_factor;
    }

    /// Equivalent to `MoreLikeThis.getSimilarity()`.
    pub fn get_similarity(&self) -> &Arc<dyn TFIDFSimilarity> {
        &self.similarity
    }

    /// Equivalent to `MoreLikeThis.setSimilarity(TFIDFSimilarity)`.
    pub fn set_similarity(&mut self, similarity: Arc<dyn TFIDFSimilarity>) {
        self.similarity = similarity;
    }

    /// Returns the analyzer that will be used to parse source doc with.
    ///
    /// Equivalent to `MoreLikeThis.getAnalyzer()`; `None` where Java returns
    /// `null`, the default.
    pub fn get_analyzer(&self) -> Option<&Arc<dyn Analyzer>> {
        self.analyzer.as_ref()
    }

    /// Sets the analyzer to use. An analyzer is not required for generating a
    /// query with the [`like`](Self::like) method; all other `like` methods
    /// require an analyzer.
    ///
    /// Equivalent to `MoreLikeThis.setAnalyzer(Analyzer)`.
    pub fn set_analyzer(&mut self, analyzer: Option<Arc<dyn Analyzer>>) {
        self.analyzer = analyzer;
    }

    /// Returns the frequency below which terms will be ignored in the source
    /// doc. The default frequency is [`DEFAULT_MIN_TERM_FREQ`](Self::DEFAULT_MIN_TERM_FREQ).
    ///
    /// Equivalent to `MoreLikeThis.getMinTermFreq()`.
    pub fn get_min_term_freq(&self) -> i32 {
        self.min_term_freq
    }

    /// Sets the frequency below which terms will be ignored in the source doc.
    ///
    /// Equivalent to `MoreLikeThis.setMinTermFreq(int)`.
    pub fn set_min_term_freq(&mut self, min_term_freq: i32) {
        self.min_term_freq = min_term_freq;
    }

    /// Returns the frequency at which words will be ignored which do not occur
    /// in at least this many docs. The default frequency is
    /// [`DEFAULT_MIN_DOC_FREQ`](Self::DEFAULT_MIN_DOC_FREQ).
    ///
    /// Equivalent to `MoreLikeThis.getMinDocFreq()`.
    pub fn get_min_doc_freq(&self) -> i32 {
        self.min_doc_freq
    }

    /// Sets the frequency at which words will be ignored which do not occur in
    /// at least this many docs.
    ///
    /// Equivalent to `MoreLikeThis.setMinDocFreq(int)`.
    pub fn set_min_doc_freq(&mut self, min_doc_freq: i32) {
        self.min_doc_freq = min_doc_freq;
    }

    /// Returns the maximum frequency in which words may still appear. Words
    /// that appear in more than this many docs will be ignored. The default
    /// frequency is [`DEFAULT_MAX_DOC_FREQ`](Self::DEFAULT_MAX_DOC_FREQ).
    ///
    /// Equivalent to `MoreLikeThis.getMaxDocFreq()`.
    pub fn get_max_doc_freq(&self) -> i32 {
        self.max_doc_freq
    }

    /// Sets the maximum frequency in which words may still appear.
    ///
    /// Equivalent to `MoreLikeThis.setMaxDocFreq(int)`.
    pub fn set_max_doc_freq(&mut self, max_freq: i32) {
        self.max_doc_freq = max_freq;
    }

    /// Sets the maximum percentage in which words may still appear. Words that
    /// appear in more than this many percent of all docs will be ignored.
    ///
    /// Equivalent to `MoreLikeThis.setMaxDocFreqPct(int)`.
    ///
    /// # Errors
    ///
    /// Returns the `ArithmeticException` of `Math.toIntExact` when the
    /// resulting count does not fit an `int`.
    pub fn set_max_doc_freq_pct(&mut self, max_percentage: i32) -> Result<()> {
        let value = i64::from(max_percentage).wrapping_mul(i64::from(self.ir.max_doc())) / 100;
        let value = i32::try_from(value).map_err(|_| {
            LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.ArithmeticException",
                message: Some("integer overflow".to_string()),
                cause: None,
            })
        })?;
        self.set_max_doc_freq(value);
        Ok(())
    }

    /// Returns whether to boost terms in query based on "score" or not. The
    /// default is [`DEFAULT_BOOST`](Self::DEFAULT_BOOST).
    ///
    /// Equivalent to `MoreLikeThis.isBoost()`.
    pub fn is_boost(&self) -> bool {
        self.boost
    }

    /// Sets whether to boost terms in query based on "score" or not.
    ///
    /// Equivalent to `MoreLikeThis.setBoost(boolean)`.
    pub fn set_boost(&mut self, boost: bool) {
        self.boost = boost;
    }

    /// Returns the field names that will be used when generating the 'More
    /// Like This' query. The default field names are
    /// [`DEFAULT_FIELD_NAMES`](Self::DEFAULT_FIELD_NAMES).
    ///
    /// Equivalent to `MoreLikeThis.getFieldNames()`; `None` where Java returns
    /// `null`.
    pub fn get_field_names(&self) -> Option<&[String]> {
        self.field_names.as_deref()
    }

    /// Sets the field names that will be used when generating the 'More Like
    /// This' query. Set this to `None` for the field names to be determined at
    /// runtime from the index reader provided in the constructor.
    ///
    /// Equivalent to `MoreLikeThis.setFieldNames(String[])`.
    pub fn set_field_names(&mut self, field_names: Option<Vec<String>>) {
        self.field_names = field_names;
    }

    /// Returns the minimum word length below which words will be ignored. Set
    /// this to 0 for no minimum word length.
    ///
    /// Equivalent to `MoreLikeThis.getMinWordLen()`.
    pub fn get_min_word_len(&self) -> i32 {
        self.min_word_len
    }

    /// Sets the minimum word length below which words will be ignored.
    ///
    /// Equivalent to `MoreLikeThis.setMinWordLen(int)`.
    pub fn set_min_word_len(&mut self, min_word_len: i32) {
        self.min_word_len = min_word_len;
    }

    /// Returns the maximum word length above which words will be ignored. Set
    /// this to 0 for no maximum word length.
    ///
    /// Equivalent to `MoreLikeThis.getMaxWordLen()`.
    pub fn get_max_word_len(&self) -> i32 {
        self.max_word_len
    }

    /// Sets the maximum word length above which words will be ignored.
    ///
    /// Equivalent to `MoreLikeThis.setMaxWordLen(int)`.
    pub fn set_max_word_len(&mut self, max_word_len: i32) {
        self.max_word_len = max_word_len;
    }

    /// Sets the set of stopwords. Any word in this set is considered
    /// "uninteresting" and ignored.
    ///
    /// Equivalent to `MoreLikeThis.setStopWords(Set<?>)`; see the type
    /// documentation.
    pub fn set_stop_words(&mut self, stop_words: Option<Arc<HashSet<String>>>) {
        self.stop_words = stop_words;
    }

    /// Gets the current stop words being used.
    ///
    /// Equivalent to `MoreLikeThis.getStopWords()`.
    pub fn get_stop_words(&self) -> Option<&Arc<HashSet<String>>> {
        self.stop_words.as_ref()
    }

    /// Returns the maximum number of query terms that will be included in any
    /// generated query. The default is
    /// [`DEFAULT_MAX_QUERY_TERMS`](Self::DEFAULT_MAX_QUERY_TERMS).
    ///
    /// Equivalent to `MoreLikeThis.getMaxQueryTerms()`.
    pub fn get_max_query_terms(&self) -> i32 {
        self.max_query_terms
    }

    /// Sets the maximum number of query terms that will be included in any
    /// generated query.
    ///
    /// Equivalent to `MoreLikeThis.setMaxQueryTerms(int)`.
    pub fn set_max_query_terms(&mut self, max_query_terms: i32) {
        self.max_query_terms = max_query_terms;
    }

    /// Returns the maximum number of tokens to parse in each example doc field
    /// that is not stored with term vector support.
    ///
    /// Equivalent to `MoreLikeThis.getMaxNumTokensParsed()`.
    pub fn get_max_num_tokens_parsed(&self) -> i32 {
        self.max_num_tokens_parsed
    }

    /// Sets the maximum number of tokens to parse in each example doc field
    /// that is not stored with term vector support.
    ///
    /// Equivalent to `MoreLikeThis.setMaxNumTokensParsed(int)`.
    pub fn set_max_num_tokens_parsed(&mut self, i: i32) {
        self.max_num_tokens_parsed = i;
    }

    /// `FieldInfos.getIndexedFields(ir).toArray(String[]::new)`
    /// (`MoreLikeThis.java:557-558, 571-572`): the names in the iteration
    /// order of the `java.util.HashSet` the core returns.
    fn indexed_fields(&self) -> Result<Vec<String>> {
        Ok(FieldInfos::get_indexed_fields(Arc::clone(&self.ir))?
            .iter()
            .cloned()
            .collect())
    }

    /// Returns a query that will return docs like the passed lucene document
    /// ID.
    ///
    /// Equivalent to `MoreLikeThis.like(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the document.
    pub fn like(&mut self, doc_num: i32) -> Result<Arc<dyn Query>> {
        if self.field_names.is_none() {
            // gather list of valid fields from lucene
            self.field_names = Some(self.indexed_fields()?);
        }

        let q = self.retrieve_terms(doc_num)?;
        Ok(Arc::new(self.create_query(q)?))
    }

    /// Returns a query that will return docs like the passed field values.
    ///
    /// Equivalent to `MoreLikeThis.like(Map<String, Collection<Object>>)`; see
    /// the type documentation for the values.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while analyzing the values.
    pub fn like_document(
        &mut self,
        filtered_document: &HashMap<String, Vec<Option<String>>>,
    ) -> Result<Arc<dyn Query>> {
        if self.field_names.is_none() {
            // gather list of valid fields from lucene
            self.field_names = Some(self.indexed_fields()?);
        }
        let q = self.retrieve_terms_from_values(filtered_document)?;
        Ok(Arc::new(self.create_query(q)?))
    }

    /// Returns a query that will return docs like the passed readers. This
    /// method only works if [`set_analyzer`](Self::set_analyzer) has been
    /// called.
    ///
    /// Equivalent to `MoreLikeThis.like(String, Reader...)`.
    ///
    /// # Errors
    ///
    /// Returns the `UnsupportedOperationException` Java raises without an
    /// analyzer, and propagates any error raised while analyzing.
    pub fn like_readers(
        &self,
        field_name: &str,
        readers: Vec<Box<dyn CharReader>>,
    ) -> Result<Arc<dyn Query>> {
        let mut per_field_term_frequencies: JavaHashMap<JavaHashMap<Int>> = JavaHashMap::new();
        for r in readers {
            self.add_term_frequencies_from_reader(r, &mut per_field_term_frequencies, field_name)?;
        }
        let q = self.create_queue(&per_field_term_frequencies)?;
        Ok(Arc::new(self.create_query(q)?))
    }

    /// Creates the "More like this" query from a priority queue.
    ///
    /// Equivalent to the private `MoreLikeThis.createQuery(PriorityQueue<ScoreTerm>)`.
    fn create_query(&self, mut q: FreqQ) -> Result<rucene::search::boolean_query::BooleanQuery> {
        let mut query = BooleanQueryBuilder::new();
        let mut best_score = -1f32;

        while let Some(score_term) = q.pop() {
            let mut tq: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text(
                &score_term.top_field,
                &score_term.word,
            )));

            if self.boost {
                if best_score == -1.0 {
                    best_score = score_term.score;
                }
                let my_score = score_term.score;
                tq = Arc::new(BoostQuery::new(
                    tq,
                    self.boost_factor * my_score / best_score,
                )?);
            }

            match query.add(tq, Occur::SHOULD) {
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        LuceneErrorKind::TooManyClauses(_)
                            | LuceneErrorKind::TooManyNestedClauses(_)
                    ) =>
                {
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(query.build())
    }

    /// Creates a priority queue from a map of field names to word frequencies.
    ///
    /// Equivalent to the private
    /// `MoreLikeThis.createQueue(Map<String, Map<String, Int>>)`; the maps are
    /// `java.util.HashMap`s, whose iteration order decides ties.
    fn create_queue(
        &self,
        per_field_term_frequencies: &JavaHashMap<JavaHashMap<Int>>,
    ) -> Result<FreqQ> {
        // have collected all words in doc and their freqs
        let limit = self
            .max_query_terms
            .min(Self::get_terms_count(per_field_term_frequencies));
        let mut queue = FreqQ::new(limit)?; // will order words by score
        for (field_name, per_word_term_frequencies) in per_field_term_frequencies.iter() {
            let mut num_docs = i64::from(self.ir.get_doc_count(field_name)?);
            if num_docs == -1 {
                num_docs = i64::from(self.ir.num_docs());
            }

            for (word, tf_entry) in per_word_term_frequencies.iter() {
                // for every word
                let tf = tf_entry.x; // term freq in the source doc
                if self.min_term_freq > 0 && tf < self.min_term_freq {
                    continue; // filter out words that don't occur enough times in the source
                }

                let doc_freq = self.ir.doc_freq(&Term::from_text(field_name, word))?;

                if self.min_doc_freq > 0 && doc_freq < self.min_doc_freq {
                    continue; // filter out words that don't occur in enough docs
                }

                if doc_freq > self.max_doc_freq {
                    continue; // filter out words that occur in too many docs
                }

                if doc_freq == 0 {
                    continue; // index update problem?
                }

                let idf = self.similarity.idf(i64::from(doc_freq), num_docs);
                let score = tf as f32 * idf;

                if (queue.size() as i32) < limit {
                    // there is still space in the queue
                    queue.add(ScoreTerm::new(word.clone(), field_name.clone(), score));
                } else {
                    // `queue.top()` on an empty queue (a limit of 0) is `null`.
                    let term = queue.top_mut().ok_or_else(LuceneError::null_pointer)?;
                    // update the smallest in the queue in place and update the queue.
                    if term.score < score {
                        term.update(word.clone(), field_name.clone(), score);
                        queue.update_top();
                    }
                }
            }
        }
        Ok(queue)
    }

    /// Equivalent to the private
    /// `MoreLikeThis.getTermsCount(Map<String, Map<String, Int>>)`.
    fn get_terms_count(per_field_term_frequencies: &JavaHashMap<JavaHashMap<Int>>) -> i32 {
        let mut total_terms_count = 0i32;
        for (_, per_word_term_frequencies) in per_field_term_frequencies.iter() {
            total_terms_count =
                total_terms_count.wrapping_add(per_word_term_frequencies.len() as i32);
        }
        total_terms_count
    }

    /// Describes the parameters that control how the "more like this" query
    /// is formed.
    ///
    /// Equivalent to `MoreLikeThis.describeParams()`.
    pub fn describe_params(&self) -> String {
        let mut sb = String::new();
        sb.push_str(&format!("\tmaxQueryTerms  : {}\n", self.max_query_terms));
        sb.push_str(&format!("\tminWordLen     : {}\n", self.min_word_len));
        sb.push_str(&format!("\tmaxWordLen     : {}\n", self.max_word_len));
        sb.push_str("\tfieldNames     : ");
        let mut delim = "";
        if let Some(field_names) = self.field_names.as_ref() {
            for field_name in field_names {
                sb.push_str(delim);
                sb.push_str(field_name);
                delim = ", ";
            }
        } else {
            // `for (String fieldName : fieldNames)` on a `null` array.
            rucene::store::java_nio_file::throw_unchecked("java.lang.NullPointerException", None);
        }
        sb.push('\n');
        sb.push_str(&format!("\tboost          : {}\n", self.boost));
        sb.push_str(&format!("\tminTermFreq    : {}\n", self.min_term_freq));
        sb.push_str(&format!("\tminDocFreq     : {}\n", self.min_doc_freq));
        sb
    }

    /// The field names, for the methods that iterate them; Java iterates a
    /// `null` array with a `NullPointerException`.
    fn field_names_or_npe(&self) -> Result<&[String]> {
        self.field_names
            .as_deref()
            .ok_or_else(LuceneError::null_pointer)
    }

    /// Finds words for a "more like this" query former.
    ///
    /// Equivalent to the private `MoreLikeThis.retrieveTerms(int)`.
    fn retrieve_terms(&self, doc_num: i32) -> Result<FreqQ> {
        let mut field2term_freq_map: JavaHashMap<JavaHashMap<Int>> = JavaHashMap::new();
        let term_vectors = self.ir.term_vectors()?;
        for field_name in self.field_names_or_npe()? {
            let vectors = term_vectors.get(doc_num)?;
            let vector: Option<Box<dyn Terms>> = match vectors {
                Some(vectors) => vectors.terms(field_name)?,
                None => None,
            };

            match vector {
                // field does not store term vector info
                None => {
                    let d = self.ir.stored_fields()?.document(doc_num)?;
                    let fields = d.get_fields_by_name(field_name);
                    for field in fields {
                        if let Some(string_value) = field.string_value() {
                            self.add_term_frequencies_from_reader(
                                Box::new(StringCharReader::new(string_value)),
                                &mut field2term_freq_map,
                                field_name,
                            )?;
                        }
                    }
                }
                Some(vector) => {
                    self.add_term_frequencies_from_vector(
                        &mut field2term_freq_map,
                        &*vector,
                        field_name,
                    )?;
                }
            }
        }

        self.create_queue(&field2term_freq_map)
    }

    /// Equivalent to the private
    /// `MoreLikeThis.retrieveTerms(Map<String, Collection<Object>>)`.
    fn retrieve_terms_from_values(
        &self,
        field2field_values: &HashMap<String, Vec<Option<String>>>,
    ) -> Result<FreqQ> {
        let mut field2term_freq_map: JavaHashMap<JavaHashMap<Int>> = JavaHashMap::new();
        for field_name in self.field_names_or_npe()? {
            let Some(field_values) = field2field_values.get(field_name) else {
                continue;
            };
            for field_value in field_values.iter().flatten() {
                self.add_term_frequencies_from_reader(
                    Box::new(StringCharReader::new(field_value.clone())),
                    &mut field2term_freq_map,
                    field_name,
                )?;
            }
        }
        self.create_queue(&field2term_freq_map)
    }

    /// Adds terms and frequencies found in a term vector into the map.
    ///
    /// Equivalent to the private
    /// `MoreLikeThis.addTermFrequencies(Map<String, Map<String, Int>>, Terms, String)`.
    fn add_term_frequencies_from_vector(
        &self,
        field2term_freq_map: &mut JavaHashMap<JavaHashMap<Int>>,
        vector: &dyn Terms,
        field_name: &str,
    ) -> Result<()> {
        let term_freq_map = field2term_freq_map.compute_if_absent(field_name, JavaHashMap::new);
        let mut terms_enum = vector.iterator()?;
        let mut spare = CharsRefBuilder::new();
        while let Some(text) = terms_enum.next()? {
            spare.copy_utf8_bytes_ref(&text);
            let term = spare.to_string();
            if self.is_noise_word(&term) {
                continue;
            }
            let freq = terms_enum.total_term_freq()? as i32;

            // increment frequency
            match term_freq_map.get_mut(&term) {
                None => {
                    let mut cnt = Int::new();
                    cnt.x = freq;
                    term_freq_map.insert(term, cnt);
                }
                Some(cnt) => {
                    cnt.x = cnt.x.wrapping_add(freq);
                }
            }
        }
        Ok(())
    }

    /// Adds term frequencies found by tokenizing text from a reader into the
    /// map.
    ///
    /// Equivalent to the private
    /// `MoreLikeThis.addTermFrequencies(Reader, Map<String, Map<String, Int>>, String)`.
    fn add_term_frequencies_from_reader(
        &self,
        r: Box<dyn CharReader>,
        per_field_term_frequencies: &mut JavaHashMap<JavaHashMap<Int>>,
        field_name: &str,
    ) -> Result<()> {
        let Some(analyzer) = self.analyzer.as_ref() else {
            return Err(LuceneError::unsupported_operation(
                "To use MoreLikeThis without term vectors, you must provide an Analyzer"
                    .to_string(),
            ));
        };
        let term_freq_map =
            per_field_term_frequencies.compute_if_absent(field_name, JavaHashMap::new);
        let ts = analyzer.token_stream(field_name, r)?;
        let mut ts = ts.borrow_mut();
        let result = (|| -> Result<()> {
            let mut token_count = 0i32;
            // for every token
            ts.attribute_source_mut()
                .add_attribute_interface::<dyn CharTermAttribute>()?;
            ts.attribute_source_mut()
                .add_attribute_interface::<dyn TermFrequencyAttribute>()?;
            ts.reset()?;
            while ts.increment_token()? {
                let word: String = {
                    let term_att = ts
                        .attribute_source()
                        .get_attribute_interface::<dyn CharTermAttribute>()
                        .expect("INVARIANT: CharTermAttribute was added above");
                    term_att.buffer()[..term_att.length()].iter().collect()
                };
                token_count = token_count.wrapping_add(1);
                if token_count > self.max_num_tokens_parsed {
                    break;
                }
                if self.is_noise_word(&word) {
                    continue;
                }
                let term_frequency = ts
                    .attribute_source()
                    .get_attribute_interface::<dyn TermFrequencyAttribute>()
                    .expect("INVARIANT: TermFrequencyAttribute was added above")
                    .get_term_frequency();

                // increment frequency
                match term_freq_map.get_mut(&word) {
                    None => {
                        term_freq_map.insert(word, Int::with_initial_value(term_frequency));
                    }
                    Some(cnt) => {
                        cnt.x = cnt.x.wrapping_add(term_frequency);
                    }
                }
            }
            ts.end()?;
            Ok(())
        })();
        let closed = ts.close();
        close_after(result, closed)
    }

    /// Determines if the passed term is likely to be of interest in "more
    /// like" comparisons.
    ///
    /// Equivalent to the private `MoreLikeThis.isNoiseWord(String)`, whose
    /// `term.length()` counts UTF-16 code units.
    fn is_noise_word(&self, term: &str) -> bool {
        let len = term.encode_utf16().count() as i32;
        if self.min_word_len > 0 && len < self.min_word_len {
            return true;
        }
        if self.max_word_len > 0 && len > self.max_word_len {
            return true;
        }
        self.stop_words
            .as_ref()
            .is_some_and(|stop_words| stop_words.contains(term))
    }

    /// Finds words for a "more like this" query former.
    ///
    /// Equivalent to the private `MoreLikeThis.retrieveTerms(Reader, String)`.
    fn retrieve_terms_from_reader(
        &self,
        r: Box<dyn CharReader>,
        field_name: &str,
    ) -> Result<FreqQ> {
        let mut field2term_freq_map: JavaHashMap<JavaHashMap<Int>> = JavaHashMap::new();
        self.add_term_frequencies_from_reader(r, &mut field2term_freq_map, field_name)?;
        self.create_queue(&field2term_freq_map)
    }

    /// Returns the interesting terms of a document, best (that is, last
    /// popped) last.
    ///
    /// Equivalent to `MoreLikeThis.retrieveInterestingTerms(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the document.
    pub fn retrieve_interesting_terms(&self, doc_num: i32) -> Result<Vec<String>> {
        let pq = self.retrieve_terms(doc_num)?;
        Ok(self.interesting_terms(pq))
    }

    /// Convenience routine to make it easy to return the most interesting
    /// words in a document. More advanced users will call
    /// [`like_readers`](Self::like_readers) directly.
    ///
    /// Equivalent to `MoreLikeThis.retrieveInterestingTerms(Reader, String)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while analyzing.
    pub fn retrieve_interesting_terms_from_reader(
        &self,
        r: Box<dyn CharReader>,
        field_name: &str,
    ) -> Result<Vec<String>> {
        let pq = self.retrieve_terms_from_reader(r, field_name)?;
        Ok(self.interesting_terms(pq))
    }

    /// The loop both `retrieveInterestingTerms` overloads run:
    /// `while ((scoreTerm = pq.pop()) != null && lim-- > 0) al.add(scoreTerm.word)`.
    fn interesting_terms(&self, mut pq: FreqQ) -> Vec<String> {
        let mut al: Vec<String> = Vec::new();
        // have to be careful, retrieveTerms returns all words but that's probably not useful to our
        // caller...
        let mut lim = self.max_query_terms;
        // we just want to return the top words
        while let Some(score_term) = pq.pop() {
            let keep = lim > 0;
            lim = lim.wrapping_sub(1);
            if !keep {
                break;
            }
            al.push(score_term.word); // the 1st entry is the interesting word
        }
        al
    }
}

/// The ordering of [`FreqQ`]: its `lessThan`, by score.
#[derive(Debug, Clone, Copy)]
struct FreqQOrder;

impl PriorityQueueComparator<ScoreTerm> for FreqQOrder {
    fn less_than(&self, a: &ScoreTerm, b: &ScoreTerm) -> bool {
        a.score < b.score
    }
}

/// Priority queue for score terms.
///
/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.mlt.MoreLikeThis.FreqQ`, which extends
/// `PriorityQueue<ScoreTerm>`.
pub(crate) struct FreqQ {
    queue: PriorityQueue<ScoreTerm, FreqQOrder>,
}

impl FreqQ {
    /// Equivalent to `FreqQ(int)`, which is `super(maxSize)`.
    ///
    /// # Errors
    ///
    /// Returns the `IllegalArgumentException` of `PriorityQueue(int)` for a
    /// negative or oversized `max_size`.
    fn new(max_size: i32) -> Result<Self> {
        if max_size < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "maxSize must be >= 0 and < {}; got: {max_size}",
                rucene::util::ArrayUtil::MAX_ARRAY_LENGTH
            )));
        }
        Ok(Self {
            queue: PriorityQueue::new(max_size as usize, FreqQOrder)?,
        })
    }

    fn size(&self) -> usize {
        self.queue.size()
    }

    fn add(&mut self, element: ScoreTerm) {
        self.queue.add(element);
    }

    fn top_mut(&mut self) -> Option<&mut ScoreTerm> {
        self.queue.top_mut()
    }

    fn update_top(&mut self) {
        self.queue.update_top();
    }

    fn pop(&mut self) -> Option<ScoreTerm> {
        self.queue.pop()
    }
}

/// One candidate term with its score.
///
/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.mlt.MoreLikeThis.ScoreTerm`.
#[derive(Debug, Clone)]
pub(crate) struct ScoreTerm {
    // only really need 1st 3 entries, other ones are for troubleshooting
    word: String,
    top_field: String,
    score: f32,
}

impl ScoreTerm {
    /// Equivalent to `ScoreTerm(String, String, float)`.
    fn new(word: String, top_field: String, score: f32) -> Self {
        Self {
            word,
            top_field,
            score,
        }
    }

    /// Equivalent to `ScoreTerm.update(String, String, float)`.
    fn update(&mut self, word: String, top_field: String, score: f32) {
        self.word = word;
        self.top_field = top_field;
        self.score = score;
    }
}

/// A mutable int: the frequency of one term.
///
/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.mlt.MoreLikeThis.Int`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Int {
    x: i32,
}

impl Int {
    /// Equivalent to `Int()`, which is `this(1)`.
    fn new() -> Self {
        Self::with_initial_value(1)
    }

    /// Equivalent to `Int(int)`.
    fn with_initial_value(initial_value: i32) -> Self {
        Self { x: initial_value }
    }
}
