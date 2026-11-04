//! Port of Apache Lucene 10.5.0 `lucene/codecs`.
//!
//! The Lucene module holds the codecs that are not part of `lucene/core`
//! (`org.apache.lucene.codecs.blockterms`, `bloom`, `memory`,
//! `simpletext`, `uniformsplit`, ...). Ported so far:
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`blockterms`] | `org.apache.lucene.codecs.blockterms` |
//! | [`memory`] | `org.apache.lucene.codecs.memory` |
//! | [`simpletext`] | `org.apache.lucene.codecs.simpletext` |
//! | [`class_loader`] | `lucene-codecs`' `META-INF/services` |

pub mod blockterms;
pub mod memory;
pub mod simpletext;

mod spi;

pub use spi::class_loader;
