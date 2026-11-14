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

//! Factory for `PathHierarchyTokenizer`, ported from
//! `org.apache.lucene.analysis.path.PathHierarchyTokenizerFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::Tokenizer;
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use super::path_hierarchy_tokenizer::{
    path_hierarchy_tokenizer_parts, DEFAULT_DELIMITER, DEFAULT_SKIP,
};
use super::reverse_path_hierarchy_tokenizer::reverse_path_hierarchy_tokenizer_parts;
use crate::spi::check_unknown_parameters;

/// The default buffer size of both tokenizers, which their `(AttributeFactory,
/// char, char, int)` constructors pass on.
const DEFAULT_BUFFER_SIZE: i32 = 1024;

/// Factory for `PathHierarchyTokenizer`.
///
/// This factory is typically configured for use only in the `index`
/// Analyzer (or only in the `query` Analyzer, but never both).
///
/// For example, in the configuration below a query for `Books/NonFic` will
/// match documents indexed with values like `Books/NonFic`,
/// `Books/NonFic/Law`, `Books/NonFic/Science/Physics`, etc. But it will not
/// match documents indexed with values like `Books`, or `Books/Fic`...
///
/// ```text
/// <fieldType name="descendent_path" class="solr.TextField">
///   <analyzer type="index">
///     <tokenizer class="solr.PathHierarchyTokenizerFactory" delimiter="/" />
///   </analyzer>
///   <analyzer type="query">
///     <tokenizer class="solr.KeywordTokenizerFactory" />
///   </analyzer>
/// </fieldType>
/// ```
///
/// In this example however we see the oposite configuration, so that a
/// query for `Books/NonFic/Science/Physics` would match documents containing
/// `Books/NonFic`, `Books/NonFic/Science`, or `Books/NonFic/Science/Physics`,
/// but not `Books/NonFic/Science/Physics/Theory` or `Books/NonFic/Law`.
///
/// ```text
/// <fieldType name="descendent_path" class="solr.TextField">
///   <analyzer type="index">
///     <tokenizer class="solr.KeywordTokenizerFactory" />
///   </analyzer>
///   <analyzer type="query">
///     <tokenizer class="solr.PathHierarchyTokenizerFactory" delimiter="/" />
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.path.PathHierarchyTokenizerFactory`
/// (since 3.1).
#[derive(Debug)]
pub struct PathHierarchyTokenizerFactory {
    base: AbstractAnalysisFactory,
    delimiter: u16,
    replacement: u16,
    reverse: bool,
    skip: i32,
}

impl PathHierarchyTokenizerFactory {
    /// SPI name.
    ///
    /// Equivalent to `PathHierarchyTokenizerFactory.NAME`.
    pub const NAME: &'static str = "pathHierarchy";

    /// Creates a new PathHierarchyTokenizerFactory.
    ///
    /// Equivalent to `PathHierarchyTokenizerFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`],
    /// [`AbstractAnalysisFactory::get_char`] and
    /// [`AbstractAnalysisFactory::get_int`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        // `getChar` accepts one UTF-16 unit, a character of the BMP.
        let delimiter = AbstractAnalysisFactory::get_char(
            args,
            "delimiter",
            char::from_u32(u32::from(DEFAULT_DELIMITER)).expect("INVARIANT: '/'"),
        )?;
        let replacement = AbstractAnalysisFactory::get_char(args, "replace", delimiter)?;
        let reverse = AbstractAnalysisFactory::get_boolean(args, "reverse", false);
        let skip = AbstractAnalysisFactory::get_int(args, "skip", DEFAULT_SKIP)?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            delimiter: delimiter as u32 as u16,
            replacement: replacement as u32 as u16,
            reverse,
            skip,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `PathHierarchyTokenizerFactory()`.
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

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenizerFactory`
    /// names it: `PathHierarchyTokenizerFactory.class`, with its `NAME` field
    /// and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.path.PathHierarchyTokenizerFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenizerFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl TokenizerFactory for PathHierarchyTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.path.PathHierarchyTokenizerFactory"
    }

    /// Equivalent to `PathHierarchyTokenizerFactory.create(AttributeFactory)`:
    /// a `ReversePathHierarchyTokenizer` when `reverse`, else a
    /// `PathHierarchyTokenizer`, over `(factory, delimiter, replacement,
    /// skip)`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        if self.reverse {
            let (source, logic) = reverse_path_hierarchy_tokenizer_parts(
                factory,
                DEFAULT_BUFFER_SIZE,
                self.delimiter,
                self.replacement,
                self.skip,
            )?;
            return Ok(Tokenizer::with_source(source, Box::new(logic)));
        }
        let (source, logic) = path_hierarchy_tokenizer_parts(
            factory,
            DEFAULT_BUFFER_SIZE,
            self.delimiter,
            self.replacement,
            self.skip,
        )?;
        Ok(Tokenizer::with_source(source, Box::new(logic)))
    }
}
