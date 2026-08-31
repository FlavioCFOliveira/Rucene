//! `MultiDocValues` ported from `org.apache.lucene.index`.
//!
//! This module presents a composite reader's per-leaf doc values as one
//! iterator over the composite's *global* doc-ID space. Every variant does the
//! same two things:
//!
//! * it walks the leaves in order, re-basing each leaf's local doc IDs by that
//!   leaf's `docBase`, and
//! * for the ordinal-bearing variants ([`MultiSortedDocValues`] and
//!   [`MultiSortedSetDocValues`]) it also re-bases each leaf's *ordinals* into
//!   a single global ordinal space, using an
//!   [`OrdinalMap`].
//!
//! # Cost
//!
//! This is the slow path, exactly as in Lucene. Resolving the owning leaf costs
//! a binary search per `advance`, and building the ordinal map merge-sorts every
//! leaf's whole term dictionary. Code that can iterate
//! [`IndexReader::leaves`] and work in per-leaf ordinal space should do that
//! instead.
//!
//! # Ordinal mapping
//!
//! The ordinal map is built over **one entry per leaf, in leaf order**, padding
//! leaves that do not have the field with an empty doc-values instance. That
//! padding is what keeps segment numbering aligned: leaf `i` is always segment
//! `i` of the map, so `mapping.get_global_ords(i)` is the right translation
//! table for leaf `i` no matter which leaves actually carry the field.
//!
//! # Reference
//!
//! - `org.apache.lucene.index.MultiDocValues`
//! - `org.apache.lucene.index.MultiDocValues.MultiSortedDocValues`
//! - `org.apache.lucene.index.MultiDocValues.MultiSortedSetDocValues`

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::doc_values::{
    BinaryDocValues, DocValues, DocValuesIterator, NumericDocValues, OrdinalMap, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues,
};
use crate::index::index_reader::IndexReader;
use crate::index::leaf_reader::LeafReader;
use crate::index::multi_reader::reader_util;
use crate::index::reader_context::LeafReaderContext;
use crate::index::DocValuesType;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::BytesRef;

/// Acceptable overhead ratio handed to [`OrdinalMap`] construction.
///
/// Equivalent to `PackedInts.DEFAULT` (`0.25f`), the value Lucene's
/// `MultiDocValues` passes to `OrdinalMap.build`.
const DEFAULT_ACCEPTABLE_OVERHEAD_RATIO: f32 = 0.25;

/// Fetches one leaf's [`NumericDocValues`] for a field.
///
/// A function pointer rather than a closure so the two numeric variants
/// (`getNormValues` and `getNumericValues` in Java) can share one cursor type
/// without a heap allocation or a generic parameter.
type NumericFetcher = fn(&dyn LeafReader, &str) -> Result<Option<Box<dyn NumericDocValues>>>;

fn fetch_norms(reader: &dyn LeafReader, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
    reader.get_norm_values(field)
}

fn fetch_numeric(
    reader: &dyn LeafReader,
    field: &str,
) -> Result<Option<Box<dyn NumericDocValues>>> {
    reader.get_numeric_doc_values(field)
}

// ---------------------------------------------------------------------------
// MultiDocValues
// ---------------------------------------------------------------------------

/// Namespace for the composite-reader doc-values accessors.
///
/// Equivalent to `org.apache.lucene.index.MultiDocValues`. Lucene models this
/// as a final class with a private constructor and only static methods; the
/// Rust analogue is a unit struct that is never instantiated.
#[derive(Debug, Clone, Copy)]
pub struct MultiDocValues;

impl MultiDocValues {
    /// Returns a [`NumericDocValues`] over `reader`'s norms for `field`,
    /// merging the leaves on the fly, or `None` when no leaf indexes norms for
    /// the field.
    ///
    /// Equivalent to `MultiDocValues.getNormValues(IndexReader, String)`.
    ///
    /// This is a slow way to read norms; prefer
    /// [`LeafReader::get_norm_values`] per leaf.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading a leaf's norms.
    pub fn get_norm_values(
        reader: &Arc<dyn IndexReader>,
        field: &str,
    ) -> Result<Option<Box<dyn NumericDocValues>>> {
        let leaves = Arc::clone(reader).leaves();
        match leaves.len() {
            0 => return Ok(None),
            1 => return leaves[0].leaf_reader().get_norm_values(field),
            _ => {}
        }

        // Only build the merged view if at least one leaf actually has norms
        // for this field.
        let norm_found = leaves.iter().any(|leaf| {
            leaf.leaf_reader()
                .get_field_infos()
                .field_info(field)
                .is_some_and(|info| info.has_norms())
        });
        if !norm_found {
            return Ok(None);
        }

        Ok(Some(Box::new(MultiNumericDocValues::new(
            leaves,
            field,
            fetch_norms,
        ))))
    }

    /// Returns a [`NumericDocValues`] over `reader`'s numeric doc values for
    /// `field`, merging the leaves on the fly, or `None` when no leaf has
    /// [`DocValuesType::NUMERIC`] for the field.
    ///
    /// Equivalent to `MultiDocValues.getNumericValues(IndexReader, String)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading a leaf's doc values.
    pub fn get_numeric_values(
        reader: &Arc<dyn IndexReader>,
        field: &str,
    ) -> Result<Option<Box<dyn NumericDocValues>>> {
        let leaves = Arc::clone(reader).leaves();
        match leaves.len() {
            0 => return Ok(None),
            1 => return leaves[0].leaf_reader().get_numeric_doc_values(field),
            _ => {}
        }

        if !any_leaf_has(&leaves, field, DocValuesType::NUMERIC) {
            return Ok(None);
        }

        Ok(Some(Box::new(MultiNumericDocValues::new(
            leaves,
            field,
            fetch_numeric,
        ))))
    }

    /// Returns a [`BinaryDocValues`] over `reader`'s binary doc values for
    /// `field`, merging the leaves on the fly, or `None` when no leaf has
    /// [`DocValuesType::BINARY`] for the field.
    ///
    /// Equivalent to `MultiDocValues.getBinaryValues(IndexReader, String)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading a leaf's doc values.
    pub fn get_binary_values(
        reader: &Arc<dyn IndexReader>,
        field: &str,
    ) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let leaves = Arc::clone(reader).leaves();
        match leaves.len() {
            0 => return Ok(None),
            1 => return leaves[0].leaf_reader().get_binary_doc_values(field),
            _ => {}
        }

        if !any_leaf_has(&leaves, field, DocValuesType::BINARY) {
            return Ok(None);
        }

        Ok(Some(Box::new(MultiBinaryDocValues::new(leaves, field))))
    }

    /// Returns a [`SortedNumericDocValues`] over `reader`'s sorted-numeric doc
    /// values for `field`, or `None` when no leaf has them.
    ///
    /// Equivalent to
    /// `MultiDocValues.getSortedNumericValues(IndexReader, String)`.
    ///
    /// Unlike the numeric and binary variants this one opens every leaf's doc
    /// values eagerly — as Lucene does — because it needs their summed
    /// [`DocIdSetIterator::cost`].
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading a leaf's doc values.
    pub fn get_sorted_numeric_values(
        reader: &Arc<dyn IndexReader>,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let leaves = Arc::clone(reader).leaves();
        let size = leaves.len();
        match size {
            0 => return Ok(None),
            1 => return leaves[0].leaf_reader().get_sorted_numeric_doc_values(field),
            _ => {}
        }

        let mut any_real = false;
        let mut values: Vec<Box<dyn SortedNumericDocValues>> = Vec::with_capacity(size);
        let mut doc_starts: Vec<i32> = Vec::with_capacity(size);
        let mut total_cost = 0i64;
        for leaf in &leaves {
            let v = match leaf.leaf_reader().get_sorted_numeric_doc_values(field)? {
                Some(v) => {
                    any_real = true;
                    v
                }
                None => {
                    Box::new(DocValues::empty_sorted_numeric()) as Box<dyn SortedNumericDocValues>
                }
            };
            total_cost += v.cost();
            values.push(v);
            doc_starts.push(leaf.doc_base());
        }

        if !any_real {
            return Ok(None);
        }

        Ok(Some(Box::new(MultiSortedNumericDocValues::new(
            values, doc_starts, total_cost,
        ))))
    }

    /// Returns a [`SortedDocValues`] over `reader`'s sorted doc values for
    /// `field`, or `None` when no leaf has them.
    ///
    /// Equivalent to `MultiDocValues.getSortedValues(IndexReader, String)`.
    ///
    /// The returned instance exposes **global** ordinals: the per-leaf ordinal
    /// spaces are merged by an [`OrdinalMap`] built over one entry per leaf, in
    /// leaf order.
    ///
    /// This is an extremely slow way to read sorted doc values, because the
    /// ordinal map merge-sorts every leaf's term dictionary. Prefer
    /// [`LeafReader::get_sorted_doc_values`] per leaf.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading a leaf's doc values, and
    /// returns [`LuceneError::IllegalState`] if a leaf reports the field on one
    /// call and not on the next.
    pub fn get_sorted_values(
        reader: &Arc<dyn IndexReader>,
        field: &str,
    ) -> Result<Option<Box<dyn SortedDocValues>>> {
        let leaves = Arc::clone(reader).leaves();
        let size = leaves.len();
        match size {
            0 => return Ok(None),
            1 => return leaves[0].leaf_reader().get_sorted_doc_values(field),
            _ => {}
        }

        let mut any_real = false;
        let mut values: Vec<Box<dyn SortedDocValues>> = Vec::with_capacity(size);
        // Parallel set of instances consumed by the ordinal map; see the
        // `dual_instances` note on this module's `get_sorted_set_values`.
        let mut map_inputs: Vec<Box<dyn SortedDocValues>> = Vec::with_capacity(size);
        let mut doc_starts: Vec<i32> = Vec::with_capacity(size + 1);
        let mut total_cost = 0i64;

        for leaf in &leaves {
            let leaf_reader = leaf.leaf_reader();
            match leaf_reader.get_sorted_doc_values(field)? {
                Some(v) => {
                    any_real = true;
                    total_cost += v.cost();
                    values.push(v);
                    map_inputs.push(second_instance(
                        leaf_reader.get_sorted_doc_values(field)?,
                        field,
                        "sorted",
                    )?);
                }
                None => {
                    values.push(Box::new(DocValues::empty_sorted()));
                    map_inputs.push(Box::new(DocValues::empty_sorted()));
                }
            }
            doc_starts.push(leaf.doc_base());
        }
        doc_starts.push(reader.max_doc());

        if !any_real {
            return Ok(None);
        }

        let mapping = OrdinalMap::build_sorted(map_inputs, DEFAULT_ACCEPTABLE_OVERHEAD_RATIO)?;
        Ok(Some(Box::new(MultiSortedDocValues::new(
            values, doc_starts, mapping, total_cost,
        )?)))
    }

    /// Returns a [`SortedSetDocValues`] over `reader`'s sorted-set doc values
    /// for `field`, or `None` when no leaf has them.
    ///
    /// Equivalent to `MultiDocValues.getSortedSetValues(IndexReader, String)`.
    ///
    /// As with [`Self::get_sorted_values`], the returned instance exposes
    /// global ordinals produced by an [`OrdinalMap`].
    ///
    /// <a name="dual_instances"></a>
    /// **Deliberate divergence**: Lucene hands the very same
    /// `SortedSetDocValues[]` both to `OrdinalMap.build` and to the returned
    /// view, because building the map only touches the term-dictionary side of
    /// each instance and leaves its document cursor untouched. Rucene's
    /// [`OrdinalMap::build_sorted_set`] takes ownership of what it wraps, so a
    /// second, independent instance is opened per leaf for the map. The two
    /// instances read the same immutable segment data and never observe each
    /// other, so the resulting ordinal map is identical; only the construction
    /// cost differs.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading a leaf's doc values, and
    /// returns [`LuceneError::IllegalState`] if a leaf reports the field on one
    /// call and not on the next.
    pub fn get_sorted_set_values(
        reader: &Arc<dyn IndexReader>,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let leaves = Arc::clone(reader).leaves();
        let size = leaves.len();
        match size {
            0 => return Ok(None),
            1 => return leaves[0].leaf_reader().get_sorted_set_doc_values(field),
            _ => {}
        }

        let mut any_real = false;
        let mut values: Vec<Box<dyn SortedSetDocValues>> = Vec::with_capacity(size);
        let mut map_inputs: Vec<Box<dyn SortedSetDocValues>> = Vec::with_capacity(size);
        let mut doc_starts: Vec<i32> = Vec::with_capacity(size + 1);
        let mut total_cost = 0i64;

        for leaf in &leaves {
            let leaf_reader = leaf.leaf_reader();
            match leaf_reader.get_sorted_set_doc_values(field)? {
                Some(v) => {
                    any_real = true;
                    total_cost += v.cost();
                    values.push(v);
                    map_inputs.push(second_instance(
                        leaf_reader.get_sorted_set_doc_values(field)?,
                        field,
                        "sorted-set",
                    )?);
                }
                None => {
                    values.push(Box::new(DocValues::empty_sorted_set()));
                    map_inputs.push(Box::new(DocValues::empty_sorted_set()));
                }
            }
            doc_starts.push(leaf.doc_base());
        }
        doc_starts.push(reader.max_doc());

        if !any_real {
            return Ok(None);
        }

        let mapping = OrdinalMap::build_sorted_set(map_inputs, DEFAULT_ACCEPTABLE_OVERHEAD_RATIO)?;
        Ok(Some(Box::new(MultiSortedSetDocValues::new(
            values, doc_starts, mapping, total_cost,
        )?)))
    }
}

