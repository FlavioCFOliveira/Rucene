//! Port of Apache Lucene 10.5.0 `lucene/codecs`.
//!
//! The Lucene module holds the codecs that are not part of `lucene/core`
//! (`org.apache.lucene.codecs.blockterms`, `bloom`, `memory`,
//! `simpletext`, `uniformsplit`, ...). Ported so far:
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`simpletext`] | `org.apache.lucene.codecs.simpletext` |
//! | [`class_loader`] | `lucene-codecs`' `META-INF/services` |

pub mod simpletext;

mod spi;

pub use spi::class_loader;
