//! Terms dictionary abstractions ported from `org.apache.lucene.index`.
//!
//! This module provides [`Fields`], [`Terms`], [`TermsEnum`] and [`TermState`],
//! the core types used by postings producers and consumers to enumerate terms
//! and retrieve their associated postings.

#![deny(unsafe_code)]

use std::any::Any;

use crate::error::{LuceneError, Result};
use crate::index::postings_enum::{ImpactsEnum, PostingsEnum};
use crate::util::attribute::AttributeSource;
use crate::util::automaton::CompiledAutomaton;
use crate::util::BytesRef;

/// Internal state that allows re-positioning a [`TermsEnum`] without re-seeking
/// the term dictionary.
///
/// Equivalent to `org.apache.lucene.index.TermState`.
pub trait TermState: Send + Sync + std::fmt::Debug {
    /// Copies the content of `other` into `self`.
    fn copy_from(&mut self, other: &dyn TermState);

    /// Returns a boxed clone of this state.
    fn clone_box(&self) -> Box<dyn TermState>;

    /// Returns this state as `Any` for downcasting.
    fn as_any(&self) -> &dyn Any;
}

/// Result of a [`TermsEnum::seek_ceil`] call.
///
/// Equivalent to `TermsEnum.SeekStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum SeekStatus {
    /// The precise term was found.
    FOUND,
    /// A different term was found after the requested term.
    NOT_FOUND,
    /// The term was not found and the end of iteration was hit.
    END,
}

/// Iterator over the terms of a single field.
///
/// Equivalent to `org.apache.lucene.index.TermsEnum`.
pub trait TermsEnum {
    /// Returns the attribute source attached to this enum.
    fn attributes(&mut self) -> &mut AttributeSource;

    /// Attempts to seek to the exact term, returning `true` if found.
    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool>;

    /// Seeks to the ceiling term and reports the result.
    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus>;

    /// Seeks to the term at the given ordinal position.
    fn seek_ord(&mut self, ord: i64) -> Result<()>;

    /// Seeks to a previously captured [`TermState`].
    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()>;

    /// Returns the current term.
    fn term(&self) -> Result<BytesRef>;

    /// Returns the ordinal position of the current term, if supported.
    fn ord(&self) -> Result<i64>;

    /// Returns the number of documents containing the current term.
    fn doc_freq(&self) -> Result<i32>;

    /// Returns the total number of occurrences of the current term.
    fn total_term_freq(&self) -> Result<i64>;

    /// Returns a postings enum for the current term with the given flags.
    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>>;

    /// Returns an impacts enum for the current term with the given flags.
    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>>;

    /// Captures the current enum state.
    fn term_state(&mut self) -> Result<Box<dyn TermState>>;

    /// Returns `true` if this enum prefers exact seeks.
    fn prefer_seek_exact(&self) -> bool {
        false
    }

    /// Returns the next term, or `None` at the end of iteration.
    fn next(&mut self) -> Result<Option<BytesRef>>;
}

/// Access to the terms of a specific field.
///
/// Equivalent to `org.apache.lucene.index.Terms`.
pub trait Terms {
    /// Returns an iterator over all terms in this field.
    fn iterator(&self) -> Result<Box<dyn TermsEnum>>;

    /// Returns a terms enum restricted to terms accepted by the automaton,
    /// starting after `start_term`.
    fn intersect(
        &self,
        _compiled: &CompiledAutomaton,
        _start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        Err(LuceneError::UnsupportedOperation(
            "intersect not implemented".to_string(),
        ))
    }

    /// Returns the number of terms in this field, or `-1` if unknown.
    fn size(&self) -> i64;

    /// Returns the sum of `totalTermFreq` for all terms in this field.
    fn sum_total_term_freq(&self) -> i64;

    /// Returns the sum of `docFreq` for all terms in this field.
    fn sum_doc_freq(&self) -> i64;

    /// Returns the number of documents with at least one term for this field.
    fn doc_count(&self) -> i32;

    /// Returns `true` if this field stores term frequencies.
    fn has_freqs(&self) -> bool;

    /// Returns `true` if this field stores offsets.
    fn has_offsets(&self) -> bool;

    /// Returns `true` if this field stores positions.
    fn has_positions(&self) -> bool;

    /// Returns `true` if this field stores payloads.
    fn has_payloads(&self) -> bool;

    /// Returns the smallest term in this field, or `None` if empty.
    fn min(&self) -> Result<Option<BytesRef>> {
        let mut it = self.iterator()?;
        it.next()
    }

    /// Returns the largest term in this field, or `None` if empty.
    fn max(&self) -> Result<Option<BytesRef>> {
        let size = self.size();
        if size == 0 {
            return Ok(None);
        }
        if size > 0 {
            let mut it = self.iterator()?;
            if it.seek_ord(size - 1).is_ok() {
                return Ok(Some(it.term()?));
            }
        }
        // Fallback: binary-search for the last term one byte at a time.
        let mut it = self.iterator()?;
        if it.next()?.is_none() {
            return Ok(None);
        }
        let mut scratch = vec![0u8];
        loop {
            let mut low = 0u8;
            let mut high = 0u8.wrapping_sub(1);
            while low != high {
                let mid = ((low as u16 + high as u16) / 2) as u8;
                *scratch.last_mut().unwrap() = mid;
                match it.seek_ceil(&BytesRef::new(scratch.clone()))? {
                    SeekStatus::END => {
                        if mid == 0 {
                            scratch.pop();
                            return Ok(Some(BytesRef::new(scratch)));
                        }
                        high = mid;
                    }
                    _ => {
                        if low == mid {
                            break;
                        }
                        low = mid;
                    }
                }
            }
            scratch.push(0);
            let _ = it.term()?;
        }
    }

