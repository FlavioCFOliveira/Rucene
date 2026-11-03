//! Port of Apache Lucene 10.5.0 `lucene/codecs`.
//!
//! The Lucene module holds the codecs that are not part of `lucene/core`
//! (`org.apache.lucene.codecs.blockterms`, `bloom`, `memory`,
//! `simpletext`, `uniformsplit`, ...). The crate exists so that the workspace
//! has one crate per Lucene module; its content is ported by later tasks.
