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

//! Port of `SpanPayloadCheckQuery.java`, of the Java package
//! `org.apache.lucene.queries.payloads`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene::index::{LeafReaderContext, PostingsEnum, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::Matches;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::weight::Weight;
use rucene::util::java_collections::java_identity_hash_code_stand_in;
use rucene::util::BytesRef;

use crate::payloads::payload_matcher::PayloadMatcher;
use crate::payloads::payload_matcher_factory::PayloadMatcherFactory;
use crate::spans::filter_spans::{AcceptStatus, FilterSpans, FilterSpansAccept};
use crate::spans::span_collector::SpanCollector;
use crate::spans::span_query::{accept_field, get_term_states, SpanQuery, TermStatesMap};
use crate::spans::span_scorer::SpanScorer;
use crate::spans::span_weight::{
    get_norm_values, new_span_weight, span_weight_explain, span_weight_matches,
    span_weight_scorer_supplier, Postings, SpanWeight, SpanWeightBase,
};
use crate::spans::spans::Spans;

/// Only returns those matches that have a specific payload at the given
/// position.
///
/// Equivalent to `org.apache.lucene.queries.payloads.SpanPayloadCheckQuery`.
#[derive(Debug, Clone)]
pub struct SpanPayloadCheckQuery {
    /// The protected final `payloadToMatch` field; Java's `null` entries are
    /// `None`.
    payload_to_match: Arc<Vec<Option<BytesRef>>>,
    /// The protected final `match` field.
    match_query: Arc<dyn SpanQuery>,
    /// The protected final `operation` field.
    operation: MatchOperation,
    /// The protected final `payloadType` field.
    payload_type: PayloadType,
}

/// The payload type. This specifies the decoding of the bytes of the payload.
///
/// Equivalent to `org.apache.lucene.queries.payloads.SpanPayloadCheckQuery.PayloadType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayloadType {
    /// Java constant `INT`: a 4 byte payload that is a packed integer.
    Int,
    /// Java constant `FLOAT`: a 4 byte payload decoded to a float (32 bit).
    Float,
    /// Java constant `STRING`: a UTF-8 encoded string, decoded from the byte
    /// array.
    String,
}

impl PayloadType {
    /// Java's `PayloadType.name()`.
    pub fn name(self) -> &'static str {
        match self {
            PayloadType::Int => "INT",
            PayloadType::Float => "FLOAT",
            PayloadType::String => "STRING",
        }
    }

    /// Java's `PayloadType.hashCode()`, an enum constant's identity hash,
    /// replaced by
    /// [`java_identity_hash_code_stand_in`].
    pub fn java_hash_code(self) -> i32 {
        java_identity_hash_code_stand_in(match self {
            PayloadType::Int => {
                "org.apache.lucene.queries.payloads.SpanPayloadCheckQuery$PayloadType.INT"
            }
            PayloadType::Float => {
                "org.apache.lucene.queries.payloads.SpanPayloadCheckQuery$PayloadType.FLOAT"
            }
            PayloadType::String => {
                "org.apache.lucene.queries.payloads.SpanPayloadCheckQuery$PayloadType.STRING"
            }
        })
    }
}

impl fmt::Display for PayloadType {
    /// `PayloadType.toString()`, the constant's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The match operation. This specifies how the payload is compared with the
/// reference.
///
/// Equivalent to `org.apache.lucene.queries.payloads.SpanPayloadCheckQuery.MatchOperation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchOperation {
    /// Java constant `EQ`: checks for binary equality of the byte array
    /// (default).
    Eq,
    /// Java constant `GT`: matches if the payload value is greater than the
    /// reference.
    Gt,
    /// Java constant `GTE`: matches if the payload value is greater than or
    /// equal to the reference.
    Gte,
    /// Java constant `LT`: matches if the payload value is less than the
    /// reference.
    Lt,
    /// Java constant `LTE`: matches if the payload value is less than or
    /// equal to the reference.
    Lte,
}

