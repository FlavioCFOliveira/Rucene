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

//! Factory class for `TypeTokenFilter`, ported from
//! `org.apache.lucene.analysis.core.TypeTokenFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;
use rucene::util::java_collections::JavaHashSet;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::type_token_filter::new_type_token_filter;
use crate::spi::check_unknown_parameters;

/// Factory class for `TypeTokenFilter`.
///
/// ```text
/// <fieldType name="chars" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///     <filter class="solr.TypeTokenFilterFactory" types="stoptypes.txt"
///                   useWhitelist="false"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.core.TypeTokenFilterFactory`
/// (since 3.6.0), which extends `TokenFilterFactory` and implements
/// `ResourceLoaderAware`.
pub struct TypeTokenFilterFactory {
    base: AbstractAnalysisFactory,
    use_whitelist: bool,
    stop_types_files: String,
    /// Java's `Set<String> stopTypes`, a `java.util.HashSet`; `None` until
    /// [`inform`](ResourceLoaderAware::inform) reads a file.
    stop_types: Option<JavaHashSet>,
}

impl std::fmt::Debug for TypeTokenFilterFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypeTokenFilterFactory")
            .field("base", &self.base)
            .field("use_whitelist", &self.use_whitelist)
            .field("stop_types_files", &self.stop_types_files)
            .field(
                "stop_types",
                &self
                    .stop_types
                    .as_ref()
                    .map(|set| set.iter().collect::<Vec<_>>()),
            )
            .finish()
    }
}

impl TypeTokenFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `TypeTokenFilterFactory.NAME`.
    pub const NAME: &'static str = "type";

    /// Creates a new TypeTokenFilterFactory.
    ///
    /// Equivalent to `TypeTokenFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns,
    /// `IllegalArgumentException` when `types` is missing, and for a
    /// leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let stop_types_files = AbstractAnalysisFactory::require(args, "types")?;
        let use_whitelist = AbstractAnalysisFactory::get_boolean(args, "useWhitelist", false);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            use_whitelist,
            stop_types_files,
            stop_types: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `TypeTokenFilterFactory()`.
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

    /// Equivalent to `getStopTypes()`: `None` (Java's `null`) until
    /// [`inform`](ResourceLoaderAware::inform) reads at least one file.
    pub fn get_stop_types(&self) -> Option<&JavaHashSet> {
        self.stop_types.as_ref()
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `TypeTokenFilterFactory.class`, with its `NAME` field and
    /// its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.core.TypeTokenFilterFactory",
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

impl ResourceLoaderAware for TypeTokenFilterFactory {
    /// Equivalent to `TypeTokenFilterFactory.inform(ResourceLoader)`: the
    /// lines of every file `types` names, gathered into a new `HashSet`
    /// when it names at least one.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        let files = AbstractAnalysisFactory::split_file_names(Some(&self.stop_types_files));
        if !files.is_empty() {
            let mut stop_types = JavaHashSet::new();
            for file in &files {
                let types_lines = AbstractAnalysisFactory::get_lines(loader, file.trim())?;
                for line in types_lines {
                    stop_types.insert(line);
                }
            }
            self.stop_types = Some(stop_types);
        }
        Ok(())
    }
}

impl TokenFilterFactory for TypeTokenFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.core.TypeTokenFilterFactory"
    }

    /// Equivalent to `TypeTokenFilterFactory.create(TokenStream)`, `new
    /// TypeTokenFilter(input, stopTypes, useWhitelist)`.
    ///
    /// # Panics
    ///
    /// When the stop types are not set (Java's `stopTypes` is then `null`),
    /// with the `NullPointerException` `TypeTokenFilter`'s
    /// `Objects.requireNonNull(stopTypes, "stopTypes")` throws.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let stop_types = match &self.stop_types {
            Some(stop_types) => stop_types.to_std(),
            None => rucene::store::java_nio_file::throw_unchecked(
                "java.lang.NullPointerException",
                Some("stopTypes"),
            ),
        };
        let filter = new_type_token_filter(input, stop_types, self.use_whitelist)?;
        Ok(Box::new(filter))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
