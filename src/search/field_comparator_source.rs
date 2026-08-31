//! Custom comparator factories, ported from
//! `org.apache.lucene.search.FieldComparatorSource`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::error::Result;
use crate::search::field_comparator::FieldComparator;
use crate::search::pruning::Pruning;

/// Provides a [`FieldComparator`] for custom field sorting.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.FieldComparatorSource`, which
/// [`SortField`](crate::search::SortField) holds for a
/// [`SortFieldType::Custom`](crate::search::SortFieldType::Custom) sort.
pub trait FieldComparatorSource: Debug + Send + Sync {
    /// Creates a comparator for the field in the given index.
    ///
    /// Equivalent to
    /// `FieldComparatorSource.newComparator(String, int, Pruning, boolean)`.
    ///
    /// * `fieldname` — name of the field to create the comparator for;
    /// * `num_hits` — the number of slots the comparator must hold;
    /// * `pruning` — how the comparator may skip non-competitive documents;
    /// * `reversed` — whether the sort on this field is reversed.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the comparator. Java
    /// declares no checked exception here, but every non-trivial source reads
    /// from the index eventually.
    fn new_comparator(
        &self,
        fieldname: &str,
        num_hits: usize,
        pruning: Pruning,
        reversed: bool,
    ) -> Result<Box<dyn FieldComparator>>;
}
