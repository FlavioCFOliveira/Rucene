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

//! Port of `VectorFieldFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`.

use rucene::error::{LuceneError, Result};
use rucene::index::{LeafReader, VectorEncoding};
use rucene::search::doc_id_set_iterator::DocIdSetIterator;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::ValueSource;

/// An implementation for retrieving [`FunctionValues`] instances for kNN
/// vector fields.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.VectorFieldFunction`. Its
/// members are [`to_doc_string`], [`exists`] and `check_field`.
pub trait VectorFieldFunction: FunctionValues {
    /// The protected `valueSource` field.
    fn value_source(&self) -> &dyn ValueSource;

    /// The package-private `lastDocID` field.
    fn last_doc_id(&mut self) -> &mut i32;

    /// Returns the iterator over the documents that have a vector.
    ///
    /// Equivalent to the abstract protected
    /// `VectorFieldFunction.getVectorIterator()`.
    fn get_vector_iterator(&mut self) -> &mut dyn DocIdSetIterator;
}

/// Equivalent to `VectorFieldFunction.toString(int)`:
/// `valueSource.description() + strVal(doc)`.
///
/// # Errors
///
/// Propagates the error of `str_val`, which the kNN values do not override.
pub fn to_doc_string<T: VectorFieldFunction + ?Sized>(this: &mut T, doc: i32) -> Result<String> {
    let value = this.str_val(doc)?;
    Ok(format!(
        "{}{}",
        this.value_source().description(),
        value.as_deref().unwrap_or("null")
    ))
}

/// Equivalent to `VectorFieldFunction.exists(int)`.
///
/// # Errors
///
/// `IllegalArgumentException` when documents are sent out of order, and any
/// I/O error of the iterator.
pub fn exists<T: VectorFieldFunction + ?Sized>(this: &mut T, doc: i32) -> Result<bool> {
    let last_doc_id = *this.last_doc_id();
    if doc < last_doc_id {
        return Err(LuceneError::illegal_argument(format!(
            "docs were sent out-of-order: lastDocID={last_doc_id} vs docID={doc}"
        )));
    }
    *this.last_doc_id() = doc;
    let mut cur_doc_id = this.get_vector_iterator().doc_id();
    if doc > cur_doc_id {
        cur_doc_id = this.get_vector_iterator().advance(doc)?;
    }
    Ok(doc == cur_doc_id)
}

/// Checks the vector encoding of a field the reader has no vectors for.
///
/// Equivalent to the package-private static
/// `VectorFieldFunction.checkField(LeafReader, String, VectorEncoding)`: a
/// field whose vectors are not of the expected encoding is an
/// `IllegalStateException`; the encodings render as their Java names.
///
/// # Errors
///
/// The `IllegalStateException` described above.
pub(crate) fn check_field(
    reader: &dyn LeafReader,
    field: &str,
    expected_encoding: VectorEncoding,
) -> Result<()> {
    let field_infos = reader.get_field_infos();
    if let Some(fi) = field_infos.field_info(field) {
        let actual = fi.has_vector_values().then(|| fi.get_vector_encoding());
        if Some(expected_encoding) != actual {
            let actual = actual.map_or_else(|| "null".to_string(), |a| format!("{a:?}"));
            return Err(LuceneError::illegal_state(format!(
                "Unexpected vector encoding ({actual}) for field {field}(expected={expected_encoding:?})"
            )));
        }
    }
    Ok(())
}
