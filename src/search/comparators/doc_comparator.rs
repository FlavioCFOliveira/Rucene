//! Doc-ID sorting, ported from
//! `org.apache.lucene.search.comparators.DocComparator`.

#![deny(unsafe_code)]

use std::any::Any;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::comparators::updateable_doc_id_set_iterator::UpdateableDocIdSetIterator;
use crate::search::doc_id_set_iterator::{all, empty, range, DocIdSetIterator};
use crate::search::field_comparator::{java_int_compare, FieldComparator, SortValue};
use crate::search::leaf_field_comparator::LeafFieldComparator;
use crate::search::pruning::Pruning;
use crate::search::scorable::Scorable;

/// The per-segment state of a [`DocComparator`].
///
/// Equivalent to the fields of the private inner class
/// `DocComparator.DocLeafComparator`; see the adaptation note on
/// [`FieldComparator`](crate::search::FieldComparator) for why it is a field
/// rather than a separate object.
#[derive(Debug)]
struct DocLeafState {
    doc_base: i32,
    min_doc: i32,
    max_doc: i32,
    /// The iterator that starts from the top value, or `None` when skipping is
    /// disabled.
    competitive_iterator: Option<UpdateableDocIdSetIterator>,
}

/// Comparator that sorts by ascending doc ID.
///
/// Equivalent to `org.apache.lucene.search.comparators.DocComparator`.
///
/// When sorting by `_doc` ascending, after collecting the top `N` matches and
/// enough hits, the comparator can skip all the following documents. When
/// sorting by `_doc` ascending and a "top" document is set, after which the
/// search should start, the comparator provides an iterator that can quickly
/// skip to the desired "top" document.
#[derive(Debug)]
pub struct DocComparator {
    doc_ids: Vec<i32>,
    /// Whether skipping functionality is enabled.
    enable_skipping: bool,
    bottom: i32,
    top_value: i32,
    top_value_set: bool,
    bottom_value_set: bool,
    hits_threshold_reached: bool,
    leaf: Option<DocLeafState>,
}

impl DocComparator {
    /// Creates a new comparator based on document IDs for `num_hits`.
    ///
    /// Equivalent to `new DocComparator(int, boolean, Pruning)`. Skipping is
    /// enabled when sorting by `_doc` ascending as a primary sort.
    pub fn new(num_hits: usize, reverse: bool, pruning: Pruning) -> Self {
        Self {
            doc_ids: vec![0; num_hits],
            enable_skipping: !reverse && pruning != Pruning::NONE,
            bottom: 0,
            top_value: 0,
            top_value_set: false,
            bottom_value_set: false,
            hits_threshold_reached: false,
            leaf: None,
        }
    }

    /// Equivalent to the private `DocLeafComparator.updateIterator()`.
    fn update_iterator(&mut self) -> Result<()> {
        if !self.enable_skipping || !self.hits_threshold_reached {
            return Ok(());
        }
        let Some(leaf) = self.leaf.as_ref() else {
            return Ok(());
        };
        let Some(competitive_iterator) = leaf.competitive_iterator.as_ref() else {
            return Ok(());
        };
        if self.bottom_value_set {
            // Since we have collected the top N matches, we can early terminate.
            // Early termination on _doc is currently also implemented in
            // TopFieldCollector, but that will be removed once all bulk scorers
            // use the collectors' iterators.
            competitive_iterator.update(Box::new(empty()));
        } else if self.top_value_set {
            // Skip to the desired top doc.
            if leaf.doc_base.saturating_add(leaf.max_doc) <= self.min_doc_of(leaf) {
                // Skip this segment.
                competitive_iterator.update(Box::new(empty()));
            } else {
                let mut segment_min_doc = competitive_iterator
                    .doc_id()
                    .max(leaf.min_doc - leaf.doc_base);
                // The competitive iterator may not be positioned yet.
                segment_min_doc = segment_min_doc.max(0);
                competitive_iterator.update(Box::new(range(segment_min_doc, leaf.max_doc)?));
            }
        }
        Ok(())
    }

    /// Reads the leaf's `minDoc`, which is the comparator's top value.
    fn min_doc_of(&self, leaf: &DocLeafState) -> i32 {
        leaf.min_doc
    }
}

impl LeafFieldComparator for DocComparator {
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        self.bottom = self.doc_ids[slot as usize];
        self.bottom_value_set = true;
        self.update_iterator()
    }

    fn compare_bottom(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        let doc_base = self.leaf.as_ref().map_or(0, |leaf| leaf.doc_base);
        // No overflow risk because doc IDs are non-negative.
        Ok(self.bottom - (doc_base + doc))
    }

    fn compare_top(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        let doc_base = self.leaf.as_ref().map_or(0, |leaf| leaf.doc_base);
        let doc_value = doc_base + doc;
        Ok(java_int_compare(self.top_value, doc_value))
    }

    fn copy(&mut self, slot: i32, doc: i32, _scorer: &mut dyn Scorable) -> Result<()> {
        let doc_base = self.leaf.as_ref().map_or(0, |leaf| leaf.doc_base);
        self.doc_ids[slot as usize] = doc_base + doc;
        Ok(())
    }

    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        // Update the iterator on a new segment.
        self.update_iterator()
    }

    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        Ok(self.leaf.as_ref().and_then(|leaf| {
            leaf.competitive_iterator
                .as_ref()
                .map(|it| Box::new(it.clone()) as Box<dyn DocIdSetIterator>)
        }))
    }

    fn set_hits_threshold_reached(&mut self) -> Result<()> {
        self.hits_threshold_reached = true;
        self.update_iterator()
    }
}

impl FieldComparator for DocComparator {
    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        // No overflow risk because doc IDs are non-negative.
        self.doc_ids[slot1 as usize] - self.doc_ids[slot2 as usize]
    }

    fn set_top_value(&mut self, value: SortValue) {
        self.top_value = match value {
            SortValue::Int(value) => value,
            _ => 0,
        };
        self.top_value_set = true;
    }

    fn value(&self, slot: i32) -> SortValue {
        SortValue::Int(self.doc_ids[slot as usize])
    }

    fn get_leaf_comparator(&mut self, context: &LeafReaderContext) -> Result<()> {
        let doc_base = context.doc_base();
        if self.enable_skipping {
            // Skip docs before topValue, but include docs starting with
            // topValue. Including topValue is necessary when sorting on
            // [_doc, other fields] in a distributed search where there are docs
            // from different indices with the same doc ID.
            let max_doc = context.leaf_reader().max_doc();
            let competitive_iterator = UpdateableDocIdSetIterator::new();
            competitive_iterator.update(Box::new(all(max_doc)?));
            self.leaf = Some(DocLeafState {
                doc_base,
                min_doc: self.top_value,
                max_doc,
                competitive_iterator: Some(competitive_iterator),
            });
        } else {
            self.leaf = Some(DocLeafState {
                doc_base,
                min_doc: -1,
                max_doc: -1,
                competitive_iterator: None,
            });
        }
        Ok(())
    }

    fn as_leaf_comparator(&mut self) -> &mut dyn LeafFieldComparator {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
