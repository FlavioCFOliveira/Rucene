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

//! Port of `SimpleFieldFragList.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use crate::vectorhighlight::field_frag_list::{FieldFragList, SubInfo, WeightedFragInfo};
use crate::vectorhighlight::field_phrase_list::WeightedPhraseInfo;

/// A simple implementation of `FieldFragList`.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.SimpleFieldFragList`.
#[derive(Debug, Clone, Default)]
pub struct SimpleFieldFragList {
    frag_infos: Vec<WeightedFragInfo>,
}

impl SimpleFieldFragList {
    /// Equivalent to `new SimpleFieldFragList(int)`: `frag_char_size` is the
    /// length of the fragment (unused, as in Java).
    pub fn new(_frag_char_size: i32) -> Self {
        Self::default()
    }
}

impl FieldFragList for SimpleFieldFragList {
    fn add(&mut self, start_offset: i32, end_offset: i32, phrase_info_list: &[WeightedPhraseInfo]) {
        let mut total_boost = 0.0f32;
        let mut sub_infos = Vec::new();
        for phrase_info in phrase_info_list {
            sub_infos.push(SubInfo::new(
                phrase_info.get_text(),
                phrase_info.get_terms_offsets().clone(),
                phrase_info.get_seqnum(),
                phrase_info.get_boost(),
            ));
            total_boost += phrase_info.get_boost();
        }
        self.frag_infos.push(WeightedFragInfo::new(
            start_offset,
            end_offset,
            sub_infos,
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
