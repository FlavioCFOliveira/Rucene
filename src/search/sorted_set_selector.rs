//! Multi-valued term reduction, ported from
//! `org.apache.lucene.search.SortedSetSelector`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::index::{DocValuesIterator, SortedDocValues, SortedSetDocValues};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::{BytesRef, FixedBitSet};

/// The type of selection to perform.
///
/// Equivalent to the enum `org.apache.lucene.search.SortedSetSelector.Type`.
///
/// Limitations:
///
/// * fields containing [`i32::MAX`] or more unique values are unsupported;
/// * selectors other than [`MIN`](SortedSetSelectorType::MIN) require optional
///   codec support, which several codecs provided by Lucene — including the
///   current default codec — do offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum SortedSetSelectorType {
    /// Selects the minimum value in the set.
    MIN,
    /// Selects the maximum value in the set.
    MAX,
    /// Selects the middle value in the set; if the set has an even number of
    /// values, the lower of the middle two is chosen.
    MIDDLE_MIN,
    /// Selects the middle value in the set; if the set has an even number of
    /// values, the higher of the middle two is chosen.
    MIDDLE_MAX,
}

impl SortedSetSelectorType {
    /// The declaration order of the variants, which is what
    /// `SortedSetSortField` serialises.
    ///
    /// Equivalent to `Enum.ordinal()`.
    pub fn ordinal(self) -> i32 {
        match self {
            SortedSetSelectorType::MIN => 0,
            SortedSetSelectorType::MAX => 1,
            SortedSetSelectorType::MIDDLE_MIN => 2,
            SortedSetSelectorType::MIDDLE_MAX => 3,
        }
    }

    /// Recovers a selector from its declaration order.
    ///
    /// Equivalent to `SortedSetSelector.Type.values()[ordinal]`.
    pub fn from_ordinal(ordinal: i32) -> Option<Self> {
        match ordinal {
            0 => Some(SortedSetSelectorType::MIN),
            1 => Some(SortedSetSelectorType::MAX),
            2 => Some(SortedSetSelectorType::MIDDLE_MIN),
            3 => Some(SortedSetSelectorType::MIDDLE_MAX),
            _ => None,
        }
    }
}

impl std::fmt::Display for SortedSetSelectorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SortedSetSelectorType::MIN => f.write_str("MIN"),
            SortedSetSelectorType::MAX => f.write_str("MAX"),
            SortedSetSelectorType::MIDDLE_MIN => f.write_str("MIDDLE_MIN"),
            SortedSetSelectorType::MIDDLE_MAX => f.write_str("MIDDLE_MAX"),
        }
    }
}

/// Selects a value from a document's set to use as the representative value.
///
/// Equivalent to `org.apache.lucene.search.SortedSetSelector`.
#[derive(Debug, Clone, Copy)]
pub struct SortedSetSelector;

impl SortedSetSelector {
    /// Wraps a multi-valued [`SortedSetDocValues`] as a single-valued view,
    /// using the given selector.
    ///
    /// Equivalent to `SortedSetSelector.wrap(SortedSetDocValues, Type)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java first calls
    /// `DocValues.unwrapSingleton(sortedSet)` and, when the field is
    /// single-valued in practice, sorts on the underlying single-valued doc
    /// values directly. [`DocValues::unwrap_singleton_sorted`](crate::index::DocValues::unwrap_singleton_sorted)
    /// hands out a borrow of the wrapped values rather than the values
    /// themselves, so this port cannot take ownership of them and always
    /// installs the selecting view. On a document with a single ordinal every
    /// one of the four selectors reads that ordinal — `MIN` and `MIDDLE_MIN`
    /// take the first, `MAX` and `MIDDLE_MAX` land on it too because the loop
    /// bound is derived from a value count of one — so only the cost of one
    /// extra delegation differs, never the selected ordinal.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] for a field containing
    /// [`i32::MAX`] or more unique terms, and propagates any I/O error raised
    /// while reading the value count.
    pub fn wrap(
        sorted_set: Box<dyn SortedSetDocValues>,
        selector: SortedSetSelectorType,
    ) -> Result<Box<dyn SortedDocValues>> {
        if sorted_set.get_value_count()? >= i64::from(i32::MAX) {
            return Err(LuceneError::UnsupportedOperation(format!(
                "fields containing more than {} unique terms are unsupported",
                i32::MAX - 1
            )));
        }
        Ok(Box::new(SelectedValue::new(sorted_set, selector)))
    }
}

/// Wraps a [`SortedSetDocValues`] and exposes the ordinal the selector picks.
///
/// Equivalent to the four package-private classes
/// `SortedSetSelector.MinValue`, `MaxValue`, `MiddleMinValue` and
/// `MiddleMaxValue`, which differ only in how many ordinals they consume before
/// keeping one; that count is the only difference and is computed here from the
/// selector.
struct SelectedValue {
    inner: Box<dyn SortedSetDocValues>,
    selector: SortedSetSelectorType,
    ord: i32,
}

impl SelectedValue {
    fn new(inner: Box<dyn SortedSetDocValues>, selector: SortedSetSelectorType) -> Self {
        Self {
            inner,
            selector,
            ord: 0,
        }
    }

    /// Equivalent to the private `setOrd()` of each of the four classes: skip
    /// the ordinals before the selected one, then keep the next.
    fn set_ord(&mut self) -> Result<()> {
        if self.doc_id() != NO_MORE_DOCS {
            let doc_value_count = self.inner.doc_value_count()?;
            let target_idx = match self.selector {
                SortedSetSelectorType::MIN => 0,
                SortedSetSelectorType::MAX => doc_value_count - 1,
                SortedSetSelectorType::MIDDLE_MIN => (doc_value_count - 1) >> 1,
                SortedSetSelectorType::MIDDLE_MAX => doc_value_count >> 1,
            };
            for _ in 0..target_idx {
                self.inner.next_ord()?;
            }
            self.ord = self.inner.next_ord()? as i32;
        }
        Ok(())
    }
}

impl DocIdSetIterator for SelectedValue {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()?;
        self.set_ord()?;
        Ok(self.doc_id())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)?;
        self.set_ord()?;
        Ok(self.doc_id())
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.into_bit_set(up_to, bit_set, offset)
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        self.inner.doc_id_run_end()
    }
}

impl DocValuesIterator for SelectedValue {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if self.inner.advance_exact(target)? {
            self.set_ord()?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl SortedDocValues for SelectedValue {
    fn ord_value(&self) -> Result<i32> {
        Ok(self.ord)
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(self.inner.get_value_count()? as i32)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        self.inner.lookup_ord(i64::from(ord))
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i32> {
        Ok(self.inner.lookup_term(key)? as i32)
    }
}