impl MatchOperation {
    /// Java's `MatchOperation.name()`.
    pub fn name(self) -> &'static str {
        match self {
            MatchOperation::Eq => "EQ",
            MatchOperation::Gt => "GT",
            MatchOperation::Gte => "GTE",
            MatchOperation::Lt => "LT",
            MatchOperation::Lte => "LTE",
        }
    }

    /// Java's `MatchOperation.hashCode()`, an enum constant's identity hash,
    /// replaced by
    /// [`java_identity_hash_code_stand_in`].
    pub fn java_hash_code(self) -> i32 {
        java_identity_hash_code_stand_in(match self {
            MatchOperation::Eq => {
                "org.apache.lucene.queries.payloads.SpanPayloadCheckQuery$MatchOperation.EQ"
            }
            MatchOperation::Gt => {
                "org.apache.lucene.queries.payloads.SpanPayloadCheckQuery$MatchOperation.GT"
            }
            MatchOperation::Gte => {
                "org.apache.lucene.queries.payloads.SpanPayloadCheckQuery$MatchOperation.GTE"
            }
            MatchOperation::Lt => {
                "org.apache.lucene.queries.payloads.SpanPayloadCheckQuery$MatchOperation.LT"
            }
            MatchOperation::Lte => {
                "org.apache.lucene.queries.payloads.SpanPayloadCheckQuery$MatchOperation.LTE"
            }
        })
    }
}

impl fmt::Display for MatchOperation {
    /// `MatchOperation.toString()`, the constant's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl SpanPayloadCheckQuery {
    /// Creates a query that checks payloads for equality with
    /// [`PayloadType::String`].
    ///
    /// Equivalent to `SpanPayloadCheckQuery(SpanQuery, List<BytesRef>)`.
    ///
    /// * `match_query` — the underlying span query to check;
    /// * `payload_to_match` — the list of payloads to match.
    pub fn new(match_query: Arc<dyn SpanQuery>, payload_to_match: Vec<Option<BytesRef>>) -> Self {
        Self::with_type_and_operation(
            match_query,
            payload_to_match,
            PayloadType::String,
            MatchOperation::Eq,
        )
    }

    /// Creates a query that checks payloads with an operation.
    ///
    /// Equivalent to
    /// `SpanPayloadCheckQuery(SpanQuery, List<BytesRef>, PayloadType, MatchOperation)`.
    ///
    /// * `match_query` — the underlying span query to check;
    /// * `payload_to_match` — the list of payloads to match;
    /// * `payload_type` — the format of the bytes in the payload (string,
    ///   integer or float);
    /// * `operation` — the equality check: lt, lte, gt, gte or eq.
    pub fn with_type_and_operation(
        match_query: Arc<dyn SpanQuery>,
        payload_to_match: Vec<Option<BytesRef>>,
        payload_type: PayloadType,
        operation: MatchOperation,
    ) -> Self {
        Self {
            match_query,
            payload_to_match: Arc::new(payload_to_match),
            payload_type,
            operation,
        }
    }

    /// `List.hashCode()` of the payloads: `31 * h + (e == null ? 0 : e.hashCode())`.
    fn payload_to_match_hash(&self) -> i32 {
        self.payload_to_match.iter().fold(1i32, |h, e| {
            h.wrapping_mul(31)
                .wrapping_add(e.as_ref().map_or(0, BytesRef::hash_code))
        })
    }
}

impl Query for SpanPayloadCheckQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        buffer.push_str("SpanPayloadCheckQuery(");
        buffer.push_str(&self.match_query.to_query_string(field));
        buffer.push_str(", payloadRef: ");
        for bytes in self.payload_to_match.iter() {
            match bytes {
                Some(bytes) => buffer.push_str(&Term::to_string_bytes_ref(bytes)),
                // `Term.toString(null)` dereferences its argument.
                None => rucene::store::java_nio_file::throw_unchecked(
                    "java.lang.NullPointerException",
                    None,
                ),
            }
            buffer.push(';');
        }
        buffer.push_str(", payloadType:");
        buffer.push_str(self.payload_type.name());
        buffer.push(';');
        buffer.push_str(", operation:");
        buffer.push_str(self.operation.name());
        buffer.push(';');
        buffer.push(')');
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if accept_field(visitor, self.match_query.get_field()) {
            self.match_query
                .visit(&mut *visitor.get_sub_visitor(Occur::MUST, self));
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(self.create_span_weight(searcher, score_mode, boost)?)
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(self.rewrite_span(searcher)?.map(|q| q as Arc<dyn Query>))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<SpanPayloadCheckQuery>()
                .is_some_and(|other| {
                    *self.payload_to_match == *other.payload_to_match
                        && self.match_query.query_eq(&*other.match_query as &dyn Query)
                        && self.operation == other.operation
                        && self.payload_type == other.payload_type
                })
    }

    fn query_hash(&self) -> i32 {
        let mut result = self.class_hash();
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(self.match_query.query_hash());
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(self.payload_to_match_hash());
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(self.operation.java_hash_code());
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(self.payload_type.java_hash_code());
        result
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.payloads.SpanPayloadCheckQuery"
    }
}

