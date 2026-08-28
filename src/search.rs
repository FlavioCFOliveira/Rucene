//! Search engine ported from `org.apache.lucene.search`.
//!
//! Queries, collectors, scorers, `IndexSearcher`, and sorting live in this
//! module. Functional parity with Java Lucene's search behavior is the goal,
//! even though the public API is async.

#![deny(unsafe_code)]

pub mod doc_id_set_iterator;
pub mod knn;
pub mod reference_manager;
pub mod similarities;
pub mod sort;

pub use doc_id_set_iterator::{
    all, empty, from_iterator_supplier, from_live_docs, range, AcceptDocs, AllDocIdSetIterator,
    BitsAcceptDocs, DocIdSetIterator, DocIdSetIteratorSupplier, EmptyDocIdSetIterator,
    IteratorAcceptDocs, RangeDocIdSetIterator, NO_MORE_DOCS,
};
pub use reference_manager::{ManagedReference, ReferenceManager, RefreshListener, RefreshSource};
pub use similarities::{compute_default_norm, BM25Similarity, Similarity};
pub use sort::{read_sort, write_sort, MissingValue, Sort, SortField, SortFieldType};
