//! Matching terms against a finite-state machine, ported from
//! `org.apache.lucene.search.AutomatonQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::Result;
use crate::index::{Term, Terms, TermsEnum};
use crate::search::index_searcher::IndexSearcher;
use crate::search::multi_term_query::{
    constant_score_blended_rewrite, multi_term_query_eq, multi_term_query_hash, multi_term_rewrite,
    MultiTermQuery, RewriteMethod,
};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::util::attribute::AttributeSource;
use crate::util::automaton::{
    Automata, Automaton, AutomatonType, ByteRunAutomaton, CompiledAutomaton,
};
use crate::util::{Accountable, RamUsageEstimator};

/// A [`Query`] that matches terms against a finite-state machine.
///
/// Equivalent to `org.apache.lucene.search.AutomatonQuery`, which implements
/// [`Accountable`]. It matches documents containing terms accepted by a given
/// finite-state machine; the automaton can be built with the
/// [`crate::util::automaton`] API, from a regular expression with
/// [`RegexpQuery`](crate::search::RegexpQuery), or from the standard Lucene
/// wildcard syntax with [`WildcardQuery`](crate::search::WildcardQuery).
///
/// When the query is executed it enumerates the term dictionary in an
/// intelligent way, to reduce the number of comparisons: the regular expression
/// `[dl]og?` makes approximately four comparisons — `do`, `dog`, `lo` and
/// `log`.
#[derive(Debug, Clone)]
pub struct AutomatonQuery {
    field: String,
    rewrite_method: Arc<dyn RewriteMethod>,
    /// The automaton to match index terms against.
    automaton: Automaton,
    /// **Divergence from Lucene 10.5.0.** Java stores the `CompiledAutomaton`
    /// in a plain field. This port's [`CompiledAutomaton`] may hold an
    /// [`NFARunAutomaton`](crate::util::automaton::NFARunAutomaton), whose
    /// on-demand determinization cache is a `RefCell`, so the compiled
    /// automaton is not `Sync` while a [`Query`] must be. The lock restores
    /// that: it is taken only to read the compiled form and to build a terms
    /// enum, which copies what it needs, so no work is serialised beyond that
    /// copy. Java documents the very same object as *not* thread-safe and warns
    /// against using an executor with a non-determinized `RegexpQuery`; here it
    /// is simply safe. It is shared behind an [`Arc`] so that a clone of the
    /// query shares the one compiled automaton, as every reference to a Java
    /// query object does.
    compiled: Arc<Mutex<CompiledAutomaton>>,
    /// The term containing the field, and possibly some pattern structure.
    term: Term,
    automaton_is_binary: bool,
    /// Cached, as in Java.
    ram_bytes_used: i64,
}

/// The shallow size of an [`AutomatonQuery`], standing in for Java's
/// `RamUsageEstimator.shallowSizeOfInstance(AutomatonQuery.class)`.
const BASE_RAM_BYTES: i64 =
    6 * RamUsageEstimator::NUM_BYTES_OBJECT_REF + RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + 8;

impl AutomatonQuery {
    /// Creates a query from an [`Automaton`], with
    /// [`constant_score_blended_rewrite`] and a non-binary automaton.
    ///
    /// Equivalent to `AutomatonQuery(Term, Automaton)`. The term carries the
    /// field and possibly some pattern structure; its text is ignored. Terms
    /// accepted by the automaton are considered a match.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while compiling the automaton.
    pub fn new(term: Term, automaton: Automaton) -> Result<Self> {
        Self::with_binary(term, automaton, false)
    }

    /// Creates a query from an [`Automaton`], with
    /// [`constant_score_blended_rewrite`].
    ///
    /// Equivalent to `AutomatonQuery(Term, Automaton, boolean)`. When
    /// `is_binary` is set, the automaton is already binary and does not go
    /// through the UTF-32 to UTF-8 conversion.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while compiling the automaton.
    pub fn with_binary(term: Term, automaton: Automaton, is_binary: bool) -> Result<Self> {
        Self::with_rewrite_method(term, automaton, is_binary, constant_score_blended_rewrite())
    }

