//! Collecting the terms a multi-term query expands to, ported from
//! `org.apache.lucene.search.TermCollectingRewrite`.

#![deny(unsafe_code)]

use std::fmt::Debug;
use std::sync::Arc;

use crate::error::Result;
use crate::index::{IndexReaderContext, LeafReaderContext, Term, Terms, TermsEnum};
use crate::search::index_searcher::IndexSearcher;
use crate::search::multi_term_query::{MultiTermQuery, RewriteMethod};
use crate::search::query::Query;
use crate::search::term_states::TermStates;
use crate::util::attribute::AttributeSource;
use crate::util::BytesRef;

/// Accumulates the expanded terms of a multi-term query into a query.
///
/// **Divergence from Lucene 10.5.0.** Java parameterises
/// `TermCollectingRewrite<B>` over the builder type and declares
/// `getTopLevelBuilder()`, `build(B)` and `addClause(B, Term, int, float,
/// TermStates)` on the rewrite. A type parameter cannot survive on a
/// `dyn RewriteMethod`, so the builder becomes a trait object and `build` and
/// `addClause` move onto it. The three concrete builders Lucene uses —
/// the boolean-query one, the boost-only one and the
/// [`BlendedTermQuery`](crate::search::BlendedTermQuery) one — are the three
/// implementations of this trait.
pub trait TopLevelBuilder {
    /// Adds a multi-term query term to the top-level query builder.
    ///
    /// Equivalent to the `protected abstract
    /// TermCollectingRewrite.addClause(B, Term, int, float, TermStates)`. Java
    /// also has a four-argument overload that passes `null` for the states;
    /// pass `None` for it.
    ///
    /// # Errors
    ///
    /// Propagates the error a full boolean query raises, and any I/O error.
    fn add_clause(
        &mut self,
        term: Term,
        doc_count: i32,
        boost: f32,
        states: Option<Arc<TermStates>>,
    ) -> Result<()>;

    /// Finalises the creation of the query from the builder.
    ///
    /// Equivalent to the `protected abstract
    /// TermCollectingRewrite.build(B)`.
    ///
    /// # Errors
    ///
    /// Propagates the error the built query's constructor raises.
    fn build(self: Box<Self>) -> Result<Arc<dyn Query>>;
}

/// The base of the rewrite methods that collect the terms a multi-term query
/// expands to.
///
/// Equivalent to the package-private abstract class
/// `org.apache.lucene.search.TermCollectingRewrite`, whose type parameter `B`
/// is the builder of the query being assembled. It is public here because Rust
/// has no package visibility; the type parameter becomes the
/// [`TopLevelBuilder`] trait object, and the two remaining abstract members —
/// `build(B)` and `addClause(B, Term, int, float, TermStates)` — move onto the
/// builder, since a `dyn RewriteMethod` cannot carry a type parameter. The
/// `collectTerms` it declares is the free function [`collect_terms`].
pub trait TermCollectingRewrite: Send + Sync + Debug {
    /// Returns a suitable builder for the top-level query holding all expanded
    /// terms.
    ///
    /// Equivalent to the `protected abstract
    /// TermCollectingRewrite.getTopLevelBuilder()`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while creating the builder.
    fn get_top_level_builder(&self) -> Result<Box<dyn TopLevelBuilder>>;
}

/// Receives the terms collected from every segment.
///
/// Equivalent to the abstract nested class
/// `TermCollectingRewrite.TermCollector`.
///
/// **Divergence from Lucene 10.5.0.** Java's `setNextEnum(TermsEnum)` stores
/// the enum on the collector and `collect(BytesRef)` reads it back. Rust cannot
/// hold that borrow across calls, so the enum is passed to every method
/// instead — the same adaptation
/// [`LeafCollector::collect`](crate::search::LeafCollector::collect) makes for
/// `setScorer`.
pub trait TermCollector {
    /// The attributes used for communication with the enum.
    ///
    /// Equivalent to reading the `public final AttributeSource attributes`
    /// field, which is shared by every segment of one rewrite.
    fn attributes(&mut self) -> &mut AttributeSource;

    /// Records the contexts the following terms are collected from.
    ///
    /// Equivalent to
    /// `TermCollector.setReaderContext(IndexReaderContext, LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates any error an implementation raises while preparing.
    fn set_reader_context(
        &mut self,
        top_reader_context: &Arc<dyn IndexReaderContext>,
        reader_context: &Arc<LeafReaderContext>,
    ) -> Result<()>;

    /// Announces the next segment's terms enum.
    ///
    /// Equivalent to `TermCollector.setNextEnum(TermsEnum)`.
    ///
    /// # Errors
    ///
    /// Propagates any error an implementation raises while preparing.
    fn set_next_enum(&mut self, terms_enum: &mut dyn TermsEnum) -> Result<()>;

    /// Collects one term; returns `false` to stop collecting.
    ///
    /// Equivalent to `TermCollector.collect(BytesRef)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the term's statistics.
    fn collect(&mut self, bytes: &BytesRef, terms_enum: &mut dyn TermsEnum) -> Result<bool>;
}

/// Visits every leaf, expands the query's terms there and feeds them to
/// `collector`.
///
/// Equivalent to the package-private
/// `TermCollectingRewrite.collectTerms(IndexReader, MultiTermQuery, TermCollector)`.
///
/// **Divergence from Lucene 10.5.0.** Java takes the `IndexReader` and reads
/// `reader.getContext()` off it; this port takes the searcher, which already
/// exposes the same top-level context and its leaves, and is what both callers
/// hold. Java also skips a segment whose terms enum is the `TermsEnum.EMPTY`
/// singleton; there is no such singleton to compare against here, and iterating
/// an [`EmptyTermsEnum`](crate::index::EmptyTermsEnum) yields no term, so the
/// segment contributes nothing either way.
///
/// # Errors
///
/// Propagates any I/O error raised while enumerating terms.
pub fn collect_terms(
    searcher: &IndexSearcher,
    rewrite_method: &dyn RewriteMethod,
    query: &dyn MultiTermQuery,
    collector: &mut dyn TermCollector,
) -> Result<()> {
    let top_reader_context = searcher.get_top_reader_context().clone();
    for context in searcher.get_leaf_contexts() {
        let Some(terms) = context.leaf_reader().terms(query.get_field())? else {
            // The field does not exist.
            continue;
        };
        let terms: Arc<dyn Terms> = Arc::from(terms);

        let mut terms_enum =
            rewrite_method.get_terms_enum(query, &terms, collector.attributes())?;

        collector.set_reader_context(&top_reader_context, context)?;
        collector.set_next_enum(&mut *terms_enum)?;
        while let Some(bytes) = terms_enum.next()? {
            if !collector.collect(&bytes, &mut *terms_enum)? {
                // Interrupt the whole term collection, so do not iterate the
                // other sub-readers either.
                return Ok(());
            }
        }
    }
    Ok(())
}
