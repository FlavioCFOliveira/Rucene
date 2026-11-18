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

//! Port of `FuzzyLikeThisQuery.java`, of the Java package
//! `org.apache.lucene.sandbox.queries`.

use std::any::Any;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use rucene::analysis::tokenattributes::CharTermAttribute;
use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::TermsEnum;
use rucene::index::{IndexReader, MultiTerms, Term, TermStates};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::fuzzy_terms_enum::FuzzyTermsEnum;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::similarities::tfidf_similarity::{ClassicSimilarity, TFIDFSimilarity};
use rucene::search::term_query::TermQuery;
use rucene::util::automaton::levenshtein_automata::MAXIMUM_SUPPORTED_DISTANCE;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};
use rucene::util::java_collections::{
    java_boolean_hash_code, java_identity_hash_code_stand_in, java_string_hash_code,
    JavaObjectHashMap,
};
use rucene::util::java_fmt::float_to_string;

/// The similarity every instance scores its variants with.
///
/// Equivalent to the private static final `FuzzyLikeThisQuery.sim`, a
/// `ClassicSimilarity`.
static SIM: LazyLock<ClassicSimilarity> = LazyLock::new(ClassicSimilarity::new);

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

/// Fuzzifies ALL terms provided as strings and then picks the best n
/// differentiating terms.
///
/// Equivalent to `org.apache.lucene.sandbox.queries.FuzzyLikeThisQuery`.
///
/// In effect this mixes the behaviour of a fuzzy query and more-like-this but
/// with special consideration of fuzzy scoring factors. This generally
/// produces good results for queries where users may provide details in a
/// number of fields and have no knowledge of boolean query syntax and also
/// want a degree of fuzzy matching and a fast query.
///
/// For each source term the fuzzy variants are held in a boolean query with no
/// coord factor (because we are not looking for matches on multiple variants
/// in any one doc). Additionally, a specialized term query is used for
/// variants and does not use that variant term's IDF because this would
/// favour rarer terms, e.g. misspellings. Instead, all variants use the same
/// IDF ranking (the one for the source query term) and this is factored into
/// the variant's boost. If the source query term does not exist in the index
/// the average IDF of the variants is used.
///
/// **Divergences from Lucene 10.5.0.**
///
/// * `toString(String)` returns `null` in Java (`FuzzyLikeThisQuery.java:370-372`),
///   which every caller that renders a query prints as `"null"`; a Rust
///   `String` cannot be absent, so this port returns `"null"`.
/// * Java compares the analyzers with `Objects.equals`, which is identity
///   for an analyzer, and hashes them with their identity hash
///   (`FuzzyLikeThisQuery.java:82, 95`). Identity is [`Arc::ptr_eq`] here, and
///   the identity hash is
///   [`java_identity_hash_code_stand_in`] over the analyzer's Java class name,
///   which is consistent with it.
///
/// Java's `null` analyzer and `null` field name are `None`: a `null` field
/// has no terms (`MultiTerms.getTerms(reader, null)` is `null` through
/// `FieldInfos.fieldInfo(null)`, `FieldInfos.java:330-331`,
/// `CodecReader.java:119-122`), and a `null` analyzer throws Java's
/// `NullPointerException` where `addTerms` dereferences it
/// (`FuzzyLikeThisQuery.java:198`).
#[derive(Debug, Clone)]
pub struct FuzzyLikeThisQuery {
    /// The package-private `fieldVals` field.
    field_vals: Vec<FieldVals>,
    /// The package-private `analyzer` field, `None` for `null`.
    analyzer: Option<Arc<dyn Analyzer>>,
    /// The package-private `MAX_VARIANTS_PER_TERM` field.
    max_variants_per_term: i32,
    /// The package-private `ignoreTF` field.
    ignore_tf: bool,
    max_num_terms: i32,
}