/// Returns `true` if any leaf declares `field` with the given doc-values type.
fn any_leaf_has(leaves: &[Arc<LeafReaderContext>], field: &str, wanted: DocValuesType) -> bool {
    leaves.iter().any(|leaf| {
        leaf.leaf_reader()
            .get_field_infos()
            .field_info(field)
            .is_some_and(|info| info.get_doc_values_type() == wanted)
    })
}

/// Unwraps the second instance opened for the ordinal map, turning a leaf that
/// changed its mind between two calls into a clear error instead of a silent
/// ordinal-space corruption.
fn second_instance<T: ?Sized>(values: Option<Box<T>>, field: &str, kind: &str) -> Result<Box<T>> {
    values.ok_or_else(|| {
        LuceneError::IllegalState(format!(
            "leaf reader returned {kind} doc values for field '{field}' on the first call \
             but not on the second"
        ))
    })
}

// ---------------------------------------------------------------------------
// MultiNumericDocValues
// ---------------------------------------------------------------------------

/// Merged view of one numeric doc-values field (or one norms field) across a
/// composite reader's leaves.
///
/// Backs both `MultiDocValues.getNormValues` and
/// `MultiDocValues.getNumericValues`, which in Lucene are two anonymous classes
/// differing only in the leaf accessor they call.
///
/// Leaves are opened lazily, one at a time, and the open instance is held in
/// `current_values` until it is exhausted or a seek moves past it — so a leaf's
/// doc values are fetched at most once per forward pass, never once per
/// document.
struct MultiNumericDocValues {
    leaves: Vec<Arc<LeafReaderContext>>,
    field: String,
    fetch: NumericFetcher,
    /// Index of the first leaf not yet opened.
    next_leaf: usize,
    /// Doc values of the leaf currently being iterated, if it has any.
    current_values: Option<Box<dyn NumericDocValues>>,
    /// `docBase` of the leaf currently being iterated.
    current_doc_base: i32,
    /// Current global doc ID (`-1` before the first positioning).
    doc_id: i32,
}

impl MultiNumericDocValues {
    fn new(leaves: Vec<Arc<LeafReaderContext>>, field: &str, fetch: NumericFetcher) -> Self {
        Self {
            leaves,
            field: field.to_string(),
            fetch,
            next_leaf: 0,
            current_values: None,
            current_doc_base: 0,
            doc_id: -1,
        }
    }

    /// Opens leaf `index` and makes it current.
    fn open_leaf(&mut self, index: usize) -> Result<()> {
        let leaf = &self.leaves[index];
        self.current_doc_base = leaf.doc_base();
        self.current_values = (self.fetch)(leaf.leaf_reader().as_ref(), &self.field)?;
        Ok(())
    }

    fn values_mut(&mut self) -> Result<&mut Box<dyn NumericDocValues>> {
        self.current_values
            .as_mut()
            .ok_or_else(|| LuceneError::IllegalState(no_current("numeric")))
    }
}

impl DocIdSetIterator for MultiNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.current_values.is_none() {
                if self.next_leaf == self.leaves.len() {
                    self.doc_id = NO_MORE_DOCS;
                    return Ok(self.doc_id);
                }
                let leaf = self.next_leaf;
                self.next_leaf += 1;
                self.open_leaf(leaf)?;
                continue;
            }

            let new_doc_id = self.values_mut()?.next_doc()?;
            if new_doc_id == NO_MORE_DOCS {
                self.current_values = None;
            } else {
                self.doc_id = self.current_doc_base + new_doc_id;
                return Ok(self.doc_id);
            }
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target <= self.doc_id {
            return Err(advance_backwards(self.doc_id, target));
        }
        let reader_index = reader_util::sub_index_from_leaves(target, &self.leaves);
        if reader_index >= self.next_leaf {
            if reader_index >= self.leaves.len() {
                self.current_values = None;
                self.doc_id = NO_MORE_DOCS;
                return Ok(self.doc_id);
            }
            self.open_leaf(reader_index)?;
            self.next_leaf = reader_index + 1;
        }
        if self.current_values.is_none() {
            // The leaf owning `target` has no values for this field, so every
            // remaining document with a value lives in a later leaf — which is
            // exactly what the sequential scan walks. It can only return a doc
            // ID greater than `target`, so the iterator never moves backwards.
            return self.next_doc();
        }
        let local_target = target - self.current_doc_base;
        let new_doc_id = self.values_mut()?.advance(local_target)?;
        if new_doc_id == NO_MORE_DOCS {
            self.current_values = None;
            self.next_doc()
        } else {
            self.doc_id = self.current_doc_base + new_doc_id;
            Ok(self.doc_id)
        }
    }

    fn cost(&self) -> i64 {
        // Matches Lucene, which returns 0 here with a "TODO" — the per-leaf
        // costs are not summed because the leaves are opened lazily.
        0
    }
}

impl DocValuesIterator for MultiNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if target < self.doc_id {
            return Err(advance_backwards(self.doc_id, target));
        }
        let reader_index = reader_util::sub_index_from_leaves(target, &self.leaves);
        if reader_index >= self.next_leaf {
            if reader_index >= self.leaves.len() {
                return Err(out_of_range(target));
            }
            self.open_leaf(reader_index)?;
            self.next_leaf = reader_index + 1;
        }
        self.doc_id = target;
        match self.current_values.as_mut() {
            None => Ok(false),
            Some(values) => values.advance_exact(target - self.current_doc_base),
        }
    }
}

impl NumericDocValues for MultiNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        self.current_values
            .as_ref()
            .ok_or_else(|| LuceneError::IllegalState(no_current("numeric")))?
            .long_value()
    }
}

// ---------------------------------------------------------------------------
// MultiBinaryDocValues
// ---------------------------------------------------------------------------

/// Merged view of one binary doc-values field across a composite reader's
/// leaves.
///
/// Equivalent to the anonymous `BinaryDocValues` returned by
/// `MultiDocValues.getBinaryValues`. Leaves are opened lazily, exactly as in
/// [`MultiNumericDocValues`].
struct MultiBinaryDocValues {
    leaves: Vec<Arc<LeafReaderContext>>,
    field: String,
    next_leaf: usize,
    current_values: Option<Box<dyn BinaryDocValues>>,
    current_doc_base: i32,
    doc_id: i32,
}

impl MultiBinaryDocValues {
    fn new(leaves: Vec<Arc<LeafReaderContext>>, field: &str) -> Self {
        Self {
            leaves,
            field: field.to_string(),
            next_leaf: 0,
            current_values: None,
            current_doc_base: 0,
            doc_id: -1,
        }
    }

    fn open_leaf(&mut self, index: usize) -> Result<()> {
        let leaf = &self.leaves[index];
        self.current_doc_base = leaf.doc_base();
        self.current_values = leaf.leaf_reader().get_binary_doc_values(&self.field)?;
        Ok(())
    }

    fn values_mut(&mut self) -> Result<&mut Box<dyn BinaryDocValues>> {
        self.current_values
            .as_mut()
            .ok_or_else(|| LuceneError::IllegalState(no_current("binary")))
    }
}

impl DocIdSetIterator for MultiBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.current_values.is_none() {
                if self.next_leaf == self.leaves.len() {
                    self.doc_id = NO_MORE_DOCS;
                    return Ok(self.doc_id);
                }
                let leaf = self.next_leaf;
                self.next_leaf += 1;
                self.open_leaf(leaf)?;
                continue;
            }

            let new_doc_id = self.values_mut()?.next_doc()?;
            if new_doc_id == NO_MORE_DOCS {
                self.current_values = None;
            } else {
                self.doc_id = self.current_doc_base + new_doc_id;
                return Ok(self.doc_id);
            }
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target <= self.doc_id {
            return Err(advance_backwards(self.doc_id, target));
        }
        let reader_index = reader_util::sub_index_from_leaves(target, &self.leaves);
        if reader_index >= self.next_leaf {
            if reader_index >= self.leaves.len() {
                self.current_values = None;
                self.doc_id = NO_MORE_DOCS;
                return Ok(self.doc_id);
            }
            self.open_leaf(reader_index)?;
            self.next_leaf = reader_index + 1;
        }
        if self.current_values.is_none() {
            // See the note on `MultiNumericDocValues::advance`.
            return self.next_doc();
        }
        let local_target = target - self.current_doc_base;
        let new_doc_id = self.values_mut()?.advance(local_target)?;
        if new_doc_id == NO_MORE_DOCS {
            self.current_values = None;
            self.next_doc()
        } else {
            self.doc_id = self.current_doc_base + new_doc_id;
            Ok(self.doc_id)
        }
    }

    fn cost(&self) -> i64 {
        // Matches Lucene's "TODO": lazily opened leaves have no summed cost.
        0
    }
}

impl DocValuesIterator for MultiBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if target < self.doc_id {
            return Err(advance_backwards(self.doc_id, target));
        }
        let reader_index = reader_util::sub_index_from_leaves(target, &self.leaves);
        if reader_index >= self.next_leaf {
            if reader_index >= self.leaves.len() {
                return Err(out_of_range(target));
            }
            self.open_leaf(reader_index)?;
            self.next_leaf = reader_index + 1;
        }
        self.doc_id = target;
        match self.current_values.as_mut() {
            None => Ok(false),
            Some(values) => values.advance_exact(target - self.current_doc_base),
        }
    }
}

impl BinaryDocValues for MultiBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        self.current_values
            .as_ref()
            .ok_or_else(|| LuceneError::IllegalState(no_current("binary")))?
            .binary_value()
    }
}

// ---------------------------------------------------------------------------
// MultiSortedNumericDocValues
// ---------------------------------------------------------------------------

/// Merged view of one sorted-numeric doc-values field across a composite
/// reader's leaves.
///
/// Equivalent to the anonymous `SortedNumericDocValues` returned by
/// `MultiDocValues.getSortedNumericValues`. Every leaf's instance is opened up
/// front — leaves without the field are padded with
/// [`DocValues::empty_sorted_numeric`] — so `current_values` is an index into
/// `values`, never an `Option`.
struct MultiSortedNumericDocValues {
    values: Vec<Box<dyn SortedNumericDocValues>>,
    /// `docBase` of each leaf, parallel with `values`.
    doc_starts: Vec<i32>,
    total_cost: i64,
    next_leaf: usize,
    /// Index into `values` of the leaf being iterated, or `None` between
    /// leaves.
    current: Option<usize>,
    doc_id: i32,
}

impl MultiSortedNumericDocValues {
    fn new(
        values: Vec<Box<dyn SortedNumericDocValues>>,
        doc_starts: Vec<i32>,
        total_cost: i64,
    ) -> Self {
        debug_assert_eq!(values.len(), doc_starts.len());
        Self {
            values,
            doc_starts,
            total_cost,
            next_leaf: 0,
            current: None,
            doc_id: -1,
        }
    }