    /// Creates a query from an [`Automaton`].
    ///
    /// Equivalent to
    /// `AutomatonQuery(Term, Automaton, boolean, RewriteMethod)`.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while compiling the automaton.
    pub fn with_rewrite_method(
        term: Term,
        automaton: Automaton,
        is_binary: bool,
        rewrite_method: Arc<dyn RewriteMethod>,
    ) -> Result<Self> {
        let compiled = Arc::new(Mutex::new(CompiledAutomaton::new(
            automaton.clone(),
            false,
            true,
            is_binary,
        )?));
        // Java sums `term.ramBytesUsed()`, `automaton.ramBytesUsed()` and
        // `compiled.ramBytesUsed()`. Neither `Term`, `Automaton` nor
        // `CompiledAutomaton` reports its heap usage in this port, so the two
        // automata are estimated from their state and transition counts and the
        // term from its bytes. The figure is only ever an estimate, used for
        // cache accounting.
        let automaton_bytes = RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + 4 * i64::from(automaton.get_num_states())
            + 12 * i64::from(automaton.get_num_transitions_total());
        let term_bytes = RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + term.field().len() as i64
            + term.bytes().length as i64;
        let ram_bytes_used = BASE_RAM_BYTES + term_bytes + 2 * automaton_bytes;
        Ok(Self {
            field: term.field().to_string(),
            rewrite_method,
            automaton,
            compiled,
            term,
            automaton_is_binary: is_binary,
            ram_bytes_used,
        })
    }

    /// Returns the automaton used to create this query.
    ///
    /// Equivalent to `AutomatonQuery.getAutomaton()`.
    pub fn get_automaton(&self) -> &Automaton {
        &self.automaton
    }

    /// Returns the compiled automaton.
    ///
    /// Equivalent to `AutomatonQuery.getCompiled()`; the guard is what the
    /// lock described on the field requires.
    pub fn get_compiled(&self) -> MutexGuard<'_, CompiledAutomaton> {
        self.lock_compiled()
    }

    fn lock_compiled(&self) -> MutexGuard<'_, CompiledAutomaton> {
        self.compiled
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether this is a binary (byte) oriented automaton.
    ///
    /// Equivalent to `AutomatonQuery.isAutomatonBinary()`.
    pub fn is_automaton_binary(&self) -> bool {
        self.automaton_is_binary
    }

    /// Returns the term this query was built from.
    ///
    /// Equivalent to reading the `protected final Term term` field.
    pub fn term(&self) -> &Term {
        &self.term
    }

    /// Returns the field name for this query.
    ///
    /// Equivalent to the `final MultiTermQuery.getField()`.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the rewrite method.
    ///
    /// Equivalent to `MultiTermQuery.getRewriteMethod()`.
    pub fn rewrite_method(&self) -> &Arc<dyn RewriteMethod> {
        &self.rewrite_method
    }

    /// Returns the terms enum the compiled automaton drives.
    ///
    /// Equivalent to
    /// `AutomatonQuery.getTermsEnum(Terms, AttributeSource)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while intersecting the terms
    /// dictionary.
    pub fn terms_enum(&self, terms: &dyn Terms) -> Result<Box<dyn TermsEnum>> {
        self.lock_compiled().get_terms_enum(terms)
    }
}

/// Hashes a [`Term`], standing in for Java's `Term.hashCode()`.
///
/// Java's is `31 * field.hashCode() + bytes.hashCode()`; this port's [`Term`]
/// has no [`Hash`] implementation, so its two components are hashed here.
pub fn term_hash(term: &Term) -> u64 {
    let mut hasher = DefaultHasher::new();
    term.field().hash(&mut hasher);
    term.bytes().slice().hash(&mut hasher);
    hasher.finish()
}

