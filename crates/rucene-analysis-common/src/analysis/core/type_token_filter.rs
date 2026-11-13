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

//! Removes tokens whose types appear in a set of blocked types, ported from
//! `org.apache.lucene.analysis.core.TypeTokenFilter`.

use std::collections::HashSet;

use rucene::analysis::{
    new_filtering_token_filter, FilteringTokenFilter, FilteringTokenFilterLogic, TokenStream,
    TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// The `accept` of [`TypeTokenFilter`]: what Lucene's `TypeTokenFilter`
/// adds to `FilteringTokenFilter`.
#[derive(Debug)]
pub struct TypeTokenFilterLogic {
    stop_types: HashSet<String>,
    use_white_list: bool,
}

impl FilteringTokenFilterLogic for TypeTokenFilterLogic {
    /// By default accept the token if its type is not a stop type. When the
    /// useWhiteList parameter is set to true then accept the token if its
    /// type is contained in the stopTypes.
    ///
    /// Equivalent to `TypeTokenFilter.accept()`.
    fn accept(&mut self, source: &AttributeSource) -> Result<bool> {
        let type_attribute = source
            .get_attribute_interface::<dyn TypeAttribute>()
            .expect("INVARIANT: the constructor added the TypeAttribute");
        Ok(self.use_white_list == self.stop_types.contains(type_attribute.type_value()))
    }
}

/// Removes tokens whose types appear in a set of blocked types from a token
/// stream.
///
/// Equivalent to `org.apache.lucene.analysis.core.TypeTokenFilter`.
pub type TypeTokenFilter = FilteringTokenFilter<TypeTokenFilterLogic>;

/// Create a new [`TypeTokenFilter`]: `input` is the `TokenStream` to consume,
/// `stop_types` the types to filter, and when `use_white_list` is true,
/// tokens whose type is in `stop_types` will be kept, otherwise they will be
/// filtered out.
///
/// Equivalent to `TypeTokenFilter(TokenStream, Set<String>, boolean)`, whose
/// `super(input)` is [`new_filtering_token_filter`], then adds the
/// `TypeAttribute`. Java's `Objects.requireNonNull(stopTypes, "stopTypes")`
/// has no counterpart: `stop_types` cannot be null here.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_type_token_filter(
    input: Box<dyn TokenStream>,
    stop_types: HashSet<String>,
    use_white_list: bool,
) -> Result<TypeTokenFilter> {
    let mut filter = new_filtering_token_filter(
        input,
        TypeTokenFilterLogic {
            stop_types,
            use_white_list,
        },
    )?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(filter)
}

/// Create a new [`TypeTokenFilter`] that filters tokens out
/// (useWhiteList=false).
///
/// Equivalent to `TypeTokenFilter(TokenStream, Set<String>)`, `this(input,
/// stopTypes, false)`.
///
/// # Errors
///
/// As [`new_type_token_filter`].
pub fn new_type_token_filter_with_stop_types(
    input: Box<dyn TokenStream>,
    stop_types: HashSet<String>,
) -> Result<TypeTokenFilter> {
    new_type_token_filter(input, stop_types, false)
}
