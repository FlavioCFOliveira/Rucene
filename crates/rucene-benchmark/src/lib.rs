//! This crate ports Apache Lucene 10.5.0 `lucene/benchmark` to Rust: the
//! `byTask` benchmark framework (algorithms, content sources, query makers,
//! tasks and reports) and the `quality` search-quality evaluation with its TREC
//! readers.
//!
//! Every type of the module is ported, each documented with the Java type
//! it is equivalent to. Java's `Class.forName` lookups of tasks, sources,
//! makers and parsers go through [`classes`]; the JDK and icu4j behaviours
//! the module relies on are reproduced in crate-private modules.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.benchmark`: the types of that package sit at the crate
//! root and `org.apache.lucene.benchmark.byTask.feeds` is the module
//! [`by_task::feeds`]. Each Java source file is one module, named in snake_case
//! (`Constants.java` is [`constants`]); a Java package name that is not
//! snake_case becomes a snake_case module, and one that is a Rust keyword a raw
//! identifier. A nested Java type is declared in the module of its source file
//! under its own name; the name of its enclosing type is prefixed only when its
//! own name is a name of the Rust prelude or the name of another item of that
//! module, and the Java kind is appended when the prefixed name is itself taken.


pub mod by_task;
pub mod constants;
pub mod quality;
pub mod utils;

pub mod classes;
pub mod icu;
pub mod jdk;