impl FuzzyLikeThisQuery {
    /// Creates the query.
    ///
    /// Equivalent to `FuzzyLikeThisQuery(int, Analyzer)`.
    ///
    /// * `max_num_terms` — the total number of terms clauses that will appear
    ///   once rewritten as a boolean query;
    /// * `analyzer` — `None` for Java's `null`, which fails at rewrite.
    pub fn new(max_num_terms: i32, analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        Self {
            field_vals: Vec::new(),
            analyzer,
            max_variants_per_term: 50,
            ignore_tf: false,
            max_num_terms,
        }
    }

    /// Equivalent to the private `FuzzyLikeThisQuery.equalsTo(FuzzyLikeThisQuery)`.
    fn equals_to(&self, other: &FuzzyLikeThisQuery) -> bool {
        // `Objects.equals(analyzer, other.analyzer)`: identity, `null`-safe
        let same_analyzer = match (&self.analyzer, &other.analyzer) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        same_analyzer
            && self.field_vals == other.field_vals
            && self.ignore_tf == other.ignore_tf
            && self.max_num_terms == other.max_num_terms
    }

    /// Adds user input for "fuzzification".
    ///
    /// Equivalent to `FuzzyLikeThisQuery.addTerms(String, String, float, int)`;
    /// Java's `null` query string and field name are `None`.
    ///
    /// * `query_string` — the string which will be parsed by the analyzer and
    ///   for which fuzzy variants will be parsed;
    /// * `min_similarity` — the minimum similarity of the term variants; must
    ///   be 0, 1 or 2 (see `FuzzyTermsEnum`);
    /// * `prefix_length` — length of required common prefix on variant terms
    ///   (see `FuzzyTermsEnum`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`]
    /// when `min_similarity` is not an integer between 0 and
    /// `LevenshteinAutomata.MAXIMUM_SUPPORTED_DISTANCE`.
    pub fn add_terms(
        &mut self,
        query_string: Option<String>,
        field_name: Option<String>,
        min_similarity: f32,
        prefix_length: i32,
    ) -> Result<()> {
        let max_edits = min_similarity as i32;
        if max_edits as f32 != min_similarity
            || !(0..=MAXIMUM_SUPPORTED_DISTANCE).contains(&max_edits)
        {
            return Err(LuceneError::illegal_argument(format!(
                "minSimilarity must integer value between 0 and {MAXIMUM_SUPPORTED_DISTANCE}, \
                 inclusive; got {}",
                float_to_string(min_similarity)
            )));
        }
        self.field_vals.push(FieldVals::new(
            field_name,
            max_edits,
            prefix_length,
            query_string,
        ));
        Ok(())
    }

