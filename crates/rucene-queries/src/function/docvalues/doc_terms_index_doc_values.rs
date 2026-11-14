/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `DocTermsIndexDocValues.java`, of the Java package
//! `org.apache.lucene.queries.function.docvalues`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`; its fields are a [`DocTermsIndexState`] the
//! subclass owns. A subclass delegates the members it inherits with
//! `doc_terms_index_doc_values_members!`.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::{DocValues, LeafReaderContext, SortedDocValues};
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::util::mutable::{MutableValue, MutableValueStr};
use rucene::util::refs::CharsRefBuilder;
use rucene::util::{BytesRef, BytesRefBuilder};

use crate::function::function_values::{FunctionValues, ValueFiller};
use crate::function::value_source::ValueSource;
use crate::function::value_source_scorer::ValueSourceScorerMatcher;

/// The fields of a [`DocTermsIndexDocValues`].
///
/// Equivalent to the protected fields `termsIndex`, `val` and `spareChars`,
/// and the private `lastDocID`, of `DocTermsIndexDocValues`.
pub struct DocTermsIndexState {
    /// The protected `termsIndex` field.
    pub terms_index: Box<dyn SortedDocValues>,
    /// The protected `val` field.
    pub val: MutableValueStr,
    /// The protected `spareChars` field.
    pub spare_chars: CharsRefBuilder,
    /// The private `lastDocID` field.
    last_doc_id: i32,
}

impl std::fmt::Debug for DocTermsIndexState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocTermsIndexState")
            .field("last_doc_id", &self.last_doc_id)
            .finish_non_exhaustive()
    }
}

impl DocTermsIndexState {
    /// The fields over the sorted doc values of `field`.
    ///
    /// Equivalent to the state built by
    /// `DocTermsIndexDocValues(ValueSource, LeafReaderContext, String)`, which
    /// delegates to the protected constructor with `open(context, field)`.
    ///
    /// # Errors
    ///
    /// The [`DocTermsIndexException`] of [`open`].
    pub fn open(context: &LeafReaderContext, field: &str) -> Result<Self> {
        Ok(Self::new(open(context, field)?))
    }

    /// The fields over `terms_index`.
    ///
    /// Equivalent to the state built by the protected
    /// `DocTermsIndexDocValues(ValueSource, SortedDocValues)`.
    pub fn new(terms_index: Box<dyn SortedDocValues>) -> Self {
        Self {
            terms_index,
            val: MutableValueStr::new(),
            spare_chars: CharsRefBuilder::new(),
            last_doc_id: 0,
        }
    }
}

/// Serves as base for [`FunctionValues`] instances that are backed by a
/// single-valued sorted doc-values field.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.docvalues.DocTermsIndexDocValues`,
/// whose `toTerm(String)` and `objectVal(int)` are abstract.
///
/// **Divergence.** As for
/// [`FloatDocValues`](crate::function::docvalues::float_doc_values::FloatDocValues):
/// the class is this trait plus the module functions, and its fields are the
/// [`DocTermsIndexState`] the implementor exposes.
pub trait DocTermsIndexDocValues: FunctionValues {
    /// The protected `vs` field.
    fn vs(&self) -> &dyn ValueSource;

    /// The fields of the class.
    fn state(&mut self) -> &mut DocTermsIndexState;

    /// The fields of the class, read-only.
    fn state_ref(&self) -> &DocTermsIndexState;

    /// Converts a readable value into its indexed form.
    ///
    /// Equivalent to the abstract protected
    /// `DocTermsIndexDocValues.toTerm(String)`.
    fn to_term(&self, readable_value: &str) -> String;

