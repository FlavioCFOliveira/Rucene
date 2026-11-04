//! Port of Apache Lucene 10.5.0 `lucene/codecs`.
//!
//! The Lucene module holds the codecs that are not part of `lucene/core`
//! (`org.apache.lucene.codecs.blockterms`, `bloom`, `memory`,
//! `simpletext`, `uniformsplit`, ...). Ported so far:
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`bitvectors`] | `org.apache.lucene.codecs.bitvectors` |
//! | [`blockterms`] | `org.apache.lucene.codecs.blockterms` |
//! | [`bloom`] | `org.apache.lucene.codecs.bloom` |
//! | [`memory`] | `org.apache.lucene.codecs.memory` |
//! | [`simpletext`] | `org.apache.lucene.codecs.simpletext` |
//! | [`class_loader`] | `lucene-codecs`' `META-INF/services` |

pub mod bitvectors;
pub mod blockterms;
pub mod bloom;
pub mod memory;
pub mod simpletext;

mod spi;

pub use spi::class_loader;
