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

//! Port of `org.apache.lucene.tests.index.MismatchedLeafReader`: shuffles
//! field numbers around to try to trip bugs where field numbers are assumed
//! to always be consistent across segments.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::util::shuffle;
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
use rucene::index::{
    FieldInfo, FieldInfos, LeafReader, StoredFieldVisitor, StoredFieldVisitorStatus,
};

/// Port of `org.apache.lucene.tests.index.MismatchedLeafReader`
/// (`extends FilterLeafReader`): the wrapped reader with its field infos
/// renumbered in a random order, and its stored fields reporting the
/// renumbered infos.
pub struct MismatchedLeafReader {
    core: IndexReaderCore,
    r#in: Arc<dyn LeafReader>,
    shuffled: Arc<FieldInfos>,
}

impl MismatchedLeafReader {
    /// `new MismatchedLeafReader(in, random)`: creates a new reader which
    /// will renumber fields in `in`.
    ///
    /// # Errors
    ///
    /// What [`shuffle_infos`] raises.
    pub fn new(r#in: Arc<dyn LeafReader>, random: &dyn Random) -> Result<Self> {
        let shuffled = Arc::new(shuffle_infos(&r#in.get_field_infos(), random)?);
        Ok(MismatchedLeafReader {
            core: super::filter_leaf_core(&r#in),
            r#in,
            shuffled,
        })
    }

    /// `getDelegate()`: the wrapped reader.
    pub fn get_delegate(&self) -> &Arc<dyn LeafReader> {
        &self.r#in
    }
}

/// `MismatchedLeafReader.shuffleInfos(infos, random)`: the infos shuffled
/// with `Collections.shuffle(list, random)`, then renumbered `0..n` in the
/// shuffled order, every other property kept.
///
/// # Errors
///
/// What `new FieldInfo(...)` (its consistency check) and `new
/// FieldInfos(...)` raise.
pub fn shuffle_infos(infos: &FieldInfos, random: &dyn Random) -> Result<FieldInfos> {
    // first, shuffle the order
    let mut shuffled: Vec<FieldInfo> = infos.iter().cloned().collect();
    shuffle(&mut shuffled, random);
    // now renumber:
    for (i, slot) in shuffled.iter_mut().enumerate() {
        let old_info = &*slot;
        // TODO: should we introduce "gaps" too?
        let new_info = FieldInfo::new_full(
            old_info.name.clone(),                      // name
            i as i32,                                   // number
            old_info.has_term_vectors(),                // storeTermVector
            old_info.omits_norms(),                     // omitNorms
            old_info.has_payloads(),                    // storePayloads
            old_info.get_index_options(),               // indexOptions
            old_info.get_doc_values_type(),             // docValuesType
            old_info.doc_values_skip_index_type(),      // docValuesSkipIndexType
            old_info.get_doc_values_gen(),              // dvGen
            old_info.attributes(),                      // attributes
            old_info.get_point_dimension_count(),       // data dimension count
            old_info.get_point_index_dimension_count(), // index dimension count
            old_info.get_point_num_bytes(),             // dimension numBytes
            old_info.get_vector_dimension(), // number of dimensions of the field's vector
            old_info.get_vector_encoding(),  // numeric type of vector samples
            // distance function for calculating similarity of the field's vector
            old_info.get_vector_similarity_function(),
            old_info.is_soft_deletes_field(), // used as soft-deletes field
            old_info.is_parent_field(),
        )?;
        *slot = new_info;
    }
    FieldInfos::new(shuffled)
}

impl fmt::Debug for MismatchedLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MismatchedLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for MismatchedLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FilterLeafReader({})", self.r#in)
    }
}

impl LeafReader for MismatchedLeafReader {
    forward_leaf_reader!(filter r#in;
        core,
        filter_leaf_reader_delegate,
        term_vectors,
        num_docs,
        max_doc,
        do_close,
        terms,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
        get_sorted_numeric_doc_values,
        get_sorted_set_doc_values,
        get_norm_values,
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
        (*self.shuffled).clone()
    }

    /// `in.storedFields()`, directly (no `ensureOpen()` of this reader).
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        Ok(Box::new(MismatchedStoredFields {
            in_stored_fields: self.r#in.stored_fields()?,
            shuffled: Arc::clone(&self.shuffled),
        }))
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_core_cache_helper()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_reader_cache_helper()
    }
}

/// The anonymous `StoredFields` of `MismatchedLeafReader.storedFields()`:
/// visits through a [`MismatchedVisitor`]. It does not override `prefetch`,
/// whose default does nothing.
struct MismatchedStoredFields {
    in_stored_fields: Box<dyn StoredFields>,
    shuffled: Arc<FieldInfos>,
}

impl fmt::Debug for MismatchedStoredFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MismatchedStoredFields")
            .finish_non_exhaustive()
    }
}

impl StoredFields for MismatchedStoredFields {
    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        self.in_stored_fields
            .document_with_visitor(doc_id, &mut MismatchedVisitor::new(visitor, &self.shuffled))
    }
}

/// Port of `MismatchedLeafReader.MismatchedVisitor`: a
/// `StoredFieldVisitor` that remaps actual field numbers to our new shuffled
/// ones. Java's `Objects.requireNonNull(value, ...)` on a string value has
/// nothing to check here: a `&str` is never null.
pub struct MismatchedVisitor<'a> {
    r#in: &'a mut dyn StoredFieldVisitor,
    shuffled: &'a FieldInfos,
}

impl<'a> MismatchedVisitor<'a> {
    /// `new MismatchedVisitor(in, shuffled)`.
    pub fn new(r#in: &'a mut dyn StoredFieldVisitor, shuffled: &'a FieldInfos) -> Self {
        MismatchedVisitor { r#in, shuffled }
    }

    /// `renumber(original)`: the shuffled info of the same name, or the
    /// `AssertionError` (thrown, not asserted) when there is none.
    fn renumber(&self, original: &FieldInfo) -> Result<&'a FieldInfo> {
        self.shuffled.field_info(&original.name).ok_or_else(|| {
            LuceneError::AssertionError("stored fields sending bogus infos!".to_string())
        })
    }
}

impl StoredFieldVisitor for MismatchedVisitor<'_> {
    fn binary_field(&mut self, field_info: &FieldInfo, value: &[u8]) -> Result<()> {
        let renumbered = self.renumber(field_info)?;
        self.r#in.binary_field(renumbered, value)
    }

    fn string_field(&mut self, field_info: &FieldInfo, value: &str) -> Result<()> {
        let renumbered = self.renumber(field_info)?;
        self.r#in.string_field(renumbered, value)
    }

    fn int_field(&mut self, field_info: &FieldInfo, value: i32) -> Result<()> {
        let renumbered = self.renumber(field_info)?;
        self.r#in.int_field(renumbered, value)
    }

    fn long_field(&mut self, field_info: &FieldInfo, value: i64) -> Result<()> {
        let renumbered = self.renumber(field_info)?;
        self.r#in.long_field(renumbered, value)
    }

    fn float_field(&mut self, field_info: &FieldInfo, value: f32) -> Result<()> {
        let renumbered = self.renumber(field_info)?;
        self.r#in.float_field(renumbered, value)
    }

    fn double_field(&mut self, field_info: &FieldInfo, value: f64) -> Result<()> {
        let renumbered = self.renumber(field_info)?;
        self.r#in.double_field(renumbered, value)
    }

    fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        let renumbered = self.renumber(field_info)?;
        self.r#in.needs_field(renumbered)
    }
}