/// Hashes a [`CompiledAutomaton`], standing in for Java's
/// `CompiledAutomaton.hashCode()`.
///
/// **Divergence from Lucene 10.5.0.** Java mixes the type, the singleton term
/// and `runAutomaton.hashCode()`, which is built from the run automaton's
/// alphabet size, number of interval points and number of states. Those fields
/// are private on this port's `RunAutomaton` and have no accessor, so the hash
/// is built from the type and the singleton term alone. A weaker hash is still
/// a correct one — [`CompiledAutomaton`]s that compare equal still hash equal —
/// and only the distribution of hash buckets changes.
pub fn compiled_automaton_hash(compiled: &CompiledAutomaton) -> u64 {
    let prime = 31u64;
    let mut result = 1u64;
    let mut hasher = DefaultHasher::new();
    match compiled.automaton_type {
        AutomatonType::None => 0u8.hash(&mut hasher),
        AutomatonType::All => 1u8.hash(&mut hasher),
        AutomatonType::Single => 2u8.hash(&mut hasher),
        AutomatonType::Normal => 3u8.hash(&mut hasher),
    }
    result = prime.wrapping_mul(result).wrapping_add(hasher.finish());
    let mut hasher = DefaultHasher::new();
    match compiled.term.as_ref() {
        None => 0usize.hash(&mut hasher),
        Some(term) => term.slice().hash(&mut hasher),
    }
    prime.wrapping_mul(result).wrapping_add(hasher.finish())
}

/// The hash code shared by every automaton query.
///
/// Equivalent to `AutomatonQuery.hashCode()`. `outer` is the query itself, so
/// that a subclass contributes its own class hash exactly as Java's
/// `super.hashCode()` chain does.
pub fn automaton_query_hash(outer: &dyn MultiTermQuery, inner: &AutomatonQuery) -> u64 {
    let prime = 31u64;
    let mut result = multi_term_query_hash(outer);
    result = prime
        .wrapping_mul(result)
        .wrapping_add(compiled_automaton_hash(&inner.lock_compiled()));
    prime
        .wrapping_mul(result)
        .wrapping_add(term_hash(&inner.term))
}

/// The equality shared by every automaton query.
///
/// Equivalent to `AutomatonQuery.equals(Object)` beyond its `super.equals`
/// call, which is [`multi_term_query_eq`]; the caller has already established
/// that the classes match.
pub fn automaton_query_eq(
    outer_a: &dyn MultiTermQuery,
    a: &AutomatonQuery,
    outer_b: &dyn MultiTermQuery,
    b: &AutomatonQuery,
) -> bool {
    if !multi_term_query_eq(outer_a, outer_b) {
        return false;
    }
    // Locking the same mutex twice would deadlock, and two clones of one query
    // share it; comparing the pointers first both short-circuits that and
    // answers `true`, which is what comparing the contents would answer.
    let compiled_eq =
        Arc::ptr_eq(&a.compiled, &b.compiled) || *a.lock_compiled() == *b.lock_compiled();
    compiled_eq && a.term == b.term
}

