//! In-memory doc-values field updates for a single segment.
//!
//! Holds per-document updates to a single DocValues field, buffered in RAM
//! before they are written to a generation-named file.  Each update is either
//! a numeric (`i64`) value, a binary (`BytesRef`) value, or a *reset* that
//! clears the field for a document.
//!
//! Equivalent to `org.apache.lucene.index.DocValuesFieldUpdates`.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::{LuceneError, Result};
use crate::index::DocValuesType;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::DocIdSetIterator;
use crate::util::{Accountable, BytesRef, RamUsageEstimator};

/// Page size for the internal doc-ID storage, matching Lucene's
/// `DocValuesFieldUpdates.PAGE_SIZE`.
const PAGE_SIZE: usize = 1024;

/// Mask stored in the low bit of each packed doc entry: `1` = has value,
/// `0` = reset (no value).
const HAS_VALUE_MASK: u64 = 1;
const HAS_NO_VALUE_MASK: u64 = 0;
const SHIFT: u32 = 1;

/// A single update value: either a long, a binary blob, or a reset.
#[derive(Clone, Debug)]
enum UpdateValue {
    /// Numeric update.
    Long(i64),
    /// Binary update.
    Binary(Vec<u8>),
    /// Reset (clear the field for this doc).
    Reset,
}

/// Holds updates of a single DocValues field for a set of documents within one
/// segment.
///
/// Equivalent to `org.apache.lucene.index.DocValuesFieldUpdates`.
pub struct DocValuesFieldUpdates {
    /// Field name being updated.
    pub field: String,
    /// DocValues type of the field.
    pub dv_type: DocValuesType,
    /// Generation of this update packet.
    pub del_gen: i64,
    /// Maximum document ID for this segment.
    max_doc: i32,

    // Packed doc entries: (doc_id << SHIFT) | has_value_mask
    docs: Vec<u64>,
    /// Per-entry values, indexed by position (0..size).
    values: Vec<UpdateValue>,
    /// Number of updates currently buffered.
    size: usize,
    /// Whether `finish()` has been called.
    finished: bool,
}