    /// Equivalent to the private
    /// `FuzzyLikeThisQuery.addTerms(IndexReader, FieldVals, ScoreTermQueue)`.
    fn add_terms_to_queue(
        &self,
        reader: &Arc<dyn IndexReader>,
        f: &FieldVals,
        q: &mut ScoreTermQueue,
    ) -> Result<()> {
        let Some(query_string) = f.query_string.as_deref() else {
            return Ok(());
        };
        // `MultiTerms.getTerms(reader, f.fieldName)`: a `null` field has no
        // terms (`FieldInfos.fieldInfo(null)` is a `HashMap` lookup that
        // answers `null`, so `CodecReader.terms(null)` returns `null`)
        let Some(field_name) = f.field_name.as_deref() else {
            return Ok(());
        };
        let Some(terms) = MultiTerms::get_terms(reader, field_name)? else {
            return Ok(());
        };
        // `analyzer.tokenStream(...)` dereferences a `null` analyzer; the
        // message is JDK 25's helpful one
        let analyzer = self.analyzer.as_ref().ok_or_else(|| {
            LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.NullPointerException",
                message: Some(
                    "Cannot invoke \"org.apache.lucene.analysis.Analyzer.tokenStream(String, \
                     String)\" because \"this.analyzer\" is null"
                        .to_string(),
                ),
                cause: None,
            })
        })?;
        let ts = analyzer.token_stream_from_str(field_name, query_string)?;
        let mut ts = ts.borrow_mut();
        let result = (|| -> Result<()> {
            ts.attribute_source_mut()
                .add_attribute_interface::<dyn CharTermAttribute>()?;

            let corpus_num_docs = reader.num_docs();
            let mut processed_terms: HashSet<String> = HashSet::new();
            ts.reset()?;
            while ts.increment_token()? {
                let term: String = {
                    let term_att = ts
                        .attribute_source()
                        .get_attribute_interface::<dyn CharTermAttribute>()
                        .expect("INVARIANT: CharTermAttribute was added above");
                    term_att.buffer()[..term_att.length()].iter().collect()
                };
                if !processed_terms.contains(&term) {
                    processed_terms.insert(term.clone());
                    // maxNum variants considered for any one term
                    let mut variants_q = ScoreTermQueue::new(self.max_variants_per_term)?;
                    let mut min_score = 0f32;
                    let start_term = Term::from_text(field_name.to_string(), &term);
                    let mut fe = FuzzyTermsEnum::new(
                        Arc::clone(&terms),
                        start_term.clone(),
                        f.max_edits,
                        f.prefix_length,
                        true,
                    )?;
                    // store the df so all variants use same idf
                    let mut df = reader.doc_freq(&start_term)?;
                    let mut num_variants = 0i32;
                    let mut total_variant_doc_freqs = 0i32;
                    // `fe.attributes().addAttribute(BoostAttribute.class)` is the
                    // attribute the enum installed, which `get_boost` reads.
                    while let Some(possible_match) = fe.next()?.cloned() {
                        num_variants = num_variants.wrapping_add(1);
                        total_variant_doc_freqs =
                            total_variant_doc_freqs.wrapping_add(fe.doc_freq()?);
                        let score = fe.get_boost();
                        if (variants_q.size() as i32) < self.max_variants_per_term
                            || score > min_score
                        {
                            let st = ScoreTerm::new(
                                Term::new(start_term.field().to_string(), possible_match),
                                score,
                                start_term.clone(),
                            );
                            variants_q.insert_with_overflow(st);
                            // maintain minScore
                            min_score = variants_q
                                .top()
                                .ok_or_else(LuceneError::null_pointer)?
                                .score;
                        }
                        fe.set_max_non_competitive_boost(
                            if variants_q.size() as i32 >= self.max_variants_per_term {
                                min_score
                            } else {
                                f32::NEG_INFINITY
                            },
                        );
                    }

                    if num_variants > 0 {
                        let avg_df = total_variant_doc_freqs / num_variants;
                        if df == 0 {
                            // no direct match we can use as df for all variants
                            df = avg_df; // use avg df of all variants
                        }

                        // take the top variants (scored by edit distance) and reset the score
                        // to include an IDF factor then add to the global queue for ranking
                        // overall top query terms
                        let size = variants_q.size();
                        for _ in 0..size {
                            let mut st = variants_q.pop().ok_or_else(LuceneError::null_pointer)?;
                            st.score = (st.score * st.score)
                                * SIM.idf(i64::from(df), i64::from(corpus_num_docs));
                            q.insert_with_overflow(st);
                        }
                    }
                }
            }
            ts.end()?;
            Ok(())
        })();
        let closed = ts.close();
        close_after(result, closed)
    }

    /// Equivalent to the private `FuzzyLikeThisQuery.newTermQuery(IndexReader, Term)`.
    fn new_term_query(&self, reader: &Arc<dyn IndexReader>, term: &Term) -> Result<Arc<dyn Query>> {
        if self.ignore_tf {
            Ok(Arc::new(ConstantScoreQuery::new(Arc::new(TermQuery::new(
                term.clone(),
            )))))
        } else {
            // we build an artificial TermStates that will give an overall df and ttf
            // equal to 1
            let mut context = TermStates::new(&Arc::clone(reader).get_context()?)?;
            for leaf_context in Arc::clone(reader).leaves()? {
                let terms =
                    rucene::index::terms::get_terms(&*leaf_context.leaf_reader(), term.field())?;
                let mut terms_enum = terms.iterator()?;
                if terms_enum.seek_exact(term.bytes())? {
                    let freq = 1 - context.doc_freq()?; // we want the total df and ttf to be 1
                    context.register(
                        terms_enum.term_state()?,
                        leaf_context.ord() as usize,
                        freq,
                        i64::from(freq),
                    );
                }
            }
            Ok(Arc::new(TermQuery::with_states(
                term.clone(),
                Arc::new(context),
            )))
        }
    }

    /// Equivalent to `FuzzyLikeThisQuery.isIgnoreTF()`.
    pub fn is_ignore_tf(&self) -> bool {
        self.ignore_tf
    }

    /// Equivalent to `FuzzyLikeThisQuery.setIgnoreTF(boolean)`.
    pub fn set_ignore_tf(&mut self, ignore_tf: bool) {
        self.ignore_tf = ignore_tf;
    }
}