impl SpanQuery for SpanPayloadCheckQuery {
    fn get_field(&self) -> Option<&str> {
        self.match_query.get_field()
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        let match_weight = self
            .match_query
            .create_span_weight(searcher, score_mode, boost)?;
        let term_states = if score_mode.needs_scores() {
            Some(get_term_states(&[&*match_weight]))
        } else {
            None
        };
        Ok(SpanPayloadCheckWeight::new(
            Arc::new(self.clone()),
            searcher,
            term_states.as_ref(),
            match_weight,
            boost,
            self.payload_type,
        )?)
    }

    /// Equivalent to `SpanPayloadCheckQuery.rewrite(IndexSearcher)`, whose
    /// `matchRewritten instanceof SpanQuery` test always holds for a rewritten
    /// span query.
    fn rewrite_span(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        if let Some(match_rewritten) = self.match_query.rewrite_span(searcher)? {
            return Ok(Some(Arc::new(SpanPayloadCheckQuery {
                match_query: match_rewritten,
                payload_to_match: Arc::clone(&self.payload_to_match),
                payload_type: self.payload_type,
                operation: self.operation,
            })));
        }
        Ok(None)
    }
}

/// Weight that pulls its spans using a payload span collector.
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.payloads.SpanPayloadCheckQuery.SpanPayloadCheckWeight`.
pub struct SpanPayloadCheckWeight {
    base: SpanWeightBase,
    /// The package-private final `matchWeight` field.
    match_weight: Arc<dyn SpanWeight>,
    payload_to_match: Arc<Vec<Option<BytesRef>>>,
    payload_type: PayloadType,
    operation: MatchOperation,
}

impl std::fmt::Debug for SpanPayloadCheckWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanPayloadCheckWeight")
            .field("payload_type", &self.payload_type)
            .field("operation", &self.operation)
            .finish_non_exhaustive()
    }
}

impl SpanPayloadCheckWeight {
    /// Equivalent to
    /// `SpanPayloadCheckWeight(IndexSearcher, Map<Term, TermStates>, SpanWeight, float, PayloadType)`;
    /// `query` is the enclosing query. Java's `payloadType` argument is not
    /// stored: the collector reads the enclosing query's.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub fn new(
        query: Arc<SpanPayloadCheckQuery>,
        searcher: &IndexSearcher,
        term_states: Option<&TermStatesMap>,
        match_weight: Arc<dyn SpanWeight>,
        boost: f32,
        _payload_type: PayloadType,
    ) -> Result<Arc<Self>> {
        let payload_to_match = Arc::clone(&query.payload_to_match);
        let payload_type = query.payload_type;
        let operation = query.operation;
        let base = SpanWeightBase::new(query, searcher, term_states, boost)?;
        Ok(new_span_weight(move |this: Weak<dyn SpanWeight>| Self {
            base: base.bind(this),
            match_weight,
            payload_to_match,
            payload_type,
            operation,
        }))
    }
}

impl SegmentCacheable for SpanPayloadCheckWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.match_weight.is_cacheable(ctx)
    }
}

impl Weight for SpanPayloadCheckWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        span_weight_explain(self, context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        span_weight_scorer_supplier(self, context)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        span_weight_matches(self, context, doc)
    }
}

impl SpanWeight for SpanPayloadCheckWeight {
    fn span_weight_base(&self) -> &SpanWeightBase {
        &self.base
    }

    fn extract_term_states(&self, contexts: &mut TermStatesMap) {
        self.match_weight.extract_term_states(contexts);
    }

    fn get_spans(
        &self,
        context: &LeafReaderContext,
        required_postings: Postings,
    ) -> Result<Option<Box<dyn Spans>>> {
        let collector = PayloadChecker::new(
            Arc::clone(&self.payload_to_match),
            self.payload_type,
            self.operation,
        );
        let match_spans = self
            .match_weight
            .get_spans(context, required_postings.at_least(Postings::Payloads))?;
        Ok(match_spans.map(|match_spans| {
            Box::new(FilterSpans::new(
                match_spans,
                PayloadCheckFilter { collector },
            )) as Box<dyn Spans>
        }))
    }

    /// Equivalent to `SpanPayloadCheckWeight.scorerSupplier(LeafReaderContext)`,
    /// which checks the field and reads the spans with
    /// [`Postings::Payloads`].
    fn span_scorer(&self, context: &LeafReaderContext) -> Result<Option<SpanScorer>> {
        let Some(field) = self.base.field() else {
            return Ok(None);
        };

        if let Some(terms) = context.leaf_reader().terms(field)? {
            if !terms.has_positions() {
                return Err(LuceneError::illegal_state(format!(
                    "field \"{field}\" was indexed without position data; cannot run SpanQuery \
                     (query={})",
                    self.base.get_query().to_query_string("")
                )));
            }
        }

        let Some(spans) = self.get_spans(context, Postings::Payloads)? else {
            return Ok(None);
        };
        let norms = get_norm_values(context, Some(field))?;
        Ok(Some(SpanScorer::new(spans, self.get_sim_scorer(), norms)))
    }
}

/// The filter of [`SpanPayloadCheckWeight`]'s spans.
///
/// Equivalent to the anonymous subclass of `FilterSpans` that
/// `SpanPayloadCheckWeight.getSpans` returns, with the `collector` it
/// captures.
struct PayloadCheckFilter {
    collector: PayloadChecker,
}

impl FilterSpansAccept for PayloadCheckFilter {
    fn accept(&mut self, candidate: &mut dyn Spans) -> Result<AcceptStatus> {
        self.collector.reset();
        candidate.collect(&mut self.collector)?;
        Ok(self.collector.match_status())
    }
}

/// Checks the payloads of the leaves of the current match against the
/// payloads to match.
///
/// Equivalent to the private inner class
/// `org.apache.lucene.queries.payloads.SpanPayloadCheckQuery.PayloadChecker`.
///
/// **Divergence from Lucene 10.5.0.** Java hands the matcher the postings'
/// payload `BytesRef`, whose backing array holds the whole payload block, so
/// the int and float matchers read four bytes from it at the payload's offset
/// whatever the payload's length (`PayloadMatcherFactory.java:172,219`). The
/// Rust postings lend only the payload's own bytes, so a payload shorter than
/// four bytes fails with the `ArrayIndexOutOfBoundsException` Java raises at
/// the end of its array instead of reading the next payload's bytes.
struct PayloadChecker {
    upto: usize,
    matches: bool,
    payload_matcher: &'static dyn PayloadMatcher,
    payload_to_match: Arc<Vec<Option<BytesRef>>>,
}

impl PayloadChecker {
    fn new(
        payload_to_match: Arc<Vec<Option<BytesRef>>>,
        payload_type: PayloadType,
        operation: MatchOperation,
    ) -> Self {
        Self {
            upto: 0,
            matches: true,
            payload_matcher: PayloadMatcherFactory::create_matcher_for_op_and_type(
                payload_type,
                Some(operation),
            ),
            payload_to_match,
        }
    }

    /// Equivalent to the package-private `PayloadChecker.match()`.
    fn match_status(&self) -> AcceptStatus {
        if self.matches && self.upto == self.payload_to_match.len() {
            AcceptStatus::Yes
        } else {
            AcceptStatus::No
        }
    }
}

impl SpanCollector for PayloadChecker {
    fn collect_leaf(
        &mut self,
        postings: &dyn PostingsEnum,
        _position: i32,
        _term: &Term,
    ) -> Result<()> {
        if !self.matches {
            return Ok(());
        }
        if self.upto >= self.payload_to_match.len() {
            self.matches = false;
            return Ok(());
        }
        let payload = postings
            .get_payload()?
            .map(|bytes| BytesRef::new(bytes.to_vec()));
        let Some(expected) = self.payload_to_match[self.upto].as_ref() else {
            self.matches = payload.is_none();
            self.upto += 1;
            return Ok(());
        };
        let Some(payload) = payload else {
            self.matches = false;
            self.upto += 1;
            return Ok(());
        };
        self.matches = self.payload_matcher.compare_payload(expected, &payload);
        self.upto += 1;
        Ok(())
    }

    fn reset(&mut self) {
        self.upto = 0;
        self.matches = true;
    }
}
