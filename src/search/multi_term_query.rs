//! Queries over a subset of the terms of a field, ported from
//! `org.apache.lucene.search.MultiTermQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::Result;
use crate::index::{Term, Terms, TermsEnum};
use crate::search::blended_term_query::{boolean_rewrite, Builder as BlendedTermQueryBuilder};
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::boost_query::BoostQuery;
use crate::search::constant_score_query::ConstantScoreQuery;
use crate::search::doc_values_rewrite_method::DocValuesRewriteMethod;
use crate::search::index_searcher::IndexSearcher;
use crate::search::multi_term_query_constant_score_blended_wrapper::MultiTermQueryConstantScoreBlendedWrapper;
use crate::search::multi_term_query_constant_score_wrapper::MultiTermQueryConstantScoreWrapper;
use crate::search::query::Query;
use crate::search::scoring_rewrite::{
    ConstantScoreBooleanRewrite, ScoringBooleanQueryBuilder, ScoringBooleanRewrite,
};
use crate::search::term_collecting_rewrite::{TermCollectingRewrite, TopLevelBuilder};
use crate::search::term_query::TermQuery;
use crate::search::term_states::TermStates;
use crate::search::top_terms_rewrite::{
    top_terms_rewrite, top_terms_rewrite_eq, top_terms_rewrite_hash, TopTermsRewrite,
};
use crate::util::attribute::AttributeSource;

/// A [`Query`] that matches documents containing a subset of terms provided by
/// a [`FilteredTermsEnum`](crate::index::FilteredTermsEnum) enumeration.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.MultiTermQuery`. This query cannot be used
/// directly: an implementation must define
/// [`get_terms_enum_with_atts`](Self::get_terms_enum_with_atts) so that it
/// provides a terms enum iterating the terms to be matched.
///
/// **NOTE**: with [`scoring_boolean_rewrite`] or
/// [`constant_score_boolean_rewrite`], searching may report
/// [`TooManyClauses`](crate::search::TooManyClauses) when the number of terms
/// to be searched exceeds
/// [`IndexSearcher::get_max_clause_count`]. Using
/// [`constant_score_blended_rewrite`] or [`constant_score_rewrite`] prevents
/// that.
///
/// The recommended rewrite method is [`constant_score_blended_rewrite`]: it
/// does not spend CPU computing unhelpful scores, and is the most performant
/// rewrite method given the query. When scoring is needed — as
/// [`FuzzyQuery`](crate::search::FuzzyQuery) does — use
/// [`TopTermsScoringBooleanQueryRewrite`], which uses a priority queue to
/// collect only competitive terms and therefore does not hit that limitation.
///
/// **Divergence from Lucene 10.5.0.** Java's abstract class holds the `field`
/// and `rewriteMethod` fields and makes `rewrite`, `getTermsEnum(Terms)`,
/// `hashCode` and `equals` concrete. A Rust trait cannot hold state, so the
/// fields become the accessors [`get_field`](Self::get_field) and
/// [`get_rewrite_method`](Self::get_rewrite_method), and the concrete methods
/// become the free functions [`multi_term_rewrite`], [`get_terms_enum`],
/// [`multi_term_query_hash`] and [`multi_term_query_eq`] of this module, which
/// every implementation calls.
pub trait MultiTermQuery: Query {
    /// Returns the field name for this query.
    ///
    /// Equivalent to the `final MultiTermQuery.getField()`.
    fn get_field(&self) -> &str;

    /// Returns the rewrite method used to build the final query.
    ///
    /// Equivalent to `MultiTermQuery.getRewriteMethod()`.
    fn get_rewrite_method(&self) -> Arc<dyn RewriteMethod>;

