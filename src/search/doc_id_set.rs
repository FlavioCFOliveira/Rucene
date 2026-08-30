//! Doc-id sets ported from `org.apache.lucene.search.DocIdSet`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`DocIdSet`] | `org.apache.lucene.search.DocIdSet` |
//! | [`EmptyDocIdSet`] | `DocIdSet.EMPTY` |
//! | [`AllDocIdSet`] | `DocIdSet.all(int)` |
//!
//! A `DocIdSet` holds a set of document identifiers. Implementations only have
//! to provide [`DocIdSet::iterator`]; [`DocIdSet::bits`] is an optional
//! random-access view that Lucene 10.5.0 already marks deprecated but still
//! exposes, so it is reproduced here.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::doc_id_set_iterator::{
    self, AllDocIdSetIterator, DocIdSetIterator, EmptyDocIdSetIterator,
};
use crate::util::{Accountable, Bits, MatchAllBits};

/// A set of document identifiers.
///
/// Port of `org.apache.lucene.search.DocIdSet`.
pub trait DocIdSet: Accountable {
    /// Returns an iterator over the documents in this set.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying storage cannot be read.
    fn iterator(&self) -> Result<Box<dyn DocIdSetIterator>>;

    /// Optionally provides a [`Bits`] interface for random access to matching
    /// documents.
    ///
    /// Returns `None` when this set does not support random access. As in
    /// Lucene, `None` does **not** mean that no document matches.
    ///
    /// Lucene 10.5.0 marks this method deprecated ("this method is redundant
    /// and will be removed"); it is kept here for parity.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying storage cannot be read.
    fn bits(&self) -> Result<Option<Box<dyn Bits>>> {
        Ok(None)
    }
}

/// The empty [`DocIdSet`].
///
/// Port of `DocIdSet.EMPTY`.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyDocIdSet;

impl Accountable for EmptyDocIdSet {
    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

impl DocIdSet for EmptyDocIdSet {
    fn iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        Ok(Box::new(EmptyDocIdSetIterator::new()))
    }

    /// Deliberately provides no random access: the set is 100% sparse, so the
    /// iterator exits faster. This mirrors `DocIdSet.EMPTY`.
    fn bits(&self) -> Result<Option<Box<dyn Bits>>> {
        Ok(None)
    }
}

/// A [`DocIdSet`] matching every doc id below `max_doc`.
///
/// Port of the anonymous class returned by `DocIdSet.all(int)`, which Lucene
/// 10.5.0 marks deprecated ("no longer needed since Query and Filter were
/// merged").
#[derive(Debug, Clone, Copy)]
pub struct AllDocIdSet {
    max_doc: i32,
}

impl AllDocIdSet {
    /// Creates a set matching `[0, max_doc)`.
    pub fn new(max_doc: i32) -> Self {
        Self { max_doc }
    }

    /// Returns the exclusive upper bound of this set.
    pub fn max_doc(&self) -> i32 {
        self.max_doc
    }
}

impl Accountable for AllDocIdSet {
    fn ram_bytes_used(&self) -> i64 {
        // `Integer.BYTES` in Lucene.
        4
    }
}

impl DocIdSet for AllDocIdSet {
    fn iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        Ok(Box::new(AllDocIdSetIterator::new(self.max_doc)?))
    }

    fn bits(&self) -> Result<Option<Box<dyn Bits>>> {
        Ok(Some(Box::new(MatchAllBits::new(
            self.max_doc.max(0) as usize
        ))))
    }
}

/// Returns the empty [`DocIdSet`], equivalent to `DocIdSet.EMPTY`.
pub fn empty() -> EmptyDocIdSet {
    EmptyDocIdSet
}

/// Returns a [`DocIdSet`] that matches all doc ids up to `max_doc` (exclusive).
///
/// Equivalent to `DocIdSet.all(int)`.
pub fn all(max_doc: i32) -> AllDocIdSet {
    AllDocIdSet::new(max_doc)
}

/// Re-exported so that callers of [`DocIdSet::iterator`] can build the
/// canonical empty iterator without importing the sibling module.
pub use doc_id_set_iterator::empty as empty_iterator;
