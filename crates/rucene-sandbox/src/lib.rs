//! This crate ports, from Apache Lucene 10.5.0 `lucene/sandbox`, only
//! `FuzzyLikeThisQuery.java`, which the XML query parser's
//! `FuzzyLikeThisQueryBuilder` needs. The rest of the module is out of scope
//! (user decision, sprint 13).
//!
//! Every type of that file is declared here as a skeleton: a declaration that
//! stands for one Java type, without fields, methods or behaviour (an enum
//! lists its Java constants as variants), whose doc names the sprint-13 rmp task
//! that will port it. A type stays a skeleton until that task replaces it with
//! its port.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.sandbox`: `org.apache.lucene.sandbox.queries` is the module
//! [`queries`]. Each Java source file is one module, named in snake_case
//! (`FuzzyLikeThisQuery.java` is [`queries::fuzzy_like_this_query`]); a Java
//! package name that is not snake_case becomes a snake_case module. A nested
//! Java type is declared in the module of its source file under its own name
//! (`FuzzyLikeThisQuery.FieldVals` is `FieldVals`); the name of its enclosing
//! type is prefixed only when its own name is a name of the Rust prelude or the
//! name of another item of that module.

// Package-private and private Java types are `pub(crate)` skeletons that nothing
// uses yet. This allowance goes away when the module is fully ported.
#![allow(dead_code)]

pub mod queries;
