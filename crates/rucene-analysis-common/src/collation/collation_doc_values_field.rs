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

//! Indexes collation keys as a single-valued `SortedDocValuesField`, ported
//! from `org.apache.lucene.collation.CollationDocValuesField`.

use std::fmt;

use rucene::analysis::{Analyzer, TokenStream};
use rucene::document::{
    Field, InvertableType, NumericValue, SharedReader, SortedDocValuesField, StoredValue,
};
use rucene::error::Result;
use rucene::index::{IndexableField, IndexableFieldType};
use rucene::util::BytesRef;

use crate::jdk::collator::RuleBasedCollator;

/// Indexes collation keys as a single-valued `SortedDocValuesField`.
///
/// This is more efficient that `CollationKeyAnalyzer` if the field only has
/// one value: no uninversion is necessary to sort on the field,
/// locale-sensitive range queries can still work via `DocValuesRangeQuery`,
/// and the underlying data structures built at index-time are likely more
/// efficient and use less memory than FieldCache.
///
/// The keys it writes to the index are those of JDK 25's collator (the
/// emulation [`RuleBasedCollator`]), byte for byte.
///
/// Equivalent to `org.apache.lucene.collation.CollationDocValuesField`, which
/// extends `Field`: the base is the [`Field`] this field holds, built over
/// `SortedDocValuesField.TYPE` with an empty binary value, as Java's
/// `fieldsData = bytes` sets it.
#[derive(Debug)]
pub struct CollationDocValuesField {
    base: Field,
    name: String,
    collator: RuleBasedCollator,
}

impl CollationDocValuesField {
    /// Create a new ICUCollationDocValuesField.
    ///
    /// NOTE: you should not create a new one for each document, instead just
    /// make one and reuse it during your indexing process, setting the value
    /// via [`set_string_value`](Self::set_string_value).
    ///
    /// * `name`: field name
    /// * `collator`: Collator for generating collation keys.
    ///
    /// Equivalent to `CollationDocValuesField(String, Collator)`: `super(name,
    /// SortedDocValuesField.TYPE)`, `(Collator) collator.clone()`, and
    /// `fieldsData = bytes` (an empty `BytesRef`) "so wrong setters cannot be
    /// called".
    ///
    /// # Errors
    ///
    /// None for this field type: [`Field::new_with_bytes`] stands for Java's
    /// protected `Field(String, IndexableFieldType)` and the assignment of
    /// `fieldsData`, and its checks all pass for `SortedDocValuesField.TYPE`.
    // TODO: can we make this trap-free? maybe just synchronize on the collator
    // instead?
    pub fn new(name: &str, collator: &RuleBasedCollator) -> Result<Self> {
        Ok(Self {
            base: Field::new_with_bytes(
                name,
                BytesRef::default(),
                SortedDocValuesField::type_shared(),
            )?,
            name: name.to_string(),
            collator: collator.clone(),
        })
    }

    /// Equivalent to `CollationDocValuesField.setStringValue(String)`: the
    /// value becomes `collator.getCollationKey(value).toByteArray()`.
    ///
    /// # Errors
    ///
    /// None: the base holds a binary value, which a binary value replaces.
    pub fn set_string_value(&self, value: &str) -> Result<()> {
        let bytes = self.collator.get_collation_key(value).to_byte_array();
        self.base.set_bytes_value(BytesRef::new(bytes))
    }

    /// The members this field inherits from `Field`.
    pub fn base(&self) -> &Field {
        &self.base
    }
}

impl fmt::Display for CollationDocValuesField {
    /// The inherited `Field.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.base, f)
    }
}

impl IndexableField for CollationDocValuesField {
    /// Equivalent to `CollationDocValuesField.name()`.
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.base.field_type()
    }

    fn token_stream(
        &self,
        analyzer: Option<&dyn Analyzer>,
        reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        IndexableField::token_stream(&self.base, analyzer, reuse)
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.base.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.base.string_value()
    }

    fn reader_value(&self) -> Option<SharedReader> {
        self.base.reader_value()
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        self.base.numeric_value()
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        IndexableField::stored_value(&self.base)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        IndexableField::invertable_type(&self.base)
    }

    /// This field as `Any`, for the `instanceof` tests and casts Java makes on
    /// a field it holds.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}
