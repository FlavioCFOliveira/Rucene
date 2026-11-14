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

//! Factory for `CommonGramsQueryFilter`, ported from
//! `org.apache.lucene.analysis.commongrams.CommonGramsQueryFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::common_grams_filter_factory::CommonGramsFilterFactory;
use super::common_grams_query_filter::new_common_grams_query_filter;

/// Construct `CommonGramsQueryFilter`.
///
/// ```text
/// <fieldType name="text_cmmngrmsqry" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.CommonGramsQueryFilterFactory" words="commongramsquerystopwords.txt" ignoreCase="false"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.commongrams.CommonGramsQueryFilterFactory`
/// (since 3.1), which extends `CommonGramsFilterFactory`: the factory holds
/// the [`CommonGramsFilterFactory`] it extends.
#[derive(Debug)]
pub struct CommonGramsQueryFilterFactory {
    base: CommonGramsFilterFactory,
}

impl CommonGramsQueryFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `CommonGramsQueryFilterFactory.NAME`.
    pub const NAME: &'static str = "commonGramsQuery";

    /// Creates a new CommonGramsQueryFilterFactory.
    ///
    /// Equivalent to `CommonGramsQueryFilterFactory(Map<String, String>)`,
    /// `super(args)`.
    ///
    /// # Errors
    ///
    /// Returns what [`CommonGramsFilterFactory::new`] returns.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            base: CommonGramsFilterFactory::new(args)?,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `CommonGramsQueryFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The members this factory inherits from `CommonGramsFilterFactory`.
    pub fn base(&self) -> &CommonGramsFilterFactory {
        &self.base
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `CommonGramsQueryFilterFactory.class`, with its `NAME`
    /// field and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.commongrams.CommonGramsQueryFilterFactory",
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

impl ResourceLoaderAware for CommonGramsQueryFilterFactory {
    /// Equivalent to the inherited `AbstractWordsFileFilterFactory.inform`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        self.base.inform(loader)
    }
}

impl TokenFilterFactory for CommonGramsQueryFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.commongrams.CommonGramsQueryFilterFactory"
    }

    /// Create a CommonGramsFilter and wrap it with a CommonGramsQueryFilter
    ///
    /// Equivalent to `CommonGramsQueryFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let common_grams = self.base.create_common_grams_filter(input)?;
        Ok(Box::new(new_common_grams_query_filter(common_grams)?))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