/// Recurses through an automaton query.
///
/// Equivalent to `AutomatonQuery.visit(QueryVisitor)`, which delegates to
/// `CompiledAutomaton.visit(QueryVisitor, Query, String)`.
///
/// **Divergence from Lucene 10.5.0.** `CompiledAutomaton.visit` belongs to
/// `org.apache.lucene.util.automaton` and is not part of this port's
/// [`CompiledAutomaton`], so the same four-way switch is written here. The
/// visitor sees exactly the same calls.
pub fn automaton_query_visit(
    parent: &dyn Query,
    inner: &AutomatonQuery,
    visitor: &mut dyn QueryVisitor,
) {
    if !visitor.accept_field(&inner.field) {
        return;
    }
    // `CompiledAutomaton.visit` re-checks `acceptField` before switching.
    if !visitor.accept_field(&inner.field) {
        return;
    }
    let (automaton_type, run_automaton, single_term) = {
        let compiled = inner.lock_compiled();
        (
            compiled.automaton_type,
            compiled.run_automaton.clone(),
            compiled.term.clone(),
        )
    };
    match automaton_type {
        AutomatonType::Normal => {
            visitor.consume_terms_matching(parent, &inner.field, &|| {
                run_automaton
                    .clone()
                    .unwrap_or_else(empty_byte_run_automaton)
            });
        }
        AutomatonType::None => {}
        AutomatonType::All => {
            visitor.consume_terms_matching(parent, &inner.field, &|| {
                CompiledAutomaton::any_string_run_automaton()
                    .unwrap_or_else(|_| empty_byte_run_automaton())
            });
        }
        AutomatonType::Single => {
            let term = match single_term.as_ref() {
                Some(term) => Term::new(inner.field.clone(), term.clone()),
                None => Term::empty(inner.field.clone()),
            };
            visitor.consume_terms(parent, std::slice::from_ref(&term));
        }
    }
}

/// The run automaton that accepts nothing, used where Java cannot fail.
///
/// `QueryVisitor.consumeTermsMatching` takes a supplier that cannot report an
/// error, while this port's [`ByteRunAutomaton`] constructors return a
/// [`Result`]. The two calls that can fail here — cloning a compiled
/// `AutomatonType::Normal` automaton, which always has a run automaton, and
/// building the "any string" automaton, which is a constant — cannot actually
/// fail, so an automaton accepting nothing stands in for the impossible case
/// rather than a panic.
fn empty_byte_run_automaton() -> ByteRunAutomaton {
    ByteRunAutomaton::new(Automata::make_empty(), true)
        .expect("INVARIANT: the empty automaton is deterministic and binary")
}

impl Accountable for AutomatonQuery {
    fn ram_bytes_used(&self) -> i64 {
        self.ram_bytes_used
    }
}

impl Query for AutomatonQuery {
    fn is_multi_term_query(&self) -> bool {
        true
    }

    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        if self.term.field() != field {
            buffer.push_str(self.term.field());
            buffer.push(':');
        }
        buffer.push_str("AutomatonQuery");
        buffer.push_str(" {");
        buffer.push('\n');
        // **Divergence from Lucene 10.5.0.** Java appends
        // `automaton.toString()`, and `Automaton` does not override
        // `Object.toString()`, so the text is the JVM identity string
        // (`...Automaton@1b6d3586`) — neither reproducible nor informative.
        // This port appends the dot rendering instead. Only display text
        // changes.
        buffer.push_str(&self.automaton.to_dot());
        buffer.push('}');
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        automaton_query_visit(self, self, visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        multi_term_rewrite(self, index_searcher)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<AutomatonQuery>() else {
            return false;
        };
        automaton_query_eq(self, self, other, other)
    }

    fn query_hash(&self) -> u64 {
        automaton_query_hash(self, self)
    }
}

impl MultiTermQuery for AutomatonQuery {
    fn get_field(&self) -> &str {
        &self.field
    }

    fn get_rewrite_method(&self) -> Arc<dyn RewriteMethod> {
        Arc::clone(&self.rewrite_method)
    }

    fn get_terms_enum_with_atts(
        &self,
        terms: &Arc<dyn Terms>,
        _atts: &mut AttributeSource,
    ) -> Result<Box<dyn TermsEnum>> {
        self.terms_enum(&**terms)
    }

    fn as_query(&self) -> &dyn Query {
        self
    }

    fn to_multi_term_query_arc(&self) -> Arc<dyn MultiTermQuery> {
        Arc::new(self.clone())
    }

    fn to_query_arc(&self) -> Arc<dyn Query> {
        Arc::new(self.clone())
    }

    fn accountable_ram_bytes_used(&self) -> Option<i64> {
        Some(self.ram_bytes_used)
    }
}
