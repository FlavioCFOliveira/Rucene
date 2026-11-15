//! This crate ports Apache Lucene 10.5.0 `lucene/queryparser` to Rust: the
//! classic, flexible (standard and precedence), simple, surround, complex-phrase
//! and extendable query parsers, and the XML query parser.
//!
//! Every type of the module is ported: each Rust item that stands for a Java
//! type says so in its documentation (`Equivalent to ...`), and declares at its
//! point every divergence from the Java source. The JavaCC parsers (classic,
//! surround and the flexible `StandardSyntaxParser`) are translated from the
//! generated Java Lucene commits by `tools/translate/javacc_java_to_rust.py`,
//! and the `java.text`, `java.util` and JAXP behaviour the parsers rely on is
//! emulated in [`jdk`].
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

pub mod charstream;
pub mod classic;
pub mod complex_phrase;
pub mod ext;
pub mod flexible;
pub(crate) mod javacc;
pub mod jdk;
pub mod simple;
pub mod surround;
pub mod xml;
