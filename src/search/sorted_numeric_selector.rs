//! Multi-valued numeric reduction, ported from
//! `org.apache.lucene.search.SortedNumericSelector`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::index::{DocValuesIterator, NumericDocValues, SortedNumericDocValues};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::sort::SortFieldType;
use crate::util::{FixedBitSet, NumericUtils};

/// The type of selection to perform.
///
/// Equivalent to the enum `org.apache.lucene.search.SortedNumericSelector.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum SortedNumericSelectorType {
    /// Selects the minimum value in the set.
    MIN,
    /// Selects the maximum value in the set.
    MAX,
}

impl SortedNumericSelectorType {
    /// The declaration order of the variants, which is what
    /// `SortedNumericSortField` serialises.
    ///
    /// Equivalent to `Enum.ordinal()`.
    pub fn ordinal(self) -> i32 {
        match self {
            SortedNumericSelectorType::MIN => 0,
            SortedNumericSelectorType::MAX => 1,
        }
    }

    /// Recovers a selector from its declaration order.
    ///
    /// Equivalent to `SortedNumericSelector.Type.values()[ordinal]`.
    pub fn from_ordinal(ordinal: i32) -> Option<Self> {
        match ordinal {
            0 => Some(SortedNumericSelectorType::MIN),
            1 => Some(SortedNumericSelectorType::MAX),
            _ => None,
        }
    }
}

impl std::fmt::Display for SortedNumericSelectorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SortedNumericSelectorType::MIN => f.write_str("MIN"),
            SortedNumericSelectorType::MAX => f.write_str("MAX"),
        }
    }
}

/// Selects a value from a document's list to use as the representative value.
///
/// Equivalent to `org.apache.lucene.search.SortedNumericSelector`. It provides
/// a [`NumericDocValues`] view over a [`SortedNumericDocValues`], for use with
/// sorting, expressions and function queries.
#[derive(Debug, Clone, Copy)]
pub struct SortedNumericSelector;

impl SortedNumericSelector {
    /// Wraps a multi-valued [`SortedNumericDocValues`] as a single-valued view,
    /// using the given selector and numeric type.
    ///
    /// Equivalent to
    /// `SortedNumericSelector.wrap(SortedNumericDocValues, Type, SortField.Type)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java first calls
    /// `DocValues.unwrapSingleton(sortedNumeric)` and, when the field is
    /// single-valued in practice, sorts on the underlying single-valued doc
    /// values directly. [`DocValues::unwrap_singleton_numeric`](crate::index::DocValues::unwrap_singleton_numeric)
    /// hands out a borrow of the wrapped values rather than the values
    /// themselves, so this port cannot take ownership of them and always
    /// installs the selecting view. The selectors agree with the singleton on a
    /// single-valued document — `MIN` reads the one value, and `MAX` loops over
    /// the one value — so only the cost of one extra delegation differs, never
    /// the selected value.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `numeric_type` is not one
    /// of [`SortFieldType::Int`], [`SortFieldType::Long`],
    /// [`SortFieldType::Float`] or [`SortFieldType::Double`].
    pub fn wrap(
        sorted_numeric: Box<dyn SortedNumericDocValues>,
        selector: SortedNumericSelectorType,
        numeric_type: SortFieldType,
    ) -> Result<Box<dyn NumericDocValues>> {
        if numeric_type != SortFieldType::Int
            && numeric_type != SortFieldType::Long
            && numeric_type != SortFieldType::Float
            && numeric_type != SortFieldType::Double
        {
            return Err(LuceneError::IllegalArgument(
                "numericType must be a numeric type".to_string(),
            ));
        }
        let view: Box<dyn NumericDocValues> = match selector {
            SortedNumericSelectorType::MIN => Box::new(MinValue::new(sorted_numeric)),
            SortedNumericSelectorType::MAX => Box::new(MaxValue::new(sorted_numeric)),
        };
        // Undo the NumericUtils sortability.
        Ok(match numeric_type {
            SortFieldType::Float => Box::new(SortableFloatView { inner: view }),
            SortFieldType::Double => Box::new(SortableDoubleView { inner: view }),
            _ => view,
        })
    }
}

/// Wraps a [`SortedNumericDocValues`] and returns the first value (the
/// minimum).
///
/// Equivalent to the package-private `SortedNumericSelector.MinValue`.
struct MinValue {
    inner: Box<dyn SortedNumericDocValues>,
    value: i64,
}

