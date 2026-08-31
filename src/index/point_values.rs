//! Point value accessors ported from `org.apache.lucene.index`.
//!
//! Equivalent to `org.apache.lucene.index.PointValues` together with its
//! nested `PointTree` and `IntersectVisitor` interfaces.
//!
//! This module owns the **single** definition of [`Relation`] and
//! [`IntersectVisitor`] used across the crate: `crate::util::bkd` and
//! `crate::codecs::points` re-export these types rather than declaring their
//! own. Java does the same — `org.apache.lucene.util.bkd.BKDReader` imports
//! `org.apache.lucene.index.PointValues`.
//!
//! # Where the algorithms live
//!
//! In Java, `intersect`, `estimatePointCount` and `estimateDocCount` are
//! `final`: the traversal belongs to Lucene, and an implementation only
//! supplies a `PointTree`. Rust has no `final`, so the algorithms are written
//! once as the free functions [`intersect`], [`estimate_point_count`] and
//! [`estimate_doc_count`], and [`PointValues`] exposes them as default methods
//! that implementations must not override.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::index_reader::IndexReader;
use crate::search::DocIdSetIterator;
use crate::util::bkd::BKDConfig;
use crate::util::IntsRef;

/// Maximum number of bytes for each dimension.
///
/// Equivalent to `PointValues.MAX_NUM_BYTES`, which is a literal in Java too.
pub const MAX_NUM_BYTES: i32 = 16;

/// Maximum number of dimensions.
///
/// Equivalent to `PointValues.MAX_DIMENSIONS`, which is defined as
/// `BKDConfig.MAX_DIMS`; the value is derived here for the same reason, so the
/// two cannot drift apart.
pub const MAX_DIMENSIONS: i32 = BKDConfig::MAX_DIMS;

/// Maximum number of index dimensions.
///
/// Equivalent to `PointValues.MAX_INDEX_DIMENSIONS`, defined as
/// `BKDConfig.MAX_INDEX_DIMS`.
pub const MAX_INDEX_DIMENSIONS: i32 = BKDConfig::MAX_INDEX_DIMS;

// -----------------------------------------------------------------------------
// Relation
// -----------------------------------------------------------------------------

/// Relationship between a point cell and a query shape.
///
/// Equivalent to `PointValues.Relation`, declared in the same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// The cell is fully contained by the query.
    CellInsideQuery,
    /// The cell and query do not overlap.
    CellOutsideQuery,
    /// The cell partially overlaps the query.
    CellCrossesQuery,
}

// -----------------------------------------------------------------------------
// Intersect visitor
// -----------------------------------------------------------------------------

/// Visitor that guides recursion through a point index.
///
/// Equivalent to `org.apache.lucene.index.PointValues.IntersectVisitor`.
///
/// The trait deliberately carries no `Send`/`Sync` bound: Java imposes none,
/// and visitors are short-lived adapters that frequently borrow non-shareable
/// state (a writer, an output buffer) for the duration of one traversal.
pub trait IntersectVisitor {
    /// Called for all documents in a leaf cell that is fully inside the query.
    ///
    /// The consumer should **blindly accept** the doc ID: the cell has already
    /// been proven to be contained by the query, so no further filtering is
    /// needed.
    ///
    /// # Errors
    ///
    /// Returns an error when the consumer fails, for instance while writing.
    fn visit(&mut self, doc_id: i32) -> Result<()>;

    /// Bulk visit of doc IDs from an iterator.
    ///
    /// The iterator is guaranteed **not** to be positioned. The default
    /// implementation drains it and calls [`visit`](Self::visit) for each doc
    /// ID, exactly as Java's default does.
    ///
    /// # Errors
    ///
    /// Propagates iteration errors and whatever [`visit`](Self::visit) returns.
    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        loop {
            let doc_id = iterator.next_doc()?;
            if doc_id == crate::search::NO_MORE_DOCS {
                break;
            }
            self.visit(doc_id)?;
        }
        Ok(())
    }

    /// Bulk visit of doc IDs from an [`IntsRef`].
    ///
    /// Exists so that implementations can avoid one virtual call per doc ID.
    /// The default implementation walks the active slice and calls
    /// [`visit`](Self::visit).
    ///
    /// # Errors
    ///
    /// Propagates whatever [`visit`](Self::visit) returns.
    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        for doc_id in ints_ref.slice().iter().copied() {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    /// Called for each document in a leaf cell that **crosses** the query.
    ///
    /// The consumer must scrutinise `packed_value` to decide whether to accept
    /// the document.
    ///
    /// # Ordering contract
    ///
    /// In the one-dimensional case values are visited in increasing order and,
    /// on ties, in increasing doc-ID order. This is a guarantee Lucene makes to
    /// consumers and several queries rely on it.
    ///
    /// # Errors
    ///
    /// Returns an error when the consumer fails.
    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()>;

    /// Bulk visit of several doc IDs sharing one packed value.
    ///
    /// The iterator must **not** escape the scope of this method, so that
    /// [`PointValues`] implementations are free to reuse it.
    ///
    /// # Errors
    ///
    /// Propagates iteration errors and whatever
    /// [`visit_with_value`](Self::visit_with_value) returns.
    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        loop {
            let doc_id = iterator.next_doc()?;
            if doc_id == crate::search::NO_MORE_DOCS {
                break;
            }
            self.visit_with_value(doc_id, packed_value)?;
        }
        Ok(())
    }

    /// Tests a non-leaf cell against the query to decide how to recurse.
    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation;

    /// Notifies the visitor that **this many** documents are about to be
    /// visited.
    ///
    /// The count is exact, not an estimate: Lucene's consumers size their
    /// buffers from it.
    fn grow(&mut self, _count: i32) {}
}

// -----------------------------------------------------------------------------
// Doc-values visitor (write path)
// -----------------------------------------------------------------------------

/// Visitor that receives every indexed point together with its document id.
///
/// This is a Rucene-specific counterpart to Java's
/// `PointValues.IntersectVisitor` used while **writing** a field: the writer
/// needs to consume all `(doc_id, packed_value)` pairs, not only the ones
/// matching a query. Lucene's writer reads the BKD data directly, but Rucene's
/// writer reaches the same result by enumerating the values through
/// [`PointValues::visit_doc_values`].
///
/// The trait lives in the index layer (next to [`IntersectVisitor`]) so the
/// single [`PointValues`] trait can carry both the query and the write-path
/// visitor; `crate::codecs::points` re-exports it.
pub trait DocValuesVisitor {
    /// Called once for every indexed point value.
    ///
    /// # Errors
    ///
    /// Returns an error when the consumer fails, for instance while writing.
    fn visit(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()>;
}

impl<F> DocValuesVisitor for F
where
    F: FnMut(i32, &[u8]) -> Result<()> + Send + Sync,
{
    fn visit(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        (self)(doc_id, packed_value)
    }
}

/// Adapter that exposes a [`DocValuesVisitor`] as an [`IntersectVisitor`] so the
/// default [`PointValues::visit_doc_values`] can enumerate every point via the
/// tree cursor.
///
/// `compare` always reports [`Relation::CellCrossesQuery`] so the per-value
/// leaf path is taken unconditionally and every stored value is decoded and
/// forwarded through `visit_with_value`; the no-value `visit` path is never
/// reached for this adapter.
struct DocValuesIntersectAdapter<'a> {
    visitor: &'a mut dyn DocValuesVisitor,
}

impl IntersectVisitor for DocValuesIntersectAdapter<'_> {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        // The per-value walk never takes the doc-ID-only fast path.
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.visitor.visit(doc_id, packed_value)
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Relation {
        Relation::CellCrossesQuery
    }
}

// -----------------------------------------------------------------------------
// Point tree
// -----------------------------------------------------------------------------

/// Cursor over the nodes of a KD-tree.
///
/// Equivalent to `org.apache.lucene.index.PointValues.PointTree`.
///
/// A tree starts positioned on its root and is navigated with
/// [`move_to_child`](Self::move_to_child),
/// [`move_to_sibling`](Self::move_to_sibling) and
/// [`move_to_parent`](Self::move_to_parent).
pub trait PointTree {
    /// Returns a new cursor whose root is the node this cursor is on.
    ///
    /// Equivalent to `PointTree.clone()`. It is deliberately **not**
    /// [`std::clone::Clone`]: the result is re-rooted, so
    /// [`move_to_parent`](Self::move_to_parent) on the copy stops where the
    /// original currently is.
    fn clone_tree(&self) -> Box<dyn PointTree>;

