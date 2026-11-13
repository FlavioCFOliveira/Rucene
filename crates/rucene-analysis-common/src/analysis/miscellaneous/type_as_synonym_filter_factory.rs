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
//! Factory for `TypeAsSynonymFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.TypeAsSynonymFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;
use rucene::util::java_collections::JavaHashSet;

use super::type_as_synonym_filter::new_type_as_synonym_filter_with;
use crate::spi::check_unknown_parameters;

/// Factory for `TypeAsSynonymFilter`.
///
/// In Solr this might be used as such
///
/// ```text
/// <fieldType name="text_type_as_synonym" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.UAX29URLEmailTokenizerFactory"/>
///     <filter class="solr.TypeAsSynonymFilterFactory" prefix="_type_" synFlagsMask="5" ignore="foo,bar"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// If the optional prefix parameter is used, the specified value will be
/// prepended to the type, e.g. with prefix="_type_", for a token "example.com"
/// with type "&lt;URL&gt;", the emitted synonym will have text
/// "_type_&lt;URL&gt;". If the optional synFlagsMask is used then the flags
/// on the synonym will be set to `synFlagsMask & tokenFlags`. The example
/// above transfers only the lowest (1) and third (4) flags. If the ignore
/// parameter is used, tokens with types in the list will not have synonyms
/// generated.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.TypeAsSynonymFilterFactory` (since 7.3.0).
pub struct TypeAsSynonymFilterFactory {
    base: AbstractAnalysisFactory,
    /// Java's `prefix`, `null` by default.
    prefix: Option<String>,
    ignore: Option<JavaHashSet>,
    syn_flag_mask: i32,
}

impl std::fmt::Debug for TypeAsSynonymFilterFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypeAsSynonymFilterFactory")
            .field("base", &self.base)
            .field("prefix", &self.prefix)
            .field(
                "ignore",
                &self
                    .ignore
                    .as_ref()
                    .map(|set| set.iter().collect::<Vec<_>>()),
            )
            .field("syn_flag_mask", &self.syn_flag_mask)
            .finish()
    }
}

impl TypeAsSynonymFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `TypeAsSynonymFilterFactory.NAME`.
    pub const NAME: &'static str = "typeAsSynonym";

    /// Creates a new TypeAsSynonymFilterFactory.
    ///
    /// Equivalent to `TypeAsSynonymFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, what an unparsable `synFlagsMask` raises, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let prefix = AbstractAnalysisFactory::get(args, "prefix"); // default value is null
        let ignore = AbstractAnalysisFactory::get_set(args, "ignore");
        let syn_flag_mask = AbstractAnalysisFactory::get_int(args, "synFlagsMask", !0)?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            prefix,
            ignore,
            syn_flag_mask,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `TypeAsSynonymFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The members this factory inherits from `AbstractAnalysisFactory`.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        &self.base
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `TypeAsSynonymFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.TypeAsSynonymFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl TokenFilterFactory for TypeAsSynonymFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.TypeAsSynonymFilterFactory"
    }

    /// Equivalent to `TypeAsSynonymFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_type_as_synonym_filter_with(
            input,
            self.prefix.clone(),
            self.ignore.as_ref().map(JavaHashSet::to_std),
            self.syn_flag_mask,
        )?))
    }
}
