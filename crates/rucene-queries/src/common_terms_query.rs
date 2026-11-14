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

//! Port of `CommonTermsQuery.java`, of the Java package
//! `org.apache.lucene.queries`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{IndexReader, LeafReaderContext, Term, TermStates};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::term_query::TermQuery;
use rucene::util::java_collections::java_float_hash_code;
use rucene::util::java_fmt::float_to_string;

/// A query that executes high-frequency terms in an optional sub-query to
/// prevent slow queries due to "common" terms like stopwords.
///
/// Equivalent to `org.apache.lucene.queries.CommonTermsQuery`.
///
/// This query builds 2 queries off the [added](Self::add) terms: low-frequency
/// terms are added to a required boolean clause and high-frequency terms are
/// added to an optional boolean clause. The optional clause is only executed
/// if the required "low-frequency" clause matches. In most cases,
/// high-frequency terms are unlikely to significantly contribute to the
/// document score unless at least one of the low-frequency terms are matched.
/// This query can improve query execution times significantly if applicable.
///
/// A common terms query has several advantages over stopword filtering at
/// index or query time since a term can be "classified" based on the actual
/// document frequency in the index and can prevent slow queries even across
/// domains without specialized stopword files.
///
/// **Note:** if the query only contains high-frequency terms the query is
/// rewritten into a plain conjunction query, i.e. all high-frequency terms
/// need to match in order to match a document.
#[derive(Debug, Clone)]
pub struct CommonTermsQuery {
    /// The protected final `terms` field.
    terms: Vec<Term>,
    /// The protected final `maxTermFrequency` field.
    max_term_frequency: f32,
    /// The protected final `lowFreqOccur` field.
    low_freq_occur: Occur,
    /// The protected final `highFreqOccur` field.
    high_freq_occur: Occur,
    /// The protected `lowFreqBoost` field.
    low_freq_boost: f32,
    /// The protected `highFreqBoost` field.
    high_freq_boost: f32,
    /// The protected `lowFreqMinNrShouldMatch` field.
    low_freq_min_nr_should_match: f32,
    /// The protected `highFreqMinNrShouldMatch` field.
    high_freq_min_nr_should_match: f32,
}

impl CommonTermsQuery {
    /// Creates a new common terms query.
    ///
    /// Equivalent to `CommonTermsQuery(Occur, Occur, float)`.
    ///
    /// * `high_freq_occur` — the occur used for high frequency terms;
    /// * `low_freq_occur` — the occur used for low frequency terms;
    /// * `max_term_frequency` — a value in `[0..1)` (or absolute number
    ///   `>= 1`) representing the maximum threshold of a term's document
    ///   frequency to be considered a low frequency term.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](rucene::error::LuceneErrorKind::IllegalArgument)
    /// if [`Occur::MUST_NOT`] is passed as `low_freq_occur` or
    /// `high_freq_occur`.
    pub fn new(
        high_freq_occur: Occur,
        low_freq_occur: Occur,
        max_term_frequency: f32,
    ) -> Result<Self> {
        if high_freq_occur == Occur::MUST_NOT {
            return Err(LuceneError::illegal_argument(
                "highFreqOccur should be MUST or SHOULD but was MUST_NOT".to_string(),
            ));
        }
        if low_freq_occur == Occur::MUST_NOT {
            return Err(LuceneError::illegal_argument(
                "lowFreqOccur should be MUST or SHOULD but was MUST_NOT".to_string(),
            ));
        }
        Ok(Self {
            terms: Vec::new(),
            max_term_frequency,
            low_freq_occur,
            high_freq_occur,
            low_freq_boost: 1.0,
            high_freq_boost: 1.0,
            low_freq_min_nr_should_match: 0.0,
            high_freq_min_nr_should_match: 0.0,
        })
    }

    /// Adds a term to the query.
    ///
    /// Equivalent to `CommonTermsQuery.add(Term)`; Java's
    /// `IllegalArgumentException` for a `null` term cannot arise.
    pub fn add(&mut self, term: Term) {
        self.terms.push(term);
    }