    /// Moves to the first child, returning `false` for leaf nodes.
    ///
    /// # Errors
    ///
    /// Returns an error when the node data cannot be read.
    fn move_to_child(&mut self) -> Result<bool>;

    /// Moves to the next sibling, returning `false` when there are none left.
    ///
    /// # Errors
    ///
    /// Returns an error when the node data cannot be read.
    fn move_to_sibling(&mut self) -> Result<bool>;

    /// Moves to the parent, returning `false` at the root.
    ///
    /// # Errors
    ///
    /// Returns an error when the node data cannot be read.
    fn move_to_parent(&mut self) -> Result<bool>;

    /// Returns the minimum packed value of the current node.
    fn min_packed_value(&self) -> &[u8];

    /// Returns the maximum packed value of the current node.
    fn max_packed_value(&self) -> &[u8];

    /// Returns the number of points below the current node.
    fn size(&self) -> i64;

    /// Visits every doc ID below the current node.
    ///
    /// # Errors
    ///
    /// Propagates reader and visitor errors.
    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()>;

    /// Visits every doc ID and packed value below the current node.
    ///
    /// # Errors
    ///
    /// Propagates reader and visitor errors.
    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()>;

    /// Returns this cursor as a [`MutablePointTree`] when it is one.
    ///
    /// This is the Rust stand-in for Java's `values instanceof MutablePointTree`
    /// test in `Lucene90PointsWriter.writeField`
    /// (`Lucene90PointsWriter.java:159`), which is how a codec discovers that
    /// it may reorder the points in place instead of buffering them through
    /// `BKDWriter.add`. The default returns `None`, matching every tree that is
    /// not mutable — in particular the BKD-backed cursor, which reads from
    /// immutable files.
    fn as_mutable(&mut self) -> Option<&mut dyn MutablePointTree> {
        None
    }
}

/// One leaf [`PointTree`] whose point order can be changed.
///
/// Equivalent to `org.apache.lucene.codecs.MutablePointTree`. Lucene declares
/// it in the codec package; this crate declares it beside [`PointTree`], its
/// supertrait, and re-exports it from [`crate::codecs::points`] — the same
/// arrangement that module already uses for [`PointValues`],
/// [`IntersectVisitor`] and [`Relation`], which Java also splits across the
/// `index` and `codecs` packages.
///
/// The point of the trait is that a codec can **sort through it**: the BKD
/// writer partitions and orders the points by calling
/// [`byte_at`](Self::byte_at), [`swap`](Self::swap), [`save`](Self::save) and
/// [`restore`](Self::restore), never touching the underlying buffer. Any
/// deviation in these six methods changes the tree the codec builds, and
/// therefore the bytes of the `.kdd`/`.kdi`/`.kdm` files.
///
/// # The `save`/`restore` contract
///
/// Java documents them as *"Save the i-th value into the j-th position in
/// temporary storage"* and *"Restore values between i-th and j-th (excluding)
/// in temporary storage into original storage"*
/// (`MutablePointTree.java:46` and `:49`). They are **not** symmetric: `save` copies
/// one element from position `i` of the live order to position `j` of the
/// scratch, while `restore` copies the half-open range `[i, j)` from the
/// *same* positions of the scratch back over the live order. The stable
/// MSB radix sort relies on exactly that asymmetry.
///
/// # What Java makes final
///
/// `MutablePointTree` is an abstract class that finalises the navigation half
/// of `PointTree`: `clone()`, `getMinPackedValue()`, `getMaxPackedValue()` and
/// `visitDocIDs()` throw `UnsupportedOperationException`, and `moveToChild()`,
/// `moveToSibling()` and `moveToParent()` return `false` — a mutable tree is
/// always a single leaf that is also its own root. Rust traits cannot finalise
/// a supertrait's methods, so each implementation supplies that behaviour and
/// documents it.
pub trait MutablePointTree: PointTree {
    /// Returns the packed bytes of the `i`-th value.
    ///
    /// Equivalent to `MutablePointTree.getValue(int, BytesRef)`, which fills a
    /// `BytesRef` pointing into the buffer; returning a borrowed slice is the
    /// same thing without the out-parameter.
    fn value(&self, i: i32) -> &[u8];

    /// Returns the `k`-th byte of the `i`-th value.
    ///
    /// Equivalent to `MutablePointTree.getByteAt(int, int)`.
    fn byte_at(&self, i: i32, k: i32) -> u8;

    /// Returns the doc ID of the `i`-th value.
    ///
    /// Equivalent to `MutablePointTree.getDocID(int)`.
    fn doc_id(&self, i: i32) -> i32;

    /// Swaps the `i`-th and `j`-th values.
    ///
    /// Equivalent to `MutablePointTree.swap(int, int)`.
    fn swap(&mut self, i: i32, j: i32);

    /// Saves the `i`-th value into the `j`-th position of temporary storage.
    ///
    /// Equivalent to `MutablePointTree.save(int, int)`.
    fn save(&mut self, i: i32, j: i32);

    /// Restores the values in `[i, j)` from temporary storage.
    ///
    /// Equivalent to `MutablePointTree.restore(int, int)`.
    fn restore(&mut self, i: i32, j: i32);
}

// -----------------------------------------------------------------------------
// The traversal algorithms
// -----------------------------------------------------------------------------

/// Finds all documents and points matching `visitor`.
///
/// Equivalent to the body of the `final` method
/// `PointValues.intersect(IntersectVisitor)`. The tree is left back on the node
/// it started from.
///
/// This does **not** enforce live documents; the caller filters deletions.
///
/// # Errors
///
/// Propagates tree navigation and visitor errors.
pub fn intersect(visitor: &mut dyn IntersectVisitor, point_tree: &mut dyn PointTree) -> Result<()> {
    loop {
        let relation =
            visitor.compare(point_tree.min_packed_value(), point_tree.max_packed_value());
        match relation {
            Relation::CellInsideQuery => {
                // Fully inside: take every point below this cell unfiltered.
                point_tree.visit_doc_ids(visitor)?;
            }
            Relation::CellCrossesQuery => {
                // Crossing, or the cell contains the query: descend if we can,
                // otherwise filter the leaf point by point.
                if point_tree.move_to_child()? {
                    continue;
                }
                point_tree.visit_doc_values(visitor)?;
            }
            Relation::CellOutsideQuery => {}
        }
        while !point_tree.move_to_sibling()? {
            if !point_tree.move_to_parent()? {
                return Ok(());
            }
        }
    }
}

/// Estimates the number of points [`intersect`] would visit.
///
/// Equivalent to the `final` method
/// `PointValues.estimatePointCount(IntersectVisitor)`; the traversal is bounded
/// by `i64::MAX`, so it never terminates early.
///
/// # Errors
///
/// Propagates tree navigation errors.
pub fn estimate_point_count(
    visitor: &mut dyn IntersectVisitor,
    point_tree: &mut dyn PointTree,
) -> Result<i64> {
    estimate_point_count_bounded(visitor, point_tree, i64::MAX)
}

/// Returns whether the estimated point count reaches `upper_bound`.
///
/// Equivalent to
/// `PointValues.isEstimatedPointCountGreaterThanOrEqualTo(IntersectVisitor, PointTree, long)`.
/// The estimation stops as soon as the bound is reached, which is what makes
/// this cheaper than a full [`estimate_point_count`].
///
/// # Errors
///
/// Propagates tree navigation errors.
pub fn is_estimated_point_count_greater_than_or_equal_to(
    visitor: &mut dyn IntersectVisitor,
    point_tree: &mut dyn PointTree,
    upper_bound: i64,
) -> Result<bool> {
    Ok(estimate_point_count_bounded(visitor, point_tree, upper_bound)? >= upper_bound)
}

/// The recursive estimator, bounded by `upper_bound`.
///
/// Equivalent to the private static
/// `PointValues.estimatePointCount(IntersectVisitor, PointTree, long)`.
fn estimate_point_count_bounded(
    visitor: &mut dyn IntersectVisitor,
    point_tree: &mut dyn PointTree,
    upper_bound: i64,
) -> Result<i64> {
    let relation = visitor.compare(point_tree.min_packed_value(), point_tree.max_packed_value());
    match relation {
        // Fully outside: no points.
        Relation::CellOutsideQuery => Ok(0),
        // Fully inside: every point below this cell.
        Relation::CellInsideQuery => Ok(point_tree.size()),
        Relation::CellCrossesQuery => {
            if point_tree.move_to_child()? {
                let mut cost: i64 = 0;
                loop {
                    cost += estimate_point_count_bounded(visitor, point_tree, upper_bound - cost)?;
                    if cost >= upper_bound || !point_tree.move_to_sibling()? {
                        break;
                    }
                }
                point_tree.move_to_parent()?;
                Ok(cost)
            } else {
                // Leaf: assume half the points matched.
                Ok((point_tree.size() + 1) / 2)
            }
        }
    }
}