impl fmt::Debug for DocValuesFieldUpdates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocValuesFieldUpdates")
            .field("field", &self.field)
            .field("type", &self.dv_type)
            .field("del_gen", &self.del_gen)
            .field("size", &self.size)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl DocValuesFieldUpdates {
    /// Creates a new, empty update buffer.
    ///
    /// Equivalent to `DocValuesFieldUpdates(maxDoc, delGen, field, type)`.
    pub fn new(
        max_doc: i32,
        del_gen: i64,
        field: impl Into<String>,
        dv_type: DocValuesType,
    ) -> Self {
        Self {
            field: field.into(),
            dv_type,
            del_gen,
            max_doc,
            docs: Vec::with_capacity(PAGE_SIZE),
            values: Vec::with_capacity(PAGE_SIZE),
            size: 0,
            finished: false,
        }
    }

    /// Returns `true` if `finish()` has been called.
    pub fn get_finished(&self) -> bool {
        self.finished
    }

    /// Returns `true` if this instance contains any updates.
    pub fn any(&self) -> bool {
        self.size > 0
    }

    /// Returns the number of buffered updates.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Adds a numeric update for `doc`.
    ///
    /// # Panics
    ///
    /// Panics if this buffer is already finished or `doc >= max_doc`.
    pub fn add_long(&mut self, doc: i32, value: i64) {
        assert!(!self.finished, "already finished");
        assert!(doc < self.max_doc, "doc {doc} >= maxDoc {}", self.max_doc);
        let idx = self.add_internal(doc, HAS_VALUE_MASK);
        self.values[idx] = UpdateValue::Long(value);
    }

    /// Adds a binary update for `doc`.
    ///
    /// # Panics
    ///
    /// Panics if this buffer is already finished or `doc >= max_doc`.
    pub fn add_binary(&mut self, doc: i32, value: &BytesRef) {
        assert!(!self.finished, "already finished");
        assert!(doc < self.max_doc, "doc {doc} >= maxDoc {}", self.max_doc);
        let idx = self.add_internal(doc, HAS_VALUE_MASK);
        self.values[idx] = UpdateValue::Binary(value.slice().to_vec());
    }

    /// Resets (clears) the value for `doc`.
    ///
    /// Equivalent to `DocValuesFieldUpdates.reset(int)`.
    ///
    /// # Panics
    ///
    /// Panics if this buffer is already finished or `doc >= max_doc`.
    pub fn reset(&mut self, doc: i32) {
        assert!(!self.finished, "already finished");
        assert!(doc < self.max_doc, "doc {doc} >= maxDoc {}", self.max_doc);
        let idx = self.add_internal(doc, HAS_NO_VALUE_MASK);
        self.values[idx] = UpdateValue::Reset;
    }

    fn add_internal(&mut self, doc: i32, has_value_mask: u64) -> usize {
        if self.size == self.docs.len() {
            self.docs.reserve(PAGE_SIZE);
            self.values.reserve(PAGE_SIZE);
            for _ in 0..PAGE_SIZE {
                self.docs.push(0);
                self.values.push(UpdateValue::Reset);
            }
        }
        let packed = ((doc as u64) << SHIFT) | has_value_mask;
        self.docs[self.size] = packed;
        let idx = self.size;
        self.size += 1;
        idx
    }

    /// Freezes internal data structures and sorts updates by docID for
    /// efficient iteration.
    ///
    /// Equivalent to `DocValuesFieldUpdates.finish()`.
    ///
    /// # Errors
    ///
    /// Returns `IllegalState` if called twice.
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "DocValuesFieldUpdates already finished".to_string(),
            ));
        }
        self.finished = true;
        // Trim to actual size.
        self.docs.truncate(self.size);
        self.values.truncate(self.size);
        if self.size > 1 {
            self.sort();
        }
        Ok(())
    }

    /// Stable sort by docID (ascending), preserving insertion order for ties
    /// so the last update to a given docID wins during iteration.
    fn sort(&mut self) {
        // Create ordinal indices for stable sort.
        let mut ords: Vec<usize> = (0..self.size).collect();
        ords.sort_by(|&a, &b| {
            let doc_a = self.docs[a] >> SHIFT;
            let doc_b = self.docs[b] >> SHIFT;
            doc_a.cmp(&doc_b).then(a.cmp(&b))
        });
        let old_docs = self.docs.clone();
        let old_values: Vec<UpdateValue> = self.values.clone();
        for (new_pos, &old_pos) in ords.iter().enumerate() {
            self.docs[new_pos] = old_docs[old_pos];
            self.values[new_pos] = old_values[old_pos].clone();
        }
    }

    /// Returns an iterator over the updated documents and their values.
    ///
    /// Documents are returned in increasing docID order.  When the same docID
    /// was updated multiple times, only the last update is returned.
    ///
    /// Equivalent to `DocValuesFieldUpdates.iterator()`.
    pub fn iterator(&self) -> DocValuesFieldUpdatesIterator<'_> {
        assert!(self.finished, "call finish() first");
        DocValuesFieldUpdatesIterator::new(self.size, &self.docs, &self.values, self.del_gen)
    }

    /// Creates a deep copy of this update buffer, suitable for carrying into
    /// a merge.
    ///
    /// Equivalent to `DocValuesFieldUpdates.clone()`.
    pub fn clone_for_merge(&self) -> Self {
        Self {
            field: self.field.clone(),
            dv_type: self.dv_type,
            del_gen: self.del_gen,
            max_doc: self.max_doc,
            docs: self.docs.clone(),
            values: self.values.clone(),
            size: self.size,
            finished: true,
        }
    }
}

impl Accountable for DocValuesFieldUpdates {
    fn ram_bytes_used(&self) -> i64 {
        // Rough estimate: docs vec (u64 per entry) + values vec (header + per-entry).
        let docs_size = (self.docs.capacity() * std::mem::size_of::<u64>()) as i64;
        let values_header = RamUsageEstimator::NUM_BYTES_OBJECT_HEADER;
        let values_per_entry = std::mem::size_of::<UpdateValue>() as i64;
        let values_size = values_header + (self.values.capacity() as i64 * values_per_entry);
        // Add binary content sizes.
        let binary_size: i64 = self
            .values
            .iter()
            .map(|v| match v {
                UpdateValue::Binary(b) => b.len() as i64,
                _ => 0,
            })
            .sum();
        docs_size
            + values_size
            + binary_size
            + RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + (2 * std::mem::size_of::<i32>()) as i64
            + std::mem::size_of::<bool>() as i64
            + std::mem::size_of::<i64>() as i64
    }
}