    /// Constructs the enumeration to be used, expanding the pattern term.
    ///
    /// Equivalent to the `protected abstract
    /// MultiTermQuery.getTermsEnum(Terms, AttributeSource)`. It is only called
    /// if the field exists, so implementations may assume it does; it must not
    /// return an error in place of an empty enumeration — return
    /// [`EmptyTermsEnum`](crate::index::EmptyTermsEnum) when no term matches —
    /// and the returned enum must already be positioned on the first matching
    /// term. The given [`AttributeSource`] is passed by the [`RewriteMethod`]
    /// to share information between segments; [`TopTermsRewrite`] uses it to
    /// share maximum competitive boosts.
    ///
    /// [`TopTermsRewrite`]: crate::search::TopTermsRewrite
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the enumeration.
    /// **Divergence from Lucene 10.5.0.** Java takes a bare `Terms` reference,
    /// which the garbage collector keeps alive for as long as the enum it
    /// returns needs it — [`FuzzyTermsEnum`](crate::search::FuzzyTermsEnum)
    /// keeps it, and calls `intersect` on it again every time it lowers its
    /// maximum edit distance. The returned enum is `'static` in this port, so
    /// the terms are handed over as a shared [`Arc`] instead. Every caller
    /// already owns the `Terms` it opened from the leaf reader.
    fn get_terms_enum_with_atts(
        &self,
        terms: &Arc<dyn Terms>,
        atts: &mut AttributeSource,
    ) -> Result<Box<dyn TermsEnum>>;

    /// Returns the number of unique terms contained in this query when it is
    /// known up front, and `-1` when it is not.
    ///
    /// Equivalent to `MultiTermQuery.getTermsCount()`.
    fn get_terms_count(&self) -> i64 {
        -1
    }

    /// Returns this query's heap usage when it can report one.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's
    /// `AbstractMultiTermQueryConstantScoreWrapper.ramBytesUsed()` tests
    /// `query instanceof Accountable` and calls `ramBytesUsed()` when it holds.
    /// Rust cannot ask a `dyn MultiTermQuery` whether it also implements
    /// [`Accountable`](crate::util::Accountable), so the question is asked
    /// through this method instead; the implementations that are accountable —
    /// [`TermInSetQuery`](crate::search::TermInSetQuery) — override it, and the
    /// others inherit the `None` that selects Java's default estimate.
    fn accountable_ram_bytes_used(&self) -> Option<i64> {
        None
    }

    /// Returns this query viewed as a [`Query`].
    ///
    /// **Divergence from Lucene 10.5.0.** Java gets this for free, because
    /// `MultiTermQuery extends Query`. Rust cannot coerce a
    /// `&dyn MultiTermQuery` into a `&dyn Query` below 1.86, and this crate's
    /// minimum supported Rust version is 1.80, so the upcast is spelled out.
    /// Every implementation writes `self`.
    fn as_query(&self) -> &dyn Query;

    /// Returns a shared handle to a copy of this query.
    ///
    /// **Divergence from Lucene 10.5.0.** Java hands `this` to
    /// [`RewriteMethod::rewrite`], which stores it. Rust cannot produce an
    /// owning handle from `&self`, so every implementation writes
    /// `Arc::new(self.clone())`; a multi-term query is a small, immutable
    /// description, so the copy is cheap.
    fn to_multi_term_query_arc(&self) -> Arc<dyn MultiTermQuery>;

    /// Returns a shared handle to a copy of this query, viewed as a [`Query`].
    ///
    /// The [`Query`] counterpart of
    /// [`to_multi_term_query_arc`](Self::to_multi_term_query_arc); see that
    /// method for why it exists. Every implementation writes
    /// `Arc::new(self.clone())`.
    fn to_query_arc(&self) -> Arc<dyn Query>;
}

/// Constructs an enumeration that expands the pattern term.
///
/// Equivalent to the `final MultiTermQuery.getTermsEnum(Terms)`, which is
/// `getTermsEnum(terms, new AttributeSource())`. It is only called if the field
/// exists, never returns an error in place of an empty enumeration, and the
/// returned enum is positioned on the first matching term.
///
/// # Errors
///
/// Propagates any I/O error raised while building the enumeration.
pub fn get_terms_enum(
    query: &dyn MultiTermQuery,
    terms: &Arc<dyn Terms>,
) -> Result<Box<dyn TermsEnum>> {
    let mut atts = AttributeSource::new();
    query.get_terms_enum_with_atts(terms, &mut atts)
}

/// Rewrites a multi-term query through its [`RewriteMethod`].
///
/// Equivalent to the `final MultiTermQuery.rewrite(IndexSearcher)`. To rewrite
/// to a simpler form, return a simpler enum from
/// [`MultiTermQuery::get_terms_enum_with_atts`] instead — for example a
/// [`SingleTermsEnum`](crate::index::SingleTermsEnum) to rewrite to a single
/// term.
///
/// The result is always `Some`, because Java's rewrite methods always build a
/// new query rather than returning `this`.
///
/// # Errors
///
/// Propagates any I/O error raised while rewriting.
pub fn multi_term_rewrite(
    query: &dyn MultiTermQuery,
    index_searcher: &IndexSearcher,
) -> Result<Option<Arc<dyn Query>>> {
    let rewrite_method = query.get_rewrite_method();
    Ok(Some(rewrite_method.rewrite(
        index_searcher,
        query.to_multi_term_query_arc(),
    )?))
}