    fn current_index(&self) -> Result<usize> {
        self.current
            .ok_or_else(|| LuceneError::IllegalState(no_current("sorted-numeric")))
    }
}

impl DocIdSetIterator for MultiSortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.current.is_none() {
                if self.next_leaf == self.values.len() {
                    self.doc_id = NO_MORE_DOCS;
                    return Ok(self.doc_id);
                }
                self.current = Some(self.next_leaf);
                self.next_leaf += 1;
            }

            let index = self.current_index()?;
            let new_doc_id = self.values[index].next_doc()?;
            if new_doc_id == NO_MORE_DOCS {
                self.current = None;
            } else {
                self.doc_id = self.doc_starts[index] + new_doc_id;
                return Ok(self.doc_id);
            }
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target <= self.doc_id {
            return Err(advance_backwards(self.doc_id, target));
        }
        let reader_index = reader_util::sub_index(target, &self.doc_starts);
        if reader_index >= self.next_leaf {
            if reader_index >= self.values.len() {
                self.current = None;
                self.doc_id = NO_MORE_DOCS;
                return Ok(self.doc_id);
            }
            self.current = Some(reader_index);
            self.next_leaf = reader_index + 1;
        }
        let index = self.current_index()?;
        let doc_start = self.doc_starts[index];
        let new_doc_id = self.values[index].advance(target - doc_start)?;
        if new_doc_id == NO_MORE_DOCS {
            self.current = None;
            self.next_doc()
        } else {
            self.doc_id = doc_start + new_doc_id;
            Ok(self.doc_id)
        }
    }

    fn cost(&self) -> i64 {
        self.total_cost
    }
}

impl DocValuesIterator for MultiSortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if target < self.doc_id {
            return Err(advance_backwards(self.doc_id, target));
        }
        let reader_index = reader_util::sub_index(target, &self.doc_starts);
        if reader_index >= self.next_leaf {
            if reader_index >= self.values.len() {
                return Err(out_of_range(target));
            }
            self.current = Some(reader_index);
            self.next_leaf = reader_index + 1;
        }
        self.doc_id = target;
        match self.current {
            None => Ok(false),
            Some(index) => {
                let doc_start = self.doc_starts[index];
                self.values[index].advance_exact(target - doc_start)
            }
        }
    }
}

impl SortedNumericDocValues for MultiSortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        let index = self.current_index()?;
        self.values[index].next_value()
    }

    fn doc_value_count(&self) -> Result<i32> {
        let index = self.current_index()?;
        self.values[index].doc_value_count()
    }
}

// ---------------------------------------------------------------------------
// MultiSortedDocValues
// ---------------------------------------------------------------------------

/// [`SortedDocValues`] over *n* leaves, translating per-leaf ordinals into a
/// single global ordinal space through an [`OrdinalMap`].
///
/// Equivalent to `MultiDocValues.MultiSortedDocValues`.
///
/// `doc_starts` has one more entry than `values`: entry `i` is leaf `i`'s
/// `docBase` and the trailing entry is the composite reader's `maxDoc`. That
/// sentinel is what lets [`reader_util::sub_index`] answer "past the last leaf"
/// for a target at or beyond `maxDoc`, exactly as in Lucene.
pub struct MultiSortedDocValues {
    values: Vec<Box<dyn SortedDocValues>>,
    doc_starts: Vec<i32>,
    mapping: OrdinalMap,
    total_cost: i64,
    next_leaf: usize,
    current: Option<usize>,
    doc_id: i32,
}

impl std::fmt::Debug for MultiSortedDocValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiSortedDocValues")
            .field("num_leaves", &self.values.len())
            .field("doc_starts", &self.doc_starts)
            .field("doc_id", &self.doc_id)
            .finish_non_exhaustive()
    }
}

impl MultiSortedDocValues {
    /// Creates a `MultiSortedDocValues` over `values`.
    ///
    /// Equivalent to
    /// `MultiSortedDocValues(SortedDocValues[], int[], OrdinalMap, long)`.
    ///
    /// `mapping` must have been built over exactly these leaves, in this order.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] unless
    /// `doc_starts.len() == values.len() + 1`. Java asserts this, which
    /// production builds skip; a mismatch would silently mis-address leaves, so
    /// it is checked here.
    pub fn new(
        values: Vec<Box<dyn SortedDocValues>>,
        doc_starts: Vec<i32>,
        mapping: OrdinalMap,
        total_cost: i64,
    ) -> Result<Self> {
        check_doc_starts(doc_starts.len(), values.len())?;
        Ok(Self {
            values,
            doc_starts,
            mapping,
            total_cost,
            next_leaf: 0,
            current: None,
            doc_id: -1,
        })
    }

    /// Returns the per-leaf doc values being merged.
    ///
    /// Equivalent to reading the public `values` field in Java.
    pub fn values(&self) -> &[Box<dyn SortedDocValues>] {
        &self.values
    }

    /// Returns the `docBase` of each leaf, plus a trailing `maxDoc` sentinel.
    ///
    /// Equivalent to reading the public `docStarts` field in Java.
    pub fn doc_starts(&self) -> &[i32] {
        &self.doc_starts
    }

    /// Returns the ordinal map translating per-leaf ordinals to global ones.
    ///
    /// Equivalent to reading the public `mapping` field in Java.
    ///
    /// **Deliberate divergence**: Java shares one `OrdinalMap` object between
    /// several views by reference, and caches it on the reader. Rucene's
    /// [`OrdinalMap`] is built on `Box<dyn LongValues>` and is therefore
    /// neither `Send` nor `Sync`, so nothing is gained by wrapping it in a
    /// shared pointer; this view owns its map outright.
    pub fn mapping(&self) -> &OrdinalMap {
        &self.mapping
    }

    fn current_index(&self) -> Result<usize> {
        self.current
            .ok_or_else(|| LuceneError::IllegalState(no_current("sorted")))
    }
}

impl DocIdSetIterator for MultiSortedDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.current.is_none() {
                if self.next_leaf == self.values.len() {
                    self.doc_id = NO_MORE_DOCS;
                    return Ok(self.doc_id);
                }
                self.current = Some(self.next_leaf);
                self.next_leaf += 1;
            }

            let index = self.current_index()?;
            let new_doc_id = self.values[index].next_doc()?;
            if new_doc_id == NO_MORE_DOCS {
                self.current = None;
            } else {
                self.doc_id = self.doc_starts[index] + new_doc_id;
                return Ok(self.doc_id);
            }
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target <= self.doc_id {
            return Err(advance_backwards(self.doc_id, target));
        }
        let reader_index = reader_util::sub_index(target, &self.doc_starts);
        if reader_index >= self.next_leaf {
            if reader_index >= self.values.len() {
                self.current = None;
                self.doc_id = NO_MORE_DOCS;
                return Ok(self.doc_id);
            }
            self.current = Some(reader_index);
            self.next_leaf = reader_index + 1;
        }
        let index = self.current_index()?;
        let doc_start = self.doc_starts[index];
        let new_doc_id = self.values[index].advance(target - doc_start)?;
        if new_doc_id == NO_MORE_DOCS {
            self.current = None;
            self.next_doc()
        } else {
            self.doc_id = doc_start + new_doc_id;
            Ok(self.doc_id)
        }
    }

    fn cost(&self) -> i64 {
        self.total_cost
    }
}

impl DocValuesIterator for MultiSortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if target < self.doc_id {
            return Err(advance_backwards(self.doc_id, target));
        }
        let reader_index = reader_util::sub_index(target, &self.doc_starts);
        if reader_index >= self.next_leaf {
            if reader_index >= self.values.len() {
                return Err(out_of_range(target));
            }
            self.current = Some(reader_index);
            self.next_leaf = reader_index + 1;
        }
        self.doc_id = target;
        match self.current {
            None => Ok(false),
            Some(index) => {
                let doc_start = self.doc_starts[index];
                self.values[index].advance_exact(target - doc_start)
            }
        }
    }
}

impl SortedDocValues for MultiSortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        let index = self.current_index()?;
        let segment_ord = self.values[index].ord_value()?;
        Ok(self.mapping.get_global_ords(index).get(segment_ord as i64) as i32)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        let sub_index = self.mapping.get_first_segment_number(ord as i64);
        let segment_ord = self.mapping.get_first_segment_ord(ord as i64);
        self.values[sub_index].lookup_ord(segment_ord as i32)
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(self.mapping.get_value_count() as i32)
    }
}

// ---------------------------------------------------------------------------
// MultiSortedSetDocValues
// ---------------------------------------------------------------------------

/// [`SortedSetDocValues`] over *n* leaves, translating per-leaf ordinals into a
/// single global ordinal space through an [`OrdinalMap`].
///
/// Equivalent to `MultiDocValues.MultiSortedSetDocValues`. The `doc_starts`
/// contract is the same as [`MultiSortedDocValues`]'.
pub struct MultiSortedSetDocValues {
    values: Vec<Box<dyn SortedSetDocValues>>,
    doc_starts: Vec<i32>,
    mapping: OrdinalMap,
    total_cost: i64,
    next_leaf: usize,
    current: Option<usize>,
    doc_id: i32,
}

impl std::fmt::Debug for MultiSortedSetDocValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiSortedSetDocValues")
            .field("num_leaves", &self.values.len())
            .field("doc_starts", &self.doc_starts)
            .field("doc_id", &self.doc_id)
            .finish_non_exhaustive()
    }
}

impl MultiSortedSetDocValues {
    /// Creates a `MultiSortedSetDocValues` over `values`.
    ///
    /// Equivalent to
    /// `MultiSortedSetDocValues(SortedSetDocValues[], int[], OrdinalMap, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] unless
    /// `doc_starts.len() == values.len() + 1`.
    pub fn new(
        values: Vec<Box<dyn SortedSetDocValues>>,
        doc_starts: Vec<i32>,
        mapping: OrdinalMap,
        total_cost: i64,
    ) -> Result<Self> {
        check_doc_starts(doc_starts.len(), values.len())?;
        Ok(Self {
            values,
            doc_starts,
            mapping,
            total_cost,
            next_leaf: 0,
            current: None,
            doc_id: -1,
        })
    }

    /// Returns the per-leaf doc values being merged.
    pub fn values(&self) -> &[Box<dyn SortedSetDocValues>] {
        &self.values
    }

    /// Returns the `docBase` of each leaf, plus a trailing `maxDoc` sentinel.
    pub fn doc_starts(&self) -> &[i32] {
        &self.doc_starts
    }

    /// Returns the ordinal map translating per-leaf ordinals to global ones.
    ///
    /// See [`MultiSortedDocValues::mapping`] for why the map is owned rather
    /// than shared.
    pub fn mapping(&self) -> &OrdinalMap {
        &self.mapping
    }

    fn current_index(&self) -> Result<usize> {
        self.current
            .ok_or_else(|| LuceneError::IllegalState(no_current("sorted-set")))
    }
}

impl DocIdSetIterator for MultiSortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.current.is_none() {
                if self.next_leaf == self.values.len() {
                    self.doc_id = NO_MORE_DOCS;
                    return Ok(self.doc_id);
                }
                self.current = Some(self.next_leaf);
                self.next_leaf += 1;
            }

            let index = self.current_index()?;
            let new_doc_id = self.values[index].next_doc()?;
            if new_doc_id == NO_MORE_DOCS {
                self.current = None;
            } else {
                self.doc_id = self.doc_starts[index] + new_doc_id;
                return Ok(self.doc_id);
            }
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target <= self.doc_id {
            return Err(advance_backwards(self.doc_id, target));
        }
        let reader_index = reader_util::sub_index(target, &self.doc_starts);
        if reader_index >= self.next_leaf {
            if reader_index >= self.values.len() {
                self.current = None;
                self.doc_id = NO_MORE_DOCS;
                return Ok(self.doc_id);
            }
            self.current = Some(reader_index);
            self.next_leaf = reader_index + 1;
        }
        let index = self.current_index()?;
        let doc_start = self.doc_starts[index];
        let new_doc_id = self.values[index].advance(target - doc_start)?;
        if new_doc_id == NO_MORE_DOCS {
            self.current = None;
            self.next_doc()
        } else {
            self.doc_id = doc_start + new_doc_id;
            Ok(self.doc_id)
        }
    }

    fn cost(&self) -> i64 {
        self.total_cost
    }
}

