//! This crate ports Apache Lucene 10.5.0 `lucene/analysis/common` to Rust: the
//! analyzers, tokenizers, token filters and char filters for many languages,
//! with their SPI factories and the Snowball stemmers.
//!
//! Every type of the module is declared here: a ported type carries its port
//! claim (`Equivalent to ...`); a type not ported yet is a skeleton, a
//! declaration that stands for one Java type, without fields, methods or
//! behaviour, whose doc names the sprint-12 rmp task that will port it. A type
//! stays a skeleton until that task replaces it with its port.
//!
//! The module tree mirrors the Java packages: `org.apache.lucene.analysis.*`
//! under [`analysis`], `org.apache.lucene.collation.*` under [`collation`] and
//! `org.tartarus.snowball.*` under [`tartarus`]. The Java package
//! `org.apache.lucene.analysis.in` is the module `analysis::r#in`, because `in`
//! is a Rust keyword.
//!
//! Beside the packages, [`spi`] holds the module's `META-INF/services` files
//! (the factory registry), [`resources`] its resource files, and [`jdk`] the
//! JDK behaviour the module calls that `rucene` does not emulate.

// Package-private and private Java types are `pub(crate)` skeletons that nothing
// uses yet. This allowance goes away when the module is fully ported.
#![allow(dead_code)]

pub mod analysis;
pub mod collation;
pub mod jdk;
pub mod resources;
pub mod spi;
pub mod tartarus;
