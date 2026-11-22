//! This crate ports Apache Lucene 10.5.0 `lucene/spatial-extras` to Rust: the
//! spatial strategies over spatial4j shapes (recursive and term prefix trees,
//! including the S2 tree, bounding box, serialized doc values, point vector and
//! composite) and the spatial4j bridge to `spatial3d`.
//!
//! Every type of the module is declared here as a skeleton: a declaration that
//! stands for one Java type, without fields, methods or behaviour (an enum lists
//! its Java constants as variants), whose doc names the sprint-17 rmp task that
//! will port it. A type stays a skeleton until that task replaces it with its
//! port.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.spatial`: the types of that package sit at the crate root
//! and `org.apache.lucene.spatial.prefix.tree` is the module [`prefix::tree`].
//! Each Java source file is one module, named in snake_case
//! (`SpatialStrategy.java` is [`spatial_strategy`]); a Java package name that is
//! not snake_case becomes a snake_case module, and one that is a Rust keyword a
//! raw identifier. A nested Java type is declared in the module of its source
//! file under its own name; the name of its enclosing type is prefixed only when
//! its own name is a name of the Rust prelude or the name of another item of
//! that module, and the Java kind is appended when the prefixed name is itself
//! taken.

// Package-private and private Java types are `pub(crate)` skeletons that nothing
// uses yet. This allowance goes away when the module is fully ported.
#![allow(dead_code)]

pub mod bbox;
pub mod composite;
pub mod prefix;
pub mod query;
pub mod serialized;
pub mod shape_values;
pub mod shape_values_source;
pub mod spatial4j;
pub mod spatial_strategy;
pub mod util;
pub mod vector;