/// An iterator over documents and their updated values.
///
/// Only documents with updates are returned, in increasing docID order.  When
/// the same docID appears multiple times, only the final update is yielded.
///
/// Equivalent to `DocValuesFieldUpdates.Iterator`.
pub struct DocValuesFieldUpdatesIterator<'a> {
    size: usize,
    docs: &'a [u64],
    values: &'a [UpdateValue],
    del_gen: i64,
    idx: usize,
    doc: i32,
    has_value: bool,
    // Cached current value (set by next_doc).
    current_long: i64,
    current_binary: BytesRef,
    current_is_binary: bool,
}

impl<'a> DocValuesFieldUpdatesIterator<'a> {
    fn new(size: usize, docs: &'a [u64], values: &'a [UpdateValue], del_gen: i64) -> Self {
        Self {
            size,
            docs,
            values,
            del_gen,
            idx: 0,
            doc: -1,
            has_value: false,
            current_long: 0,
            current_binary: BytesRef::default(),
            current_is_binary: false,
        }
    }

    /// Returns the delGen of this packet.
    pub fn del_gen(&self) -> i64 {
        self.del_gen
    }

    /// Returns `true` if the current document has a value (not a reset).
    pub fn has_value(&self) -> bool {
        self.has_value
    }

    /// Returns the long value for the current document.
    ///
    /// # Panics
    ///
    /// Panics if this iterator is not a long iterator or the current doc has
    /// no value.
    pub fn long_value(&self) -> i64 {
        assert!(self.has_value, "no value for current doc");
        assert!(!self.current_is_binary, "not a numeric iterator");
        self.current_long
    }

    /// Returns the binary value for the current document.
    ///
    /// # Panics
    ///
    /// Panics if this iterator is not a binary iterator or the current doc has
    /// no value.
    pub fn binary_value(&self) -> BytesRef {
        assert!(self.has_value, "no value for current doc");
        assert!(self.current_is_binary, "not a binary iterator");
        self.current_binary.clone()
    }
}

impl<'a> DocIdSetIterator for DocValuesFieldUpdatesIterator<'a> {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.idx >= self.size {
            self.doc = NO_MORE_DOCS;
            return Ok(NO_MORE_DOCS);
        }
        let mut long_doc = self.docs[self.idx];
        self.idx += 1;
        // Scan forward to the last update for this doc.
        while self.idx < self.size {
            let next = self.docs[self.idx];
            if (long_doc >> SHIFT) != (next >> SHIFT) {
                break;
            }
            long_doc = next;
            self.idx += 1;
        }
        self.has_value = (long_doc & HAS_VALUE_MASK) > 0;
        if self.has_value {
            let value_idx = self.idx - 1;
            match &self.values[value_idx] {
                UpdateValue::Long(v) => {
                    self.current_long = *v;
                    self.current_is_binary = false;
                }
                UpdateValue::Binary(b) => {
                    self.current_binary = BytesRef::new(b.clone());
                    self.current_is_binary = true;
                }
                UpdateValue::Reset => {
                    // Should not happen: has_value is true but value is Reset.
                    // This means the packed entry had HAS_VALUE_MASK but the
                    // value slot was Reset.  Treat as no value.
                    self.has_value = false;
                }
            }
        }
        self.doc = (long_doc >> SHIFT) as i32;
        Ok(self.doc)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "advance not supported for DocValuesFieldUpdatesIterator".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.size as i64
    }
}

/// Merge-sorts multiple iterators, one per delGen, favoring the largest delGen
/// that has updates for a given docID.
///
/// Equivalent to `DocValuesFieldUpdates.mergedIterator(Iterator[])`.
pub fn merged_iterator(subs: Vec<DocValuesFieldUpdatesIterator<'_>>) -> MergedIterator<'_> {
    if subs.is_empty() {
        return MergedIterator::empty();
    }
    if subs.len() == 1 {
        return MergedIterator::single(subs.into_iter().next().expect("subs non-empty"));
    }
    MergedIterator::merge(subs)
}

/// A merged iterator over multiple `DocValuesFieldUpdatesIterator`s.
///
/// Equivalent to the anonymous `Iterator` created by
/// `DocValuesFieldUpdates.mergedIterator`.
pub struct MergedIterator<'a> {
    // Each sub-iterator with its current docID.
    subs: Vec<MergedSub<'a>>,
    doc: i32,
    // Index into `subs` for the current top.
    top_idx: usize,
}