/// The hash code shared by every multi-term query.
///
/// Equivalent to `MultiTermQuery.hashCode()`, which mixes the class hash, the
/// rewrite method and the field.
pub fn multi_term_query_hash(query: &dyn MultiTermQuery) -> u64 {
    let prime = 31u64;
    let mut result = query.as_query().class_hash();
    result = prime
        .wrapping_mul(result)
        .wrapping_add(query.get_rewrite_method().method_hash());
    let mut hasher = DefaultHasher::new();
    query.get_field().hash(&mut hasher);
    prime.wrapping_mul(result).wrapping_add(hasher.finish())
}

/// The equality shared by every multi-term query.
///
/// Equivalent to the private `MultiTermQuery.equalsTo(MultiTermQuery)`, which
/// compares the rewrite method and the field; `sameClassAs` is the caller's
/// responsibility, exactly as in `MultiTermQuery.equals(Object)`.
pub fn multi_term_query_eq(a: &dyn MultiTermQuery, b: &dyn MultiTermQuery) -> bool {
    a.get_rewrite_method().method_eq(&*b.get_rewrite_method()) && a.get_field() == b.get_field()
}

/// Defines how a [`MultiTermQuery`] is rewritten.
///
/// Equivalent to the abstract nested class `MultiTermQuery.RewriteMethod`.
pub trait RewriteMethod: Send + Sync + Debug {
    /// Rewrites the query.
    ///
    /// Equivalent to
    /// `RewriteMethod.rewrite(IndexSearcher, MultiTermQuery)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting.
    fn rewrite(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>>;

    /// Returns the query's [`TermsEnum`].
    ///
    /// Equivalent to the `protected
    /// RewriteMethod.getTermsEnum(MultiTermQuery, Terms, AttributeSource)`,
    /// which lets a subclass pull a terms enum from the multi-term query.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the enumeration.
    fn get_terms_enum(
        &self,
        query: &dyn MultiTermQuery,
        terms: &Arc<dyn Terms>,
        atts: &mut AttributeSource,
    ) -> Result<Box<dyn TermsEnum>> {
        query.get_terms_enum_with_atts(terms, atts)
    }

    /// Returns this method as [`Any`], so that [`method_eq`](Self::method_eq)
    /// can recover the concrete type.
    fn as_any(&self) -> &dyn Any;

    /// Rewrite-method equivalence.
    ///
    /// Equivalent to `RewriteMethod.equals(Object)`. Java's base class does not
    /// override `Object.equals`, so the singletons compare by identity; each of
    /// them is the sole instance of an anonymous class, which makes comparing
    /// the concrete type exactly equivalent.
    fn method_eq(&self, other: &dyn RewriteMethod) -> bool {
        self.as_any().type_id() == other.as_any().type_id()
    }

    /// Rewrite-method hash code, consistent with [`method_eq`](Self::method_eq).
    ///
    /// **Divergence from Lucene 10.5.0.** Java's singletons inherit
    /// `Object.hashCode()`, an identity hash that differs between JVM runs;
    /// this port hashes the [`std::any::TypeId`], which is likewise stable only
    /// within a build but at least reproducible within a process. It is only
    /// ever mixed into [`multi_term_query_hash`], never compared across
    /// processes.
    fn method_hash(&self) -> u64;
}

/// Hashes a [`std::any::TypeId`], standing in for Java's identity hash of a
/// rewrite-method singleton.
pub(crate) fn rewrite_method_type_hash<T: ?Sized + Any>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.type_id().hash(&mut hasher);
    hasher.finish()
}

