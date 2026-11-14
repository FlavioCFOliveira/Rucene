//! This crate ports Apache Lucene 10.5.0 `lucene/analysis/common` to Rust: the
//! analyzers, tokenizers, token filters and char filters for many languages,
//! with their SPI factories and the Snowball stemmers.
//!
//! Every type of the module is ported here, and carries its port claim
//! (`Equivalent to ...`).
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

pub mod analysis;
pub mod collation;
pub mod jdk;
pub mod resources;
pub mod spi;
pub mod tartarus;