struct MergedSub<'a> {
    it: DocValuesFieldUpdatesIterator<'a>,
    doc: i32,
}

impl<'a> MergedIterator<'a> {
    fn empty() -> Self {
        Self {
            subs: Vec::new(),
            doc: NO_MORE_DOCS,
            top_idx: 0,
        }
    }

    fn single(mut it: DocValuesFieldUpdatesIterator<'a>) -> Self {
        // Pre-advance to the first doc, consistent with `merge`.
        let doc = it.next_doc().unwrap_or(NO_MORE_DOCS);
        if doc == NO_MORE_DOCS {
            return Self::empty();
        }
        Self {
            subs: vec![MergedSub { it, doc }],
            doc: -1,
            top_idx: 0,
        }
    }

    fn merge(subs: Vec<DocValuesFieldUpdatesIterator<'a>>) -> Self {
        let mut merged_subs: Vec<MergedSub<'a>> = Vec::with_capacity(subs.len());
        for mut s in subs.into_iter() {
            // Pre-advance each sub.
            let doc = s.next_doc().unwrap_or(NO_MORE_DOCS);
            if doc != NO_MORE_DOCS {
                merged_subs.push(MergedSub { it: s, doc });
            }
        }
        Self {
            subs: merged_subs,
            doc: -1,
            top_idx: 0,
        }
    }

    /// Returns the delGen of the current top sub-iterator.
    pub fn del_gen(&self) -> i64 {
        if self.top_idx < self.subs.len() {
            self.subs[self.top_idx].it.del_gen()
        } else {
            -1
        }
    }

    /// Returns `true` if the current document has a value.
    pub fn has_value(&self) -> bool {
        if self.top_idx < self.subs.len() {
            self.subs[self.top_idx].it.has_value()
        } else {
            false
        }
    }

    /// Returns the long value of the current top sub-iterator.
    pub fn long_value(&self) -> i64 {
        self.subs[self.top_idx].it.long_value()
    }

    /// Returns the binary value of the current top sub-iterator.
    pub fn binary_value(&self) -> BytesRef {
        self.subs[self.top_idx].it.binary_value()
    }
}

