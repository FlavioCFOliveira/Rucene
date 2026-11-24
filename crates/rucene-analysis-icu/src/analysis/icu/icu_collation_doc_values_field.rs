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
//! from `org.apache.lucene.analysis.icu.ICUCollationDocValuesField`.

use std::fmt;
use std::sync::{Mutex, PoisonError};

use icu4j::text::collator::Collator;
use icu4j::text::raw_collation_key::RawCollationKey;
use rucene::analysis::{Analyzer, TokenStream};
use rucene::document::{
    Field, InvertableType, NumericValue, SharedReader, SortedDocValuesField, StoredValue,
};
use rucene::error::Result;
use rucene::index::{IndexableField, IndexableFieldType};
use rucene::util::{BytesRef, IOUtils};

/// Indexes collation keys as a single-valued `SortedDocValuesField`.
///
/// This is more efficient that `ICUCollationKeyAnalyzer` if the field only
/// has one value: no uninversion is necessary to sort on the field,
/// locale-sensitive range queries can still work via `DocValuesRangeQuery`,
/// and the underlying data structures built at index-time are likely more
/// efficient and use less memory than FieldCache.
///
/// Equivalent to `org.apache.lucene.analysis.icu.ICUCollationDocValuesField`,
/// a final `Field`: the base is the [`Field`] this field holds, built over
/// `SortedDocValuesField.TYPE` with an empty binary value, as Java's
/// `fieldsData = bytes` sets it.
///
/// **Divergence from Lucene 10.5.0 (inherited).** Java points its `bytes`
/// (the field's value) at the key's array (`bytes.bytes = key.bytes`,
/// `bytes.length = key.size`); here the value is a new `BytesRef` holding
/// the key's first `size` bytes, as `CollationDocValuesField` of
/// `rucene-analysis-common` sets its value. The bytes indexed are the same.
pub struct ICUCollationDocValuesField {
    base: Field,
    name: String,
    collator: Box<dyn Collator>,
    key: Mutex<RawCollationKey>,
}

impl fmt::Debug for ICUCollationDocValuesField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ICUCollationDocValuesField")
            .field("base", &self.base)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl ICUCollationDocValuesField {
    /// Create a new ICUCollationDocValuesField.
    ///
    /// NOTE: you should not create a new one for each document, instead just
    /// make one and reuse it during your indexing process, setting the value
    /// via [`set_string_value`](Self::set_string_value).
    ///
    /// * `name`: field name
    /// * `collator`: Collator for generating collation keys.
    ///
    /// Equivalent to `ICUCollationDocValuesField(String, Collator)`:
    /// `super(name, SortedDocValuesField.TYPE)`, `(Collator)
    /// collator.clone()`, and `fieldsData = bytes` (an empty `BytesRef`) "so
    /// wrong setters cannot be called".
    ///
    /// # Errors
    ///
    /// None for this field type: [`Field::new_with_bytes`] stands for Java's
    /// protected `Field(String, IndexableFieldType)` and the assignment of
    /// `fieldsData`, and its checks all pass for `SortedDocValuesField.TYPE`.
    ///
    /// # Panics
    ///
    /// With the `RuntimeException` Java wraps a `CloneNotSupportedException`
    /// of the collator in.
    // TODO: can we make this trap-free? maybe just synchronize on the collator
    // instead?
    pub fn new(name: &str, collator: &dyn Collator) -> Result<Self> {
        let collator = collator.clone_box().unwrap_or_else(|e| {
            IOUtils::rethrow_always(rucene::error::LuceneError::runtime_exception(e))
        });
        Ok(Self {
            base: Field::new_with_bytes(
                name.to_string(),
                BytesRef::default(),
                SortedDocValuesField::type_shared(),
            )?,
            name: name.to_string(),
            collator,
            key: Mutex::new(RawCollationKey::new()),
        })
    }

    /// Equivalent to `ICUCollationDocValuesField.setStringValue(String)`:
    /// the value becomes the bytes of `collator.getRawCollationKey(value,
    /// key)`, offset 0, length `key.size`.
    ///
    /// # Errors
    ///
    /// None: the base holds a binary value, which a binary value replaces.
    pub fn set_string_value(&self, value: &str) -> Result<()> {
        let units: Vec<u16> = value.encode_utf16().collect();
        let mut key = self.key.lock().unwrap_or_else(PoisonError::into_inner);
        let filled = self
            .collator
            .get_raw_collation_key(Some(&units), Some(std::mem::take(&mut *key)))
            .expect("INVARIANT: a non-null source yields the key");
        let size = filled.size.max(0) as usize;
        let bytes: Vec<u8> = filled
            .bytes
            .as_deref()
            .map(|b| b[..size].iter().map(|&x| x as u8).collect())
            .unwrap_or_default();
        *key = filled;
        drop(key);
        self.base.set_bytes_value(BytesRef::new(bytes))
    }

    /// The members this field inherits from `Field`.
    pub fn base(&self) -> &Field {
        &self.base
    }
}

impl fmt::Display for ICUCollationDocValuesField {
    /// The inherited `Field.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.base, f)
    }
}

impl IndexableField for ICUCollationDocValuesField {
    /// Equivalent to `ICUCollationDocValuesField.name()`.
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
