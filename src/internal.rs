//! Internal utilities ported from `org.apache.lucene.internal`.
//!
//! These are Lucene-internal packages: they are not part of the public API
//! contract of Apache Lucene Core, but they are load-bearing for the rest of
//! the library and are therefore ported alongside it.

#![deny(unsafe_code)]

pub mod hppc;
pub mod tests_hooks;
pub mod vectorization;
