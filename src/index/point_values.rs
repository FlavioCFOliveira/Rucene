//! Point value accessors ported from `org.apache.lucene.index`.
//!
//! Equivalent to `org.apache.lucene.index.PointValues` and its nested
//! `IntersectVisitor` interface.
//!
//! Point values provide access to the KD-tree indexed numeric values of a
//! single field. The visitor pattern guides recursive tree traversal.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::DocIdSetIterator;
use crate::util::IntsRef;

/// Maximum number of bytes for each dimension.
///
/// Equivalent to `PointValues.MAX_NUM_BYTES`.
pub const MAX_NUM_BYTES: i32 = 16;

/// Maximum number of dimensions.
///
/// Equivalent to `PointValues.MAX_DIMENSIONS`.
pub const MAX_DIMENSIONS: i32 = 16;

/// Maximum number of index dimensions.
///
/// Equivalent to `PointValues.MAX_INDEX_DIMENSIONS`.
pub const MAX_INDEX_DIMENSIONS: i32 = 8;

// -----------------------------------------------------------------------------
// Relation enum
// -----------------------------------------------------------------------------

/// Relationship between a point cell and a query shape.
///
/// Equivalent to `PointValues.Relation`.
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
pub trait IntersectVisitor: Send + Sync {
    /// Called for all documents in a leaf cell that is fully inside the query.
    fn visit(&mut self, doc_id: i32) -> Result<()>;

    /// Bulk visit of doc IDs from an iterator.
    ///
    /// The default implementation iterates and calls [`visit`](Self::visit) for
    /// each doc ID.
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
    /// The default implementation iterates the active slice and calls
    /// [`visit`](Self::visit).
    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        for doc_id in ints_ref.slice().iter().copied() {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    /// Called for each document in a leaf cell that crosses the query.
    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()>;

    /// Bulk visit of doc IDs with a shared packed value.
    ///
    /// The default implementation iterates and calls
    /// [`visit_with_value`](Self::visit_with_value) for each doc ID.
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

    /// Compares the cell range against the query.
    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation;

    /// Notifies the visitor that approximately `count` documents are about to
    /// be visited.
    fn grow(&mut self, _count: i32) {}
}

// -----------------------------------------------------------------------------
// Point values
// -----------------------------------------------------------------------------

/// Access to indexed point values for a single field.
///
/// Equivalent to `org.apache.lucene.index.PointValues`.
///
/// In Lucene this is an abstract class whose `intersect` and
/// `estimateDocCount` methods are final and rely on a `PointTree`. This Rust
/// port keeps the API surface exposed as trait methods so that implementations
/// can provide the traversal directly.
pub trait PointValues: Send + Sync {
    /// Finds all documents and points matching the provided visitor.
    fn intersect(&self, visitor: &mut dyn IntersectVisitor) -> Result<()>;

    /// Estimates the number of documents that would be matched by
    /// [`intersect`](Self::intersect).
    fn estimate_doc_count(&self, visitor: &mut dyn IntersectVisitor) -> i64;

    /// Estimates the number of points that would be visited by
    /// [`intersect`](Self::intersect).
    fn estimate_point_count(&self, visitor: &mut dyn IntersectVisitor) -> i64;

    /// Returns the total number of indexed points across all documents.
    fn size(&self) -> i64;

    /// Returns the total number of documents that have indexed at least one
    /// point.
    fn doc_count(&self) -> i32;

    /// Returns the minimum packed value, or `None` if there are no points.
    fn min_packed_value(&self) -> Result<Option<Vec<u8>>>;

    /// Returns the maximum packed value, or `None` if there are no points.
    fn max_packed_value(&self) -> Result<Option<Vec<u8>>>;

    /// Returns the number of dimensions represented in the values.
    fn num_dimensions(&self) -> Result<i32>;

    /// Returns the number of dimensions used for the index key.
    fn num_index_dimensions(&self) -> Result<i32>;

    /// Returns the number of bytes in each dimension's values.
    fn bytes_per_dimension(&self) -> Result<i32>;
}

// -----------------------------------------------------------------------------
// Empty implementation
// -----------------------------------------------------------------------------

/// A no-op point-values instance that reports zero dimensions and no values.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyPointValues;

impl PointValues for EmptyPointValues {
    fn intersect(&self, _visitor: &mut dyn IntersectVisitor) -> Result<()> {
        Ok(())
    }

    fn estimate_doc_count(&self, _visitor: &mut dyn IntersectVisitor) -> i64 {
        0
    }