    /// Returns additional debug statistics about this terms instance.
    fn stats(&self) -> String {
        format!(
            "impl=Terms,size={},docCount={},sumTotalTermFreq={},sumDocFreq={}",
            self.size(),
            self.doc_count(),
            self.sum_total_term_freq(),
            self.sum_doc_freq()
        )
    }
}

/// Collection of per-field [`Terms`] instances.
///
/// Equivalent to `org.apache.lucene.index.Fields`.
pub trait Fields {
    /// Returns an iterator over all field names.
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_>;

    /// Returns the [`Terms`] for the given field, or `None` if absent.
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>>;

    /// Returns the number of fields, or `-1` if unknown.
    fn size(&self) -> i32;
}

/// An empty [`TermsEnum`].
#[derive(Debug, Clone)]
pub struct EmptyTermsEnum {
    atts: AttributeSource,
}

impl Default for EmptyTermsEnum {
    fn default() -> Self {
        Self::new()
    }
}

impl EmptyTermsEnum {
    /// Creates an empty terms enum.
    pub fn new() -> Self {
        Self {
            atts: AttributeSource::new(),
        }
    }
}

impl TermsEnum for EmptyTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.atts
    }

    fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
        Ok(false)
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Ok(SeekStatus::END)
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Ok(())
    }

    fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        Err(LuceneError::IllegalState(
            "seek_term_state should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn term(&self) -> Result<BytesRef> {
        Err(LuceneError::IllegalState(
            "term should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::IllegalState(
            "ord should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn doc_freq(&self) -> Result<i32> {
        Err(LuceneError::IllegalState(
            "doc_freq should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn total_term_freq(&self) -> Result<i64> {
        Err(LuceneError::IllegalState(
            "total_term_freq should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        Err(LuceneError::IllegalState(
            "postings should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Err(LuceneError::IllegalState(
            "impacts should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Err(LuceneError::IllegalState(
            "term_state should never be called on EmptyTermsEnum".to_string(),
        ))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        Ok(None)
    }
}

/// An empty [`Terms`] instance.
#[derive(Debug, Clone, Default)]
pub struct EmptyTerms;

impl EmptyTerms {
    /// Creates an empty terms instance.
    pub fn new() -> Self {
        Self
    }
}

impl Terms for EmptyTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(EmptyTermsEnum::new()))
    }

    fn size(&self) -> i64 {
        0
    }

    fn sum_total_term_freq(&self) -> i64 {
        0
    }

    fn sum_doc_freq(&self) -> i64 {
        0
    }

    fn doc_count(&self) -> i32 {
        0
    }

    fn has_freqs(&self) -> bool {
        false
    }

    fn has_offsets(&self) -> bool {
        false
    }

    fn has_positions(&self) -> bool {
        false
    }

    fn has_payloads(&self) -> bool {
        false
    }
}

/// An empty [`Fields`] instance.
#[derive(Debug, Clone, Default)]
pub struct EmptyFields;

impl EmptyFields {
    /// Creates an empty fields collection.
    pub fn new() -> Self {
        Self
    }
}

impl Fields for EmptyFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(std::iter::empty())
    }

    fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(None)
    }

    fn size(&self) -> i32 {
        0
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::postings_enum::{EmptyPostingsEnum, POSTINGS_ENUM_FREQS};

    #[test]
    fn empty_terms_enum_returns_no_terms() {
        let mut it = EmptyTermsEnum::new();
        assert_eq!(it.next().unwrap(), None);
        assert_eq!(
            it.seek_ceil(&BytesRef::new(vec![0x61])).unwrap(),
            SeekStatus::END
        );
        assert!(!it.seek_exact(&BytesRef::new(vec![0x61])).unwrap());
    }

    #[test]
    fn empty_terms_reports_zero_counts() {
        let terms = EmptyTerms::new();
        assert_eq!(terms.size(), 0);
        assert_eq!(terms.sum_total_term_freq(), 0);
        assert_eq!(terms.sum_doc_freq(), 0);
        assert_eq!(terms.doc_count(), 0);
        assert!(!terms.has_freqs());
        assert!(!terms.has_positions());
        assert!(!terms.has_offsets());
        assert!(!terms.has_payloads());
    }

    #[test]
    fn empty_fields_has_no_terms() {
        let fields = EmptyFields::new();
        assert_eq!(fields.size(), 0);
        assert!(fields.terms("field").unwrap().is_none());
        assert_eq!(fields.iterator().count(), 0);
    }

    #[test]
    fn empty_terms_enum_postings_is_rejected() {
        let mut it = EmptyTermsEnum::new();
        assert!(
            it.postings(None, POSTINGS_ENUM_FREQS).is_err(),
            "postings should fail on empty enum"
        );
    }
}
