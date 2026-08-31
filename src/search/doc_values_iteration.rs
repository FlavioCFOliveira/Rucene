//! Doc-values iterators viewed as plain [`DocIdSetIterator`]s.
//!
//! In Java every doc-values iterator *is* a `DocIdSetIterator`, so an instance
//! can be handed to any API that takes one. Rust before 1.86 cannot coerce
//! `Box<dyn NumericDocValues>` — or any of its siblings — into
//! `Box<dyn DocIdSetIterator>`, and this crate's minimum supported Rust version
//! is 1.80, so the upcast is spelled out as one delegating wrapper per
//! doc-values kind. Each wrapper forwards every [`DocIdSetIterator`] method to
//! the value it holds, so the iteration is the same object's.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::{
    BinaryDocValues, NumericDocValues, SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::util::FixedBitSet;

macro_rules! doc_values_as_iterator {
    ($name:ident, $trait:ident, $ctor:ident, $doc:literal) => {
        #[doc = $doc]
        pub(crate) struct $name {
            inner: Box<dyn $trait>,
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("doc", &self.inner.doc_id())
                    .finish_non_exhaustive()
            }
        }

        impl DocIdSetIterator for $name {
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

        impl $name {
            /// Wraps the given doc values.
            pub(crate) fn new(inner: Box<dyn $trait>) -> Self {
                Self { inner }
            }

            /// Returns the wrapped doc values, so that a caller can read the
            /// value the iteration is positioned on.
            #[allow(dead_code)]
            pub(crate) fn values(&mut self) -> &mut dyn $trait {
                &mut *self.inner
            }
        }

        /// Views the given doc values as a [`DocIdSetIterator`].
        pub(crate) fn $ctor(inner: Box<dyn $trait>) -> Box<dyn DocIdSetIterator> {
            Box::new($name::new(inner))
        }
    };
}

doc_values_as_iterator!(
    NumericDocValuesIterator,
    NumericDocValues,
    numeric_as_iterator,
    "Views a [`NumericDocValues`] as a [`DocIdSetIterator`]."
);
doc_values_as_iterator!(
    BinaryDocValuesIterator,
    BinaryDocValues,
    binary_as_iterator,
    "Views a [`BinaryDocValues`] as a [`DocIdSetIterator`]."
);
doc_values_as_iterator!(
    SortedDocValuesIterator,
    SortedDocValues,
    sorted_as_iterator,
    "Views a [`SortedDocValues`] as a [`DocIdSetIterator`]."
);
doc_values_as_iterator!(
    SortedNumericDocValuesIterator,
    SortedNumericDocValues,
    sorted_numeric_as_iterator,
    "Views a [`SortedNumericDocValues`] as a [`DocIdSetIterator`]."
);
doc_values_as_iterator!(
    SortedSetDocValuesIterator,
    SortedSetDocValues,
    sorted_set_as_iterator,
    "Views a [`SortedSetDocValues`] as a [`DocIdSetIterator`]."
);
