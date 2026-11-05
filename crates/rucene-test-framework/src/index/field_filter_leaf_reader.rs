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

//! Port of `org.apache.lucene.tests.index.FieldFilterLeafReader`: a
//! `FilterLeafReader` that exposes only a subset of fields from the
//! underlying wrapped reader.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::lang::string_hash_code;
use rucene::error::Result;
use rucene::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
use rucene::index::leaf_reader::TermVectors;
use rucene::index::{
    BinaryDocValues, FieldInfo, FieldInfos, Fields, LeafReader, NumericDocValues, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues, StoredFieldVisitor, StoredFieldVisitorStatus,
    Terms,
};

/// The field set and its negation, shared by the reader and the views it
/// hands out (Java's anonymous and inner classes reach them through the
/// enclosing instance).
#[derive(Debug)]
struct FieldFilter {
    /// The set, in the iteration order of the `java.util.HashSet` Java's
    /// caller passes (see [`FieldFilterLeafReader::new`]).
    fields: Vec<String>,
    contains: HashSet<String>,
    negate: bool,
}

impl FieldFilter {
    /// `hasField(field)`: `negate ^ fields.contains(field)`.
    fn has_field(&self, field: &str) -> bool {
        self.negate ^ self.contains.contains(field)
    }
}

/// The iteration order of `new HashSet<>(collection)` holding `fields`, in
/// the collection's order: the table has `tableSizeFor(max((int) (n / .75f) +
/// 1, 16))` buckets (`HashSet(Collection)`), iterated in index order
/// (`(h ^ h >>> 16) & (capacity - 1)`), the keys of a bucket in insertion
/// order. A bucket holding eight or more colliding keys (a tree bin) would
/// order differently.
fn java_hash_set_order(fields: Vec<String>) -> Vec<String> {
    let mut distinct: Vec<String> = Vec::new();
    for field in fields {
        if !distinct.contains(&field) {
            distinct.push(field);
        }
    }
    let initial = std::cmp::max((distinct.len() as f32 / 0.75f32) as i32 + 1, 16);
    let capacity = (initial as u32).next_power_of_two() as usize;
    let mut order: Vec<(usize, usize, String)> = distinct
        .into_iter()
        .enumerate()
        .map(|(rank, field)| {
            let h = string_hash_code(&field);
            let spread = (h ^ ((h as u32) >> 16) as i32) as u32 as usize;
            (spread & (capacity - 1), rank, field)
        })
        .collect();
    order.sort();
    order.into_iter().map(|(_, _, field)| field).collect()
}

/// Port of `org.apache.lucene.tests.index.FieldFilterLeafReader` (`final`,
/// `extends FilterLeafReader`).
///
/// The fields kept are those for which `negate ^ fields.contains(field)`
/// holds. As in Java, the filter applies to the field infos, `terms`, the
/// stored fields (`needsField`), the term vectors, `getBinaryDocValues`,
/// `getSortedDocValues`, `getSortedNumericDocValues`,
/// `getSortedSetDocValues` and `getNormValues`; Java does not override
/// `getNumericDocValues`, `getDocValuesSkipper`, `getPointValues` nor the
/// vector accessors, which therefore still answer for a filtered-out field.
/// The reader has neither a core nor a reader cache helper.
pub struct FieldFilterLeafReader {
    core: IndexReaderCore,
    r#in: Arc<dyn LeafReader>,
    filter: Arc<FieldFilter>,
    field_infos: FieldInfos,
}

impl FieldFilterLeafReader {
    /// `new FieldFilterLeafReader(in, fields, negate)`.
    ///
    /// `fields` is the content of the `Set<String>` Java's caller passes, in
    /// the caller's order; the set is the `java.util.HashSet` built from it
    /// with `new HashSet<>(collection)`, as `LuceneTestCase.wrapReader` (the
    /// one caller in Lucene 10.5.0) builds it, which is what `toString()`
    /// renders in that set's iteration order.
    ///
    /// # Errors
    ///
    /// What `new FieldInfos(filteredInfos)` raises.
    pub fn new(
        r#in: Arc<dyn LeafReader>,
        fields: impl IntoIterator<Item = impl Into<String>>,
        negate: bool,
    ) -> Result<Self> {
        let fields = java_hash_set_order(fields.into_iter().map(Into::into).collect());
        let contains = fields.iter().cloned().collect();
        let filter = Arc::new(FieldFilter {
            fields,
            contains,
            negate,
        });
        let in_infos = r#in.get_field_infos();
        let filtered_infos: Vec<FieldInfo> = in_infos
            .iter()
            .filter(|fi| filter.has_field(&fi.name))
            .cloned()
            .collect();
        let field_infos = FieldInfos::new(filtered_infos)?;
        Ok(FieldFilterLeafReader {
            core: super::filter_leaf_core(&r#in),
            r#in,
            filter,
            field_infos,
        })
    }

    /// `hasField(field)`.
    pub fn has_field(&self, field: &str) -> bool {
        self.filter.has_field(field)
    }

    /// `getDelegate()`: the wrapped reader.
    pub fn get_delegate(&self) -> &Arc<dyn LeafReader> {
        &self.r#in
    }
}

impl fmt::Debug for FieldFilterLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldFilterLeafReader")
            .field("filter", &self.filter)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for FieldFilterLeafReader {
    /// `FieldFilterLeafReader(reader=<in>, fields=[!]<set>)`, the set rendered
    /// by `AbstractCollection.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FieldFilterLeafReader(reader={}, fields=", self.r#in)?;
        if self.filter.negate {
            f.write_str("!")?;
        }
        write!(f, "[{}])", self.filter.fields.join(", "))
    }
}

