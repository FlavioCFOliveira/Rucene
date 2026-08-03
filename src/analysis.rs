//! Analysis pipeline ported from `org.apache.lucene.analysis`.
//!
//! Tokenizers, token filters, analyzers, attribute sources, and the
//! character-array collections used by the pipeline live here. The API is
//! designed to mirror Lucene's `TokenStream` lifecycle while remaining safe
//! and idiomatic in Rust.

#![deny(unsafe_code)]

pub mod char_array_map;
pub mod char_array_set;
pub mod tokenattributes;

pub use char_array_map::CharArrayMap;
pub use char_array_set::CharArraySet;
pub use tokenattributes::{
    BytesTermAttribute, BytesTermAttributeImpl, CharTermAttribute, CharTermAttributeImpl,
    FlagsAttribute, FlagsAttributeImpl, KeywordAttribute, KeywordAttributeImpl, OffsetAttribute,
    OffsetAttributeImpl, PayloadAttribute, PayloadAttributeImpl, PositionIncrementAttribute,
    PositionIncrementAttributeImpl, PositionLengthAttribute, PositionLengthAttributeImpl,
    SentenceAttribute, SentenceAttributeImpl, TermFrequencyAttribute, TermFrequencyAttributeImpl,
    TermToBytesRefAttribute, TypeAttribute, TypeAttributeImpl,
};

/// A stream of tokens produced by an analyzer, equivalent to
/// `org.apache.lucene.analysis.TokenStream`.
///
/// This is a minimal placeholder trait for the indexing support layer.
pub trait TokenStream {}

/// Components that transform input text into a `TokenStream`, equivalent to
/// `org.apache.lucene.analysis.Analyzer`.
///
/// This is a minimal placeholder trait for the indexing support layer.
pub trait Analyzer {}