    fn estimate_point_count(&self, _visitor: &mut dyn IntersectVisitor) -> i64 {
        0
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

    /// Point-values stub that simulates a 1D point field with values on docs
    /// 0, 1 and 2.
    struct StubPointValues {
        min_value: Vec<u8>,
        max_value: Vec<u8>,
        bytes_per_dimension: i32,
    }

    impl StubPointValues {
        fn new() -> Self {
            Self {
                min_value: vec![0u8],
                max_value: vec![100u8],
                bytes_per_dimension: 1,
            }
        }
    }

    impl PointValues for StubPointValues {
        fn intersect(&self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
            let relation = visitor.compare(&self.min_value, &self.max_value);
            match relation {
                Relation::CellOutsideQuery => {}
                Relation::CellInsideQuery => {
                    visitor.grow(3);
                    for doc_id in 0..3 {
                        visitor.visit(doc_id)?;
                    }
                }
                Relation::CellCrossesQuery => {
                    for doc_id in 0..3 {
                        visitor.visit_with_value(doc_id, &[doc_id as u8 * 10])?;
                    }
                }
            }
            Ok(())
        }

        fn estimate_doc_count(&self, visitor: &mut dyn IntersectVisitor) -> i64 {
            match visitor.compare(&self.min_value, &self.max_value) {
                Relation::CellOutsideQuery => 0,
                Relation::CellInsideQuery => 3,
                Relation::CellCrossesQuery => 2,
            }
        }

        fn estimate_point_count(&self, visitor: &mut dyn IntersectVisitor) -> i64 {
            self.estimate_doc_count(visitor)
        }

        fn size(&self) -> i64 {
            3
        }

        fn doc_count(&self) -> i32 {
            3
        }

        fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
            Ok(Some(self.min_value.clone()))
        }

        fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
            Ok(Some(self.max_value.clone()))
        }

        fn num_dimensions(&self) -> Result<i32> {
            Ok(1)
        }

        fn num_index_dimensions(&self) -> Result<i32> {
            Ok(1)
        }

        fn bytes_per_dimension(&self) -> Result<i32> {
            Ok(self.bytes_per_dimension)
        }
    }

    /// Visitor that accepts doc IDs in a range and records callbacks.
    struct RecordingVisitor {
        min: u8,
        max: u8,
        visited: Vec<i32>,
        visited_values: Vec<(i32, Vec<u8>)>,
        grown: Vec<i32>,
    }

    impl RecordingVisitor {
        fn new(min: u8, max: u8) -> Self {
            Self {
                min,
                max,
                visited: Vec::new(),
                visited_values: Vec::new(),
                grown: Vec::new(),
            }
        }

        fn contains(&self, value: &[u8]) -> bool {
            value.len() == 1 && value[0] >= self.min && value[0] <= self.max
        }
    }

    impl IntersectVisitor for RecordingVisitor {
        fn visit(&mut self, doc_id: i32) -> Result<()> {
            self.visited.push(doc_id);
            Ok(())
        }

        fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
            if self.contains(packed_value) {
                self.visited_values.push((doc_id, packed_value.to_vec()));
            }
            Ok(())
        }

        fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
            if min_packed_value.len() != 1 || max_packed_value.len() != 1 {
                return Relation::CellCrossesQuery;
            }
            if max_packed_value[0] < self.min || min_packed_value[0] > self.max {
                Relation::CellOutsideQuery
            } else if min_packed_value[0] >= self.min && max_packed_value[0] <= self.max {
                Relation::CellInsideQuery
            } else {
                Relation::CellCrossesQuery
            }
        }

        fn grow(&mut self, count: i32) {
            self.grown.push(count);
        }
    }

    #[test]
    fn point_values_constants_match_java() {
        assert_eq!(MAX_NUM_BYTES, 16);
        assert_eq!(MAX_DIMENSIONS, 16);
        assert_eq!(MAX_INDEX_DIMENSIONS, 8);
    }

    #[test]
    fn stub_point_values_intersect_inside_query() {
        let points = StubPointValues::new();
        let mut visitor = RecordingVisitor::new(0, 100);
        points.intersect(&mut visitor).unwrap();
        assert_eq!(visitor.visited, vec![0, 1, 2]);
        assert_eq!(visitor.grown, vec![3]);
        assert!(visitor.visited_values.is_empty());
    }

    #[test]
    fn stub_point_values_intersect_crosses_query() {
        let points = StubPointValues::new();
        let mut visitor = RecordingVisitor::new(5, 15);
        points.intersect(&mut visitor).unwrap();
        assert!(visitor.visited.is_empty());
        assert_eq!(visitor.visited_values.len(), 1);
        assert_eq!(visitor.visited_values[0].0, 1);
        assert_eq!(visitor.visited_values[0].1, vec![10u8]);
    }

    #[test]
    fn stub_point_values_intersect_outside_query() {
        let points = StubPointValues::new();
        let mut visitor = RecordingVisitor::new(200, 255);
        points.intersect(&mut visitor).unwrap();
        assert!(visitor.visited.is_empty());
        assert!(visitor.visited_values.is_empty());
    }

    #[test]
    fn stub_point_values_estimate_doc_count() {
        let points = StubPointValues::new();
        assert_eq!(
            points.estimate_doc_count(&mut RecordingVisitor::new(0, 100)),
            3
        );
        assert_eq!(
            points.estimate_doc_count(&mut RecordingVisitor::new(5, 15)),
            2
        );
        assert_eq!(
            points.estimate_doc_count(&mut RecordingVisitor::new(200, 255),),
            0
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
        assert_eq!(points.bytes_per_dimension().unwrap(), 0);
        assert_eq!(
            points.estimate_doc_count(&mut RecordingVisitor::new(0, 1)),
            0
        );
        points.intersect(&mut RecordingVisitor::new(0, 1)).unwrap();
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
}