/// Estimates the number of documents [`intersect`] would match.
///
/// Equivalent to the `final` method
/// `PointValues.estimateDocCount(IntersectVisitor)`. `estimated_point_count`
/// must come from [`estimate_point_count`] over the same visitor.
///
/// For multi-valued fields the estimate uses the urn-problem approximation
/// `D * (1 - ((N - n) / N)^(N/D))`, where `D` is the doc count, `N` the point
/// count and `n` the estimated point count. Every intermediate is computed in
/// `f64`, as in Java.
///
/// # Numerical note
///
/// `Math.pow` is specified to be within 1 ulp of the exact result rather than
/// correctly rounded, so its low bit is not fixed even across JVMs; Rust's
/// `f64::powf` has the same latitude. The result is truncated to an integer,
/// which absorbs the difference in every realistic input.
pub fn estimate_doc_count(estimated_point_count: i64, doc_count: i32, point_count: i64) -> i64 {
    let size = point_count as f64;
    if estimated_point_count as f64 >= size {
        // Matches all docs.
        doc_count as i64
    } else if size == doc_count as f64 || estimated_point_count == 0 {
        // Single-valued field, or nothing matched: the point estimate is the
        // doc estimate.
        estimated_point_count
    } else {
        let doc_estimate = (doc_count as f64
            * (1.0 - ((size - estimated_point_count as f64) / size).powf(size / doc_count as f64)))
            as i64;
        if doc_estimate == 0 {
            1
        } else {
            doc_estimate
        }
    }
}

// -----------------------------------------------------------------------------
// Point values
// -----------------------------------------------------------------------------

/// Access to indexed point values for a single field.
///
/// Equivalent to `org.apache.lucene.index.PointValues`.
///
/// Implementations supply [`point_tree`](Self::point_tree) plus the metadata
/// accessors. [`intersect`](Self::intersect),
/// [`estimate_point_count`](Self::estimate_point_count) and
/// [`estimate_doc_count`](Self::estimate_doc_count) are `final` in Java and
/// **must not** be overridden here either: they are the algorithm, not a
/// customisation point.
pub trait PointValues: Send + Sync {
    /// Creates a new tree cursor to navigate the index.
    ///
    /// Equivalent to `PointValues.getPointTree()`.
    ///
    /// # Errors
    ///
    /// Returns an error when the index cannot be read.
    fn point_tree(&self) -> Result<Box<dyn PointTree>>;

    /// Finds all documents and points matching the provided visitor.
    ///
    /// Do not override: see the trait documentation.
    ///
    /// # Errors
    ///
    /// Propagates tree navigation and visitor errors.
    fn intersect(&self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        let mut tree = self.point_tree()?;
        intersect(visitor, tree.as_mut())?;
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                !tree.move_to_parent()?,
                "intersect must leave the cursor on the root"
            );
        }
        Ok(())
    }

    /// Estimates the number of points that [`intersect`](Self::intersect) would
    /// visit.
    ///
    /// Do not override: see the trait documentation.
    ///
    /// # Errors
    ///
    /// Propagates tree navigation errors.
    fn estimate_point_count(&self, visitor: &mut dyn IntersectVisitor) -> Result<i64> {
        let mut tree = self.point_tree()?;
        let count = estimate_point_count(visitor, tree.as_mut())?;
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                !tree.move_to_parent()?,
                "estimate_point_count must leave the cursor on the root"
            );
        }
        Ok(count)
    }

    /// Estimates the number of documents that [`intersect`](Self::intersect)
    /// would match.
    ///
    /// Do not override: see the trait documentation.
    ///
    /// # Errors
    ///
    /// Propagates tree navigation errors.
    fn estimate_doc_count(&self, visitor: &mut dyn IntersectVisitor) -> Result<i64> {
        let estimated_point_count = self.estimate_point_count(visitor)?;
        Ok(estimate_doc_count(
            estimated_point_count,
            self.doc_count(),
            self.size(),
        ))
    }

    /// Returns the total number of indexed points across all documents.
    fn size(&self) -> i64;

    /// Returns the total number of documents that have indexed at least one
    /// point.
    fn doc_count(&self) -> i32;

    /// Returns the minimum packed value, or `None` when [`size`](Self::size) is
    /// zero.
    ///
    /// # Errors
    ///
    /// Returns an error when the index cannot be read.
    fn min_packed_value(&self) -> Result<Option<Vec<u8>>>;

    /// Returns the maximum packed value, or `None` when [`size`](Self::size) is
    /// zero.
    ///
    /// # Errors
    ///
    /// Returns an error when the index cannot be read.
    fn max_packed_value(&self) -> Result<Option<Vec<u8>>>;

    /// Returns the number of dimensions represented in the values.
    ///
    /// # Errors
    ///
    /// Returns an error when the index cannot be read.
    fn num_dimensions(&self) -> Result<i32>;

    /// Returns the number of dimensions used for the index key.
    ///
    /// # Errors
    ///
    /// Returns an error when the index cannot be read.
    fn num_index_dimensions(&self) -> Result<i32>;

    /// Returns the number of bytes in each dimension's values.
    ///
    /// # Errors
    ///
    /// Returns an error when the index cannot be read.
    fn bytes_per_dimension(&self) -> Result<i32>;

    /// Iterates every indexed point value for this field.
    ///
    /// This is a Rucene-specific write-path entry point (Java's writer reads the
    /// BKD data directly). The default implementation walks the
    /// [`PointTree`](Self::point_tree) and forwards each decoded value to the
    /// visitor, so a BKD-backed reader only needs to supply `point_tree`.
    ///
    /// # Errors
    ///
    /// Propagates reader and visitor errors.
    fn visit_doc_values(&self, visitor: &mut dyn DocValuesVisitor) -> Result<()> {
        let mut tree = self.point_tree()?;
        let mut adapter = DocValuesIntersectAdapter { visitor };
        tree.visit_doc_values(&mut adapter)
    }
}

// -----------------------------------------------------------------------------
// Per-reader aggregation
// -----------------------------------------------------------------------------

/// Returns the cumulated number of points for `field` across every leaf.
///
/// Equivalent to the static `PointValues.size(IndexReader, String)`. Leaves
/// that have no points for the field are ignored.
///
/// # Errors
///
/// Propagates reader errors.
pub fn size(reader: Arc<dyn IndexReader>, field: &str) -> Result<i64> {
    let mut size = 0i64;
    for ctx in reader.leaves() {
        if let Some(values) = ctx.leaf_reader().get_point_values(field)? {
            size += values.size();
        }
    }
    Ok(size)
}

/// Returns the cumulated number of documents that have points for `field`.
///
/// Equivalent to the static `PointValues.getDocCount(IndexReader, String)`.
///
/// # Errors
///
/// Propagates reader errors.
pub fn doc_count(reader: Arc<dyn IndexReader>, field: &str) -> Result<i32> {
    let mut count = 0i32;
    for ctx in reader.leaves() {
        if let Some(values) = ctx.leaf_reader().get_point_values(field)? {
            count += values.doc_count();
        }
    }
    Ok(count)
}

/// Returns the minimum packed value for `field` across every leaf, or `None`
/// when no leaf has points.
///
/// Equivalent to the static `PointValues.getMinPackedValue(IndexReader, String)`.
/// The comparison is **per index dimension** and **unsigned**, over
/// `bytes_per_dimension` bytes at a time, exactly as
/// `ArrayUtil.getUnsignedComparator` does.
///
/// # Errors
///
/// Propagates reader errors.
pub fn min_packed_value(reader: Arc<dyn IndexReader>, field: &str) -> Result<Option<Vec<u8>>> {
    merge_packed_values(reader, field, PackedBound::Min)
}

/// Returns the maximum packed value for `field` across every leaf, or `None`
/// when no leaf has points.
///
/// Equivalent to the static `PointValues.getMaxPackedValue(IndexReader, String)`.
///
/// # Errors
///
/// Propagates reader errors.
pub fn max_packed_value(reader: Arc<dyn IndexReader>, field: &str) -> Result<Option<Vec<u8>>> {
    merge_packed_values(reader, field, PackedBound::Max)
}

/// Which side of the range [`merge_packed_values`] is accumulating.
#[derive(Clone, Copy)]
enum PackedBound {
    Min,
    Max,
}

