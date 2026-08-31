//! Running several collector managers in one search, ported from
//! `org.apache.lucene.search.MultiCollectorManager`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::{Collector, CollectorManager, LeafCollector};
use crate::search::multi_collector::MultiCollector;
use crate::search::score_mode::ScoreMode;
use crate::search::weight::Weight;

/// A [`Collector`] whose concrete type has been erased but can be recovered.
///
/// **Divergence from Lucene 10.5.0.** Java's `MultiCollectorManager` holds
/// `CollectorManager<Collector, ?>[]`, hands out plain `Collector`s and, in
/// `reduce`, passes them straight back to the sub-managers — a cast that Rust
/// cannot perform on a `Box<dyn Collector>`. This type therefore stores the
/// collector as `Box<dyn Any + Send>` and remembers, as a function pointer
/// captured where the concrete type was still known, how to view it as a
/// [`Collector`]. The collector itself, and every call reaching it, are
/// unchanged.
pub struct AnyCollector {
    inner: Box<dyn Any + Send>,
    as_collector: fn(&mut (dyn Any + Send)) -> &mut dyn Collector,
    score_mode: fn(&(dyn Any + Send)) -> ScoreMode,
}

impl std::fmt::Debug for AnyCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AnyCollector")
    }
}

/// Message used where a downcast is known to succeed because the accessor was
/// created together with the value it reads.
const DOWNCAST_INVARIANT: &str =
    "INVARIANT: the accessor was captured with the concrete type of the value it reads";

impl AnyCollector {
    /// Erases the concrete type of `collector`.
    pub fn new<C: Collector + Send + 'static>(collector: C) -> Self {
        Self {
            inner: Box::new(collector),
            as_collector: |any| any.downcast_mut::<C>().expect(DOWNCAST_INVARIANT),
            score_mode: |any| {
                any.downcast_ref::<C>()
                    .expect(DOWNCAST_INVARIANT)
                    .score_mode()
            },
        }
    }

    /// Returns the erased collector.
    fn collector_mut(&mut self) -> &mut dyn Collector {
        (self.as_collector)(&mut *self.inner)
    }

    /// Returns the erased collector as a value that can be downcast back to its
    /// concrete type.
    pub fn into_any(self) -> Box<dyn Any + Send> {
        self.inner
    }
}

impl Collector for AnyCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        self.collector_mut().get_leaf_collector(context)
    }

    fn score_mode(&self) -> ScoreMode {
        (self.score_mode)(&*self.inner)
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        self.collector_mut().set_weight(weight);
    }
}

/// A [`CollectorManager`] whose collector and output types have been erased.
///
/// Equivalent to the `CollectorManager<Collector, ?>` element type of
/// `MultiCollectorManager`'s array; see [`AnyCollector`] for why the erasure
/// has to be explicit here.
pub trait ErasedCollectorManager: Send + Sync {
    /// Returns a new erased collector.
    ///
    /// Equivalent to `CollectorManager.newCollector()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while creating the collector.
    fn new_collector(&self) -> Result<AnyCollector>;

    /// Reduces the erased collectors into an erased result.
    ///
    /// Equivalent to `CollectorManager.reduce(Collection<C>)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reducing.
    fn reduce(&self, collectors: Vec<Box<dyn Any + Send>>) -> Result<Box<dyn Any>>;
}

impl<M> ErasedCollectorManager for M
where
    M: CollectorManager + Send + Sync,
    M::Collector: Send + 'static,
    M::Output: 'static,
{
    fn new_collector(&self) -> Result<AnyCollector> {
        Ok(AnyCollector::new(CollectorManager::new_collector(self)?))
    }

    fn reduce(&self, collectors: Vec<Box<dyn Any + Send>>) -> Result<Box<dyn Any>> {
        let typed: Vec<M::Collector> = collectors
            .into_iter()
            .map(|collector| {
                *collector
                    .downcast::<M::Collector>()
                    .unwrap_or_else(|_| panic!("{DOWNCAST_INVARIANT}"))
            })
            .collect();
        Ok(Box::new(CollectorManager::reduce(self, typed)?))
    }
}