impl MinValue {
    fn new(inner: Box<dyn SortedNumericDocValues>) -> Self {
        Self { inner, value: 0 }
    }
}

impl DocIdSetIterator for MinValue {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc_id = self.inner.next_doc()?;
        if doc_id != NO_MORE_DOCS {
            self.value = self.inner.next_value()?;
        }
        Ok(doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc_id = self.inner.advance(target)?;
        if doc_id != NO_MORE_DOCS {
            self.value = self.inner.next_value()?;
        }
        Ok(doc_id)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for MinValue {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if self.inner.advance_exact(target)? {
            self.value = self.inner.next_value()?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl NumericDocValues for MinValue {
    fn long_value(&self) -> Result<i64> {
        Ok(self.value)
    }
}

/// Wraps a [`SortedNumericDocValues`] and returns the last value (the
/// maximum).
///
/// Equivalent to the package-private `SortedNumericSelector.MaxValue`.
struct MaxValue {
    inner: Box<dyn SortedNumericDocValues>,
    value: i64,
}

impl MaxValue {
    fn new(inner: Box<dyn SortedNumericDocValues>) -> Self {
        Self { inner, value: 0 }
    }

    /// Equivalent to the private `MaxValue.setValue()`.
    fn set_value(&mut self) -> Result<()> {
        let count = self.inner.doc_value_count()?;
        for _ in 0..count {
            self.value = self.inner.next_value()?;
        }
        Ok(())
    }
}

impl DocIdSetIterator for MaxValue {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc_id = self.inner.next_doc()?;
        if doc_id != NO_MORE_DOCS {
            self.set_value()?;
        }
        Ok(doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc_id = self.inner.advance(target)?;
        if doc_id != NO_MORE_DOCS {
            self.set_value()?;
        }
        Ok(doc_id)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for MaxValue {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if self.inner.advance_exact(target)? {
            self.set_value()?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl NumericDocValues for MaxValue {
    fn long_value(&self) -> Result<i64> {
        Ok(self.value)
    }
}

/// Undoes the sortable-int encoding of a `float` field.
///
/// Equivalent to the anonymous `FilterNumericDocValues` that
/// `SortedNumericSelector.wrap` returns for
/// [`SortFieldType::Float`](crate::search::SortFieldType::Float).
struct SortableFloatView {
    inner: Box<dyn NumericDocValues>,
}

/// Undoes the sortable-long encoding of a `double` field.
///
/// Equivalent to the anonymous `FilterNumericDocValues` that
/// `SortedNumericSelector.wrap` returns for
/// [`SortFieldType::Double`](crate::search::SortFieldType::Double).
struct SortableDoubleView {
    inner: Box<dyn NumericDocValues>,
}

macro_rules! delegate_numeric_view {
    ($ty:ty, $decode:expr) => {
        impl DocIdSetIterator for $ty {
            fn doc_id(&self) -> i32 {
                self.inner.doc_id()
            }

            fn next_doc(&mut self) -> Result<i32> {
                self.inner.next_doc()
            }

            fn advance(&mut self, target: i32) -> Result<i32> {
                self.inner.advance(target)
            }

            fn cost(&self) -> i64 {
                self.inner.cost()
            }

            fn into_bit_set(
                &mut self,
                up_to: i32,
                bit_set: &mut FixedBitSet,
                offset: i32,
            ) -> Result<()> {
                self.inner.into_bit_set(up_to, bit_set, offset)
            }

            fn doc_id_run_end(&self) -> Result<i32> {
                self.inner.doc_id_run_end()
            }
        }

        impl DocValuesIterator for $ty {
            fn advance_exact(&mut self, target: i32) -> Result<bool> {
                self.inner.advance_exact(target)
            }
        }

        impl NumericDocValues for $ty {
            fn long_value(&self) -> Result<i64> {
                let raw = self.inner.long_value()?;
                Ok($decode(raw))
            }
        }
    };
}

delegate_numeric_view!(SortableFloatView, |raw: i64| i64::from(
    NumericUtils::sortable_float_bits(raw as i32)
));
delegate_numeric_view!(SortableDoubleView, |raw: i64| {
    NumericUtils::sortable_double_bits(raw)
});
