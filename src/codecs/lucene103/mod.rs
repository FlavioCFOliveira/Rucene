//! Lucene 10.3 blocktree terms dictionary.
//!
//! This module ports `org.apache.lucene.codecs.lucene103.blocktree`, the terms
//! dictionary used by the default `Lucene104` postings format to read and write
//! the `.tim`, `.tmd` and `.tip` index files.
//!
//! Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree`.

pub mod blocktree;

pub use blocktree::{
    CompressionAlgorithm, Lucene103BlockTreeTermsReader, Lucene103BlockTreeTermsWriter, Stats,
    TrieBuilder, TrieReader,
};