/// The collector a [`MultiCollectorManager`] produces.
///
/// Equivalent to what `MultiCollector.wrap` returns inside
/// `MultiCollectorManager.newCollector()`: the single collector when there is
/// only one sub-manager, and a [`MultiCollector`] otherwise. Java expresses the
/// distinction with an `instanceof` test in `reduce`; Rust expresses it as an
/// enum, so that the sub-collectors can be recovered by value.
pub enum MultiCollectorHandle {
    /// The only sub-manager's collector, handed out unwrapped.
    Single(AnyCollector),
    /// Every sub-manager's collector, wrapped in a [`MultiCollector`].
    Multi(MultiCollector<AnyCollector>),
}

impl std::fmt::Debug for MultiCollectorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single(_) => f.write_str("MultiCollectorHandle::Single"),
            Self::Multi(collector) => f
                .debug_tuple("MultiCollectorHandle::Multi")
                .field(collector)
                .finish(),
        }
    }
}

impl MultiCollectorHandle {
    /// Returns the erased sub-collectors, one per sub-manager and in the same
    /// order.
    fn into_collectors(self) -> Vec<AnyCollector> {
        match self {
            Self::Single(collector) => vec![collector],
            Self::Multi(collector) => collector.into_collectors(),
        }
    }
}

impl Collector for MultiCollectorHandle {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        match self {
            Self::Single(collector) => collector.get_leaf_collector(context),
            Self::Multi(collector) => collector.get_leaf_collector(context),
        }
    }

    fn score_mode(&self) -> ScoreMode {
        match self {
            Self::Single(collector) => collector.score_mode(),
            Self::Multi(collector) => collector.score_mode(),
        }
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        match self {
            Self::Single(collector) => collector.set_weight(weight),
            Self::Multi(collector) => collector.set_weight(weight),
        }
    }
}

/// A [`CollectorManager`] that wraps a set of collector managers, as
/// [`MultiCollector`] does for collectors.
///
/// Equivalent to `org.apache.lucene.search.MultiCollectorManager`, whose Java
/// signature is `CollectorManager<Collector, Object[]>`; the `Object[]` result
/// becomes a `Vec<Box<dyn Any>>`, one entry per sub-manager and in the same
/// order.
pub struct MultiCollectorManager {
    collector_managers: Vec<Box<dyn ErasedCollectorManager>>,
}

impl std::fmt::Debug for MultiCollectorManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiCollectorManager")
            .field("collector_managers", &self.collector_managers.len())
            .finish()
    }
}

impl MultiCollectorManager {
    /// Wraps the given collector managers.
    ///
    /// Equivalent to
    /// `MultiCollectorManager(CollectorManager<? extends Collector, ?>...)`.
    /// Java's per-element null check has no counterpart, because a
    /// `Box<dyn ErasedCollectorManager>` cannot be null.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when no manager is supplied,
    /// which is the `IllegalArgumentException` Java throws.
    pub fn new(collector_managers: Vec<Box<dyn ErasedCollectorManager>>) -> Result<Self> {
        if collector_managers.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "There must be at least one collector manager".to_string(),
            ));
        }
        Ok(Self { collector_managers })
    }
}

impl CollectorManager for MultiCollectorManager {
    type Collector = MultiCollectorHandle;
    type Output = Vec<Box<dyn Any>>;

    fn new_collector(&self) -> Result<Self::Collector> {
        let mut collectors = Vec::with_capacity(self.collector_managers.len());
        for manager in &self.collector_managers {
            collectors.push(manager.new_collector()?);
        }
        // Equivalent to `MultiCollector.wrap(collectors)`, which returns the
        // single collector unchanged rather than wrapping it.
        if collectors.len() == 1 {
            let collector = collectors
                .pop()
                .expect("INVARIANT: the vector was just observed to hold one element");
            Ok(MultiCollectorHandle::Single(collector))
        } else {
            Ok(MultiCollectorHandle::Multi(MultiCollector::new(collectors)))
        }
    }

    fn reduce(&self, collectors: Vec<Self::Collector>) -> Result<Self::Output> {
        let mut per_manager: Vec<Vec<Box<dyn Any + Send>>> = (0..self.collector_managers.len())
            .map(|_| Vec::with_capacity(collectors.len()))
            .collect();
        for collector in collectors {
            for (index, sub) in collector.into_collectors().into_iter().enumerate() {
                per_manager[index].push(sub.into_any());
            }
        }
        let mut results = Vec::with_capacity(self.collector_managers.len());
        for (manager, reducible) in self.collector_managers.iter().zip(per_manager) {
            results.push(manager.reduce(reducible)?);
        }
        Ok(results)
    }
}