    /// Returns the ordinal of `doc`, or `-1` when it has no value.
    ///
    /// Equivalent to the protected `DocTermsIndexDocValues.getOrdForDoc(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when documents are sent out of order, and
    /// any I/O error of the doc values.
    fn get_ord_for_doc(&mut self, doc: i32) -> Result<i32> {
        let state = self.state();
        if doc < state.last_doc_id {
            return Err(LuceneError::illegal_argument(format!(
                "docs were sent out-of-order: lastDocID={} vs docID={doc}",
                state.last_doc_id
            )));
        }
        state.last_doc_id = doc;
        let mut cur_doc_id = state.terms_index.doc_id();
        if doc > cur_doc_id {
            cur_doc_id = state.terms_index.advance(doc)?;
        }
        if doc == cur_doc_id {
            state.terms_index.ord_value()
        } else {
            Ok(-1)
        }
    }
}

/// Equivalent to `DocTermsIndexDocValues.exists(int)`:
/// `getOrdForDoc(doc) >= 0`.
///
/// # Errors
///
/// Propagates the error of `get_ord_for_doc`.
pub fn exists<T: DocTermsIndexDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<bool> {
    Ok(this.get_ord_for_doc(doc)? >= 0)
}

/// Equivalent to `DocTermsIndexDocValues.ordVal(int)`: `getOrdForDoc(doc)`.
///
/// # Errors
///
/// Propagates the error of `get_ord_for_doc`.
pub fn ord_val<T: DocTermsIndexDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<i32> {
    this.get_ord_for_doc(doc)
}

/// Equivalent to `DocTermsIndexDocValues.numOrd()`:
/// `termsIndex.getValueCount()`.
///
/// # Errors
///
/// Propagates any I/O error of the doc values.
pub fn num_ord<T: DocTermsIndexDocValues + ?Sized>(this: &T) -> Result<i32> {
    this.state_ref().terms_index.get_value_count()
}

/// Equivalent to `DocTermsIndexDocValues.bytesVal(int, BytesRefBuilder)`.
///
/// # Errors
///
/// Propagates the errors of the doc values.
pub fn bytes_val<T: DocTermsIndexDocValues + ?Sized>(
    this: &mut T,
    doc: i32,
    target: &mut BytesRefBuilder,
) -> Result<bool> {
    target.clear();
    if this.get_ord_for_doc(doc)? == -1 {
        Ok(false)
    } else {
        let state = this.state();
        let term = state
            .terms_index
            .lookup_ord(state.terms_index.ord_value()?)?;
        target.copy_bytes(term.slice(), 0, term.length);
        Ok(true)
    }
}

/// Equivalent to `DocTermsIndexDocValues.strVal(int)`: the UTF-8 term of the
/// document's ordinal, `null` without one.
///
/// # Errors
///
/// Propagates the errors of the doc values.
pub fn str_val<T: DocTermsIndexDocValues + ?Sized>(
    this: &mut T,
    doc: i32,
) -> Result<Option<String>> {
    if this.get_ord_for_doc(doc)? == -1 {
        return Ok(None);
    }
    let state = this.state();
    let term = state
        .terms_index
        .lookup_ord(state.terms_index.ord_value()?)?;
    state.spare_chars.copy_utf8_bytes_ref(&term);
    Ok(Some(state.spare_chars.to_string()))
}

/// Equivalent to `DocTermsIndexDocValues.boolVal(int)`: `exists(doc)`.
///
/// # Errors
///
/// Propagates the error of `exists`.
pub fn bool_val<T: DocTermsIndexDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<bool> {
    this.exists(doc)
}

