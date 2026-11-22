//! This crate ports Apache Lucene 10.5.0 `lucene/spatial3d` to Rust: shapes on
//! the surface of a sphere or ellipsoid modelled with 3D planar geometry
//! (`geom`), and the `Geo3DPoint` field, its doc values, sorts and queries.
//!
//! Every type of the module is declared here as a skeleton: a declaration that
//! stands for one Java type, without fields, methods or behaviour (an enum lists
//! its Java constants as variants), whose doc names the sprint-17 rmp task that
//! will port it. A type stays a skeleton until that task replaces it with its
//! port.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.spatial3d`: the types of that package sit at the crate
//! root and `org.apache.lucene.spatial3d.geom` is the module [`geom`]. Each Java
//! source file is one module, named in snake_case (`Geo3DPoint.java` is
//! [`geo3_d_point`]); a Java package name that is not snake_case becomes a
//! snake_case module, and one that is a Rust keyword a raw identifier. A nested
//! Java type is declared in the module of its source file under its own name;
//! the name of its enclosing type is prefixed only when its own name is a name
//! of the Rust prelude or the name of another item of that module, and the Java
//! kind is appended when the prefixed name is itself taken.

// Package-private and private Java types are `pub(crate)` skeletons that nothing
// uses yet. This allowance goes away when the module is fully ported.
#![allow(dead_code)]

pub mod geo3_d_doc_values_field;
pub mod geo3_d_point;
pub mod geo3_d_point_distance_comparator;
pub mod geo3_d_point_outside_distance_comparator;
pub mod geo3_d_point_outside_sort_field;
pub mod geo3_d_point_sort_field;
pub mod geo3_d_util;
pub mod geom;
pub mod point_in_geo3_d_shape_query;
pub mod point_in_shape_intersect_visitor;
