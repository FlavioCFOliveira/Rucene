//! Rucene — a Rust port of Apache Lucene Core 10.5.0.
//!
//! This crate aims for functional parity and 100% index-file compatibility
//! with the reference Java implementation. Module organization mirrors
//! Lucene Core's package structure to simplify porting and comparison.
//!
//! Reference: <https://lucene.apache.org/core/10_5_0/index.html>

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod analysis;
pub mod codecs;
pub mod document;
pub mod error;
pub mod geo;
pub mod index;
pub mod internal;
pub mod search;
pub mod store;
pub mod util;

pub use error::{LuceneError, Result};
