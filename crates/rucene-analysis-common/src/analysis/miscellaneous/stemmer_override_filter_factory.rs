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
//! Factory for `StemmerOverrideFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.StemmerOverrideFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::LuceneError;
use rucene::error::Result;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::stemmer_override_filter::{new_stemmer_override_filter, Builder, StemmerOverrideMap};
use crate::spi::check_unknown_parameters;

/// Factory for `StemmerOverrideFilter`.
///
/// ```text
/// <fieldType name="text_dicstem" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.StemmerOverrideFilterFactory" dictionary="dictionary.txt" ignoreCase="false"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.StemmerOverrideFilterFactory` (since 3.1), which implements `ResourceLoaderAware`.
#[derive(Debug)]
pub struct StemmerOverrideFilterFactory {
    base: AbstractAnalysisFactory,
    dictionary: Option<Arc<StemmerOverrideMap>>,
    dictionary_files: Option<String>,
    ignore_case: bool,
}

impl StemmerOverrideFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `StemmerOverrideFilterFactory.NAME`.
    pub const NAME: &'static str = "stemmerOverride";

    /// Creates a new StemmerOverrideFilterFactory.
    ///
    /// Equivalent to `StemmerOverrideFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let dictionary_files = AbstractAnalysisFactory::get(args, "dictionary");
        let ignore_case = AbstractAnalysisFactory::get_boolean(args, "ignoreCase", false);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            dictionary: None,
            dictionary_files,
            ignore_case,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `StemmerOverrideFilterFactory()`.
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

    /// Equivalent to `isIgnoreCase()`.
    pub fn is_ignore_case(&self) -> bool {
        self.ignore_case
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `StemmerOverrideFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.StemmerOverrideFilterFactory",
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

impl ResourceLoaderAware for StemmerOverrideFilterFactory {
    /// Equivalent to `StemmerOverrideFilterFactory.inform(ResourceLoader)`:
    /// every line of the dictionary files is split at its first tab into an
    /// input and its override.
    ///
    /// A line without a tab fails as Java's `mapping[1]` does, with
    /// `ArrayIndexOutOfBoundsException`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        if let Some(dictionary_files) = &self.dictionary_files {
            let files = AbstractAnalysisFactory::split_file_names(Some(dictionary_files));
            if !files.is_empty() {
                let mut builder = Builder::with_ignore_case(self.ignore_case);
                for file in &files {
                    let list = AbstractAnalysisFactory::get_lines(loader, file.trim())?;
                    for line in list {
                        let mut mapping = line.splitn(2, '\t');
                        let input = mapping.next().unwrap_or_default();
                        let Some(output) = mapping.next() else {
                            return Err(LuceneError::unchecked(
                                "java.lang.ArrayIndexOutOfBoundsException",
                                "Index 1 out of bounds for length 1",
                            ));
                        };
                        builder.add(input, output)?;
                    }
                }
                self.dictionary = Some(Arc::new(builder.build()?));
            }
        }
        Ok(())
    }
}

impl TokenFilterFactory for StemmerOverrideFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.StemmerOverrideFilterFactory"
    }

    /// Equivalent to `StemmerOverrideFilterFactory.create(TokenStream)`: `input`
    /// itself when there is no dictionary.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        match &self.dictionary {
            None => Ok(input),
            Some(dictionary) => Ok(Box::new(new_stemmer_override_filter(
                input,
                Arc::clone(dictionary),
            )?)),
        }
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