fn merge_packed_values(
    reader: Arc<dyn IndexReader>,
    field: &str,
    bound: PackedBound,
) -> Result<Option<Vec<u8>>> {
    let mut merged: Option<Vec<u8>> = None;
    for ctx in reader.leaves() {
        let leaf = ctx.leaf_reader();
        let Some(values) = leaf.get_point_values(field)? else {
            continue;
        };
        let leaf_value = match bound {
            PackedBound::Min => values.min_packed_value()?,
            PackedBound::Max => values.max_packed_value()?,
        };
        let Some(leaf_value) = leaf_value else {
            continue;
        };
        match merged.as_mut() {
            None => merged = Some(leaf_value),
            Some(current) => {
                let num_dimensions = values.num_index_dimensions()?;
                let num_bytes_per_dimension = values.bytes_per_dimension()?;
                for i in 0..num_dimensions {
                    let offset = (i * num_bytes_per_dimension) as usize;
                    let end = offset + num_bytes_per_dimension as usize;
                    let candidate = &leaf_value[offset..end];
                    let replace = match bound {
                        PackedBound::Min => candidate < &current[offset..end],
                        PackedBound::Max => candidate > &current[offset..end],
                    };
                    if replace {
                        current[offset..end].copy_from_slice(candidate);
                    }
                }
            }
        }
    }
    Ok(merged)
}

// -----------------------------------------------------------------------------
// In-memory reference tree
// -----------------------------------------------------------------------------

/// One node of an [`InMemoryPointTree`].
#[derive(Debug, Clone)]
struct MemoryNode {
    min_packed_value: Vec<u8>,
    max_packed_value: Vec<u8>,
    /// Points stored on this node; always empty for inner nodes.
    points: Vec<(i32, Vec<u8>)>,
    children: Vec<usize>,
    parent: Option<usize>,
    size: i64,
}

/// A [`PointTree`] held entirely in memory.
///
/// Lucene has no such class: every real tree is produced by `BKDReader`. This
/// one exists so the traversal algorithms in this module have a reference
/// implementation of the [`PointTree`] contract to be validated against, both
/// by unit tests and by the portability suite, before the BKD-backed cursor
/// arrives. Its call shape mirrors `BKDReader.BKDPointTree`:
///
/// * [`visit_doc_ids`](PointTree::visit_doc_ids) calls `grow` **once** with the
///   size of the whole subtree, then visits each leaf in order;
/// * [`visit_doc_values`](PointTree::visit_doc_values) calls `grow` **per
///   leaf** with that leaf's point count, then visits each point.
///
/// The tree is a balanced binary tree over the leaf blocks, which is the shape
/// a BKD index has.
#[derive(Debug, Clone)]
pub struct InMemoryPointTree {
    nodes: Arc<Vec<MemoryNode>>,
    root: usize,
    current: usize,
}

impl InMemoryPointTree {
    /// Builds a tree over `leaves`, each a block of `(doc_id, packed_value)`
    /// points in visit order.
    ///
    /// `num_index_dims` and `bytes_per_dim` describe the packed values and are
    /// used to compute the per-dimension bounds of every node, with the same
    /// unsigned comparison Lucene uses.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `leaves` is empty, when a
    /// leaf is empty, when a packed value has the wrong length, or when a
    /// one-dimensional tree is given points that are not in the order the
    /// [`IntersectVisitor::visit_with_value`] contract requires.
    pub fn new(
        num_index_dims: i32,
        bytes_per_dim: i32,
        leaves: Vec<Vec<(i32, Vec<u8>)>>,
    ) -> Result<Self> {
        if num_index_dims <= 0 || bytes_per_dim <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "num_index_dims and bytes_per_dim must be positive, got {num_index_dims} and {bytes_per_dim}"
            )));
        }
        if leaves.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "an in-memory point tree needs at least one leaf".to_string(),
            ));
        }
        let packed_len = (num_index_dims * bytes_per_dim) as usize;
        for leaf in &leaves {
            if leaf.is_empty() {
                return Err(LuceneError::IllegalArgument(
                    "an in-memory point tree leaf must hold at least one point".to_string(),
                ));
            }
            for (_, value) in leaf {
                if value.len() != packed_len {
                    return Err(LuceneError::IllegalArgument(format!(
                        "packed value length {} does not match {num_index_dims} * {bytes_per_dim}",
                        value.len()
                    )));
                }
            }
        }
        if num_index_dims == 1 {
            check_one_dimensional_order(&leaves)?;
        }

        let mut nodes: Vec<MemoryNode> = Vec::new();
        build_subtree(
            &mut nodes,
            None,
            &leaves,
            bytes_per_dim as usize,
            packed_len,
        );
        Ok(Self {
            nodes: Arc::new(nodes),
            root: 0,
            current: 0,
        })
    }

    fn node(&self) -> &MemoryNode {
        &self.nodes[self.current]
    }

    /// Visits every leaf below `index`, in order.
    fn for_each_leaf(
        &self,
        index: usize,
        f: &mut impl FnMut(&MemoryNode) -> Result<()>,
    ) -> Result<()> {
        let node = &self.nodes[index];
        if node.children.is_empty() {
            return f(node);
        }
        for &child in &node.children {
            self.for_each_leaf(child, f)?;
        }
        Ok(())
    }
}

/// Rejects point sequences that break the documented 1-D visit order.
fn check_one_dimensional_order(leaves: &[Vec<(i32, Vec<u8>)>]) -> Result<()> {
    let mut previous: Option<(&[u8], i32)> = None;
    for leaf in leaves {
        for (doc_id, value) in leaf {
            if let Some((prev_value, prev_doc)) = previous {
                let out_of_order = value.as_slice() < prev_value
                    || (value.as_slice() == prev_value && *doc_id < prev_doc);
                if out_of_order {
                    return Err(LuceneError::IllegalArgument(format!(
                        "one-dimensional points must be visited in increasing value order, \
                         ties by increasing doc id; {prev_value:?}/{prev_doc} is followed by \
                         {value:?}/{doc_id}"
                    )));
                }
            }
            previous = Some((value.as_slice(), *doc_id));
        }
    }
    Ok(())
}

/// Appends the subtree covering `leaves` and returns its node index.
fn build_subtree(
    nodes: &mut Vec<MemoryNode>,
    parent: Option<usize>,
    leaves: &[Vec<(i32, Vec<u8>)>],
    bytes_per_dim: usize,
    packed_len: usize,
) -> usize {
    let index = nodes.len();
    nodes.push(MemoryNode {
        min_packed_value: vec![0xff; packed_len],
        max_packed_value: vec![0x00; packed_len],
        points: Vec::new(),
        children: Vec::new(),
        parent,
        size: 0,
    });

    if leaves.len() == 1 {
        let points = leaves[0].clone();
        let size = points.len() as i64;
        let (min, max) = bounds(
            points.iter().map(|(_, v)| v.as_slice()),
            bytes_per_dim,
            packed_len,
        );
        let node = &mut nodes[index];
        node.points = points;
        node.size = size;
        node.min_packed_value = min;
        node.max_packed_value = max;
        return index;
    }

    let mid = leaves.len() / 2;
    let left = build_subtree(
        nodes,
        Some(index),
        &leaves[..mid],
        bytes_per_dim,
        packed_len,
    );
    let right = build_subtree(
        nodes,
        Some(index),
        &leaves[mid..],
        bytes_per_dim,
        packed_len,
    );
    let size = nodes[left].size + nodes[right].size;
    let min = merge_bound(
        &nodes[left].min_packed_value,
        &nodes[right].min_packed_value,
        bytes_per_dim,
        PackedBound::Min,
    );
    let max = merge_bound(
        &nodes[left].max_packed_value,
        &nodes[right].max_packed_value,
        bytes_per_dim,
        PackedBound::Max,
    );
    let node = &mut nodes[index];
    node.children = vec![left, right];
    node.size = size;
    node.min_packed_value = min;
    node.max_packed_value = max;
    index
}

/// Computes the per-dimension unsigned bounds of a set of packed values.
fn bounds<'a>(
    values: impl Iterator<Item = &'a [u8]>,
    bytes_per_dim: usize,
    packed_len: usize,
) -> (Vec<u8>, Vec<u8>) {
    let mut min: Option<Vec<u8>> = None;
    let mut max: Option<Vec<u8>> = None;
    for value in values {
        min = Some(match min {
            None => value.to_vec(),
            Some(current) => merge_bound(&current, value, bytes_per_dim, PackedBound::Min),
        });
        max = Some(match max {
            None => value.to_vec(),
            Some(current) => merge_bound(&current, value, bytes_per_dim, PackedBound::Max),
        });
    }
    (
        min.unwrap_or_else(|| vec![0u8; packed_len]),
        max.unwrap_or_else(|| vec![0u8; packed_len]),
    )
}