impl Query for FuzzyLikeThisQuery {
    /// See the type documentation: Java returns `null`.
    fn to_query_string(&self, _field: &str) -> String {
        "null".to_string()
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let reader = index_searcher.get_index_reader();
        let mut q = ScoreTermQueue::new(self.max_num_terms)?;
        // load up the list of possible terms
        for f in &self.field_vals {
            self.add_terms_to_queue(reader, f, &mut q)?;
        }

        let mut bq = BooleanQueryBuilder::new();

        // create BooleanQueries to hold the variants for each token/field pair and ensure it
        // has no coord factor
        // Step 1: sort the termqueries by term/field
        let mut variant_queries: JavaObjectHashMap<Term, Vec<ScoreTerm>> = JavaObjectHashMap::new();
        let size = q.size();
        for _ in 0..size {
            let st = q.pop().ok_or_else(LuceneError::null_pointer)?;
            match variant_queries.get_mut(&st.fuzzied_source_term) {
                Some(l) => l.push(st),
                None => {
                    let key = st.fuzzied_source_term.clone();
                    variant_queries.insert(key, vec![st]);
                }
            }
        }
        // Step 2: Organize the sorted termqueries into zero-coord scoring boolean queries
        for variants in variant_queries.values() {
            if variants.len() == 1 {
                // optimize where only one selected variant
                let st = &variants[0];
                let tq = self.new_term_query(reader, &st.term)?;
                // set the boost to a mix of IDF and score
                bq.add(Arc::new(BoostQuery::new(tq, st.score)?), Occur::SHOULD)?;
            } else {
                let mut term_variants = BooleanQueryBuilder::new();
                for st in variants {
                    // found a match
                    let tq = self.new_term_query(reader, &st.term)?;
                    // set the boost using the ScoreTerm's score
                    term_variants.add(Arc::new(BoostQuery::new(tq, st.score)?), Occur::SHOULD)?;
                    // add to query
                }
                bq.add(Arc::new(term_variants.build()), Occur::SHOULD)?; // add to query
            }
        }
        // TODO(lucene): possible alternative step 3 - organize above booleans into a new layer of
        // field-based booleans with a minimum-should-match of NumFields-1?
        Ok(Some(Arc::new(bq.build())))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<FuzzyLikeThisQuery>()
                .is_some_and(|other| self.equals_to(other))
    }

    fn query_hash(&self) -> i32 {
        let prime = 31i32;
        let mut result = self.class_hash();
        result = prime
            .wrapping_mul(result)
            // `Objects.hashCode(analyzer)`: `0` for `null`
            .wrapping_add(
                self.analyzer
                    .as_ref()
                    .map_or(0, |a| java_identity_hash_code_stand_in(a.java_class_name())),
            );
        let field_vals_hash = self
            .field_vals
            .iter()
            .fold(1i32, |h, f| h.wrapping_mul(31).wrapping_add(f.hash_code()));
        result = prime.wrapping_mul(result).wrapping_add(field_vals_hash);
        result = prime
            .wrapping_mul(result)
            .wrapping_add(java_boolean_hash_code(self.ignore_tf));
        result = prime.wrapping_mul(result).wrapping_add(self.max_num_terms);
        result
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.sandbox.queries.FuzzyLikeThisQuery"
    }
}

