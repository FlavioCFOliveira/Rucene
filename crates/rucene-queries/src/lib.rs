//! This crate ports Apache Lucene 10.5.0 `lucene/queries` to Rust: the query
//! types outside `lucene/core`, namely function queries and value sources,
//! intervals, spans, payload queries, more-like-this and `CommonTermsQuery`.
//!
//! Every type of the module is declared here as a skeleton: a declaration that
//! stands for one Java type, without fields, methods or behaviour (an enum
//! lists its Java constants as variants), whose doc names the sprint-13 rmp task
//! that will port it. A type stays a skeleton until that task replaces it with
//! its port.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.queries`: the types of that package sit at the crate root
//! and `org.apache.lucene.queries.function.valuesource` is the module
//! [`function::valuesource`]. Each Java source file is one module, named in
//! snake_case (`CommonTermsQuery.java` is [`common_terms_query`]); a Java package
//! name that is not snake_case becomes a snake_case module. A nested Java type
//! is declared in the module of its source file under its own name
//! (`MoreLikeThis.FreqQ` is `FreqQ`); the name of its enclosing type is prefixed
//! only when its own name is a name of the Rust prelude or the name of another
//! item of that module.

// Package-private and private Java types are `pub(crate)` skeletons that nothing
// uses yet. This allowance goes away when the module is fully ported.
#![allow(dead_code)]

pub mod common_terms_query;
pub mod function;
pub mod intervals;
pub mod mlt;
pub mod payloads;
pub mod spans;
