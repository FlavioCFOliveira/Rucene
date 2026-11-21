//! `MultiLeafReader` ported from `org.apache.lucene.index`.
//!
//! Equivalent to `org.apache.lucene.index.MultiLeafReader`: the place where
//! Lucene documents how to treat an [`IndexReader`](crate::index::IndexReader)
//! as if it were a [`LeafReader`](crate::index::LeafReader).
//!
//! **NOTE**: for composite readers you will get better performance by gathering
//! the sub-readers through
//! [`IndexReader::get_context`](crate::index::IndexReader::get_context) to
//! obtain the atomic leaves and then operating per-`LeafReader`, instead of
//! using the flattened views described here. Every one of them resolves the
//! owning sub-reader on each lookup.
//!
//! # Why this module carries no code
//!
//! In Lucene Core 10.5.0 `MultiLeafReader` is a `public class` with nothing but
//! a private constructor — verified against
//! `lucene/core/src/java/org/apache/lucene/index/MultiLeafReader.java` at tag
//! `releases/lucene/10.5.0`, which is 32 lines of which only the class
//! declaration and `private MultiLeafReader() {}` are code. Nothing in Lucene
//! Core references it. The class survives as the documentation anchor for the
//! flattening helpers, which live on the types that produce them.
//!
//! Porting it therefore means porting that anchor, not inventing an API Lucene
//! does not have: a Rust module cannot be instantiated, so it expresses "a
//! namespace you cannot construct" exactly as the Java class does. Adding
//! wrapper functions here would break functional parity in the direction that
//! matters least — extra public surface this crate would then have to keep.
//!
//! # Where the flattened views actually live
//!
//! | Flattened view | Entry point |
//! | --- | --- |
//! | Fields across all leaves | [`MultiFields::get_fields`](crate::index::MultiFields::get_fields) |
//! | Terms for one field across all leaves | [`MultiTerms::get_terms`](crate::index::MultiTerms::get_terms) |
//! | Postings for one term across all leaves | [`MultiTerms::get_term_postings_enum`](crate::index::MultiTerms::get_term_postings_enum) |
//! | Live docs across all leaves | [`MultiBits::get_live_docs`](crate::index::MultiBits::get_live_docs) |
//! | Doc-values ordinals across all leaves | [`OrdinalMap`](crate::index::OrdinalMap) |
//! | Global doc ID to leaf mapping | [`reader_util`](crate::index::multi_reader::reader_util) |

#![deny(unsafe_code)]
