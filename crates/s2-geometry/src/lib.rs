//! This crate ports, from s2-geometry-library-java 1.0.0
//! (`io.sgr:s2-geometry-library-java:1.0.0`, tag `v1.0.0`, Apache-2.0, Copyright
//! Google Inc.), the types Apache Lucene 10.5.0 `lucene/spatial-extras` imports
//! (`S2Cell`, `S2CellId`, `S2LatLng`, `S2Point`, `S2Projections`) and their
//! transitive closure over the library's main code. The rest of the library is
//! out of scope (user decision, sprint 17). Each file keeps its upstream
//! copyright and license notice.
//!
//! Every type of that subset is declared here as a skeleton: a declaration that
//! stands for one Java type, without fields, methods or behaviour (an enum lists
//! its Java constants as variants), whose doc names the sprint-17 rmp task that
//! will port it. A type stays a skeleton until that task replaces it with its
//! port.
//!
//! The module tree mirrors the Java packages below the package root
//! `com.google.common.geometry`: the types of that package sit at the crate
//! root. Each Java source file is one module, named in snake_case
//! (`S2CellId.java` is [`s2_cell_id`]); a Java package name that is not
//! snake_case becomes a snake_case module, and one that is a Rust keyword a raw
//! identifier. A nested Java type is declared in the module of its source file
//! under its own name; the name of its enclosing type is prefixed only when its
//! own name is a name of the Rust prelude or the name of another item of that
//! module, and the Java kind is appended when the prefixed name is itself taken.

// Package-private and private Java types are `pub(crate)` skeletons that nothing
// uses yet. This allowance goes away when the module is fully ported.
#![allow(dead_code)]

pub mod mutable_integer;
pub mod r1_interval;
pub mod r2_vector;
pub mod s1_angle;
pub mod s1_interval;
pub mod s2;
pub mod s2_cap;
pub mod s2_cell;
pub mod s2_cell_id;
pub mod s2_edge_util;
pub mod s2_lat_lng;
pub mod s2_lat_lng_rect;
pub mod s2_point;
pub mod s2_projections;
pub mod s2_region;