/// A rewrite method where documents are assigned a constant score equal to the
/// query's boost, maintaining a boolean-query-like implementation over the most
/// costly terms while pre-processing the less costly terms into a filter
/// bitset.
///
/// Equivalent to the anonymous class behind
/// `MultiTermQuery.CONSTANT_SCORE_BLENDED_REWRITE`. It enforces an upper limit
/// on the number of terms allowed in the boolean-query-like implementation, so
/// it balances the benefits of [`constant_score_boolean_rewrite`] and
/// [`constant_score_rewrite`]: it enables skipping and early termination over
/// costly terms while limiting the overhead of a boolean query with many terms,
/// and it cannot report
/// [`TooManyClauses`](crate::search::TooManyClauses). For a use case with only
/// low-cost terms, [`constant_score_rewrite`] may be more performant; for one
/// with only high-cost terms, [`constant_score_boolean_rewrite`] may be better.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConstantScoreBlendedRewrite;

impl RewriteMethod for ConstantScoreBlendedRewrite {
    fn rewrite(
        &self,
        _index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(MultiTermQueryConstantScoreBlendedWrapper::new(
            query,
        )))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_hash(&self) -> u64 {
        rewrite_method_type_hash(self)
    }
}

/// A rewrite method that first creates a private filter, by visiting each term
/// in sequence and marking all docs for that term; matching documents are
/// assigned a constant score equal to the query's boost.
///
/// Equivalent to the anonymous class behind
/// `MultiTermQuery.CONSTANT_SCORE_REWRITE`. It is faster than the boolean-query
/// rewrite methods when the number of matched terms or matched documents is
/// non-trivial, and it never reports
/// [`TooManyClauses`](crate::search::TooManyClauses).
#[derive(Debug, Default, Clone, Copy)]
pub struct ConstantScoreRewrite;

impl RewriteMethod for ConstantScoreRewrite {
    fn rewrite(
        &self,
        _index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(MultiTermQueryConstantScoreWrapper::new(query)))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_hash(&self) -> u64 {
        rewrite_method_type_hash(self)
    }
}

/// Returns the [`ConstantScoreBlendedRewrite`] singleton.
///
/// Equivalent to the `MultiTermQuery.CONSTANT_SCORE_BLENDED_REWRITE` constant.
pub fn constant_score_blended_rewrite() -> Arc<dyn RewriteMethod> {
    Arc::new(ConstantScoreBlendedRewrite)
}

/// Returns the [`ConstantScoreRewrite`] singleton.
///
/// Equivalent to the `MultiTermQuery.CONSTANT_SCORE_REWRITE` constant.
pub fn constant_score_rewrite() -> Arc<dyn RewriteMethod> {
    Arc::new(ConstantScoreRewrite)
}

/// Returns the [`DocValuesRewriteMethod`] singleton.
///
/// Equivalent to the `MultiTermQuery.DOC_VALUES_REWRITE` constant, a rewrite
/// method that uses `SORTED` / `SORTED_SET` doc values to find matching docs
/// through a post-filtering approach. It is very slow in isolation, but likely
/// the most performant option when combined with a sparse query clause; all
/// matching docs are assigned a constant score equal to the query's boost.
pub fn doc_values_rewrite() -> Arc<dyn RewriteMethod> {
    Arc::new(DocValuesRewriteMethod)
}

/// Returns the [`ScoringBooleanRewrite`] singleton.
///
/// Equivalent to the `MultiTermQuery.SCORING_BOOLEAN_REWRITE` constant, which
/// translates each term into an [`Occur::SHOULD`](crate::search::Occur) clause
/// of a boolean query and keeps the scores as computed by that query. Such
/// scores are typically meaningless to the user and require non-trivial CPU to
/// compute, so [`constant_score_rewrite`] is almost always better.
///
/// **NOTE**: this rewrite method reports
/// [`TooManyClauses`](crate::search::TooManyClauses) when the number of terms
/// exceeds [`IndexSearcher::get_max_clause_count`].
pub fn scoring_boolean_rewrite() -> Arc<dyn RewriteMethod> {
    Arc::new(ScoringBooleanRewrite)
}

/// Returns the [`ConstantScoreBooleanRewrite`] singleton.
///
/// Equivalent to the `MultiTermQuery.CONSTANT_SCORE_BOOLEAN_REWRITE` constant,
/// which is [`scoring_boolean_rewrite`] except that scores are not computed:
/// each matching document receives a constant score equal to the query's boost.
///
/// **NOTE**: this rewrite method reports
/// [`TooManyClauses`](crate::search::TooManyClauses) when the number of terms
/// exceeds [`IndexSearcher::get_max_clause_count`].
pub fn constant_score_boolean_rewrite() -> Arc<dyn RewriteMethod> {
    Arc::new(ConstantScoreBooleanRewrite)
}

