//! This crate ports Apache Lucene 10.5.0 `lucene/memory` to Rust: a
//! high-performance single-document main memory Apache Lucene fulltext search
//! index (package `org.apache.lucene.index.memory`).
//!
//! [`MemoryIndex`] holds the fields of one document in memory and answers
//! queries against it without a `Directory`: add fields (text through an
//! analyzer, token streams, or indexable fields), then [`MemoryIndex::search`] for a
//! relevance score, or [`MemoryIndex::create_searcher`] for an `IndexSearcher` over its
//! single-document reader. It suits real-time matching of a stream of
//! documents against stored queries, and transient indexes such as the one
//! the highlighters build to find matches in a single text.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.index.memory`: the types of that package sit at the crate
//! root. Each Java source file is one module, named in snake_case
//! (`MemoryIndex.java` is [`memory_index`]); a Java package name that is not
//! snake_case becomes a snake_case module, and one that is a Rust keyword a raw
//! identifier. A nested Java type is declared in the module of its source file
//! under its own name; the name of its enclosing type is prefixed only when its
//! own name is a name of the Rust prelude or the name of another item of that
//! module, and the Java kind is appended when the prefixed name is itself taken.

pub mod memory_index;

pub use memory_index::MemoryIndex;