/// Merges `candidate` into `current` dimension by dimension.
fn merge_bound(
    current: &[u8],
    candidate: &[u8],
    bytes_per_dim: usize,
    bound: PackedBound,
) -> Vec<u8> {
    let mut merged = current.to_vec();
    let mut offset = 0;
    while offset < merged.len() {
        let end = offset + bytes_per_dim;
        let replace = match bound {
            PackedBound::Min => candidate[offset..end] < merged[offset..end],
            PackedBound::Max => candidate[offset..end] > merged[offset..end],
        };
        if replace {
            merged[offset..end].copy_from_slice(&candidate[offset..end]);
        }
        offset = end;
    }
    merged
}

impl PointTree for InMemoryPointTree {
    fn clone_tree(&self) -> Box<dyn PointTree> {
        Box::new(Self {
            nodes: Arc::clone(&self.nodes),
            root: self.current,
            current: self.current,
        })
    }

    fn move_to_child(&mut self) -> Result<bool> {
        match self.node().children.first() {
            Some(&child) => {
                self.current = child;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        if self.current == self.root {
            return Ok(false);
        }
        let Some(parent) = self.node().parent else {
            return Ok(false);
        };
        let siblings = &self.nodes[parent].children;
        let position = siblings
            .iter()
            .position(|&node| node == self.current)
            .expect("INVARIANT: a non-root node is listed among its parent's children");
        match siblings.get(position + 1) {
            Some(&next) => {
                self.current = next;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        if self.current == self.root {
            return Ok(false);
        }
        match self.node().parent {
            Some(parent) => {
                self.current = parent;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn min_packed_value(&self) -> &[u8] {
        &self.node().min_packed_value
    }

    fn max_packed_value(&self) -> &[u8] {
        &self.node().max_packed_value
    }

    fn size(&self) -> i64 {
        self.node().size
    }

    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        let size = self.size();
        if size <= i32::MAX as i64 {
            visitor.grow(size as i32);
        }
        let current = self.current;
        let mut doc_ids: Vec<i32> = Vec::new();
        self.for_each_leaf(current, &mut |leaf| {
            doc_ids.extend(leaf.points.iter().map(|(doc_id, _)| *doc_id));
            Ok(())
        })?;
        for doc_id in doc_ids {
            visitor.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        let current = self.current;
        let mut blocks: Vec<Vec<(i32, Vec<u8>)>> = Vec::new();
        self.for_each_leaf(current, &mut |leaf| {
            blocks.push(leaf.points.clone());
            Ok(())
        })?;
        for block in blocks {
            visitor.grow(block.len() as i32);
            for (doc_id, value) in block {
                visitor.visit_with_value(doc_id, &value)?;
            }
        }
        Ok(())
    }
}

/// [`PointValues`] over an [`InMemoryPointTree`].
///
/// The counterpart of [`InMemoryPointTree`]: it makes the traversal algorithms
/// in this module runnable without an on-disk index. Real segments are served
/// by the BKD-backed implementation.
#[derive(Debug, Clone)]
pub struct InMemoryPointValues {
    tree: InMemoryPointTree,
    num_dims: i32,
    num_index_dims: i32,
    bytes_per_dim: i32,
    size: i64,
    doc_count: i32,
}

impl InMemoryPointValues {
    /// Builds point values over the given leaf blocks.
    ///
    /// `doc_count` is derived from the distinct doc IDs, matching
    /// `PointValues.getDocCount()`.
    ///
    /// # Errors
    ///
    /// Propagates the validation errors of [`InMemoryPointTree::new`].
    pub fn new(
        num_dims: i32,
        num_index_dims: i32,
        bytes_per_dim: i32,
        leaves: Vec<Vec<(i32, Vec<u8>)>>,
    ) -> Result<Self> {
        let mut docs: Vec<i32> = leaves
            .iter()
            .flat_map(|leaf| leaf.iter().map(|(doc_id, _)| *doc_id))
            .collect();
        let size = docs.len() as i64;
        docs.sort_unstable();
        docs.dedup();
        let doc_count = docs.len() as i32;
        let tree = InMemoryPointTree::new(num_index_dims, bytes_per_dim, leaves)?;
        Ok(Self {
            tree,
            num_dims,
            num_index_dims,
            bytes_per_dim,
            size,
            doc_count,
        })
    }
}

impl PointValues for InMemoryPointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(self.tree.clone()))
    }

    fn size(&self) -> i64 {
        self.size
    }

    fn doc_count(&self) -> i32 {
        self.doc_count
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        if self.size == 0 {
            return Ok(None);
        }
        Ok(Some(self.tree.min_packed_value().to_vec()))
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        if self.size == 0 {
            return Ok(None);
        }
        Ok(Some(self.tree.max_packed_value().to_vec()))
    }

    fn num_dimensions(&self) -> Result<i32> {
        Ok(self.num_dims)
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        Ok(self.num_index_dims)
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        Ok(self.bytes_per_dim)
    }
}

// -----------------------------------------------------------------------------
// Empty implementation
// -----------------------------------------------------------------------------

/// A [`PointTree`] with a single empty node.
#[derive(Debug, Default, Clone, Copy)]
struct EmptyPointTree;

impl PointTree for EmptyPointTree {
    fn clone_tree(&self) -> Box<dyn PointTree> {
        Box::new(*self)
    }

    fn move_to_child(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn min_packed_value(&self) -> &[u8] {
        &[]
    }

    fn max_packed_value(&self) -> &[u8] {
        &[]
    }

    fn size(&self) -> i64 {
        0
    }

    fn visit_doc_ids(&mut self, _visitor: &mut dyn IntersectVisitor) -> Result<()> {
        Ok(())
    }

    fn visit_doc_values(&mut self, _visitor: &mut dyn IntersectVisitor) -> Result<()> {
        Ok(())
    }
}

/// A no-op point-values instance that reports zero dimensions and no values.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyPointValues;

impl PointValues for EmptyPointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(EmptyPointTree))
    }

    fn size(&self) -> i64 {
        0
    }

    fn doc_count(&self) -> i32 {
        0
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    fn num_dimensions(&self) -> Result<i32> {
        Ok(0)
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        Ok(0)
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        Ok(0)
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::DocIdSetIterator;
    use crate::util::IntsRef;

    /// One callback recorded by [`TracingVisitor`].
    #[derive(Debug, PartialEq, Eq, Clone)]
    enum Call {
        Compare(Vec<u8>, Vec<u8>, Relation),
        Grow(i32),
        Visit(i32),
        VisitWithValue(i32, Vec<u8>),
    }

    /// Visitor over one-byte, one-dimension points that records every call.
    ///
    /// The trace is what proves the traversal order, not just the final set of
    /// accepted documents.
    struct TracingVisitor {
        min: u8,
        max: u8,
        trace: std::cell::RefCell<Vec<Call>>,
        accepted: Vec<i32>,
    }

    impl TracingVisitor {
        fn new(min: u8, max: u8) -> Self {
            Self {
                min,
                max,
                trace: std::cell::RefCell::new(Vec::new()),
                accepted: Vec::new(),
            }
        }

        fn trace(&self) -> Vec<Call> {
            self.trace.borrow().clone()
        }
    }

    impl IntersectVisitor for TracingVisitor {
        fn visit(&mut self, doc_id: i32) -> Result<()> {
            self.trace.borrow_mut().push(Call::Visit(doc_id));
            self.accepted.push(doc_id);
            Ok(())
        }

        fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
            self.trace
                .borrow_mut()
                .push(Call::VisitWithValue(doc_id, packed_value.to_vec()));
            if packed_value[0] >= self.min && packed_value[0] <= self.max {
                self.accepted.push(doc_id);
            }
            Ok(())
        }

        fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
            // A cell with no bounds holds no points, so it cannot overlap the
            // query. Only `EmptyPointValues` produces one.
            let disjoint = min_packed_value.is_empty()
                || max_packed_value.is_empty()
                || max_packed_value[0] < self.min
                || min_packed_value[0] > self.max;
            let relation = if disjoint {
                Relation::CellOutsideQuery
            } else if min_packed_value[0] >= self.min && max_packed_value[0] <= self.max {
                Relation::CellInsideQuery
            } else {
                Relation::CellCrossesQuery
            };
            self.trace.borrow_mut().push(Call::Compare(
                min_packed_value.to_vec(),
                max_packed_value.to_vec(),
                relation,
            ));
            relation
        }

        fn grow(&mut self, count: i32) {
            self.trace.borrow_mut().push(Call::Grow(count));
        }
    }

    /// A two-leaf tree: docs 0..2 hold 10, 20, 30 and docs 3..5 hold 40, 50, 60.
    fn two_leaf_values() -> InMemoryPointValues {
        InMemoryPointValues::new(
            1,
            1,
            1,
            vec![
                vec![(0, vec![10]), (1, vec![20]), (2, vec![30])],
                vec![(3, vec![40]), (4, vec![50]), (5, vec![60])],
            ],
        )
        .unwrap()
    }

    fn single_leaf_values() -> InMemoryPointValues {
        InMemoryPointValues::new(
            1,
            1,
            1,
            vec![vec![(0, vec![10]), (1, vec![20]), (2, vec![30])]],
        )
        .unwrap()
    }

    #[test]
    fn point_values_constants_match_java() {
        assert_eq!(MAX_NUM_BYTES, 16);
        assert_eq!(MAX_DIMENSIONS, 16);
        assert_eq!(MAX_INDEX_DIMENSIONS, 8);
        // The two dimension limits are Java's BKDConfig constants, not copies.
        assert_eq!(MAX_DIMENSIONS, BKDConfig::MAX_DIMS);
        assert_eq!(MAX_INDEX_DIMENSIONS, BKDConfig::MAX_INDEX_DIMS);
    }

    #[test]
    fn relation_declaration_order_matches_java() {
        assert_eq!(Relation::CellInsideQuery as usize, 0);
        assert_eq!(Relation::CellOutsideQuery as usize, 1);
        assert_eq!(Relation::CellCrossesQuery as usize, 2);
    }

    #[test]
    fn tree_navigation_follows_the_java_contract() {
        let values = two_leaf_values();
        let mut tree = values.point_tree().unwrap();

        // The root spans every value and holds every point.
        assert_eq!(tree.min_packed_value(), &[10]);
        assert_eq!(tree.max_packed_value(), &[60]);
        assert_eq!(tree.size(), 6);
        assert!(!tree.move_to_parent().unwrap(), "the root has no parent");
        assert!(!tree.move_to_sibling().unwrap(), "the root has no sibling");

        assert!(tree.move_to_child().unwrap());
        assert_eq!(tree.min_packed_value(), &[10]);
        assert_eq!(tree.max_packed_value(), &[30]);
        assert_eq!(tree.size(), 3);
        assert!(!tree.move_to_child().unwrap(), "leaves have no children");

        assert!(tree.move_to_sibling().unwrap());
        assert_eq!(tree.min_packed_value(), &[40]);
        assert_eq!(tree.size(), 3);
        assert!(!tree.move_to_sibling().unwrap());

        assert!(tree.move_to_parent().unwrap());
        assert_eq!(tree.size(), 6);
        assert!(!tree.move_to_parent().unwrap());
    }

    #[test]
    fn clone_tree_re_roots_at_the_current_node() {
        let values = two_leaf_values();
        let mut tree = values.point_tree().unwrap();
        tree.move_to_child().unwrap();
        tree.move_to_sibling().unwrap();

        let mut clone = tree.clone_tree();
        assert_eq!(clone.min_packed_value(), &[40]);
        // The clone's root is the node the original was on.
        assert!(!clone.move_to_parent().unwrap());
        assert!(!clone.move_to_sibling().unwrap());
        // The original is untouched.
        assert!(tree.move_to_parent().unwrap());
    }

    #[test]
    fn intersect_on_a_fully_inside_cell_visits_doc_ids_only() {
        let values = single_leaf_values();
        let mut visitor = TracingVisitor::new(0, 100);
        values.intersect(&mut visitor).unwrap();
        assert_eq!(
            visitor.trace(),
            vec![
                Call::Compare(vec![10], vec![30], Relation::CellInsideQuery),
                Call::Grow(3),
                Call::Visit(0),
                Call::Visit(1),
                Call::Visit(2),
            ]
        );
        assert_eq!(visitor.accepted, vec![0, 1, 2]);
    }

    #[test]
    fn intersect_on_a_crossing_leaf_visits_values() {
        let values = single_leaf_values();
        let mut visitor = TracingVisitor::new(15, 25);
        values.intersect(&mut visitor).unwrap();
        assert_eq!(
            visitor.trace(),
            vec![
                Call::Compare(vec![10], vec![30], Relation::CellCrossesQuery),
                Call::Grow(3),
                Call::VisitWithValue(0, vec![10]),
                Call::VisitWithValue(1, vec![20]),
                Call::VisitWithValue(2, vec![30]),
            ]
        );
        assert_eq!(visitor.accepted, vec![1]);
    }

    #[test]
    fn intersect_on_an_outside_cell_visits_nothing() {
        let values = single_leaf_values();
        let mut visitor = TracingVisitor::new(200, 255);
        values.intersect(&mut visitor).unwrap();
        assert_eq!(
            visitor.trace(),
            vec![Call::Compare(
                vec![10],
                vec![30],
                Relation::CellOutsideQuery
            )]
        );
        assert!(visitor.accepted.is_empty());
    }

    /// The interesting case: the root crosses, so the traversal descends and
    /// takes a different decision per child.
    #[test]
    fn intersect_descends_and_mixes_relations_per_child() {
        let values = two_leaf_values();
        let mut visitor = TracingVisitor::new(0, 45);
        values.intersect(&mut visitor).unwrap();
        assert_eq!(
            visitor.trace(),
            vec![
                // Root [10, 60] crosses [0, 45].
                Call::Compare(vec![10], vec![60], Relation::CellCrossesQuery),
                // Left child [10, 30] is inside: doc ids only.
                Call::Compare(vec![10], vec![30], Relation::CellInsideQuery),
                Call::Grow(3),
                Call::Visit(0),
                Call::Visit(1),
                Call::Visit(2),
                // Right child [40, 60] crosses: full filtering.
                Call::Compare(vec![40], vec![60], Relation::CellCrossesQuery),
                Call::Grow(3),
                Call::VisitWithValue(3, vec![40]),
                Call::VisitWithValue(4, vec![50]),
                Call::VisitWithValue(5, vec![60]),
            ]
        );
        assert_eq!(visitor.accepted, vec![0, 1, 2, 3]);
    }

    #[test]
    fn estimate_point_count_covers_the_three_relations() {
        let values = single_leaf_values();
        // Fully inside: the exact size.
        assert_eq!(
            values
                .estimate_point_count(&mut TracingVisitor::new(0, 100))
                .unwrap(),
            3
        );
        // Fully outside: zero.
        assert_eq!(
            values
                .estimate_point_count(&mut TracingVisitor::new(200, 255))
                .unwrap(),
            0
        );
        // Crossing leaf: (size + 1) / 2, i.e. "assume half matched".
        assert_eq!(
            values
                .estimate_point_count(&mut TracingVisitor::new(15, 25))
                .unwrap(),
            2
        );
    }

    #[test]
    fn estimate_point_count_rounds_odd_leaves_up() {
        let leaf: Vec<(i32, Vec<u8>)> = (0..7).map(|i| (i, vec![i as u8 * 10])).collect();
        let values = InMemoryPointValues::new(1, 1, 1, vec![leaf]).unwrap();
        // (7 + 1) / 2 == 4
        assert_eq!(
            values
                .estimate_point_count(&mut TracingVisitor::new(5, 15))
                .unwrap(),
            4
        );
    }

    #[test]
    fn estimate_point_count_sums_children() {
        let values = two_leaf_values();
        // Left inside (3), right crossing leaf ((3 + 1) / 2 == 2).
        assert_eq!(
            values
                .estimate_point_count(&mut TracingVisitor::new(0, 45))
                .unwrap(),
            5
        );
    }

    #[test]
    fn bounded_estimation_stops_early() {
        let values = two_leaf_values();
        let mut visitor = TracingVisitor::new(0, 45);
        let mut tree = values.point_tree().unwrap();
        // The left child alone reaches the bound, so the right child is never
        // compared: the trace has exactly two `compare` calls.
        assert!(
            is_estimated_point_count_greater_than_or_equal_to(&mut visitor, tree.as_mut(), 3)
                .unwrap()
        );
        let compares = visitor
            .trace()
            .iter()
            .filter(|call| matches!(call, Call::Compare(..)))
            .count();
        assert_eq!(compares, 2);

        let mut visitor = TracingVisitor::new(0, 45);
        let mut tree = values.point_tree().unwrap();
        assert!(
            !is_estimated_point_count_greater_than_or_equal_to(&mut visitor, tree.as_mut(), 6)
                .unwrap()
        );
    }

    #[test]
    fn estimate_doc_count_returns_all_docs_when_every_point_matches() {
        // epc >= size -> docCount
        assert_eq!(estimate_doc_count(10, 4, 10), 4);
        assert_eq!(estimate_doc_count(11, 4, 10), 4);
    }

    #[test]
    fn estimate_doc_count_is_the_point_count_for_single_valued_fields() {
        // size == docCount -> epc
        assert_eq!(estimate_doc_count(3, 10, 10), 3);
        // epc == 0 -> 0
        assert_eq!(estimate_doc_count(0, 10, 20), 0);
    }

    #[test]
    fn estimate_doc_count_uses_the_urn_approximation_for_multi_valued_fields() {
        // D = 10, N = 20, n = 5:
        //   10 * (1 - ((20 - 5) / 20)^(20 / 10)) = 10 * (1 - 0.75^2) = 4.375 -> 4
        assert_eq!(estimate_doc_count(5, 10, 20), 4);

        // Hand-checked second point: D = 100, N = 1000, n = 10
        //   100 * (1 - 0.99^10) = 100 * (1 - 0.904382...) = 9.5617... -> 9
        assert_eq!(estimate_doc_count(10, 100, 1000), 9);
    }

    #[test]
    fn estimate_doc_count_floors_at_one() {
        // D = 100, N = 1_000_000, n = 1 gives a raw estimate below 1, which
        // Java lifts to 1 rather than reporting "no docs".
        assert_eq!(estimate_doc_count(1, 100, 1_000_000), 1);
    }

    #[test]
    fn point_values_estimate_doc_count_wires_the_pieces_together() {
        let values = two_leaf_values();
        // Six single-valued docs: size == doc_count, so the doc estimate is the
        // point estimate.
        assert_eq!(
            values
                .estimate_doc_count(&mut TracingVisitor::new(0, 45))
                .unwrap(),
            5
        );
    }

    #[test]
    fn empty_point_values_reports_zero() {
        let points = EmptyPointValues;
        assert_eq!(points.size(), 0);
        assert_eq!(points.doc_count(), 0);
        assert!(points.min_packed_value().unwrap().is_none());
        assert!(points.max_packed_value().unwrap().is_none());
        assert_eq!(points.num_dimensions().unwrap(), 0);
        assert_eq!(points.num_index_dimensions().unwrap(), 0);
        assert_eq!(points.bytes_per_dimension().unwrap(), 0);

        let mut visitor = TracingVisitor::new(0, 1);
        points.intersect(&mut visitor).unwrap();
        assert_eq!(points.estimate_doc_count(&mut visitor).unwrap(), 0);
    }

    #[test]
    fn in_memory_tree_rejects_malformed_input() {
        assert!(InMemoryPointTree::new(1, 1, vec![]).is_err());
        assert!(InMemoryPointTree::new(1, 1, vec![vec![]]).is_err());
        assert!(InMemoryPointTree::new(0, 1, vec![vec![(0, vec![1])]]).is_err());
        // Packed value of the wrong length.
        assert!(InMemoryPointTree::new(2, 1, vec![vec![(0, vec![1])]]).is_err());
    }

    /// Pins the documented 1-D ordering contract of `visit(int, byte[])`.
    #[test]
    fn in_memory_tree_rejects_points_out_of_the_one_dimensional_order() {
        // Decreasing value.
        assert!(InMemoryPointTree::new(1, 1, vec![vec![(0, vec![20]), (1, vec![10])]]).is_err());
        // Equal value, decreasing doc id.
        assert!(InMemoryPointTree::new(1, 1, vec![vec![(5, vec![10]), (4, vec![10])]]).is_err());
        // Equal value, increasing doc id is fine.
        assert!(InMemoryPointTree::new(1, 1, vec![vec![(4, vec![10]), (5, vec![10])]]).is_ok());
        // The order is checked across leaves, not only within one.
        assert!(
            InMemoryPointTree::new(1, 1, vec![vec![(0, vec![30])], vec![(1, vec![20])]]).is_err()
        );
    }

    #[test]
    fn multi_dimensional_bounds_are_computed_per_dimension() {
        // Two 2-D points; the bounds mix dimensions independently.
        let values =
            InMemoryPointValues::new(2, 2, 1, vec![vec![(0, vec![10, 90]), (1, vec![50, 20])]])
                .unwrap();
        assert_eq!(values.min_packed_value().unwrap().unwrap(), vec![10, 20]);
        assert_eq!(values.max_packed_value().unwrap().unwrap(), vec![50, 90]);
    }

    #[test]
    fn doc_count_deduplicates_multi_valued_documents() {
        let values = InMemoryPointValues::new(
            1,
            1,
            1,
            vec![vec![(0, vec![10]), (0, vec![20]), (1, vec![30])]],
        )
        .unwrap();
        assert_eq!(values.size(), 3);
        assert_eq!(values.doc_count(), 2);
    }

    // -------------------------------------------------------------------------
    // Reader stubs for the per-reader aggregation helpers
    // -------------------------------------------------------------------------

    use crate::codecs::stub::StoredFieldVisitor;
    use crate::index::doc_values::{
        BinaryDocValues, DocValuesSkipper, NumericDocValues, SortedDocValues,
        SortedNumericDocValues, SortedSetDocValues,
    };
    use crate::index::index_reader::{
        build_composite_context, CacheHelper, CompositeReader, IndexReaderCore, StoredFields,
    };
    use crate::index::leaf_reader::{LeafMetaData, LeafReader, TermVectors};
    use crate::index::reader_context::IndexReaderContext;
    use crate::index::vector_values::{ByteVectorValues, FloatVectorValues};
    use crate::index::{FieldInfos, Fields, Terms};
    use crate::search::knn::KnnCollector;
    use crate::search::AcceptDocs;
    use crate::util::Bits;
    use std::sync::Weak;

    #[derive(Debug)]
    struct StubTermVectors;
    impl TermVectors for StubTermVectors {
        fn get(&self, _doc: i32) -> Result<Option<Box<dyn Fields>>> {
            Ok(None)
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

        fn document(&self, _doc_id: i32) -> Result<crate::document::Document> {
            Ok(crate::document::Document::new())
        }

        fn document_fields(
            &self,
            _doc_id: i32,
            _fields_to_load: &std::collections::HashSet<String>,
        ) -> Result<crate::document::Document> {
            Ok(crate::document::Document::new())
        }
    }

    /// A leaf reader that serves one point field and nothing else.
    #[derive(Debug)]
    struct PointsLeafReader {
        core: IndexReaderCore,
        field: String,
        values: Option<InMemoryPointValues>,
    }

    impl PointsLeafReader {
        fn new(field: &str, values: Option<InMemoryPointValues>) -> Arc<Self> {
            Arc::new(Self {
                core: IndexReaderCore::new(),
                field: field.to_string(),
                values,
            })
        }
    }

    impl LeafReader for PointsLeafReader {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }
        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            Ok(Box::new(StubTermVectors))
        }
        fn num_docs(&self) -> i32 {
            self.values.as_ref().map(|v| v.doc_count()).unwrap_or(0)
        }
        fn max_doc(&self) -> i32 {
            LeafReader::num_docs(self)
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
        fn get_numeric_doc_values(&self, _f: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }
        fn get_binary_doc_values(&self, _f: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
            Ok(None)
        }
        fn get_sorted_doc_values(&self, _f: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
            Ok(None)
        }
        fn get_sorted_numeric_doc_values(
            &self,
            _f: &str,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            Ok(None)
        }
        fn get_sorted_set_doc_values(
            &self,
            _f: &str,
        ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            Ok(None)
        }
        fn get_norm_values(&self, _f: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
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
            _f: &str,
            _t: &[f32],
            _c: &mut dyn KnnCollector,
            _a: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }
        fn search_nearest_vectors_byte(
            &self,
            _f: &str,
            _t: &[u8],
            _c: &mut dyn KnnCollector,
            _a: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }
        fn get_field_infos(&self) -> FieldInfos {
            FieldInfos::empty()
        }
        fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
            None
        }
        fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
            if field != self.field {
                return Ok(None);
            }
            Ok(self
                .values
                .clone()
                .map(|values| Box::new(values) as Box<dyn PointValues>))
        }
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_meta_data(&self) -> LeafMetaData {
            LeafMetaData::new(10, None, None, false).expect("INVARIANT: version 10 is valid")
        }
    }

