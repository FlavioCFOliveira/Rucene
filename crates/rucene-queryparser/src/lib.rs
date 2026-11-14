//! This crate ports Apache Lucene 10.5.0 `lucene/queryparser` to Rust: the
//! classic, flexible (standard and precedence), simple, surround, complex-phrase
//! and extendable query parsers, and the XML query parser.
//!
//! Every type of the module is declared here as a skeleton: a declaration that
//! stands for one Java type, without fields, methods or behaviour (an enum
//! lists its Java constants as variants), whose doc names the sprint-13 rmp task
//! that will port it. A type stays a skeleton until that task replaces it with
//! its port.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.queryparser`: `org.apache.lucene.queryparser.classic` is
//! the module [`classic`] and
//! `org.apache.lucene.queryparser.flexible.standard.parser` is the module
//! [`flexible::standard::parser`]. Each Java source file is one module, named in
//! snake_case (`QueryParserBase.java` is [`classic::query_parser_base`]). A Java
//! package name that is not snake_case becomes a snake_case module: the Java
//! package `org.apache.lucene.queryparser.complexPhrase` is the module
//! [`complex_phrase`]. A nested Java type is declared in the module of its
//! source file under its own name (`QueryParser.Operator` is `Operator`); the
//! name of its enclosing type is prefixed only when its own name is a name of
//! the Rust prelude or the name of another item of that module.

// Package-private and private Java types are `pub(crate)` skeletons that nothing
// uses yet. This allowance goes away when the module is fully ported.
#![allow(dead_code)]

pub mod charstream;
pub mod classic;
pub mod complex_phrase;
pub mod ext;
pub mod flexible;
pub mod simple;
pub mod surround;
pub mod xml;
