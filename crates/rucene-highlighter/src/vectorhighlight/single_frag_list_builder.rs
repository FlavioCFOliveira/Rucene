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

//! Port of `SingleFragListBuilder.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use rucene::error::Result;

use crate::vectorhighlight::field_frag_list::FieldFragList;
use crate::vectorhighlight::field_phrase_list::FieldPhraseList;
use crate::vectorhighlight::frag_list_builder::FragListBuilder;
use crate::vectorhighlight::simple_field_frag_list::SimpleFieldFragList;

/// An implementation class of `FragListBuilder` that generates one
/// `WeightedFragInfo` object. Typical use case of this class is that you can
/// get an entire field contents by using both of this class and
/// `SimpleFragmentsBuilder`.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.SingleFragListBuilder`.
#[derive(Debug, Clone, Default)]
pub struct SingleFragListBuilder;

impl SingleFragListBuilder {
    /// Equivalent to `new SingleFragListBuilder()`.
    pub fn new() -> Self {
        Self
    }
}

impl FragListBuilder for SingleFragListBuilder {
    fn create_field_frag_list(
        &self,
        field_phrase_list: &FieldPhraseList,
        frag_char_size: i32,
    ) -> Result<Box<dyn FieldFragList>> {
        let mut ffl = SimpleFieldFragList::new(frag_char_size);

        let mut wpil = Vec::new();
        for phrase_info in &field_phrase_list.phrase_list {
            wpil.push(phrase_info.clone());
        }
        if !wpil.is_empty() {
            ffl.add(0, i32::MAX, &wpil);
        }
        Ok(Box::new(ffl))
    }
}
