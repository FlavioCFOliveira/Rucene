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

//! Port of `org.apache.lucene.codecs.bloom.BloomFilterFactory`.

use std::fmt;

use rucene::codecs::SegmentWriteState;
use rucene::error::Result;
use rucene::index::FieldInfo;

use super::fuzzy_set::FuzzySet;

/// Class used to create index-time [`FuzzySet`] appropriately configured
/// for each field. Also called to right-size bitsets for serialization.
///
/// Port of the abstract class `org.apache.lucene.codecs.bloom.BloomFilterFactory`.
pub trait BloomFilterFactory: Send + Sync + fmt::Debug {
    /// Returns an appropriately sized set or `None` if no BloomFiltering
    /// required for the field `info` of the content `state` indexes.
    ///
    /// Port of `getSetForField(SegmentWriteState, FieldInfo)`.
    ///
    /// # Errors
    ///
    /// Whatever reading `state` raises (for one, `IllegalStateException`
    /// when the segment's `maxDoc` is not set yet).
    fn get_set_for_field(
        &self,
        state: &SegmentWriteState<'_>,
        info: &FieldInfo,
    ) -> Result<Option<FuzzySet>>;

    /// Called when downsizing bitsets for serialization: `None` or a
    /// hopefully more densely packed, smaller bitset of `initial_set`, the
    /// bits accumulated for the field with sparse set bits `field_info`.
    ///
    /// Port of `downsize(FieldInfo, FuzzySet)`.
    fn downsize(&self, field_info: &FieldInfo, initial_set: &FuzzySet) -> Option<FuzzySet> {
        let _ = field_info;
        let target_max_saturation = initial_set.get_target_max_saturation();
        initial_set.downsize(target_max_saturation)
    }

    /// Used to determine if the given filter has reached saturation and
    /// should be retired i.e. not saved any more: true if the set has
    /// reached saturation and should be retired.
    ///
    /// Port of `isSaturated(FuzzySet, FieldInfo)`.
    fn is_saturated(&self, bloom_filter: &FuzzySet, field_info: &FieldInfo) -> bool;
}