/// A rewrite method that translates each term into an
/// [`Occur::SHOULD`](crate::search::Occur) clause of a boolean query and keeps
/// the scores as computed by that query, using only the top-scoring terms so
/// that the maximum clause count cannot overflow.
///
/// Equivalent to the `final class
/// MultiTermQuery.TopTermsScoringBooleanQueryRewrite`.
#[derive(Debug, Clone, Copy)]
pub struct TopTermsScoringBooleanQueryRewrite {
    size: i32,
}

impl TopTermsScoringBooleanQueryRewrite {
    /// Creates a rewrite for at most `size` terms.
    ///
    /// Equivalent to `TopTermsScoringBooleanQueryRewrite(int)`. When
    /// [`IndexSearcher::get_max_clause_count`] is smaller than `size`, it is
    /// used instead.
    pub fn new(size: i32) -> Self {
        Self { size }
    }
}

impl TermCollectingRewrite for TopTermsScoringBooleanQueryRewrite {
    fn get_top_level_builder(&self) -> Result<Box<dyn TopLevelBuilder>> {
        Ok(Box::new(ScoringBooleanQueryBuilder::new()))
    }
}

impl TopTermsRewrite for TopTermsScoringBooleanQueryRewrite {
    fn get_size(&self) -> i32 {
        self.size
    }

    fn get_max_size(&self) -> i32 {
        IndexSearcher::get_max_clause_count()
    }
}

impl RewriteMethod for TopTermsScoringBooleanQueryRewrite {
    fn rewrite(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        top_terms_rewrite(self, index_searcher, &*query)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_eq(&self, other: &dyn RewriteMethod) -> bool {
        match other
            .as_any()
            .downcast_ref::<TopTermsScoringBooleanQueryRewrite>()
        {
            Some(other) => top_terms_rewrite_eq(self, other),
            None => false,
        }
    }

    fn method_hash(&self) -> u64 {
        top_terms_rewrite_hash(self)
    }
}

/// The top-level builder of [`TopTermsBoostOnlyBooleanQueryRewrite`].
///
/// Equivalent to that class's `getTopLevelBuilder`, `addClause` and `build`
/// overrides taken together; see
/// [`TopLevelBuilder`](crate::search::TopLevelBuilder) for why they live on the
/// builder here.
#[derive(Debug, Default)]
pub struct BoostOnlyBooleanQueryBuilder {
    builder: BooleanQueryBuilder,
}

impl BoostOnlyBooleanQueryBuilder {
    /// Creates an empty builder.
    pub fn new() -> Self {
        Self::default()
    }
}

impl TopLevelBuilder for BoostOnlyBooleanQueryBuilder {
    fn add_clause(
        &mut self,
        term: Term,
        _doc_freq: i32,
        boost: f32,
        states: Option<Arc<TermStates>>,
    ) -> Result<()> {
        let tq: Arc<dyn Query> = match states {
            Some(states) => Arc::new(TermQuery::with_states(term, states)),
            None => Arc::new(TermQuery::new(term)),
        };
        let q: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(tq));
        self.builder
            .add(Arc::new(BoostQuery::new(q, boost)?), Occur::SHOULD)?;
        Ok(())
    }

    fn build(self: Box<Self>) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(self.builder.build()))
    }
}

/// A rewrite method that translates each term into an
/// [`Occur::SHOULD`](crate::search::Occur) clause of a boolean query whose
/// scores are only the boost, using only the top-scoring terms so that the
/// maximum clause count cannot overflow.
///
/// Equivalent to the `final class
/// MultiTermQuery.TopTermsBoostOnlyBooleanQueryRewrite`.
#[derive(Debug, Clone, Copy)]
pub struct TopTermsBoostOnlyBooleanQueryRewrite {
    size: i32,
}

impl TopTermsBoostOnlyBooleanQueryRewrite {
    /// Creates a rewrite for at most `size` terms.
    ///
    /// Equivalent to `TopTermsBoostOnlyBooleanQueryRewrite(int)`. When
    /// [`IndexSearcher::get_max_clause_count`] is smaller than `size`, it is
    /// used instead.
    pub fn new(size: i32) -> Self {
        Self { size }
    }
}