impl LeafReader for FieldFilterLeafReader {
    forward_leaf_reader!(filter r#in;
        core,
        filter_leaf_reader_delegate,
        num_docs,
        max_doc,
        do_close,
        get_numeric_doc_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_live_docs,
        get_point_values,
        check_integrity,
        get_meta_data,
    );

    fn get_field_infos(&self) -> FieldInfos {
        self.field_infos.clone()
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.core.ensure_open()?;
        Ok(Box::new(FieldFilterTermVectors {
            orig: self.r#in.term_vectors()?,
            filter: Arc::clone(&self.filter),
        }))
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.core.ensure_open()?;
        Ok(Box::new(FieldFilterStoredFields {
            orig: self.r#in.stored_fields()?,
            filter: Arc::clone(&self.filter),
        }))
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        if self.has_field(field) {
            self.core.ensure_open()?;
            self.r#in.terms(field)
        } else {
            Ok(None)
        }
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        if self.has_field(field) {
            self.core.ensure_open()?;
            self.r#in.get_binary_doc_values(field)
        } else {
            Ok(None)
        }
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        if self.has_field(field) {
            self.core.ensure_open()?;
            self.r#in.get_sorted_doc_values(field)
        } else {
            Ok(None)
        }
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        if self.has_field(field) {
            self.core.ensure_open()?;
            self.r#in.get_sorted_numeric_doc_values(field)
        } else {
            Ok(None)
        }
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        if self.has_field(field) {
            self.core.ensure_open()?;
            self.r#in.get_sorted_set_doc_values(field)
        } else {
            Ok(None)
        }
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        if self.has_field(field) {
            self.core.ensure_open()?;
            self.r#in.get_norm_values(field)
        } else {
            Ok(None)
        }
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// The anonymous `TermVectors` of `FieldFilterLeafReader.termVectors()`:
/// filters the fields of each document's vectors, and answers `null` for a
/// document left without any. It does not override `prefetch`, whose default
/// does nothing.
struct FieldFilterTermVectors {
    orig: Box<dyn TermVectors>,
    filter: Arc<FieldFilter>,
}

impl fmt::Debug for FieldFilterTermVectors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldFilterTermVectors")
            .finish_non_exhaustive()
    }
}

impl TermVectors for FieldFilterTermVectors {
    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        let Some(f) = self.orig.get(doc)? else {
            return Ok(None);
        };
        let f = FieldFilterFields {
            r#in: f,
            filter: Arc::clone(&self.filter),
        };
        // we need to check for emptyness, so we can return
        // null:
        let has_next = f.iterator().next().is_some();
        Ok(if has_next { Some(Box::new(f)) } else { None })
    }
}

/// Port of the private inner `FieldFilterLeafReader.FieldFilterFields`
/// (`extends FilterFields`).
struct FieldFilterFields {
    r#in: Box<dyn Fields>,
    filter: Arc<FieldFilter>,
}

impl Fields for FieldFilterFields {
    /// `FilterIterator` over `super.iterator()` keeping the fields
    /// `hasField` accepts.
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(
            self.r#in
                .iterator()
                .filter(move |field| self.filter.has_field(field)),
        )
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        if self.filter.has_field(field) {
            self.r#in.terms(field)
        } else {
            Ok(None)
        }
    }

    /// this information is not cheap, return -1 like MultiFields does:
    fn size(&self) -> i32 {
        -1
    }
}

/// The anonymous `StoredFields` of `FieldFilterLeafReader.storedFields()`.
/// It does not override `prefetch`, whose default does nothing.
struct FieldFilterStoredFields {
    orig: Box<dyn StoredFields>,
    filter: Arc<FieldFilter>,
}

impl fmt::Debug for FieldFilterStoredFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldFilterStoredFields")
            .finish_non_exhaustive()
    }
}

impl StoredFields for FieldFilterStoredFields {
    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        let mut filtering = FieldFilterVisitor {
            visitor,
            filter: &self.filter,
        };
        self.orig.document_with_visitor(doc_id, &mut filtering)
    }
}

/// The anonymous `StoredFieldVisitor` of `FieldFilterLeafReader.storedFields()`:
/// forwards every value, and refuses (`Status.NO`) the fields `hasField`
/// rejects. Java's `Objects.requireNonNull(value, ...)` on a string value has
/// nothing to check here: a `&str` is never null.
struct FieldFilterVisitor<'a> {
    visitor: &'a mut dyn StoredFieldVisitor,
    filter: &'a FieldFilter,
}

impl StoredFieldVisitor for FieldFilterVisitor<'_> {
    fn binary_field(&mut self, field_info: &FieldInfo, value: &[u8]) -> Result<()> {
        self.visitor.binary_field(field_info, value)
    }

    fn string_field(&mut self, field_info: &FieldInfo, value: &str) -> Result<()> {
        self.visitor.string_field(field_info, value)
    }

    fn int_field(&mut self, field_info: &FieldInfo, value: i32) -> Result<()> {
        self.visitor.int_field(field_info, value)
    }

    fn long_field(&mut self, field_info: &FieldInfo, value: i64) -> Result<()> {
        self.visitor.long_field(field_info, value)
    }

    fn float_field(&mut self, field_info: &FieldInfo, value: f32) -> Result<()> {
        self.visitor.float_field(field_info, value)
    }

    fn double_field(&mut self, field_info: &FieldInfo, value: f64) -> Result<()> {
        self.visitor.double_field(field_info, value)
    }

    fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        if self.filter.has_field(&field_info.name) {
            self.visitor.needs_field(field_info)
        } else {
            Ok(StoredFieldVisitorStatus::No)
        }
    }
}
