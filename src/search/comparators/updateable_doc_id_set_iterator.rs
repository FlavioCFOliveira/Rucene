//! Replaceable iteration, ported from
//! `org.apache.lucene.search.comparators.UpdateableDocIdSetIterator`.

#![deny(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::Result;
use crate::search::doc_id_set_iterator::{empty, DocIdSetIterator};
use crate::util::FixedBitSet;

/// The state two handles onto the same updateable iterator share.
struct Shared {
    /// The current doc ID and the iterator currently being delegated to.
    ///
    /// Equivalent to the `protected int doc` inherited from
    /// `AbstractDocIdSetIterator` and the `private DocIdSetIterator in` field.
    state: RefCell<State>,
    /// An iterator installed by a re-entrant [`UpdateableDocIdSetIterator::update`]
    /// call — one made from inside the delegate's own `advance` — which is
    /// installed as soon as that call returns.
    pending: RefCell<Option<Box<dyn DocIdSetIterator>>>,
}

struct State {
    doc: i32,
    inner: Box<dyn DocIdSetIterator>,
}

/// A [`DocIdSetIterator`] whose delegate can be replaced at any time.
///
/// Equivalent to `org.apache.lucene.search.comparators.UpdateableDocIdSetIterator`,
/// a package-private `final` class. It is public here because Rust has no
/// package visibility and it is what
/// [`LeafFieldComparator::competitive_iterator`](crate::search::LeafFieldComparator::competitive_iterator)
/// hands to the collector.
///
/// **Divergence from Lucene 10.5.0.** In Java the comparator keeps the very
/// object it gave the collector and calls `update(DocIdSetIterator)` on it
/// while the collector iterates. That is a shared mutable alias, which Rust
/// forbids for plain references, so the iterator is a cheap handle onto shared
/// state: [`clone`](Clone::clone) produces a second handle onto the same doc ID
/// and the same delegate, exactly as a second Java reference would. The
/// observable behaviour is unchanged.
///
/// A delegate may replace itself by calling [`update`](Self::update) from
/// inside its own `advance` — [`TermOrdValComparator`](crate::search::comparators::TermOrdValComparator)
/// does exactly that when it gives up on skipping. Java simply reassigns the
/// field; here the new delegate is parked and installed the moment the ongoing
/// call returns, which is the same point at which Java's reassignment first
/// takes effect.
#[derive(Clone)]
pub struct UpdateableDocIdSetIterator {
    shared: Rc<Shared>,
}

impl Default for UpdateableDocIdSetIterator {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for UpdateableDocIdSetIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateableDocIdSetIterator")
            .field("doc", &self.doc_id())
            .finish_non_exhaustive()
    }
}

impl UpdateableDocIdSetIterator {
    /// Creates an unpositioned iterator delegating to the empty iterator.
    ///
    /// Equivalent to `new UpdateableDocIdSetIterator()`, whose `in` field is
    /// initialised to `DocIdSetIterator.empty()`.
    pub fn new() -> Self {
        Self {
            shared: Rc::new(Shared {
                state: RefCell::new(State {
                    doc: -1,
                    inner: Box::new(empty()),
                }),
                pending: RefCell::new(None),
            }),
        }
    }

    /// Replaces the wrapped [`DocIdSetIterator`]. It does not need to be
    /// positioned on the same doc ID as this iterator.
    ///
    /// Equivalent to `UpdateableDocIdSetIterator.update(DocIdSetIterator)`.
    pub fn update(&self, iterator: Box<dyn DocIdSetIterator>) {
        match self.shared.state.try_borrow_mut() {
            Ok(mut state) => {
                state.inner = iterator;
                *self.shared.pending.borrow_mut() = None;
            }
            // A re-entrant update: the delegate is replacing itself from within
            // its own call. Park the replacement; it is installed as soon as
            // that call returns.
            Err(_) => *self.shared.pending.borrow_mut() = Some(iterator),
        }
    }

    /// Installs a delegate parked by a re-entrant [`update`](Self::update).
    fn install_pending(&self) {
        let pending = self.shared.pending.borrow_mut().take();
        if let Some(iterator) = pending {
            self.shared.state.borrow_mut().inner = iterator;
        }
    }
}

impl DocIdSetIterator for UpdateableDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.shared.state.borrow().doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.shared.state.borrow().doc;
        self.advance(doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = {
            let mut state = self.shared.state.borrow_mut();
            let mut cur_doc = state.inner.doc_id();
            if cur_doc < target {
                cur_doc = state.inner.advance(target)?;
            }
            state.doc = cur_doc;
            cur_doc
        };
        self.install_pending();
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.shared.state.borrow().inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        {
            let mut state = self.shared.state.borrow_mut();
            // `update` may have just been called.
            let doc = state.doc;
            if state.inner.doc_id() < doc {
                state.inner.advance(doc)?;
            }
            state.inner.into_bit_set(up_to, bit_set, offset)?;
            state.doc = state.inner.doc_id();
        }
        self.install_pending();
        Ok(())
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        let mut state = self.shared.state.borrow_mut();
        // `update` may have just been called.
        let doc = state.doc;
        if state.inner.doc_id() < doc {
            state.inner.advance(doc)?;
        }
        if state.inner.doc_id() == doc {
            state.inner.doc_id_run_end()
        } else {
            Ok(doc + 1)
        }
    }
}