    /// Equivalent to the protected
    /// `CommonTermsQuery.calcLowFreqMinimumNumberShouldMatch(int)`.
    pub fn calc_low_freq_minimum_number_should_match(&self, num_optional: i32) -> i32 {
        Self::min_nr_should_match(self.low_freq_min_nr_should_match, num_optional)
    }

    /// Equivalent to the protected
    /// `CommonTermsQuery.calcHighFreqMinimumNumberShouldMatch(int)`.
    pub fn calc_high_freq_minimum_number_should_match(&self, num_optional: i32) -> i32 {
        Self::min_nr_should_match(self.high_freq_min_nr_should_match, num_optional)
    }

    /// Equivalent to the private final
    /// `CommonTermsQuery.minNrShouldMatch(float, int)`.
    fn min_nr_should_match(min_nr_should_match: f32, num_optional: i32) -> i32 {
        if min_nr_should_match >= 1.0 || min_nr_should_match == 0.0 {
            return min_nr_should_match as i32;
        }
        java_math_round(min_nr_should_match * num_optional as f32)
    }

    /// Builds the boolean query of the low- and high-frequency terms.
    ///
    /// Equivalent to the protected
    /// `CommonTermsQuery.buildQuery(int, TermStates[], Term[])`; Java's `null`
    /// states are `None`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the document
    /// frequencies, and the errors of the boolean builder.
    pub fn build_query(
        &self,
        max_doc: i32,
        context_array: &[Option<Arc<TermStates>>],
        query_terms: &[Term],
    ) -> Result<Arc<dyn Query>> {
        let mut low_freq_queries: Vec<Arc<dyn Query>> = Vec::new();
        let mut high_freq_queries: Vec<Arc<dyn Query>> = Vec::new();
        for (i, query_term) in query_terms.iter().enumerate() {
            match context_array[i].as_ref() {
                None => {
                    low_freq_queries.push(self.new_term_query(query_term.clone(), None));
                }
                Some(term_states) => {
                    let doc_freq = term_states.doc_freq()?;
                    if (self.max_term_frequency >= 1.0 && doc_freq as f32 > self.max_term_frequency)
                        || (doc_freq
                            > f64::from(self.max_term_frequency * max_doc as f32).ceil() as i32)
                    {
                        high_freq_queries.push(
                            self.new_term_query(query_term.clone(), Some(Arc::clone(term_states))),
                        );
                    } else {
                        low_freq_queries.push(
                            self.new_term_query(query_term.clone(), Some(Arc::clone(term_states))),
                        );
                    }
                }
            }
        }
        let num_low_freq_clauses = low_freq_queries.len() as i32;
        let num_high_freq_clauses = high_freq_queries.len() as i32;
        let low_freq_occur = self.low_freq_occur;
        let mut high_freq_occur = self.high_freq_occur;
        let mut low_freq_min_should_match = 0;
        let mut high_freq_min_should_match = 0;
        if low_freq_occur == Occur::SHOULD && num_low_freq_clauses > 0 {
            low_freq_min_should_match =
                self.calc_low_freq_minimum_number_should_match(num_low_freq_clauses);
        }
        if high_freq_occur == Occur::SHOULD && num_high_freq_clauses > 0 {
            high_freq_min_should_match =
                self.calc_high_freq_minimum_number_should_match(num_high_freq_clauses);
        }
        if low_freq_queries.is_empty() {
            // if lowFreq is empty we rewrite the high freq terms in a
            // conjunction to prevent slow queries.
            if high_freq_min_should_match == 0 && high_freq_occur != Occur::MUST {
                high_freq_occur = Occur::MUST;
            }
        }
        let mut builder = BooleanQueryBuilder::new();

        if !low_freq_queries.is_empty() {
            let mut low_freq = BooleanQueryBuilder::new();
            for query in low_freq_queries {
                low_freq.add(query, low_freq_occur)?;
            }
            low_freq.set_minimum_number_should_match(low_freq_min_should_match);
            let low_freq_query: Arc<dyn Query> = Arc::new(low_freq.build());
            builder.add(
                Arc::new(BoostQuery::new(low_freq_query, self.low_freq_boost)?),
                Occur::MUST,
            )?;
        }
        if !high_freq_queries.is_empty() {
            let mut high_freq = BooleanQueryBuilder::new();
            for query in high_freq_queries {
                high_freq.add(query, high_freq_occur)?;
            }
            high_freq.set_minimum_number_should_match(high_freq_min_should_match);
            let high_freq_query: Arc<dyn Query> = Arc::new(high_freq.build());
            builder.add(
                Arc::new(BoostQuery::new(high_freq_query, self.high_freq_boost)?),
                Occur::SHOULD,
            )?;
        }
        Ok(Arc::new(builder.build()))
    }

