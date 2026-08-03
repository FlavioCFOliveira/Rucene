//! Document, field, and field-type abstractions ported from
//! `org.apache.lucene.document`.
//!
//! This module models how documents are built, what field types are available,
//! and how field values are stored, indexed, or used for doc values.

#![deny(unsafe_code)]

/// Describes how an `IndexableField` should be inverted for indexing terms and
/// postings.
///
/// Equivalent to `org.apache.lucene.document.InvertableType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum InvertableType {
    /// The field is treated as a single binary value.
    BINARY,
    /// The field is inverted through its token stream.
    TOKEN_STREAM,
}

/// A numeric value carried by an `IndexableField`, corresponding to Java's
/// `java.lang.Number`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumericValue {
    /// A 32-bit signed integer.
    Int(i32),
    /// A 64-bit signed integer.
    Long(i64),
    /// A 32-bit IEEE-754 float.
    Float(f32),
    /// A 64-bit IEEE-754 double.
    Double(f64),
}

/// Abstraction around a stored value.
///
/// Equivalent to `org.apache.lucene.document.StoredValue`. This is a minimal
/// placeholder for the indexing support layer.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredValue;
