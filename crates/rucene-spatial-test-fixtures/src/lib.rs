//! This crate ports Apache Lucene 10.5.0 `lucene/spatial-test-fixtures` to Rust:
//! `RandomGeo3dShapeGenerator`, the random `spatial3d` shapes the spatial tests
//! draw.
//!
//! From the module's `overview.html`: internal test fixtures for spatial
//! modules.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.spatial3d.tests`: the types of that package sit at the
//! crate root. Each Java source file is one module, named in snake_case
//! (`RandomGeo3dShapeGenerator.java` is [`random_geo3d_shape_generator`]); a
//! Java package name that is not snake_case becomes a snake_case module, and one
//! that is a Rust keyword a raw identifier. A nested Java type is declared in
//! the module of its source file under its own name; the name of its enclosing
//! type is prefixed only when its own name is a name of the Rust prelude or the
//! name of another item of that module, and the Java kind is appended when the
//! prefixed name is itself taken.

pub mod random_geo3d_shape_generator;