    /// Collects the [`TermStates`] of the query terms over the leaves of
    /// `reader`.
    ///
    /// Equivalent to `CommonTermsQuery.collectTermStates(IndexReader,
    /// List<LeafReaderContext>, TermStates[], Term[])`; Java's `null` states
    /// are `None`.
    ///
    /// Java skips a leaf whose terms enum is the `TermsEnum.EMPTY` singleton,
    /// an identity test this port cannot make; `seekExact` on that enum
    /// answers `false`, so the leaf registers nothing either way.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while seeking the terms.
    pub fn collect_term_states(
        &self,
        reader: &Arc<dyn IndexReader>,
        leaves: &[Arc<LeafReaderContext>],
        context_array: &mut [Option<TermStates>],
        query_terms: &[Term],
    ) -> Result<()> {
        for context in leaves {
            for (i, term) in query_terms.iter().enumerate() {
                let Some(terms) = context.leaf_reader().terms(term.field())? else {
                    // field does not exist
                    continue;
                };
                let mut terms_enum = terms.iterator()?;

                if terms_enum.seek_exact(term.bytes())? {
                    let ord = context.ord() as usize;
                    match context_array[i].as_mut() {
                        None => {
                            context_array[i] = Some(TermStates::with_state(
                                &Arc::clone(reader).get_context()?,
                                terms_enum.term_state()?,
                                ord,
                                terms_enum.doc_freq()?,
                                terms_enum.total_term_freq()?,
                            )?);
                        }
                        Some(term_states) => {
                            term_states.register(
                                terms_enum.term_state()?,
                                ord,
                                terms_enum.doc_freq()?,
                                terms_enum.total_term_freq()?,
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Specifies a minimum number of the low frequent optional boolean clauses
    /// which must be satisfied in order to produce a match on the low
    /// frequency terms query part.
    ///
    /// Equivalent to `CommonTermsQuery.setLowFreqMinimumNumberShouldMatch(float)`.
    /// This accepts a value in the range `[0..1)` as a fraction of the actual
    /// query terms in the low frequent clause, or a number `>= 1` as an
    /// absolute number of clauses that need to match. By default no optional
    /// clauses are necessary for a match (unless there are no required
    /// clauses).
    pub fn set_low_freq_minimum_number_should_match(&mut self, min: f32) {
        self.low_freq_min_nr_should_match = min;
    }

    /// Gets the minimum number of the optional low frequent boolean clauses
    /// which must be satisfied.
    ///
    /// Equivalent to `CommonTermsQuery.getLowFreqMinimumNumberShouldMatch()`.
    pub fn get_low_freq_minimum_number_should_match(&self) -> f32 {
        self.low_freq_min_nr_should_match
    }

    /// Specifies a minimum number of the high frequent optional boolean
    /// clauses which must be satisfied in order to produce a match on the low
    /// frequency terms query part.
    ///
    /// Equivalent to `CommonTermsQuery.setHighFreqMinimumNumberShouldMatch(float)`;
    /// see [`set_low_freq_minimum_number_should_match`](Self::set_low_freq_minimum_number_should_match).
    pub fn set_high_freq_minimum_number_should_match(&mut self, min: f32) {
        self.high_freq_min_nr_should_match = min;
    }

    /// Gets the minimum number of the optional high frequent boolean clauses
    /// which must be satisfied.
    ///
    /// Equivalent to `CommonTermsQuery.getHighFreqMinimumNumberShouldMatch()`.
    pub fn get_high_freq_minimum_number_should_match(&self) -> f32 {
        self.high_freq_min_nr_should_match
    }

    /// Gets the list of terms.
    ///
    /// Equivalent to `CommonTermsQuery.getTerms()`, an unmodifiable view.
    pub fn get_terms(&self) -> &[Term] {
        &self.terms
    }

    /// Gets the maximum threshold of a term's document frequency to be
    /// considered a low frequency term.
    ///
    /// Equivalent to `CommonTermsQuery.getMaxTermFrequency()`.
    pub fn get_max_term_frequency(&self) -> f32 {
        self.max_term_frequency
    }

    /// Gets the occur used for low frequency terms.
    ///
    /// Equivalent to `CommonTermsQuery.getLowFreqOccur()`.
    pub fn get_low_freq_occur(&self) -> Occur {
        self.low_freq_occur
    }

    /// Gets the occur used for high frequency terms.
    ///
    /// Equivalent to `CommonTermsQuery.getHighFreqOccur()`.
    pub fn get_high_freq_occur(&self) -> Occur {
        self.high_freq_occur
    }

    /// Gets the boost used for low frequency terms.
    ///
    /// Equivalent to `CommonTermsQuery.getLowFreqBoost()`.
    pub fn get_low_freq_boost(&self) -> f32 {
        self.low_freq_boost
    }

    /// Gets the boost used for high frequency terms.
    ///
    /// Equivalent to `CommonTermsQuery.getHighFreqBoost()`.
    pub fn get_high_freq_boost(&self) -> f32 {
        self.high_freq_boost
    }

    /// Builds a new term query instance.
    ///
    /// Equivalent to the protected `CommonTermsQuery.newTermQuery(Term,
    /// TermStates)`; `term_states` may be `None`, where Java passes `null`.
    pub fn new_term_query(
        &self,
        term: Term,
        term_states: Option<Arc<TermStates>>,
    ) -> Arc<dyn Query> {
        match term_states {
            None => Arc::new(TermQuery::new(term)),
            Some(term_states) => Arc::new(TermQuery::with_states(term, term_states)),
        }
    }

    /// Equivalent to the private `CommonTermsQuery.equalsTo(CommonTermsQuery)`.
    fn equals_to(&self, other: &CommonTermsQuery) -> bool {
        java_float_hash_code(self.high_freq_boost) == java_float_hash_code(other.high_freq_boost)
            && self.high_freq_occur == other.high_freq_occur
            && self.low_freq_occur == other.low_freq_occur
            && java_float_hash_code(self.low_freq_boost)
                == java_float_hash_code(other.low_freq_boost)
            && java_float_hash_code(self.max_term_frequency)
                == java_float_hash_code(other.max_term_frequency)
            && self.low_freq_min_nr_should_match == other.low_freq_min_nr_should_match
            && self.high_freq_min_nr_should_match == other.high_freq_min_nr_should_match
            && self.terms == other.terms
    }
}

/// Java's `Math.round(float)`: `floor(a + 0.5)` computed exactly on the bits
/// of `a` (JDK 7+), with NaN mapping to `0` and saturation at the `int`
/// bounds.
fn java_math_round(a: f32) -> i32 {
    const SIGNIFICAND_WIDTH: i32 = 24;
    const EXP_BIAS: i32 = 127;
    const EXP_BIT_MASK: i32 = 0x7F80_0000;
    const SIGNIF_BIT_MASK: i32 = 0x007F_FFFF;
    let int_bits = a.to_bits() as i32;
    let biased_exp = (int_bits & EXP_BIT_MASK) >> (SIGNIFICAND_WIDTH - 1);
    let shift = (SIGNIFICAND_WIDTH - 2 + EXP_BIAS) - biased_exp;
    if (shift & -32) == 0 {
        // shift >= 0 && shift < 32
        let mut r = (int_bits & SIGNIF_BIT_MASK) | (SIGNIF_BIT_MASK + 1);
        if int_bits < 0 {
            r = -r;
        }
        ((r >> shift) + 1) >> 1
    } else {
        // `(int) a`: saturating, NaN to 0.
        a as i32
    }
}

impl Query for CommonTermsQuery {
    fn to_query_string(&self, _field: &str) -> String {
        let mut buffer = String::new();
        let need_parens = self.get_low_freq_minimum_number_should_match() > 0.0;
        if need_parens {
            buffer.push('(');
        }
        for (i, t) in self.terms.iter().enumerate() {
            buffer.push_str(&self.new_term_query(t.clone(), None).to_query_string(""));

            if i != self.terms.len() - 1 {
                buffer.push_str(", ");
            }
        }
        if need_parens {
            buffer.push(')');
        }
        if self.get_low_freq_minimum_number_should_match() > 0.0
            || self.get_high_freq_minimum_number_should_match() > 0.0
        {
            buffer.push('~');
            buffer.push('(');
            buffer.push_str(&float_to_string(
                self.get_low_freq_minimum_number_should_match(),
            ));
            buffer.push_str(&float_to_string(
                self.get_high_freq_minimum_number_should_match(),
            ));
            buffer.push(')');
        }
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let selected_terms: Vec<Term> = self
            .terms
            .iter()
            .filter(|t| visitor.accept_field(t.field()))
            .cloned()
            .collect();
        if !selected_terms.is_empty() {
            let mut v = visitor.get_sub_visitor(Occur::SHOULD, self);
            v.consume_terms(self, &selected_terms);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let reader = index_searcher.get_index_reader();
        if self.terms.is_empty() {
            return Ok(Some(Arc::new(MatchNoDocsQuery::new(
                "CommonTermsQuery with no terms",
            ))));
        } else if self.terms.len() == 1 {
            return Ok(Some(self.new_term_query(self.terms[0].clone(), None)));
        }
        let leaves = Arc::clone(reader).leaves()?;
        let max_doc = reader.max_doc();
        let mut context_array: Vec<Option<TermStates>> =
            (0..self.terms.len()).map(|_| None).collect();
        let query_terms: Vec<Term> = self.terms.clone();
        self.collect_term_states(reader, &leaves, &mut context_array, &query_terms)?;
        let context_array: Vec<Option<Arc<TermStates>>> = context_array
            .into_iter()
            .map(|ts| ts.map(Arc::new))
            .collect();
        Ok(Some(self.build_query(
            max_doc,
            &context_array,
            &query_terms,
        )?))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<CommonTermsQuery>()
                .is_some_and(|other| self.equals_to(other))
    }

    fn query_hash(&self) -> i32 {
        let prime = 31i32;
        let mut result = self.class_hash();
        result = prime
            .wrapping_mul(result)
            .wrapping_add(java_float_hash_code(self.high_freq_boost));
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.high_freq_occur.java_hash_code());
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.low_freq_occur.java_hash_code());
        result = prime
            .wrapping_mul(result)
            .wrapping_add(java_float_hash_code(self.low_freq_boost));
        result = prime
            .wrapping_mul(result)
            .wrapping_add(java_float_hash_code(self.max_term_frequency));
        result = prime
            .wrapping_mul(result)
            .wrapping_add(java_float_hash_code(self.low_freq_min_nr_should_match));
        result = prime
            .wrapping_mul(result)
            .wrapping_add(java_float_hash_code(self.high_freq_min_nr_should_match));
        let terms_hash = self
            .terms
            .iter()
            .fold(1i32, |h, t| h.wrapping_mul(31).wrapping_add(t.hash_code()));
        result = prime.wrapping_mul(result).wrapping_add(terms_hash);
        result
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.CommonTermsQuery"
    }
}
