//! Analysis pipeline ported from `org.apache.lucene.analysis`.
//!
//! Tokenizers, token filters, analyzers, and attribute sources live here.
//! The API is designed to mirror Lucene's `TokenStream` lifecycle while
//! remaining safe and idiomatic in Rust.