impl TermCollectingRewrite for TopTermsBoostOnlyBooleanQueryRewrite {
    fn get_top_level_builder(&self) -> Result<Box<dyn TopLevelBuilder>> {
        Ok(Box::new(BoostOnlyBooleanQueryBuilder::new()))
    }
}

impl TopTermsRewrite for TopTermsBoostOnlyBooleanQueryRewrite {
    fn get_size(&self) -> i32 {
        self.size
    }

    fn get_max_size(&self) -> i32 {
        IndexSearcher::get_max_clause_count()
    }
}

impl RewriteMethod for TopTermsBoostOnlyBooleanQueryRewrite {
    fn rewrite(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        top_terms_rewrite(self, index_searcher, &*query)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_eq(&self, other: &dyn RewriteMethod) -> bool {
        match other
            .as_any()
            .downcast_ref::<TopTermsBoostOnlyBooleanQueryRewrite>()
        {
            Some(other) => top_terms_rewrite_eq(self, other),
            None => false,
        }
    }

    fn method_hash(&self) -> u64 {
        top_terms_rewrite_hash(self)
    }
}

/// The top-level builder of [`TopTermsBlendedFreqScoringRewrite`].
///
/// Equivalent to that class's `getTopLevelBuilder`, `addClause` and `build`
/// overrides taken together.
#[derive(Debug)]
pub struct BlendedFreqScoringBuilder {
    builder: BlendedTermQueryBuilder,
}

impl Default for BlendedFreqScoringBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl BlendedFreqScoringBuilder {
    /// Creates a builder whose blended query uses
    /// [`boolean_rewrite`](crate::search::boolean_rewrite).
    pub fn new() -> Self {
        let mut builder = BlendedTermQueryBuilder::new();
        builder.set_rewrite_method(boolean_rewrite());
        Self { builder }
    }
}

impl TopLevelBuilder for BlendedFreqScoringBuilder {
    fn add_clause(
        &mut self,
        term: Term,
        _doc_count: i32,
        boost: f32,
        states: Option<Arc<TermStates>>,
    ) -> Result<()> {
        self.builder.add(term, boost, states)?;
        Ok(())
    }

    fn build(self: Box<Self>) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(self.builder.build()))
    }
}

/// A rewrite method that translates each term into an
/// [`Occur::SHOULD`](crate::search::Occur) clause of a boolean query, adjusting
/// the frequencies used for scoring so that they are blended across the terms.
///
/// Equivalent to the `final class
/// MultiTermQuery.TopTermsBlendedFreqScoringRewrite`. Without the blending the
/// rarest term typically ranks highest, which is often not useful — in the set
/// of expanded terms of a [`FuzzyQuery`](crate::search::FuzzyQuery), for
/// instance. It uses only the top-scoring terms, so the maximum clause count
/// cannot overflow.
#[derive(Debug, Clone, Copy)]
pub struct TopTermsBlendedFreqScoringRewrite {
    size: i32,
}

impl TopTermsBlendedFreqScoringRewrite {
    /// Creates a rewrite for at most `size` terms.
    ///
    /// Equivalent to `TopTermsBlendedFreqScoringRewrite(int)`. When
    /// [`IndexSearcher::get_max_clause_count`] is smaller than `size`, it is
    /// used instead.
    pub fn new(size: i32) -> Self {
        Self { size }
    }
}

impl TermCollectingRewrite for TopTermsBlendedFreqScoringRewrite {
    fn get_top_level_builder(&self) -> Result<Box<dyn TopLevelBuilder>> {
        Ok(Box::new(BlendedFreqScoringBuilder::new()))
    }
}

impl TopTermsRewrite for TopTermsBlendedFreqScoringRewrite {
    fn get_size(&self) -> i32 {
        self.size
    }

    fn get_max_size(&self) -> i32 {
        IndexSearcher::get_max_clause_count()
    }
}

impl RewriteMethod for TopTermsBlendedFreqScoringRewrite {
    fn rewrite(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        top_terms_rewrite(self, index_searcher, &*query)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_eq(&self, other: &dyn RewriteMethod) -> bool {
        match other
            .as_any()
            .downcast_ref::<TopTermsBlendedFreqScoringRewrite>()
        {
            Some(other) => top_terms_rewrite_eq(self, other),
            None => false,
        }
    }

    fn method_hash(&self) -> u64 {
        top_terms_rewrite_hash(self)
    }
}