/// Equivalent to `DocTermsIndexDocValues.getRangeScorer(LeafReaderContext,
/// String, String, boolean, boolean)`: the bounds converted with
/// [`DocTermsIndexDocValues::to_term`] and looked up in the terms index, and
/// the anonymous `ValueSourceScorer` matching `exists(doc)` with the
/// ordinal, as a `float`, in `[ll, uu]`.
///
/// # Errors
///
/// Propagates the errors of the doc values.
pub fn get_range_matcher<T: DocTermsIndexDocValues + ?Sized>(
    this: &mut T,
    lower_val: Option<&str>,
    upper_val: Option<&str>,
    include_lower: bool,
    include_upper: bool,
) -> Result<Box<dyn ValueSourceScorerMatcher>> {
    // TODO: are lowerVal and upperVal in indexed form or not?
    let lower_val = lower_val.map(|v| this.to_term(v));
    let upper_val = upper_val.map(|v| this.to_term(v));

    let mut lower = i32::MIN;
    if let Some(lower_val) = lower_val {
        lower = this
            .state()
            .terms_index
            .lookup_term(&BytesRef::from_char_sequence(&lower_val))?;
        if lower < 0 {
            lower = -lower - 1;
        } else if !include_lower {
            lower += 1;
        }
    }

    let mut upper = i32::MAX;
    if let Some(upper_val) = upper_val {
        upper = this
            .state()
            .terms_index
            .lookup_term(&BytesRef::from_char_sequence(&upper_val))?;
        if upper < 0 {
            upper = -upper - 2;
        } else if !include_upper {
            upper -= 1;
        }
    }

    Ok(Box::new(OrdRangeMatcher {
        ll: lower,
        uu: upper,
    }))
}

/// The anonymous `ValueSourceScorer` of
/// `DocTermsIndexDocValues.getRangeScorer`; the ordinal is widened to a
/// `float` before the comparison, as Java's `float docVal = ordVal(doc)` does.
#[derive(Debug, Clone, Copy)]
struct OrdRangeMatcher {
    ll: i32,
    uu: i32,
}

impl ValueSourceScorerMatcher for OrdRangeMatcher {
    fn matches(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<bool> {
        if !values.exists(doc)? {
            return Ok(false);
        }
        let doc_val = values.ord_val(doc)? as f32;
        Ok(doc_val >= self.ll as f32 && doc_val <= self.uu as f32)
    }
}

/// Equivalent to `DocTermsIndexDocValues.toString(int)`:
/// `vs.description() + '=' + strVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `str_val`.
pub fn to_doc_string<T: DocTermsIndexDocValues + ?Sized>(this: &mut T, doc: i32) -> Result<String> {
    let value = this.str_val(doc)?;
    Ok(format!(
        "{}={}",
        this.vs().description(),
        value.as_deref().unwrap_or("null")
    ))
}

/// Equivalent to `DocTermsIndexDocValues.getValueFiller()`.
pub fn get_value_filler<T: DocTermsIndexDocValues + ?Sized>(_this: &T) -> Box<dyn ValueFiller> {
    Box::new(DocTermsIndexValuesFiller {
        mval: MutableValueStr::new(),
    })
}

/// The anonymous `ValueFiller` of `DocTermsIndexDocValues.getValueFiller`.
///
/// **Divergence.** Java's filler reads `getOrdForDoc(doc)` and copies
/// `termsIndex.lookupOrd(ord)`, through its outer instance. This filler reads
/// the same two things through the values it is handed (see
/// [`ValueFiller`]): `ordVal(doc)`, which is `getOrdForDoc(doc)`, and
/// `bytesVal(doc, value)`, which copies the term of that same ordinal.
struct DocTermsIndexValuesFiller {
    mval: MutableValueStr,
}

impl ValueFiller for DocTermsIndexValuesFiller {
    fn get_value(&self) -> &dyn MutableValue {
        &self.mval
    }

    fn fill_value(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<()> {
        let ord = values.ord_val(doc)?;
        self.mval.value.clear();
        self.mval.exists = ord >= 0;
        if self.mval.exists {
            values.bytes_val(doc, &mut self.mval.value)?;
        }
        Ok(())
    }
}

/// Opens the sorted doc values of `field`.
///
/// Equivalent to the package-private static
/// `DocTermsIndexDocValues.open(LeafReaderContext, String)`, which wraps a
/// `RuntimeException` of `DocValues.getSorted` in a
/// [`DocTermsIndexException`].
///
/// # Errors
///
/// The [`DocTermsIndexException`], and any I/O error unchanged.
pub fn open(context: &LeafReaderContext, field: &str) -> Result<Box<dyn SortedDocValues>> {
    DocValues::get_sorted(context.leaf_reader().as_ref(), field).map_err(|e| {
        // `catch (RuntimeException e)`: a checked `IOException` or an `Error`
        // passes through unchanged.
        if e.is_io_exception() || matches!(e.kind(), LuceneErrorKind::AssertionError(_)) {
            e
        } else {
            DocTermsIndexException::new(field, e)
        }
    })
}

/// The exception [`open`] raises when the terms index cannot be initialised.
///
/// Equivalent to the static final class
/// `org.apache.lucene.queries.function.docvalues.DocTermsIndexDocValues.DocTermsIndexException`,
/// a `RuntimeException`.
#[derive(Debug, Clone, Copy)]
pub struct DocTermsIndexException;

impl DocTermsIndexException {
    /// The binary name of the Java class.
    pub const CLASS: &'static str =
        "org.apache.lucene.queries.function.docvalues.DocTermsIndexDocValues$DocTermsIndexException";