    /// Minimal composite reader so the aggregation helpers can see several
    /// leaves.
    #[derive(Debug)]
    struct StubCompositeReader {
        core: IndexReaderCore,
        subs: Vec<Arc<dyn IndexReader>>,
    }

    impl IndexReader for StubCompositeReader {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }
        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            Ok(Box::new(StubTermVectors))
        }
        fn num_docs(&self) -> i32 {
            self.subs.iter().map(|r| r.num_docs()).sum()
        }
        fn max_doc(&self) -> i32 {
            self.subs.iter().map(|r| r.max_doc()).sum()
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
        fn doc_freq(&self, _term: &crate::index::Term) -> Result<i32> {
            Ok(0)
        }
        fn total_term_freq(&self, _term: &crate::index::Term) -> Result<i64> {
            Ok(0)
        }
        fn get_sum_doc_freq(&self, _field: &str) -> Result<i64> {
            Ok(0)
        }
        fn get_doc_count(&self, _field: &str) -> Result<i32> {
            Ok(0)
        }
        fn get_sum_total_term_freq(&self, _field: &str) -> Result<i64> {
            Ok(0)
        }
        fn build_context(
            self: Arc<Self>,
            parent: Option<Weak<dyn IndexReaderContext>>,
            ord_in_parent: i32,
            doc_base_in_parent: i32,
            leaf_ord: i32,
            leaf_doc_base: i32,
        ) -> Arc<dyn IndexReaderContext> {
            build_composite_context(
                self as Arc<dyn CompositeReader>,
                parent,
                ord_in_parent,
                doc_base_in_parent,
                leaf_ord,
                leaf_doc_base,
            )
        }
    }

    impl CompositeReader for StubCompositeReader {
        fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
            self.subs.iter().map(Arc::clone).collect()
        }
    }

    fn composite(subs: Vec<Arc<dyn IndexReader>>) -> Arc<dyn IndexReader> {
        Arc::new(StubCompositeReader {
            core: IndexReaderCore::new(),
            subs,
        })
    }

    /// Two 2-D leaves whose bounds cross: neither leaf dominates the other in
    /// both dimensions, so a whole-array comparison would give the wrong answer
    /// and only the per-dimension merge is correct.
    fn crossing_leaves() -> Arc<dyn IndexReader> {
        let left = InMemoryPointValues::new(
            2,
            2,
            1,
            vec![vec![(0, vec![0x10, 0xF0]), (1, vec![0x20, 0xFF])]],
        )
        .unwrap();
        let right = InMemoryPointValues::new(
            2,
            2,
            1,
            vec![vec![(2, vec![0x05, 0x01]), (3, vec![0x90, 0x02])]],
        )
        .unwrap();
        composite(vec![
            PointsLeafReader::new("p", Some(left)) as Arc<dyn IndexReader>,
            PointsLeafReader::new("p", None) as Arc<dyn IndexReader>,
            PointsLeafReader::new("p", Some(right)) as Arc<dyn IndexReader>,
        ])
    }

    #[test]
    fn aggregated_size_and_doc_count_skip_leaves_without_points() {
        let reader = crossing_leaves();
        assert_eq!(size(Arc::clone(&reader), "p").unwrap(), 4);
        assert_eq!(doc_count(Arc::clone(&reader), "p").unwrap(), 4);
        // A field no leaf indexes aggregates to nothing.
        assert_eq!(size(Arc::clone(&reader), "absent").unwrap(), 0);
        assert_eq!(doc_count(reader, "absent").unwrap(), 0);
    }

    #[test]
    fn aggregated_bounds_are_merged_per_index_dimension() {
        let reader = crossing_leaves();
        // Dimension 0: min(0x10, 0x05) = 0x05, max(0x20, 0x90) = 0x90.
        // Dimension 1: min(0xF0, 0x01) = 0x01, max(0xFF, 0x02) = 0xFF.
        assert_eq!(
            min_packed_value(Arc::clone(&reader), "p").unwrap().unwrap(),
            vec![0x05, 0x01]
        );
        assert_eq!(
            max_packed_value(Arc::clone(&reader), "p").unwrap().unwrap(),
            vec![0x90, 0xFF]
        );
    }

    /// Bytes above 0x7F must compare as large, not as negative. A signed
    /// comparison would report 0x80 as the minimum here.
    #[test]
    fn aggregated_bounds_compare_bytes_as_unsigned() {
        let low = InMemoryPointValues::new(1, 1, 1, vec![vec![(0, vec![0x01])]]).unwrap();
        let high = InMemoryPointValues::new(1, 1, 1, vec![vec![(1, vec![0x80])]]).unwrap();
        let reader = composite(vec![
            PointsLeafReader::new("p", Some(high)) as Arc<dyn IndexReader>,
            PointsLeafReader::new("p", Some(low)) as Arc<dyn IndexReader>,
        ]);
        assert_eq!(
            min_packed_value(Arc::clone(&reader), "p").unwrap().unwrap(),
            vec![0x01]
        );
        assert_eq!(max_packed_value(reader, "p").unwrap().unwrap(), vec![0x80]);
    }

    #[test]
    fn aggregated_bounds_are_none_when_no_leaf_has_points() {
        let reader = composite(vec![
            PointsLeafReader::new("p", None) as Arc<dyn IndexReader>,
            PointsLeafReader::new("p", None) as Arc<dyn IndexReader>,
        ]);
        assert!(min_packed_value(Arc::clone(&reader), "p")
            .unwrap()
            .is_none());
        assert!(max_packed_value(reader, "p").unwrap().is_none());
    }

    #[test]
    fn aggregation_over_a_single_leaf_reader_works() {
        let values = InMemoryPointValues::new(
            1,
            1,
            1,
            vec![vec![(0, vec![10]), (1, vec![20]), (2, vec![30])]],
        )
        .unwrap();
        let reader: Arc<dyn IndexReader> = PointsLeafReader::new("p", Some(values));
        assert_eq!(size(Arc::clone(&reader), "p").unwrap(), 3);
        assert_eq!(doc_count(Arc::clone(&reader), "p").unwrap(), 3);
        assert_eq!(
            min_packed_value(Arc::clone(&reader), "p").unwrap().unwrap(),
            vec![10]
        );
        assert_eq!(max_packed_value(reader, "p").unwrap().unwrap(), vec![30]);
    }

    #[test]
    fn intersect_visitor_default_bulk_methods() {
        struct BulkVisitor {
            ids: Vec<i32>,
        }
        impl IntersectVisitor for BulkVisitor {
            fn visit(&mut self, doc_id: i32) -> Result<()> {
                self.ids.push(doc_id);
                Ok(())
            }
            fn visit_with_value(&mut self, doc_id: i32, _packed_value: &[u8]) -> Result<()> {
                self.ids.push(doc_id);
                Ok(())
            }
            fn compare(&self, _min: &[u8], _max: &[u8]) -> Relation {
                Relation::CellCrossesQuery
            }
        }

        let mut visitor = BulkVisitor { ids: Vec::new() };
        let ints = IntsRef::new(vec![1, 3, 5]);
        visitor.visit_ints_ref(&ints).unwrap();
        assert_eq!(visitor.ids, vec![1, 3, 5]);

        // Use a boxed range iterator [2, 5).
        let mut it: Box<dyn DocIdSetIterator> = Box::new(crate::search::range(2, 5).unwrap());
        visitor.visit_iterator(it.as_mut()).unwrap();
        assert_eq!(visitor.ids, vec![1, 3, 5, 2, 3, 4]);

        let mut it2: Box<dyn DocIdSetIterator> = Box::new(crate::search::range(0, 2).unwrap());
        visitor
            .visit_iterator_with_value(it2.as_mut(), &[7u8])
            .unwrap();
        assert_eq!(visitor.ids, vec![1, 3, 5, 2, 3, 4, 0, 1]);
    }

    #[test]
    fn visitor_errors_propagate_out_of_intersect() {
        struct FailingVisitor;
        impl IntersectVisitor for FailingVisitor {
            fn visit(&mut self, _doc_id: i32) -> Result<()> {
                Err(LuceneError::CorruptIndex("boom".to_string()))
            }
            fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
                Ok(())
            }
            fn compare(&self, _min: &[u8], _max: &[u8]) -> Relation {
                Relation::CellInsideQuery
            }
        }
        let values = single_leaf_values();
        assert!(values.intersect(&mut FailingVisitor).is_err());
    }
}
