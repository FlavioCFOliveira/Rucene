//! Indexing engine ported from `org.apache.lucene.index`.
//!
//! `IndexWriter`, `IndexReader`, segment management, merge policies, and
//! commit semantics are defined here. The implementation must produce index
//! files that Java Lucene 10.5.0 can read, and read files produced by it.