    /// Builds the exception.
    ///
    /// Equivalent to `DocTermsIndexException(String, RuntimeException)`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(field_name: &str, cause: LuceneError) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Unchecked {
            class: Self::CLASS,
            message: Some(format!(
                "Can't initialize DocTermsIndex to generate (function) FunctionValues for field: {field_name}"
            )),
            cause: Some(Arc::new(cause)),
        })
    }
}

/// Delegates, inside an `impl FunctionValues` block, each listed member to
/// the `DocTermsIndexDocValues` member of the same name.
macro_rules! doc_terms_index_doc_values_members {
    ($($member:ident),* $(,)?) => {
        $( $crate::function::docvalues::doc_terms_index_doc_values::doc_terms_index_doc_values_members!(@ $member); )*
    };
    (@ exists) => {
        fn exists(&mut self, doc: i32) -> ::rucene::error::Result<bool> {
            $crate::function::docvalues::doc_terms_index_doc_values::exists(self, doc)
        }
    };
    (@ ord_val) => {
        fn ord_val(&mut self, doc: i32) -> ::rucene::error::Result<i32> {
            $crate::function::docvalues::doc_terms_index_doc_values::ord_val(self, doc)
        }
    };
    (@ num_ord) => {
        fn num_ord(&self) -> ::rucene::error::Result<i32> {
            $crate::function::docvalues::doc_terms_index_doc_values::num_ord(self)
        }
    };
    (@ bytes_val) => {
        fn bytes_val(
            &mut self,
            doc: i32,
            target: &mut ::rucene::util::BytesRefBuilder,
        ) -> ::rucene::error::Result<bool> {
            $crate::function::docvalues::doc_terms_index_doc_values::bytes_val(self, doc, target)
        }
    };
    (@ str_val) => {
        fn str_val(&mut self, doc: i32) -> ::rucene::error::Result<Option<String>> {
            $crate::function::docvalues::doc_terms_index_doc_values::str_val(self, doc)
        }
    };
    (@ bool_val) => {
        fn bool_val(&mut self, doc: i32) -> ::rucene::error::Result<bool> {
            $crate::function::docvalues::doc_terms_index_doc_values::bool_val(self, doc)
        }
    };
    (@ get_range_matcher) => {
        fn get_range_matcher(
            &mut self,
            lower_val: Option<&str>,
            upper_val: Option<&str>,
            include_lower: bool,
            include_upper: bool,
        ) -> ::rucene::error::Result<
            Box<dyn $crate::function::value_source_scorer::ValueSourceScorerMatcher>,
        > {
            $crate::function::docvalues::doc_terms_index_doc_values::get_range_matcher(
                self,
                lower_val,
                upper_val,
                include_lower,
                include_upper,
            )
        }
    };
    (@ to_doc_string) => {
        fn to_doc_string(&mut self, doc: i32) -> ::rucene::error::Result<String> {
            $crate::function::docvalues::doc_terms_index_doc_values::to_doc_string(self, doc)
        }
    };
    (@ get_value_filler) => {
        fn get_value_filler(&self) -> Box<dyn $crate::function::function_values::ValueFiller> {
            $crate::function::docvalues::doc_terms_index_doc_values::get_value_filler(self)
        }
    };
}
pub(crate) use doc_terms_index_doc_values_members;