/// One field's input for "fuzzification".
///
/// Equivalent to the package-private static nested class
/// `org.apache.lucene.sandbox.queries.FuzzyLikeThisQuery.FieldVals`.
#[derive(Debug, Clone)]
pub(crate) struct FieldVals {
    query_string: Option<String>,
    field_name: Option<String>,
    max_edits: i32,
    prefix_length: i32,
}

impl FieldVals {
    /// Equivalent to `FieldVals(String, int, int, String)`.
    fn new(
        name: Option<String>,
        max_edits: i32,
        length: i32,
        query_string: Option<String>,
    ) -> Self {
        Self {
            field_name: name,
            max_edits,
            prefix_length: length,
            query_string,
        }
    }

    /// Equivalent to `FieldVals.hashCode()`.
    fn hash_code(&self) -> i32 {
        let prime = 31i32;
        let mut result = 1i32;
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.field_name.as_deref().map_or(0, java_string_hash_code));
        result = prime.wrapping_mul(result).wrapping_add(self.max_edits);
        result = prime.wrapping_mul(result).wrapping_add(self.prefix_length);
        result = prime.wrapping_mul(result).wrapping_add(
            self.query_string
                .as_deref()
                .map_or(0, java_string_hash_code),
        );
        result
    }
}

impl PartialEq for FieldVals {
    /// Equivalent to `FieldVals.equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.field_name == other.field_name
            && self.max_edits == other.max_edits
            && self.prefix_length == other.prefix_length
            && self.query_string == other.query_string
    }
}

/// Holds info for a fuzzy term variant: initially the score is set to the
/// edit distance (for ranking best term variants), then it is reset with IDF
/// for use in ranking against all other terms/fields.
///
/// Equivalent to the private static nested class
/// `org.apache.lucene.sandbox.queries.FuzzyLikeThisQuery.ScoreTerm`.
#[derive(Debug, Clone)]
pub(crate) struct ScoreTerm {
    term: Term,
    score: f32,
    fuzzied_source_term: Term,
}

impl ScoreTerm {
    /// Equivalent to `ScoreTerm(Term, float, Term)`.
    fn new(term: Term, score: f32, fuzzied_source_term: Term) -> Self {
        Self {
            term,
            score,
            fuzzied_source_term,
        }
    }
}

/// The ordering of [`ScoreTermQueue`]: its `lessThan`.
#[derive(Debug, Clone, Copy)]
struct ScoreTermOrder;

impl PriorityQueueComparator<ScoreTerm> for ScoreTermOrder {
    fn less_than(&self, term_a: &ScoreTerm, term_b: &ScoreTerm) -> bool {
        if term_a.score == term_b.score {
            term_a.term > term_b.term
        } else {
            term_a.score < term_b.score
        }
    }
}

/// A priority queue of score terms, by score and then by term.
///
/// Equivalent to the private static nested class
/// `org.apache.lucene.sandbox.queries.FuzzyLikeThisQuery.ScoreTermQueue`,
/// which extends `PriorityQueue<ScoreTerm>`.
pub(crate) struct ScoreTermQueue {
    queue: PriorityQueue<ScoreTerm, ScoreTermOrder>,
}

impl ScoreTermQueue {
    /// Equivalent to `ScoreTermQueue(int)`, which is `super(size)`.
    ///
    /// # Errors
    ///
    /// Returns the `IllegalArgumentException` of `PriorityQueue(int)` for a
    /// negative or oversized `size`.
    fn new(size: i32) -> Result<Self> {
        if size < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "maxSize must be >= 0 and < {}; got: {size}",
                rucene::util::ArrayUtil::MAX_ARRAY_LENGTH
            )));
        }
        Ok(Self {
            queue: PriorityQueue::new(size as usize, ScoreTermOrder)?,
        })
    }

    fn size(&self) -> usize {
        self.queue.size()
    }

    fn top(&self) -> Option<&ScoreTerm> {
        self.queue.top()
    }

    fn pop(&mut self) -> Option<ScoreTerm> {
        self.queue.pop()
    }

    fn insert_with_overflow(&mut self, element: ScoreTerm) -> Option<ScoreTerm> {
        self.queue.insert_with_overflow(element)
    }
}
