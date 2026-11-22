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

//! Port of `WeightedFieldFragList.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::collections::HashSet;

use crate::vectorhighlight::field_frag_list::{FieldFragList, SubInfo, WeightedFragInfo};
use crate::vectorhighlight::field_phrase_list::WeightedPhraseInfo;

/// A weighted implementation of `FieldFragList`.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.WeightedFieldFragList`.
#[derive(Debug, Clone, Default)]
pub struct WeightedFieldFragList {
    frag_infos: Vec<WeightedFragInfo>,
}

impl WeightedFieldFragList {
    /// Equivalent to `new WeightedFieldFragList(int)`: `frag_char_size` is
    /// the length of the fragment (unused, as in Java).
    pub fn new(_frag_char_size: i32) -> Self {
        Self::default()
    }
}

impl FieldFragList for WeightedFieldFragList {
    fn add(&mut self, start_offset: i32, end_offset: i32, phrase_info_list: &[WeightedPhraseInfo]) {
        let mut temp_sub_infos = Vec::new();
        let mut real_sub_infos = Vec::new();
        let mut distinct_terms: HashSet<String> = HashSet::new();
        let mut length: i32 = 0;

        for phrase_info in phrase_info_list {
            let mut phrase_total_boost = 0.0f32;
            for ti in phrase_info.get_terms_infos() {
                if distinct_terms.insert(ti.get_text().to_string()) {
                    phrase_total_boost += ti.get_weight() * phrase_info.get_boost();
                }
                length += 1;
            }
            temp_sub_infos.push(SubInfo::new(
                phrase_info.get_text(),
                phrase_info.get_terms_offsets().clone(),
                phrase_info.get_seqnum(),
                phrase_total_boost,
            ));
        }

        // We want that terms per fragment (length) is included into the
        // weight. Otherwise a one-word-query would cause an equal weight for
        // all fragments regardless of how much words they contain. To avoid
        // that fragments containing a high number of words possibly "outrank"
        // more relevant fragments we "bend" the length with a standard-
        // normalization a little bit.
        let norm = length as f32 * (1.0 / (length as f64).sqrt() as f32);

        let mut total_boost = 0.0f32;
        for temp_sub_info in temp_sub_infos {
            let sub_info_boost = temp_sub_info.boost() * norm;
            real_sub_infos.push(SubInfo::new(
                temp_sub_info.text().to_string(),
                temp_sub_info.terms_offsets().clone(),
                temp_sub_info.seqnum(),
                sub_info_boost,
            ));
            total_boost += sub_info_boost;
        }

        self.frag_infos.push(WeightedFragInfo::new(
            start_offset,
            end_offset,
            real_sub_infos,
            total_boost,
        ));
    }

    fn get_frag_infos(&self) -> &Vec<WeightedFragInfo> {
        &self.frag_infos
    }

    fn get_frag_infos_mut(&mut self) -> &mut Vec<WeightedFragInfo> {
        &mut self.frag_infos
    }
}