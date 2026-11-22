//! This crate ports, from LocationTech spatial4j 0.8
//! (`org.locationtech.spatial4j:spatial4j:0.8`, tag `spatial4j-0.8`,
//! Apache-2.0), the types Apache Lucene 10.5.0 `lucene/spatial-extras` and
//! `lucene/benchmark` import and their transitive closure over the library's
//! main code, plus the test helpers `lucene/spatial-extras`' tests import
//! (`TestLog`, `shape::RandomizedShapeTest`,
//! `shape::RectIntersectionTestHelper`). The rest of the library is out of scope
//! (user decision, sprint 17). Each file keeps its upstream copyright and
//! license notice.
//!
//! Every type of that subset is declared here as a skeleton: a declaration that
//! stands for one Java type, without fields, methods or behaviour (an enum lists
//! its Java constants as variants), whose doc names the sprint-17 rmp task that
//! will port it. A type stays a skeleton until that task replaces it with its
//! port.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.locationtech.spatial4j`: `org.locationtech.spatial4j.shape.impl` is the
//! module `shape::r#impl`. Each Java source file is one module, named in
//! snake_case (`SpatialContext.java` is [`context::spatial_context`]); a Java
//! package name that is not snake_case becomes a snake_case module, and one that
//! is a Rust keyword a raw identifier. A nested Java type is declared in the
//! module of its source file under its own name; the name of its enclosing type
//! is prefixed only when its own name is a name of the Rust prelude or the name
//! of another item of that module, and the Java kind is appended when the
//! prefixed name is itself taken.

// Package-private and private Java types are `pub(crate)` skeletons that nothing
// uses yet. This allowance goes away when the module is fully ported.
#![allow(dead_code)]

pub mod context;
pub mod distance;
pub mod exception;
pub mod io;
pub mod shape;
pub mod test_log;