impl DocValuesIterator for MultiSortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if target < self.doc_id {
            return Err(advance_backwards(self.doc_id, target));
        }
        let reader_index = reader_util::sub_index(target, &self.doc_starts);
        if reader_index >= self.next_leaf {
            if reader_index >= self.values.len() {
                return Err(out_of_range(target));
            }
            self.current = Some(reader_index);
            self.next_leaf = reader_index + 1;
        }
        self.doc_id = target;
        match self.current {
            None => Ok(false),
            Some(index) => {
                let doc_start = self.doc_starts[index];
                self.values[index].advance_exact(target - doc_start)
            }
        }
    }
}

impl SortedSetDocValues for MultiSortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        let index = self.current_index()?;
        let segment_ord = self.values[index].next_ord()?;
        Ok(self.mapping.get_global_ords(index).get(segment_ord))
    }

    fn doc_value_count(&self) -> Result<i32> {
        let index = self.current_index()?;
        self.values[index].doc_value_count()
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        let sub_index = self.mapping.get_first_segment_number(ord);
        let segment_ord = self.mapping.get_first_segment_ord(ord);
        self.values[sub_index].lookup_ord(segment_ord)
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(self.mapping.get_value_count())
    }
}

// ---------------------------------------------------------------------------
// Shared error helpers
// ---------------------------------------------------------------------------

/// Error for a seek that would move backwards.
///
/// Equivalent to the `IllegalArgumentException("can only advance beyond current
/// document: ...")` raised by every `MultiDocValues` iterator.
fn advance_backwards(doc_id: i32, target: i32) -> LuceneError {
    LuceneError::IllegalArgument(format!(
        "can only advance beyond current document: on docID={doc_id} but targetDocID={target}"
    ))
}

/// Error for an `advance_exact` target past the last leaf.
///
/// Equivalent to `IllegalArgumentException("Out of range: " + targetDocID)`.
fn out_of_range(target: i32) -> LuceneError {
    LuceneError::IllegalArgument(format!("Out of range: {target}"))
}

/// Message used when a per-document accessor is called while unpositioned.
///
/// Java dereferences a null `currentValues` here and throws
/// `NullPointerException`; reporting the misuse explicitly is strictly more
/// informative and keeps the port panic-free.
fn no_current(kind: &str) -> String {
    format!("MultiDocValues: no current {kind} doc values; call next_doc/advance first")
}

