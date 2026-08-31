//! Field comparators, ported from `org.apache.lucene.search.comparators`.
//!
//! This package contains the concrete
//! [`FieldComparator`](crate::search::FieldComparator) implementations that
//! back the [`SortField`](crate::search::SortField) types, together with the
//! machinery that lets them skip non-competitive documents during a sorted
//! search.

#![deny(unsafe_code)]

pub mod doc_comparator;
pub mod double_comparator;
pub mod float_comparator;
pub mod int_comparator;
pub mod long_comparator;
pub mod numeric_comparator;
pub mod term_ord_val_comparator;
pub mod updateable_doc_id_set_iterator;

pub use doc_comparator::DocComparator;
pub use double_comparator::DoubleComparator;
pub use float_comparator::FloatComparator;
pub use int_comparator::IntComparator;
pub use long_comparator::LongComparator;
pub use numeric_comparator::{NumericComparator, NumericLeafState, SortableBytes};
pub use term_ord_val_comparator::TermOrdValComparator;
pub use updateable_doc_id_set_iterator::UpdateableDocIdSetIterator;