impl<'a> DocIdSetIterator for MergedIterator<'a> {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.subs.is_empty() {
                self.doc = NO_MORE_DOCS;
                return Ok(NO_MORE_DOCS);
            }
            // Find the minimum docID, breaking ties by largest delGen.
            let mut min_idx = 0;
            let mut min_doc = self.subs[0].doc;
            for i in 1..self.subs.len() {
                if self.subs[i].doc < min_doc
                    || (self.subs[i].doc == min_doc
                        && self.subs[i].it.del_gen() > self.subs[min_idx].it.del_gen())
                {
                    min_idx = i;
                    min_doc = self.subs[i].doc;
                }
            }
            if min_doc == NO_MORE_DOCS {
                self.doc = NO_MORE_DOCS;
                return Ok(NO_MORE_DOCS);
            }
            if min_doc != self.doc {
                self.doc = min_doc;
                self.top_idx = min_idx;
                // Advance all NON-TOP subs at this docID past it, so the top
                // sub remains positioned at the current doc for value reads.
                let current = self.doc;
                let mut i = 0;
                while i < self.subs.len() {
                    if i != min_idx && self.subs[i].doc == current {
                        let next = self.subs[i].it.next_doc().unwrap_or(NO_MORE_DOCS);
                        self.subs[i].doc = next;
                        if next == NO_MORE_DOCS {
                            self.subs.swap_remove(i);
                            if min_idx > i {
                                min_idx -= 1;
                            }
                            continue;
                        }
                    }
                    i += 1;
                }
                self.top_idx = min_idx;
                return Ok(self.doc);
            }
            // min_doc == self.doc: advance the top sub past the current doc.
            let next = self.subs[min_idx].it.next_doc().unwrap_or(NO_MORE_DOCS);
            self.subs[min_idx].doc = next;
            if next == NO_MORE_DOCS {
                self.subs.swap_remove(min_idx);
            }
        }
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "advance not supported for MergedIterator".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.subs.iter().map(|s| s.it.cost()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_updates_finish_and_iterate() {
        let mut updates = DocValuesFieldUpdates::new(100, 5, "count", DocValuesType::NUMERIC);
        updates.add_long(3, 30);
        updates.add_long(1, 10);
        updates.add_long(3, 31); // overwrite doc 3
        updates.finish().unwrap();

        let mut it = updates.iterator();
        assert_eq!(it.next_doc().unwrap(), 1);
        assert!(it.has_value());
        assert_eq!(it.long_value(), 10);

        assert_eq!(it.next_doc().unwrap(), 3);
        assert!(it.has_value());
        assert_eq!(it.long_value(), 31); // last update wins

        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn binary_updates_finish_and_iterate() {
        let mut updates = DocValuesFieldUpdates::new(100, 2, "label", DocValuesType::BINARY);
        updates.add_binary(0, &BytesRef::new(b"hello".to_vec()));
        updates.add_binary(2, &BytesRef::new(b"world".to_vec()));
        updates.finish().unwrap();

        let mut it = updates.iterator();
        assert_eq!(it.next_doc().unwrap(), 0);
        assert_eq!(it.binary_value().slice(), b"hello");
        assert_eq!(it.next_doc().unwrap(), 2);
        assert_eq!(it.binary_value().slice(), b"world");
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn reset_clears_value() {
        let mut updates = DocValuesFieldUpdates::new(100, 1, "count", DocValuesType::NUMERIC);
        updates.add_long(5, 50);
        updates.reset(5); // reset after add — last wins
        updates.finish().unwrap();

        let mut it = updates.iterator();
        assert_eq!(it.next_doc().unwrap(), 5);
        assert!(!it.has_value());
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn finish_twice_errors() {
        let mut updates = DocValuesFieldUpdates::new(100, 1, "f", DocValuesType::NUMERIC);
        updates.add_long(0, 1);
        updates.finish().unwrap();
        assert!(updates.finish().is_err());
    }

    #[test]
    fn any_and_size() {
        let mut updates = DocValuesFieldUpdates::new(100, 1, "f", DocValuesType::NUMERIC);
        assert!(!updates.any());
        assert_eq!(updates.size(), 0);
        updates.add_long(0, 1);
        updates.add_long(1, 2);
        assert!(updates.any());
        assert_eq!(updates.size(), 2);
    }

    #[test]
    fn merged_iterator_favors_largest_del_gen() {
        let mut u1 = DocValuesFieldUpdates::new(100, 1, "f", DocValuesType::NUMERIC);
        u1.add_long(0, 100);
        u1.add_long(2, 200);
        u1.finish().unwrap();

        let mut u2 = DocValuesFieldUpdates::new(100, 5, "f", DocValuesType::NUMERIC);
        u2.add_long(0, 999); // larger delGen, should win for doc 0
        u2.add_long(1, 111);
        u2.finish().unwrap();

        let i1 = u1.iterator();
        let i2 = u2.iterator();
        let mut merged = merged_iterator(vec![i1, i2]);

        assert_eq!(merged.next_doc().unwrap(), 0);
        assert!(merged.has_value());
        assert_eq!(merged.long_value(), 999); // delGen 5 wins

        assert_eq!(merged.next_doc().unwrap(), 1);
        assert_eq!(merged.long_value(), 111);

        assert_eq!(merged.next_doc().unwrap(), 2);
        assert_eq!(merged.long_value(), 200);

        assert_eq!(merged.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn merged_iterator_single_sub() {
        let mut u = DocValuesFieldUpdates::new(100, 1, "f", DocValuesType::NUMERIC);
        u.add_long(3, 30);
        u.add_long(7, 70);
        u.finish().unwrap();

        let mut merged = merged_iterator(vec![u.iterator()]);
        assert_eq!(merged.next_doc().unwrap(), 3);
        assert_eq!(merged.long_value(), 30);
        assert_eq!(merged.next_doc().unwrap(), 7);
        assert_eq!(merged.long_value(), 70);
        assert_eq!(merged.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn ram_bytes_used_is_positive() {
        let mut u = DocValuesFieldUpdates::new(100, 1, "f", DocValuesType::NUMERIC);
        u.add_long(0, 1);
        u.finish().unwrap();
        assert!(u.ram_bytes_used() > 0);
    }
}