/// Validates the `doc_starts` length invariant shared by the two
/// ordinal-mapping views.
fn check_doc_starts(doc_starts_len: usize, values_len: usize) -> Result<()> {
    if doc_starts_len != values_len + 1 {
        return Err(LuceneError::IllegalArgument(format!(
            "doc_starts must have one more entry than values: got {doc_starts_len} \
             doc starts for {values_len} leaves"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    use crate::codecs::stub::StoredFieldVisitor;
    use crate::document::Document;
    use crate::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
    use crate::index::leaf_reader::{LeafMetaData, TermVectors};
    use crate::index::multi_reader::MultiReader;
    use crate::index::{
        ByteVectorValues, DocValuesSkipIndexType, DocValuesSkipper, EmptyFields, FieldInfo,
        FieldInfos, Fields, FloatVectorValues, IndexOptions, PointValues, Terms, VectorEncoding,
        VectorSimilarityFunction,
    };
    use crate::search::knn::KnnCollector;
    use crate::search::AcceptDocs;
    use crate::util::Bits;

    // -----------------------------------------------------------------------
    // In-memory doc-values fixtures
    // -----------------------------------------------------------------------

    /// Shared cursor over a sorted list of documents that carry a value.
    ///
    /// Every fixture below drives its iteration through this helper so they all
    /// obey the same contract: `next_doc`/`advance` land on the next document
    /// with a value, while `advance_exact` lands on `target` whether or not it
    /// carries one.
    struct Cursor {
        docs: Vec<i32>,
        /// Index of the next entry to consider.
        pos: usize,
        /// Index of the entry whose value is current, if any.
        current: Option<usize>,
        doc: i32,
    }

    impl Cursor {
        fn new(docs: Vec<i32>) -> Self {
            Self {
                docs,
                pos: 0,
                current: None,
                doc: -1,
            }
        }

        fn next_doc(&mut self) -> i32 {
            if self.pos < self.docs.len() {
                self.doc = self.docs[self.pos];
                self.current = Some(self.pos);
                self.pos += 1;
            } else {
                self.doc = NO_MORE_DOCS;
                self.current = None;
            }
            self.doc
        }

        fn advance(&mut self, target: i32) -> i32 {
            while self.pos < self.docs.len() && self.docs[self.pos] < target {
                self.pos += 1;
            }
            self.next_doc()
        }

        fn advance_exact(&mut self, target: i32) -> bool {
            while self.pos < self.docs.len() && self.docs[self.pos] < target {
                self.pos += 1;
            }
            self.doc = target;
            if self.pos < self.docs.len() && self.docs[self.pos] == target {
                self.current = Some(self.pos);
                self.pos += 1;
                true
            } else {
                self.current = None;
                false
            }
        }

        fn index(&self) -> Result<usize> {
            self.current
                .ok_or_else(|| LuceneError::IllegalState("no value at current doc".to_string()))
        }
    }

    /// [`NumericDocValues`] over an explicit `(doc, value)` list.
    struct ListNumeric {
        cursor: Cursor,
        values: Vec<i64>,
    }

    impl ListNumeric {
        fn boxed(entries: &[(i32, i64)]) -> Box<dyn NumericDocValues> {
            Box::new(Self {
                cursor: Cursor::new(entries.iter().map(|(d, _)| *d).collect()),
                values: entries.iter().map(|(_, v)| *v).collect(),
            })
        }
    }

    impl DocIdSetIterator for ListNumeric {
        fn doc_id(&self) -> i32 {
            self.cursor.doc
        }
        fn next_doc(&mut self) -> Result<i32> {
            Ok(self.cursor.next_doc())
        }
        fn advance(&mut self, target: i32) -> Result<i32> {
            Ok(self.cursor.advance(target))
        }
        fn cost(&self) -> i64 {
            self.values.len() as i64
        }
    }

    impl DocValuesIterator for ListNumeric {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            Ok(self.cursor.advance_exact(target))
        }
    }

    impl NumericDocValues for ListNumeric {
        fn long_value(&self) -> Result<i64> {
            Ok(self.values[self.cursor.index()?])
        }
    }

    /// [`BinaryDocValues`] over an explicit `(doc, bytes)` list.
    struct ListBinary {
        cursor: Cursor,
        values: Vec<Vec<u8>>,
    }

    impl ListBinary {
        fn boxed(entries: &[(i32, &[u8])]) -> Box<dyn BinaryDocValues> {
            Box::new(Self {
                cursor: Cursor::new(entries.iter().map(|(d, _)| *d).collect()),
                values: entries.iter().map(|(_, v)| v.to_vec()).collect(),
            })
        }
    }

    impl DocIdSetIterator for ListBinary {
        fn doc_id(&self) -> i32 {
            self.cursor.doc
        }
        fn next_doc(&mut self) -> Result<i32> {
            Ok(self.cursor.next_doc())
        }
        fn advance(&mut self, target: i32) -> Result<i32> {
            Ok(self.cursor.advance(target))
        }
        fn cost(&self) -> i64 {
            self.values.len() as i64
        }
    }

    impl DocValuesIterator for ListBinary {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            Ok(self.cursor.advance_exact(target))
        }
    }

    impl BinaryDocValues for ListBinary {
        fn binary_value(&self) -> Result<BytesRef> {
            Ok(BytesRef::new(self.values[self.cursor.index()?].clone()))
        }
    }

    /// [`SortedNumericDocValues`] over an explicit `(doc, values)` list.
    struct ListSortedNumeric {
        cursor: Cursor,
        values: Vec<Vec<i64>>,
        value_upto: usize,
    }

    impl ListSortedNumeric {
        fn boxed(entries: &[(i32, &[i64])]) -> Box<dyn SortedNumericDocValues> {
            Box::new(Self {
                cursor: Cursor::new(entries.iter().map(|(d, _)| *d).collect()),
                values: entries.iter().map(|(_, v)| v.to_vec()).collect(),
                value_upto: 0,
            })
        }
    }

    impl DocIdSetIterator for ListSortedNumeric {
        fn doc_id(&self) -> i32 {
            self.cursor.doc
        }
        fn next_doc(&mut self) -> Result<i32> {
            self.value_upto = 0;
            Ok(self.cursor.next_doc())
        }
        fn advance(&mut self, target: i32) -> Result<i32> {
            self.value_upto = 0;
            Ok(self.cursor.advance(target))
        }
        fn cost(&self) -> i64 {
            self.values.len() as i64
        }
    }

    impl DocValuesIterator for ListSortedNumeric {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.value_upto = 0;
            Ok(self.cursor.advance_exact(target))
        }
    }

    impl SortedNumericDocValues for ListSortedNumeric {
        fn next_value(&mut self) -> Result<i64> {
            let index = self.cursor.index()?;
            let value = self.values[index][self.value_upto];
            self.value_upto += 1;
            Ok(value)
        }

        fn doc_value_count(&self) -> Result<i32> {
            Ok(self.values[self.cursor.index()?].len() as i32)
        }
    }

    /// [`SortedDocValues`] over a term dictionary plus a `(doc, ord)` list.
    struct ListSorted {
        cursor: Cursor,
        terms: Vec<String>,
        ords: Vec<i32>,
    }

    impl ListSorted {
        fn boxed(terms: &[&str], entries: &[(i32, i32)]) -> Box<dyn SortedDocValues> {
            Box::new(Self {
                cursor: Cursor::new(entries.iter().map(|(d, _)| *d).collect()),
                terms: terms.iter().map(|t| t.to_string()).collect(),
                ords: entries.iter().map(|(_, o)| *o).collect(),
            })
        }
    }

    impl DocIdSetIterator for ListSorted {
        fn doc_id(&self) -> i32 {
            self.cursor.doc
        }
        fn next_doc(&mut self) -> Result<i32> {
            Ok(self.cursor.next_doc())
        }
        fn advance(&mut self, target: i32) -> Result<i32> {
            Ok(self.cursor.advance(target))
        }
        fn cost(&self) -> i64 {
            self.ords.len() as i64
        }
    }

    impl DocValuesIterator for ListSorted {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            Ok(self.cursor.advance_exact(target))
        }
    }

    impl SortedDocValues for ListSorted {
        fn ord_value(&self) -> Result<i32> {
            Ok(self.ords[self.cursor.index()?])
        }

        fn get_value_count(&self) -> Result<i32> {
            Ok(self.terms.len() as i32)
        }

        fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
            self.terms
                .get(ord as usize)
                .map(|t| BytesRef::new(t.as_bytes().to_vec()))
                .ok_or_else(|| LuceneError::IllegalArgument(format!("ord {ord} out of range")))
        }
    }

    /// [`SortedSetDocValues`] over a term dictionary plus a `(doc, ords)` list.
    struct ListSortedSet {
        cursor: Cursor,
        terms: Vec<String>,
        ords: Vec<Vec<i64>>,
        ord_upto: usize,
    }

    impl ListSortedSet {
        fn boxed(terms: &[&str], entries: &[(i32, &[i64])]) -> Box<dyn SortedSetDocValues> {
            Box::new(Self {
                cursor: Cursor::new(entries.iter().map(|(d, _)| *d).collect()),
                terms: terms.iter().map(|t| t.to_string()).collect(),
                ords: entries.iter().map(|(_, o)| o.to_vec()).collect(),
                ord_upto: 0,
            })
        }
    }

    impl DocIdSetIterator for ListSortedSet {
        fn doc_id(&self) -> i32 {
            self.cursor.doc
        }
        fn next_doc(&mut self) -> Result<i32> {
            self.ord_upto = 0;
            Ok(self.cursor.next_doc())
        }
        fn advance(&mut self, target: i32) -> Result<i32> {
            self.ord_upto = 0;
            Ok(self.cursor.advance(target))
        }
        fn cost(&self) -> i64 {
            self.ords.len() as i64
        }
    }

    impl DocValuesIterator for ListSortedSet {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.ord_upto = 0;
            Ok(self.cursor.advance_exact(target))
        }
    }

    impl SortedSetDocValues for ListSortedSet {
        fn next_ord(&mut self) -> Result<i64> {
            let index = self.cursor.index()?;
            let ord = self.ords[index][self.ord_upto];
            self.ord_upto += 1;
            Ok(ord)
        }

        fn doc_value_count(&self) -> Result<i32> {
            Ok(self.ords[self.cursor.index()?].len() as i32)
        }

        fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
            self.terms
                .get(ord as usize)
                .map(|t| BytesRef::new(t.as_bytes().to_vec()))
                .ok_or_else(|| LuceneError::IllegalArgument(format!("ord {ord} out of range")))
        }

        fn get_value_count(&self) -> Result<i64> {
            Ok(self.terms.len() as i64)
        }
    }

    // -----------------------------------------------------------------------
    // Stub leaf reader
    // -----------------------------------------------------------------------

    #[derive(Debug)]
    struct StubTermVectors;
    impl TermVectors for StubTermVectors {
        fn get(&self, _doc: i32) -> Result<Option<Box<dyn Fields>>> {
            Ok(Some(Box::new(EmptyFields)))
        }
    }

    #[derive(Debug)]
    struct StubStoredFields;
    impl StoredFields for StubStoredFields {
        fn document_with_visitor(
            &self,
            _doc_id: i32,
            _visitor: &mut dyn StoredFieldVisitor,
        ) -> Result<()> {
            Ok(())
        }
        fn document(&self, _doc_id: i32) -> Result<Document> {
            Ok(Document::new())
        }
        fn document_fields(
            &self,
            _doc_id: i32,
            _fields_to_load: &HashSet<String>,
        ) -> Result<Document> {
            Ok(Document::new())
        }
    }

    /// A term dictionary plus the `(doc, ordinal)` list of a sorted field.
    type SortedTable = (Vec<String>, Vec<(i32, i32)>);
    /// A term dictionary plus the `(doc, ordinals)` list of a sorted-set field.
    type SortedSetTable = (Vec<String>, Vec<(i32, Vec<i64>)>);

    /// A leaf reader whose doc values are declared up front, recording every
    /// fetch so tests can assert how often each leaf is opened.
    struct StubLeaf {
        core: IndexReaderCore,
        max_doc: i32,
        infos: FieldInfos,
        numeric: HashMap<String, Vec<(i32, i64)>>,
        norms: HashMap<String, Vec<(i32, i64)>>,
        binary: HashMap<String, Vec<(i32, Vec<u8>)>>,
        sorted: HashMap<String, SortedTable>,
        sorted_numeric: HashMap<String, Vec<(i32, Vec<i64>)>>,
        sorted_set: HashMap<String, SortedSetTable>,
        fetches: Arc<Mutex<Vec<String>>>,
    }

    impl std::fmt::Debug for StubLeaf {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("StubLeaf")
                .field("max_doc", &self.max_doc)
                .finish_non_exhaustive()
        }
    }

    /// Builder for [`StubLeaf`], keeping the `FieldInfos` in step with the
    /// doc-values actually declared.
    struct StubLeafBuilder {
        max_doc: i32,
        infos: Vec<FieldInfo>,
        numeric: HashMap<String, Vec<(i32, i64)>>,
        norms: HashMap<String, Vec<(i32, i64)>>,
        binary: HashMap<String, Vec<(i32, Vec<u8>)>>,
        sorted: HashMap<String, SortedTable>,
        sorted_numeric: HashMap<String, Vec<(i32, Vec<i64>)>>,
        sorted_set: HashMap<String, SortedSetTable>,
        fetches: Arc<Mutex<Vec<String>>>,
    }

    fn field_info(
        name: &str,
        number: i32,
        dv: DocValuesType,
        indexed_with_norms: bool,
    ) -> FieldInfo {
        let index_options = if indexed_with_norms {
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
        } else {
            IndexOptions::NONE
        };
        FieldInfo::new_full(
            name,
            number,
            false,
            !indexed_with_norms,
            false,
            index_options,
            dv,
            DocValuesSkipIndexType::NONE,
            -1,
            HashMap::new(),
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        )
        .expect("INVARIANT: the field-info combinations used in tests are consistent")
    }

    impl StubLeafBuilder {
        fn new(max_doc: i32) -> Self {
            Self {
                max_doc,
                infos: Vec::new(),
                numeric: HashMap::new(),
                norms: HashMap::new(),
                binary: HashMap::new(),
                sorted: HashMap::new(),
                sorted_numeric: HashMap::new(),
                sorted_set: HashMap::new(),
                fetches: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn declare(&mut self, name: &str, dv: DocValuesType, norms: bool) {
            let number = self.infos.len() as i32;
            self.infos.push(field_info(name, number, dv, norms));
        }

        fn numeric(mut self, field: &str, entries: &[(i32, i64)]) -> Self {
            self.declare(field, DocValuesType::NUMERIC, false);
            self.numeric.insert(field.to_string(), entries.to_vec());
            self
        }

        fn norms(mut self, field: &str, entries: &[(i32, i64)]) -> Self {
            self.declare(field, DocValuesType::NONE, true);
            self.norms.insert(field.to_string(), entries.to_vec());
            self
        }

        /// Declares an indexed field that has no norms (`omitNorms`).
        fn no_norms_field(mut self, field: &str) -> Self {
            let number = self.infos.len() as i32;
            let mut info = field_info(field, number, DocValuesType::NONE, true);
            info.set_omits_norms().unwrap();
            self.infos.push(info);
            self
        }

        fn binary(mut self, field: &str, entries: &[(i32, &[u8])]) -> Self {
            self.declare(field, DocValuesType::BINARY, false);
            self.binary.insert(
                field.to_string(),
                entries.iter().map(|(d, v)| (*d, v.to_vec())).collect(),
            );
            self
        }

        fn sorted(mut self, field: &str, terms: &[&str], entries: &[(i32, i32)]) -> Self {
            self.declare(field, DocValuesType::SORTED, false);
            self.sorted.insert(
                field.to_string(),
                (
                    terms.iter().map(|t| t.to_string()).collect(),
                    entries.to_vec(),
                ),
            );
            self
        }

        fn sorted_numeric(mut self, field: &str, entries: &[(i32, &[i64])]) -> Self {
            self.declare(field, DocValuesType::SORTED_NUMERIC, false);
            self.sorted_numeric.insert(
                field.to_string(),
                entries.iter().map(|(d, v)| (*d, v.to_vec())).collect(),
            );
            self
        }

        fn sorted_set(mut self, field: &str, terms: &[&str], entries: &[(i32, &[i64])]) -> Self {
            self.declare(field, DocValuesType::SORTED_SET, false);
            self.sorted_set.insert(
                field.to_string(),
                (
                    terms.iter().map(|t| t.to_string()).collect(),
                    entries.iter().map(|(d, v)| (*d, v.to_vec())).collect(),
                ),
            );
            self
        }

        fn fetch_log(&self) -> Arc<Mutex<Vec<String>>> {
            Arc::clone(&self.fetches)
        }

        fn build(self) -> Arc<dyn IndexReader> {
            Arc::new(StubLeaf {
                core: IndexReaderCore::new(),
                max_doc: self.max_doc,
                infos: FieldInfos::new(self.infos).unwrap(),
                numeric: self.numeric,
                norms: self.norms,
                binary: self.binary,
                sorted: self.sorted,
                sorted_numeric: self.sorted_numeric,
                sorted_set: self.sorted_set,
                fetches: self.fetches,
            }) as Arc<dyn IndexReader>
        }
    }

    impl StubLeaf {
        fn record(&self, kind: &str, field: &str) {
            self.fetches.lock().unwrap().push(format!("{kind}:{field}"));
        }
    }

    impl LeafReader for StubLeaf {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }
        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            Ok(Box::new(StubTermVectors))
        }
        fn num_docs(&self) -> i32 {
            self.max_doc
        }
        fn max_doc(&self) -> i32 {
            self.max_doc
        }
        fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
            Ok(Box::new(StubStoredFields))
        }
        fn do_close(&self) -> Result<()> {
            Ok(())
        }
        fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }
        fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }
        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }

        fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            self.record("numeric", field);
            Ok(self.numeric.get(field).map(|e| ListNumeric::boxed(e)))
        }

        fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
            self.record("binary", field);
            Ok(self.binary.get(field).map(|entries| {
                let borrowed: Vec<(i32, &[u8])> =
                    entries.iter().map(|(d, v)| (*d, v.as_slice())).collect();
                ListBinary::boxed(&borrowed)
            }))
        }

        fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
            self.record("sorted", field);
            Ok(self.sorted.get(field).map(|(terms, entries)| {
                let borrowed: Vec<&str> = terms.iter().map(|t| t.as_str()).collect();
                ListSorted::boxed(&borrowed, entries)
            }))
        }

        fn get_sorted_numeric_doc_values(
            &self,
            field: &str,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            self.record("sorted_numeric", field);
            Ok(self.sorted_numeric.get(field).map(|entries| {
                let borrowed: Vec<(i32, &[i64])> =
                    entries.iter().map(|(d, v)| (*d, v.as_slice())).collect();
                ListSortedNumeric::boxed(&borrowed)
            }))
        }

        fn get_sorted_set_doc_values(
            &self,
            field: &str,
        ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            self.record("sorted_set", field);
            Ok(self.sorted_set.get(field).map(|(terms, entries)| {
                let term_refs: Vec<&str> = terms.iter().map(|t| t.as_str()).collect();
                let borrowed: Vec<(i32, &[i64])> =
                    entries.iter().map(|(d, v)| (*d, v.as_slice())).collect();
                ListSortedSet::boxed(&term_refs, &borrowed)
            }))
        }

        fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            self.record("norms", field);
            Ok(self.norms.get(field).map(|e| ListNumeric::boxed(e)))
        }

        fn get_doc_values_skipper(&self, _f: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            Ok(None)
        }
        fn get_float_vector_values(&self, _f: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
            Ok(None)
        }
        fn get_byte_vector_values(&self, _f: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
            Ok(None)
        }
        fn search_nearest_vectors(
            &self,
            _field: &str,
            _target: &[f32],
            _collector: &mut dyn KnnCollector,
            _accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }
        fn search_nearest_vectors_byte(
            &self,
            _field: &str,
            _target: &[u8],
            _collector: &mut dyn KnnCollector,
            _accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }
        fn get_field_infos(&self) -> FieldInfos {
            self.infos.clone()
        }
        fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
            None
        }
        fn get_point_values(&self, _f: &str) -> Result<Option<Box<dyn PointValues>>> {
            Ok(None)
        }
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_meta_data(&self) -> LeafMetaData {
            LeafMetaData::new(10, None, None, false).unwrap()
        }
    }

    fn composite(leaves: Vec<Arc<dyn IndexReader>>) -> Arc<dyn IndexReader> {
        Arc::new(MultiReader::new(leaves, false).unwrap()) as Arc<dyn IndexReader>
    }

    /// Drains a numeric iterator into `(doc, value)` pairs.
    fn drain_numeric(values: &mut dyn NumericDocValues) -> Vec<(i32, i64)> {
        let mut out = Vec::new();
        loop {
            let doc = values.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return out;
            }
            out.push((doc, values.long_value().unwrap()));
        }
    }

    fn text(bytes: &BytesRef) -> String {
        String::from_utf8(bytes.slice().to_vec()).unwrap()
    }

    /// Drains a binary iterator into `(doc, bytes)` pairs.
    fn drain_binary(values: &mut dyn BinaryDocValues) -> Vec<(i32, String)> {
        let mut out = Vec::new();
        loop {
            let doc = values.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return out;
            }
            out.push((doc, text(&values.binary_value().unwrap())));
        }
    }

    /// Drains a sorted-numeric iterator into `(doc, values)` pairs.
    fn drain_sorted_numeric(values: &mut dyn SortedNumericDocValues) -> Vec<(i32, Vec<i64>)> {
        let mut out = Vec::new();
        loop {
            let doc = values.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return out;
            }
            let count = values.doc_value_count().unwrap();
            let mut per_doc = Vec::with_capacity(count as usize);
            for _ in 0..count {
                per_doc.push(values.next_value().unwrap());
            }
            out.push((doc, per_doc));
        }
    }

    /// Drains a sorted iterator into `(doc, global ordinal, term)` triples.
    fn drain_sorted(values: &mut dyn SortedDocValues) -> Vec<(i32, i32, String)> {
        let mut out = Vec::new();
        loop {
            let doc = values.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return out;
            }
            let ord = values.ord_value().unwrap();
            out.push((doc, ord, text(&values.lookup_ord(ord).unwrap())));
        }
    }

    /// Drains a sorted-set iterator into `(doc, global ordinals)` pairs.
    fn drain_sorted_set(values: &mut dyn SortedSetDocValues) -> Vec<(i32, Vec<i64>)> {
        let mut out = Vec::new();
        loop {
            let doc = values.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return out;
            }
            let count = values.doc_value_count().unwrap();
            let mut ords = Vec::with_capacity(count as usize);
            for _ in 0..count {
                ords.push(values.next_ord().unwrap());
            }
            out.push((doc, ords));
        }
    }

    // =======================================================================
    // Numeric doc values
    // =======================================================================

    /// Three leaves of 4, 3 and 5 documents (`docBase` 0, 4 and 7).
    fn numeric_composite() -> Arc<dyn IndexReader> {
        composite(vec![
            StubLeafBuilder::new(4)
                .numeric("n", &[(0, 10), (2, 12)])
                .build(),
            StubLeafBuilder::new(3).numeric("n", &[(1, 21)]).build(),
            StubLeafBuilder::new(5)
                .numeric("n", &[(0, 30), (4, 34)])
                .build(),
        ])
    }

    #[test]
    fn numeric_values_concatenate_leaves_with_doc_base_offsets() {
        let reader = numeric_composite();
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .expect("at least one leaf declares the field");
        assert_eq!(
            drain_numeric(values.as_mut()),
            vec![(0, 10), (2, 12), (5, 21), (7, 30), (11, 34)],
            "local doc IDs must be re-based by each leaf's docBase"
        );
        assert_eq!(values.doc_id(), NO_MORE_DOCS);
    }

    #[test]
    fn numeric_values_are_none_for_a_composite_without_leaves() {
        let reader = composite(vec![]);
        assert!(MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .is_none());
    }

    #[test]
    fn numeric_values_bypass_the_merge_for_a_single_leaf() {
        let reader = composite(vec![StubLeafBuilder::new(4)
            .numeric("n", &[(0, 10), (3, 13)])
            .build()]);
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        // The merged view reports cost 0 (Lucene's "TODO"); the leaf's own
        // instance reports its real cost, which is how we can tell that the
        // single-leaf path returned the leaf's instance untouched.
        assert_eq!(values.cost(), 2);
        assert_eq!(drain_numeric(values.as_mut()), vec![(0, 10), (3, 13)]);
    }

    #[test]
    fn numeric_values_report_cost_zero_when_merging() {
        let reader = numeric_composite();
        let values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert_eq!(
            values.cost(),
            0,
            "leaves are opened lazily, so no cost can be summed up front"
        );
    }

    #[test]
    fn numeric_values_are_none_when_no_leaf_declares_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(3).numeric("other", &[(0, 1)]).build(),
            StubLeafBuilder::new(3).numeric("other", &[(0, 2)]).build(),
        ]);
        assert!(MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .is_none());
    }

    #[test]
    fn numeric_values_are_none_when_the_field_is_declared_with_another_doc_values_type() {
        // The field exists in both leaves, but as BINARY; Lucene keys the
        // "anyReal" check on DocValuesType, not on the field name.
        let reader = composite(vec![
            StubLeafBuilder::new(3).binary("n", &[(0, b"x")]).build(),
            StubLeafBuilder::new(3).binary("n", &[(0, b"y")]).build(),
        ]);
        assert!(MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .is_none());
    }

    #[test]
    fn numeric_values_skip_leaves_without_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).numeric("n", &[(0, 1)]).build(),
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2).numeric("n", &[(1, 5)]).build(),
        ]);
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert_eq!(drain_numeric(values.as_mut()), vec![(0, 1), (5, 5)]);
    }

    #[test]
    fn numeric_values_open_each_leaf_at_most_once_per_pass() {
        let first = StubLeafBuilder::new(2).numeric("n", &[(0, 1), (1, 2)]);
        let log = first.fetch_log();
        let reader = composite(vec![
            first.build(),
            StubLeafBuilder::new(2).numeric("n", &[(0, 3)]).build(),
        ]);
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        // `get_numeric_values` itself only reads FieldInfos, so nothing has
        // been fetched yet.
        assert!(log.lock().unwrap().is_empty());
        drain_numeric(values.as_mut());
        assert_eq!(
            *log.lock().unwrap(),
            vec!["numeric:n".to_string()],
            "the first leaf's doc values are opened once and then cached for its two documents"
        );
    }

    #[test]
    fn numeric_values_advance_jumps_over_whole_leaves() {
        let reader = numeric_composite();
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert_eq!(values.advance(5).unwrap(), 5);
        assert_eq!(values.long_value().unwrap(), 21);
        assert_eq!(
            values.advance(8).unwrap(),
            11,
            "leaf 2 has no doc 8, 9 or 10"
        );
        assert_eq!(values.long_value().unwrap(), 34);
    }

    #[test]
    fn numeric_values_advance_lands_on_the_next_document_with_a_value() {
        let reader = numeric_composite();
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        // Doc 1 has no value in leaf 0, and doc 3 has none either, so the next
        // value lives in leaf 1.
        assert_eq!(values.advance(1).unwrap(), 2);
        assert_eq!(values.advance(3).unwrap(), 5);
    }

    #[test]
    fn numeric_values_advance_past_the_last_document_is_exhausted() {
        let reader = numeric_composite();
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert_eq!(values.advance(12).unwrap(), NO_MORE_DOCS);
        assert_eq!(values.doc_id(), NO_MORE_DOCS);
    }

    #[test]
    fn numeric_values_reject_a_backwards_advance() {
        let reader = numeric_composite();
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert_eq!(values.next_doc().unwrap(), 0);
        assert_eq!(values.advance(2).unwrap(), 2);
        assert!(matches!(
            values.advance(2),
            Err(LuceneError::IllegalArgument(_))
        ));
        assert!(matches!(
            values.advance(1),
            Err(LuceneError::IllegalArgument(_))
        ));
    }

    #[test]
    fn numeric_values_advance_exact_reports_documents_without_a_value() {
        let reader = numeric_composite();
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert!(values.advance_exact(0).unwrap());
        assert_eq!(values.long_value().unwrap(), 10);
        assert!(!values.advance_exact(1).unwrap(), "doc 1 has no value");
        assert_eq!(values.doc_id(), 1, "the cursor still moves to the target");
        assert!(values.advance_exact(2).unwrap());
        assert_eq!(values.long_value().unwrap(), 12);
        // Crossing into leaf 1 (docBase 4) and then leaf 2 (docBase 7).
        assert!(values.advance_exact(5).unwrap());
        assert_eq!(values.long_value().unwrap(), 21);
        assert!(values.advance_exact(11).unwrap());
        assert_eq!(values.long_value().unwrap(), 34);
    }

    #[test]
    fn numeric_values_advance_exact_returns_false_inside_a_leaf_without_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).numeric("n", &[(0, 1)]).build(),
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2).numeric("n", &[(1, 5)]).build(),
        ]);
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert!(!values.advance_exact(2).unwrap(), "leaf 1 has no values");
        assert_eq!(values.doc_id(), 2);
        assert!(values.advance_exact(5).unwrap());
        assert_eq!(values.long_value().unwrap(), 5);
    }

    /// After `advance_exact` has parked the cursor inside a leaf that has no
    /// values for the field, `advance` cannot ask that leaf anything — it must
    /// resume the forward scan from the next leaf. Java dereferences a null
    /// `currentValues` on this path.
    #[test]
    fn numeric_values_advance_after_advance_exact_parked_on_a_leaf_without_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).numeric("n", &[(0, 1)]).build(),
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2).numeric("n", &[(0, 7)]).build(),
        ]);
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert!(!values.advance_exact(2).unwrap());
        assert_eq!(values.advance(3).unwrap(), 4);
        assert_eq!(values.long_value().unwrap(), 7);
    }

    #[test]
    fn binary_values_advance_after_advance_exact_parked_on_a_leaf_without_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).binary("b", &[(0, b"a")]).build(),
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2).binary("b", &[(0, b"z")]).build(),
        ]);
        let mut values = MultiDocValues::get_binary_values(&reader, "b")
            .unwrap()
            .unwrap();
        assert!(!values.advance_exact(2).unwrap());
        assert_eq!(values.advance(3).unwrap(), 4);
        assert_eq!(text(&values.binary_value().unwrap()), "z");
    }

    #[test]
    fn numeric_values_reject_a_backwards_advance_exact() {
        let reader = numeric_composite();
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert!(values.advance_exact(2).unwrap());
        assert!(matches!(
            values.advance_exact(1),
            Err(LuceneError::IllegalArgument(_))
        ));
        assert!(
            values.advance_exact(2).is_ok(),
            "advance_exact may repeat the current document, unlike advance, which rejects it"
        );
    }

    #[test]
    fn numeric_values_report_no_value_before_being_positioned() {
        let reader = numeric_composite();
        let values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert_eq!(values.doc_id(), -1);
        assert!(matches!(
            values.long_value(),
            Err(LuceneError::IllegalState(_))
        ));
    }

    // =======================================================================
    // Norms
    // =======================================================================

    #[test]
    fn norm_values_merge_across_leaves() {
        let reader = composite(vec![
            StubLeafBuilder::new(2)
                .norms("body", &[(0, 1), (1, 2)])
                .build(),
            StubLeafBuilder::new(2).norms("body", &[(1, 3)]).build(),
        ]);
        let mut values = MultiDocValues::get_norm_values(&reader, "body")
            .unwrap()
            .unwrap();
        assert_eq!(drain_numeric(values.as_mut()), vec![(0, 1), (1, 2), (3, 3)]);
    }

    #[test]
    fn norm_values_are_none_when_every_leaf_omits_norms() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).no_norms_field("body").build(),
            StubLeafBuilder::new(2).no_norms_field("body").build(),
        ]);
        assert!(MultiDocValues::get_norm_values(&reader, "body")
            .unwrap()
            .is_none());
    }

    #[test]
    fn norm_values_are_none_when_no_leaf_knows_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2).build(),
        ]);
        assert!(MultiDocValues::get_norm_values(&reader, "body")
            .unwrap()
            .is_none());
    }

    /// Regression test for a deliberate divergence from Lucene 10.5.0.
    ///
    /// `MultiDocValues.getNormValues(...).advance(target)` updates `nextLeaf`
    /// *after* checking whether the target leaf has norms, so a leaf without
    /// norms leaves `nextLeaf` pointing at an already-consumed leaf and the
    /// fallback `nextDoc()` restarts from there — returning a document *before*
    /// `target` and breaking the `DocIdSetIterator` contract. The sibling
    /// `getNumericValues` does the bookkeeping in the other order and is
    /// correct; this port follows the correct order in both.
    #[test]
    fn norm_values_advance_over_a_leaf_without_norms_never_moves_backwards() {
        let reader = composite(vec![
            StubLeafBuilder::new(3)
                .norms("body", &[(0, 1), (1, 2), (2, 3)])
                .build(),
            // Declares the field but omits norms, so `get_norm_values` yields
            // nothing for this leaf.
            StubLeafBuilder::new(3).no_norms_field("body").build(),
            StubLeafBuilder::new(3).norms("body", &[(0, 9)]).build(),
        ]);
        let mut values = MultiDocValues::get_norm_values(&reader, "body")
            .unwrap()
            .unwrap();
        let doc = values.advance(4).unwrap();
        assert_eq!(
            doc, 6,
            "advance must land at or after the target, never rewind into leaf 0"
        );
        assert_eq!(values.long_value().unwrap(), 9);
    }

    #[test]
    fn norm_values_advance_exact_across_leaves() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).norms("body", &[(0, 1)]).build(),
            StubLeafBuilder::new(2)
                .norms("body", &[(0, 7), (1, 8)])
                .build(),
        ]);
        let mut values = MultiDocValues::get_norm_values(&reader, "body")
            .unwrap()
            .unwrap();
        assert!(values.advance_exact(0).unwrap());
        assert_eq!(values.long_value().unwrap(), 1);
        assert!(!values.advance_exact(1).unwrap());
        assert!(values.advance_exact(3).unwrap());
        assert_eq!(values.long_value().unwrap(), 8);
    }

    // =======================================================================
    // Binary doc values
    // =======================================================================

    #[test]
    fn binary_values_concatenate_leaves_with_doc_base_offsets() {
        let reader = composite(vec![
            StubLeafBuilder::new(2)
                .binary("b", &[(0, b"alpha"), (1, b"beta")])
                .build(),
            StubLeafBuilder::new(3)
                .binary("b", &[(2, b"gamma")])
                .build(),
        ]);
        let mut values = MultiDocValues::get_binary_values(&reader, "b")
            .unwrap()
            .unwrap();
        assert_eq!(
            drain_binary(values.as_mut()),
            vec![
                (0, "alpha".to_string()),
                (1, "beta".to_string()),
                (4, "gamma".to_string()),
            ]
        );
    }

    #[test]
    fn binary_values_are_none_when_no_leaf_declares_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).numeric("b", &[(0, 1)]).build(),
            StubLeafBuilder::new(2).build(),
        ]);
        assert!(MultiDocValues::get_binary_values(&reader, "b")
            .unwrap()
            .is_none());
    }

    #[test]
    fn binary_values_bypass_the_merge_for_a_single_leaf() {
        let reader = composite(vec![StubLeafBuilder::new(2)
            .binary("b", &[(1, b"only")])
            .build()]);
        let mut values = MultiDocValues::get_binary_values(&reader, "b")
            .unwrap()
            .unwrap();
        assert_eq!(values.cost(), 1);
        assert_eq!(drain_binary(values.as_mut()), vec![(1, "only".to_string())]);
    }

    #[test]
    fn binary_values_advance_and_advance_exact_across_leaves() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).binary("b", &[(0, b"a")]).build(),
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2).binary("b", &[(1, b"c")]).build(),
        ]);
        let mut values = MultiDocValues::get_binary_values(&reader, "b")
            .unwrap()
            .unwrap();
        assert_eq!(
            values.advance(1).unwrap(),
            5,
            "leaves 0 and 1 are exhausted"
        );
        assert_eq!(text(&values.binary_value().unwrap()), "c");
        assert!(matches!(
            values.advance(5),
            Err(LuceneError::IllegalArgument(_))
        ));
    }

    #[test]
    fn binary_values_advance_exact_reports_missing_documents() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).binary("b", &[(0, b"a")]).build(),
            StubLeafBuilder::new(2).binary("b", &[(1, b"d")]).build(),
        ]);
        let mut values = MultiDocValues::get_binary_values(&reader, "b")
            .unwrap()
            .unwrap();
        assert!(values.advance_exact(0).unwrap());
        assert!(!values.advance_exact(2).unwrap());
        assert!(values.advance_exact(3).unwrap());
        assert_eq!(text(&values.binary_value().unwrap()), "d");
    }

    // =======================================================================
    // Sorted-numeric doc values
    // =======================================================================

    fn sorted_numeric_composite() -> Arc<dyn IndexReader> {
        composite(vec![
            StubLeafBuilder::new(3)
                .sorted_numeric("sn", &[(0, &[1, 2]), (2, &[3])])
                .build(),
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2)
                .sorted_numeric("sn", &[(1, &[7, 8, 9])])
                .build(),
        ])
    }

    #[test]
    fn sorted_numeric_values_merge_leaves_and_keep_every_value() {
        let reader = sorted_numeric_composite();
        let mut values = MultiDocValues::get_sorted_numeric_values(&reader, "sn")
            .unwrap()
            .unwrap();
        assert_eq!(
            drain_sorted_numeric(values.as_mut()),
            vec![(0, vec![1, 2]), (2, vec![3]), (6, vec![7, 8, 9])],
            "leaf 2 starts at docBase 5, so its doc 1 is global doc 6"
        );
    }

    #[test]
    fn sorted_numeric_values_sum_the_cost_of_every_leaf() {
        let reader = sorted_numeric_composite();
        let values = MultiDocValues::get_sorted_numeric_values(&reader, "sn")
            .unwrap()
            .unwrap();
        assert_eq!(
            values.cost(),
            3,
            "two documents in leaf 0, none in the padded leaf 1, one in leaf 2"
        );
    }

    #[test]
    fn sorted_numeric_values_are_none_when_no_leaf_has_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2).build(),
        ]);
        assert!(MultiDocValues::get_sorted_numeric_values(&reader, "sn")
            .unwrap()
            .is_none());
    }

    #[test]
    fn sorted_numeric_values_advance_over_the_padded_leaf() {
        let reader = sorted_numeric_composite();
        let mut values = MultiDocValues::get_sorted_numeric_values(&reader, "sn")
            .unwrap()
            .unwrap();
        assert_eq!(values.advance(3).unwrap(), 6);
        assert_eq!(values.doc_value_count().unwrap(), 3);
        assert_eq!(values.advance(7).unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn sorted_numeric_values_advance_exact_reports_missing_documents() {
        let reader = sorted_numeric_composite();
        let mut values = MultiDocValues::get_sorted_numeric_values(&reader, "sn")
            .unwrap()
            .unwrap();
        assert!(values.advance_exact(0).unwrap());
        assert_eq!(values.doc_value_count().unwrap(), 2);
        assert!(!values.advance_exact(4).unwrap(), "leaf 1 is padded");
        assert!(values.advance_exact(6).unwrap());
        assert_eq!(values.next_value().unwrap(), 7);
    }

    #[test]
    fn sorted_numeric_values_bypass_the_merge_for_a_single_leaf() {
        let reader = composite(vec![StubLeafBuilder::new(2)
            .sorted_numeric("sn", &[(0, &[4])])
            .build()]);
        let mut values = MultiDocValues::get_sorted_numeric_values(&reader, "sn")
            .unwrap()
            .unwrap();
        assert_eq!(drain_sorted_numeric(values.as_mut()), vec![(0, vec![4])]);
    }

    // =======================================================================
    // Sorted doc values and global ordinals
    // =======================================================================

    /// Two leaves whose term dictionaries interleave: leaf 0 holds `b` and `d`,
    /// leaf 1 holds `a`, `c` and `e`. Global order is `a b c d e`.
    fn sorted_composite() -> Arc<dyn IndexReader> {
        composite(vec![
            StubLeafBuilder::new(2)
                .sorted("s", &["b", "d"], &[(0, 0), (1, 1)])
                .build(),
            StubLeafBuilder::new(2)
                .sorted("s", &["a", "c", "e"], &[(0, 0), (1, 2)])
                .build(),
        ])
    }

    #[test]
    fn sorted_values_translate_segment_ordinals_into_a_global_space() {
        let reader = sorted_composite();
        let mut values = MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .unwrap();
        assert_eq!(
            drain_sorted(values.as_mut()),
            vec![
                (0, 1, "b".to_string()),
                (1, 3, "d".to_string()),
                (2, 0, "a".to_string()),
                (3, 4, "e".to_string()),
            ],
            "segment ordinals 0/1 of leaf 0 become global 1/3, and 0/2 of leaf 1 become 0/4"
        );
    }

    #[test]
    fn sorted_values_count_distinct_terms_across_leaves() {
        // "a" appears in both leaves, so the global space holds three terms,
        // not the four the per-leaf counts would suggest.
        let reader = composite(vec![
            StubLeafBuilder::new(2)
                .sorted("s", &["a", "c"], &[(0, 0), (1, 1)])
                .build(),
            StubLeafBuilder::new(2)
                .sorted("s", &["a", "b"], &[(0, 1), (1, 0)])
                .build(),
        ]);
        let values = MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .unwrap();
        assert_eq!(values.get_value_count().unwrap(), 3);
    }

    #[test]
    fn sorted_values_lookup_ord_resolves_every_global_ordinal() {
        let reader = composite(vec![
            StubLeafBuilder::new(2)
                .sorted("s", &["a", "c"], &[(0, 0), (1, 1)])
                .build(),
            StubLeafBuilder::new(2)
                .sorted("s", &["a", "b"], &[(0, 1), (1, 0)])
                .build(),
        ]);
        let values = MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .unwrap();
        assert_eq!(text(&values.lookup_ord(0).unwrap()), "a");
        assert_eq!(text(&values.lookup_ord(1).unwrap()), "b");
        assert_eq!(text(&values.lookup_ord(2).unwrap()), "c");
    }

    /// The acceptance criterion for this task: a leaf that does not have the
    /// field must still occupy its own slot in the ordinal map, so that the
    /// leaves after it keep the segment number the map was built with.
    #[test]
    fn sorted_values_keep_segment_numbering_when_a_leaf_lacks_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(1)
                .sorted("s", &["b"], &[(0, 0)])
                .build(),
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2)
                .sorted("s", &["a", "c"], &[(0, 0), (1, 1)])
                .build(),
        ]);
        let mut values = MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .unwrap();
        assert_eq!(values.get_value_count().unwrap(), 3);
        assert_eq!(
            drain_sorted(values.as_mut()),
            vec![
                (0, 1, "b".to_string()),
                (3, 0, "a".to_string()),
                (4, 2, "c".to_string()),
            ],
            "leaf 2 must still be segment 2 of the ordinal map, not segment 1"
        );
    }

    #[test]
    fn sorted_values_are_none_when_no_leaf_has_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2).build(),
        ]);
        assert!(MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .is_none());
    }

    #[test]
    fn sorted_values_bypass_the_merge_for_a_single_leaf() {
        let reader = composite(vec![StubLeafBuilder::new(2)
            .sorted("s", &["x", "y"], &[(0, 1), (1, 0)])
            .build()]);
        let mut values = MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .unwrap();
        assert_eq!(
            drain_sorted(values.as_mut()),
            vec![(0, 1, "y".to_string()), (1, 0, "x".to_string())],
            "a single leaf keeps its own ordinals; no ordinal map is built"
        );
    }

    #[test]
    fn sorted_values_sum_the_cost_of_leaves_that_have_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(2)
                .sorted("s", &["b", "d"], &[(0, 0), (1, 1)])
                .build(),
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2)
                .sorted("s", &["a"], &[(1, 0)])
                .build(),
        ]);
        let values = MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .unwrap();
        assert_eq!(values.cost(), 3, "padded leaves contribute no cost");
    }

    #[test]
    fn sorted_values_advance_and_advance_exact_across_leaves() {
        let reader = sorted_composite();
        let mut values = MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .unwrap();
        assert_eq!(values.advance(2).unwrap(), 2);
        assert_eq!(values.ord_value().unwrap(), 0);
        assert!(values.advance_exact(3).unwrap());
        assert_eq!(values.ord_value().unwrap(), 4);
    }

    /// The trailing `maxDoc` sentinel in `doc_starts` is what makes this branch
    /// reachable: a target at or past `maxDoc` resolves to the sentinel slot,
    /// which is one past the last leaf.
    #[test]
    fn sorted_values_advance_past_max_doc_is_exhausted() {
        let reader = sorted_composite();
        let mut values = MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .unwrap();
        assert_eq!(values.advance(4).unwrap(), NO_MORE_DOCS);
        assert_eq!(values.doc_id(), NO_MORE_DOCS);
    }

    #[test]
    fn sorted_values_advance_exact_past_max_doc_is_out_of_range() {
        let reader = sorted_composite();
        let mut values = MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .unwrap();
        let err = values.advance_exact(4).unwrap_err();
        assert!(
            matches!(&err, LuceneError::IllegalArgument(m) if m.contains("Out of range")),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn sorted_values_reject_a_backwards_advance() {
        let reader = sorted_composite();
        let mut values = MultiDocValues::get_sorted_values(&reader, "s")
            .unwrap()
            .unwrap();
        assert_eq!(values.next_doc().unwrap(), 0);
        assert!(matches!(
            values.advance(0),
            Err(LuceneError::IllegalArgument(_))
        ));
    }

    #[test]
    fn multi_sorted_doc_values_expose_their_inputs() {
        let leaf_values: Vec<Box<dyn SortedDocValues>> = vec![
            ListSorted::boxed(&["b"], &[(0, 0)]),
            ListSorted::boxed(&["a"], &[(0, 0)]),
        ];
        let map_inputs: Vec<Box<dyn SortedDocValues>> = vec![
            ListSorted::boxed(&["b"], &[(0, 0)]),
            ListSorted::boxed(&["a"], &[(0, 0)]),
        ];
        let mapping = OrdinalMap::build_sorted(map_inputs, 0.25).unwrap();
        let values = MultiSortedDocValues::new(leaf_values, vec![0, 1, 2], mapping, 2).unwrap();
        assert_eq!(values.doc_starts(), &[0, 1, 2]);
        assert_eq!(values.values().len(), 2);
        assert_eq!(values.mapping().get_value_count(), 2);
        assert_eq!(values.cost(), 2);
    }

    #[test]
    fn multi_sorted_doc_values_reject_a_doc_start_table_of_the_wrong_length() {
        let leaf_values: Vec<Box<dyn SortedDocValues>> = vec![ListSorted::boxed(&["b"], &[(0, 0)])];
        let map_inputs: Vec<Box<dyn SortedDocValues>> = vec![ListSorted::boxed(&["b"], &[(0, 0)])];
        let mapping = OrdinalMap::build_sorted(map_inputs, 0.25).unwrap();
        // One leaf needs two doc starts (its docBase plus the maxDoc sentinel).
        let err = MultiSortedDocValues::new(leaf_values, vec![0], mapping, 1).unwrap_err();
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    // =======================================================================
    // Sorted-set doc values and global ordinals
    // =======================================================================

    /// Leaf 0 holds `b` and `d`, leaf 1 holds `a`, `c` and `d`. Global order is
    /// `a b c d`, so `d` is shared and collapses to a single global ordinal.
    fn sorted_set_composite() -> Arc<dyn IndexReader> {
        composite(vec![
            StubLeafBuilder::new(2)
                .sorted_set("ss", &["b", "d"], &[(0, &[0, 1]), (1, &[1])])
                .build(),
            StubLeafBuilder::new(2)
                .sorted_set("ss", &["a", "c", "d"], &[(1, &[0, 2])])
                .build(),
        ])
    }

    #[test]
    fn sorted_set_values_translate_every_ordinal_of_every_document() {
        let reader = sorted_set_composite();
        let mut values = MultiDocValues::get_sorted_set_values(&reader, "ss")
            .unwrap()
            .unwrap();
        assert_eq!(
            drain_sorted_set(values.as_mut()),
            vec![(0, vec![1, 3]), (1, vec![3]), (3, vec![0, 3])],
            "leaf 0's ords 0/1 map to global 1/3; leaf 1's ords 0/2 map to global 0/3"
        );
    }

    #[test]
    fn sorted_set_values_count_distinct_terms_across_leaves() {
        let reader = sorted_set_composite();
        let values = MultiDocValues::get_sorted_set_values(&reader, "ss")
            .unwrap()
            .unwrap();
        assert_eq!(
            values.get_value_count().unwrap(),
            4,
            "a, b, c, d — the shared 'd' is counted once"
        );
    }

    #[test]
    fn sorted_set_values_lookup_ord_resolves_every_global_ordinal() {
        let reader = sorted_set_composite();
        let values = MultiDocValues::get_sorted_set_values(&reader, "ss")
            .unwrap()
            .unwrap();
        let terms: Vec<String> = (0..4)
            .map(|ord| text(&values.lookup_ord(ord).unwrap()))
            .collect();
        assert_eq!(terms, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn sorted_set_values_keep_segment_numbering_when_a_leaf_lacks_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(1)
                .sorted_set("ss", &["b"], &[(0, &[0])])
                .build(),
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2)
                .sorted_set("ss", &["a", "c"], &[(1, &[0, 1])])
                .build(),
        ]);
        let mut values = MultiDocValues::get_sorted_set_values(&reader, "ss")
            .unwrap()
            .unwrap();
        assert_eq!(values.get_value_count().unwrap(), 3);
        assert_eq!(
            drain_sorted_set(values.as_mut()),
            vec![(0, vec![1]), (4, vec![0, 2])]
        );
    }

    #[test]
    fn sorted_set_values_are_none_when_no_leaf_has_the_field() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).build(),
            StubLeafBuilder::new(2).build(),
        ]);
        assert!(MultiDocValues::get_sorted_set_values(&reader, "ss")
            .unwrap()
            .is_none());
    }

    #[test]
    fn sorted_set_values_bypass_the_merge_for_a_single_leaf() {
        let reader = composite(vec![StubLeafBuilder::new(1)
            .sorted_set("ss", &["x", "y"], &[(0, &[0, 1])])
            .build()]);
        let mut values = MultiDocValues::get_sorted_set_values(&reader, "ss")
            .unwrap()
            .unwrap();
        assert_eq!(drain_sorted_set(values.as_mut()), vec![(0, vec![0, 1])]);
    }

    #[test]
    fn sorted_set_values_advance_and_advance_exact_across_leaves() {
        let reader = sorted_set_composite();
        let mut values = MultiDocValues::get_sorted_set_values(&reader, "ss")
            .unwrap()
            .unwrap();
        assert_eq!(values.advance(2).unwrap(), 3, "leaf 1 has no doc 0");
        assert_eq!(values.doc_value_count().unwrap(), 2);
        assert_eq!(values.next_ord().unwrap(), 0);
        assert_eq!(values.next_ord().unwrap(), 3);
    }

    #[test]
    fn sorted_set_values_advance_exact_past_max_doc_is_out_of_range() {
        let reader = sorted_set_composite();
        let mut values = MultiDocValues::get_sorted_set_values(&reader, "ss")
            .unwrap()
            .unwrap();
        let err = values.advance_exact(4).unwrap_err();
        assert!(
            matches!(&err, LuceneError::IllegalArgument(m) if m.contains("Out of range")),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn sorted_set_values_advance_past_max_doc_is_exhausted() {
        let reader = sorted_set_composite();
        let mut values = MultiDocValues::get_sorted_set_values(&reader, "ss")
            .unwrap()
            .unwrap();
        assert_eq!(values.advance(4).unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn sorted_set_values_sum_the_cost_of_leaves_that_have_the_field() {
        let reader = sorted_set_composite();
        let values = MultiDocValues::get_sorted_set_values(&reader, "ss")
            .unwrap()
            .unwrap();
        assert_eq!(values.cost(), 3);
    }

    #[test]
    fn multi_sorted_set_doc_values_reject_a_doc_start_table_of_the_wrong_length() {
        let leaf_values: Vec<Box<dyn SortedSetDocValues>> =
            vec![ListSortedSet::boxed(&["b"], &[(0, &[0])])];
        let map_inputs: Vec<Box<dyn SortedSetDocValues>> =
            vec![ListSortedSet::boxed(&["b"], &[(0, &[0])])];
        let mapping = OrdinalMap::build_sorted_set(map_inputs, 0.25).unwrap();
        let err = MultiSortedSetDocValues::new(leaf_values, vec![0, 1, 2], mapping, 1).unwrap_err();
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn multi_sorted_set_doc_values_expose_their_inputs() {
        let leaf_values: Vec<Box<dyn SortedSetDocValues>> = vec![
            ListSortedSet::boxed(&["b"], &[(0, &[0])]),
            ListSortedSet::boxed(&["a"], &[(0, &[0])]),
        ];
        let map_inputs: Vec<Box<dyn SortedSetDocValues>> = vec![
            ListSortedSet::boxed(&["b"], &[(0, &[0])]),
            ListSortedSet::boxed(&["a"], &[(0, &[0])]),
        ];
        let mapping = OrdinalMap::build_sorted_set(map_inputs, 0.25).unwrap();
        let values = MultiSortedSetDocValues::new(leaf_values, vec![0, 1, 2], mapping, 2).unwrap();
        assert_eq!(values.doc_starts(), &[0, 1, 2]);
        assert_eq!(values.values().len(), 2);
        assert_eq!(values.mapping().get_value_count(), 2);
    }

    // =======================================================================
    // advance_exact past maxDoc: the numeric / binary / norms family
    // =======================================================================
    //
    // The sorted and sorted-set views are backed by a `doc_starts` table that
    // carries a trailing `maxDoc` sentinel (`docStarts.length == values.length
    // + 1`, `MultiDocValues.java:704`), so a target at or past `maxDoc`
    // resolves to a reader index equal to `values.len()` and the
    // "Out of range" guard fires.
    //
    // The numeric, binary and norms views resolve against the *leaves* list,
    // which has no sentinel, so `ReaderUtil.subIndex` can never return
    // `leaves.size()` and the identical guard in
    // `MultiDocValues.java:151` / `:281` is dead code. The target is instead
    // handed to the last leaf, offset by its `docBase`. That asymmetry is
    // Lucene's, and these tests pin the port to it: what must not happen is a
    // spurious "Out of range" error on this family, or a panic.

    #[test]
    fn numeric_values_advance_exact_past_max_doc_delegates_to_the_last_leaf() {
        let reader = numeric_composite(); // leaves of 4, 3 and 5 docs; maxDoc 12
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        // maxDoc itself, and well past it: the last leaf has no such document,
        // so the answer is "no value here", not an error.
        assert!(!values.advance_exact(12).unwrap());
        assert!(!values.advance_exact(1_000).unwrap());
        // The out-of-range target still counts as a position, so the
        // advance-backwards guard fires afterwards, exactly as in Java.
        assert!(matches!(
            values.advance_exact(11),
            Err(LuceneError::IllegalArgument(_))
        ));
    }

    #[test]
    fn numeric_values_advance_past_max_doc_is_exhausted() {
        let reader = numeric_composite();
        let mut values = MultiDocValues::get_numeric_values(&reader, "n")
            .unwrap()
            .unwrap();
        assert_eq!(values.advance(12).unwrap(), NO_MORE_DOCS);
        assert_eq!(values.doc_id(), NO_MORE_DOCS);
    }

    #[test]
    fn binary_values_advance_exact_past_max_doc_delegates_to_the_last_leaf() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).binary("b", &[(0, b"a")]).build(),
            StubLeafBuilder::new(3).binary("b", &[(1, b"z")]).build(),
        ]);
        let mut values = MultiDocValues::get_binary_values(&reader, "b")
            .unwrap()
            .unwrap();
        assert!(!values.advance_exact(5).unwrap(), "maxDoc is 5");
        assert!(!values.advance_exact(99).unwrap());
    }

    #[test]
    fn binary_values_advance_past_max_doc_is_exhausted() {
        let reader = composite(vec![
            StubLeafBuilder::new(2).binary("b", &[(0, b"a")]).build(),
            StubLeafBuilder::new(3).binary("b", &[(1, b"z")]).build(),
        ]);
        let mut values = MultiDocValues::get_binary_values(&reader, "b")
            .unwrap()
            .unwrap();
        assert_eq!(values.advance(5).unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn norm_values_advance_exact_past_max_doc_delegates_to_the_last_leaf() {
        let reader = composite(vec![
            StubLeafBuilder::new(2)
                .norms("body", &[(0, 1), (1, 2)])
                .build(),
            StubLeafBuilder::new(2).norms("body", &[(1, 3)]).build(),
        ]);
        let mut values = MultiDocValues::get_norm_values(&reader, "body")
            .unwrap()
            .unwrap();
        assert!(!values.advance_exact(4).unwrap(), "maxDoc is 4");
        assert!(!values.advance_exact(50).unwrap());
    }

    #[test]
    fn norm_values_advance_past_max_doc_is_exhausted() {
        let reader = composite(vec![
            StubLeafBuilder::new(2)
                .norms("body", &[(0, 1), (1, 2)])
                .build(),
            StubLeafBuilder::new(2).norms("body", &[(1, 3)]).build(),
        ]);
        let mut values = MultiDocValues::get_norm_values(&reader, "body")
            .unwrap()
            .unwrap();
        assert_eq!(values.advance(4).unwrap(), NO_MORE_DOCS);
    }
}
