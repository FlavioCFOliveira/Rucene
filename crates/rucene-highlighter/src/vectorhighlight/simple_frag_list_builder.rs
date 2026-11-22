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

//! Port of `SimpleFragListBuilder.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::ops::Deref;

use rucene::error::Result;

use crate::vectorhighlight::base_frag_list_builder::BaseFragListBuilder;
use crate::vectorhighlight::field_frag_list::FieldFragList;
use crate::vectorhighlight::field_phrase_list::FieldPhraseList;
use crate::vectorhighlight::frag_list_builder::FragListBuilder;
use crate::vectorhighlight::simple_field_frag_list::SimpleFieldFragList;

/// A simple implementation of `FragListBuilder`.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.SimpleFragListBuilder`.
#[derive(Debug, Clone, Default)]
pub struct SimpleFragListBuilder {
    base: BaseFragListBuilder,
}

impl SimpleFragListBuilder {
    /// Equivalent to `new SimpleFragListBuilder()`.
    pub fn new() -> Self {
        Self {
            base: BaseFragListBuilder::new(),
        }
    }

    /// Equivalent to `new SimpleFragListBuilder(int)`.
    ///
    /// # Errors
    ///
    /// As [`BaseFragListBuilder::with_margin`].
    pub fn with_margin(margin: i32) -> Result<Self> {
        Ok(Self {
            base: BaseFragListBuilder::with_margin(margin)?,
        })
    }
}

impl Deref for SimpleFragListBuilder {
    type Target = BaseFragListBuilder;

    fn deref(&self) -> &BaseFragListBuilder {
        &self.base
    }
}

impl FragListBuilder for SimpleFragListBuilder {
    fn create_field_frag_list(
        &self,
        field_phrase_list: &FieldPhraseList,
        frag_char_size: i32,
    ) -> Result<Box<dyn FieldFragList>> {
        self.base.create_field_frag_list_into(
            field_phrase_list,
            Box::new(SimpleFieldFragList::new(frag_char_size)),
            frag_char_size,
        )
    }
}