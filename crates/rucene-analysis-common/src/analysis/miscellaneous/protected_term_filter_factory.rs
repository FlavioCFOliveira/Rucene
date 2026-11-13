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
//! Factory for `ProtectedTermFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.ProtectedTermFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    token_filter_for_name, AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField,
    TokenFilterFactory,
};
use rucene::analysis::{CharArraySet, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::conditional_token_filter_factory::ConditionalTokenFilterFactory;
use super::protected_term_filter::new_protected_term_filter;
use crate::spi::check_unknown_parameters;

/// `String.trim()`: the characters up to U+0020 removed from both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Factory for a `ProtectedTermFilter`.
///
/// CustomAnalyzer example:
///
/// ```text
/// Analyzer ana = CustomAnalyzer.builder()
///   .withTokenizer("standard")
///   .when("protectedterm", "ignoreCase", "true", "protected", "protectedTerms.txt")
///     .addTokenFilter("truncate", "prefixLength", "4")
///     .addTokenFilter("lowercase")
///   .endwhen()
///   .build();
/// ```
///
/// Solr example, in which conditional filters are specified via the
/// `wrappedFilters` parameter - a comma-separated list of case-insensitive
/// TokenFilter SPI names - and conditional filter args are specified via
/// `filterName.argName` parameters:
///
/// ```text
/// <fieldType name="reverse_lower_with_exceptions" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.ProtectedTermFilterFactory" ignoreCase="true" protected="protectedTerms.txt"
///             wrappedFilters="truncate,lowercase" truncate.prefixLength="4" />
///   </analyzer>
/// </fieldType>
/// ```
///
/// When using the `wrappedFilters` parameter, each filter name must be
/// unique, so if you need to specify the same filter more than once, you
/// must add case-insensitive unique '-id' suffixes (note that the '-id'
/// suffix is stripped prior to SPI lookup), e.g.:
///
/// ```text
/// <fieldType name="double_synonym_with_exceptions" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.ProtectedTermFilterFactory" ignoreCase="true" protected="protectedTerms.txt"
///             wrappedFilters="synonymgraph-A,synonymgraph-B"
///             synonymgraph-A.synonyms="synonyms-1.txt"
///             synonymgraph-B.synonyms="synonyms-2.txt"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// See related `CustomAnalyzer.Builder#whenTerm(Predicate)`.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ProtectedTermFilterFactory`
/// (since 7.4.0), which extends `ConditionalTokenFilterFactory`: the factory
/// holds the [`ConditionalTokenFilterFactory`] it extends.
#[derive(Debug)]
pub struct ProtectedTermFilterFactory {
    base: ConditionalTokenFilterFactory,
    term_files: String,
    ignore_case: bool,
    wrapped_filters: Option<String>,
    protected_terms: Option<CharArraySet>,
}

impl ProtectedTermFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `ProtectedTermFilterFactory.NAME`.
    pub const NAME: &'static str = "protectedTerm";

    /// Equivalent to `ProtectedTermFilterFactory.PROTECTED_TERMS`.
    pub const PROTECTED_TERMS: &'static str = "protected";

    /// Equivalent to `ProtectedTermFilterFactory.FILTER_ARG_SEPARATOR`.
    pub const FILTER_ARG_SEPARATOR: char = '.';

    /// Equivalent to `ProtectedTermFilterFactory.FILTER_NAME_ID_SEPARATOR`.
    pub const FILTER_NAME_ID_SEPARATOR: char = '-';

    /// Creates a new ProtectedTermFilterFactory.
    ///
    /// Equivalent to `ProtectedTermFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`ConditionalTokenFilterFactory::new`] returns,
    /// `IllegalArgumentException` when `protected` is missing, for a
    /// duplicate wrapped filter name, and for a leftover argument, and what
    /// looking up and building a wrapped filter factory raises.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let mut base = ConditionalTokenFilterFactory::new(args)?;
        let term_files = AbstractAnalysisFactory::require(args, Self::PROTECTED_TERMS)?;
        let ignore_case = AbstractAnalysisFactory::get_boolean(args, "ignoreCase", false);
        let wrapped_filters = AbstractAnalysisFactory::get(args, "wrappedFilters");
        if let Some(wrapped_filters) = &wrapped_filters {
            Self::handle_wrapped_filter_args(&mut base, wrapped_filters, args)?;
        }
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            term_files,
            ignore_case,
            wrapped_filters,
            protected_terms: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `ProtectedTermFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The members this factory inherits from `ConditionalTokenFilterFactory`.
    pub fn base(&self) -> &ConditionalTokenFilterFactory {
        &self.base
    }

    /// The members this factory inherits from `ConditionalTokenFilterFactory`,
    /// to set the inner filters.
    pub fn base_mut(&mut self) -> &mut ConditionalTokenFilterFactory {
        &mut self.base
    }

    /// The `wrappedFilters` argument, as given.
    pub fn wrapped_filters(&self) -> Option<&str> {
        self.wrapped_filters.as_deref()
    }

    /// Equivalent to the private `handleWrappedFilterArgs(Map<String,
    /// String>)`: the wrapped filter names (trimmed, lower-cased with
    /// `Locale.ROOT`) in a `LinkedHashMap`, whose arguments are the
    /// `filterName.argKey` entries of `args` (removed from it); the inner
    /// filters are built only when no other argument is left.
    ///
    /// Java walks `args` in its own iteration order; every entry is handled
    /// independently, so the order changes nothing but which of two
    /// identical keys wins, and `args` holds no two identical keys.
    fn handle_wrapped_filter_args(
        base: &mut ConditionalTokenFilterFactory,
        wrapped_filters: &str,
        args: &mut HashMap<String, String>,
    ) -> Result<()> {
        // Java's `LinkedHashMap<String, Map<String, String>>`, in insertion order.
        let mut wrapped_filter_args: Vec<(String, HashMap<String, String>)> = Vec::new();
        for filter_name in AbstractAnalysisFactory::split_at(',', Some(wrapped_filters)) {
            // Format: SPIname[-id]
            let filter_name = java_trim(&filter_name).to_lowercase(); // Treat case-insensitively
            if wrapped_filter_args
                .iter()
                .any(|(name, _)| *name == filter_name)
            {
                return Err(LuceneError::illegal_argument(format!(
                    "wrappedFilters contains duplicate '{filter_name}'. Add unique '-id' suffixes (stripped prior to SPI lookup)."
                )));
            }
            wrapped_filter_args.push((filter_name, HashMap::new()));
        }
        let keys: Vec<String> = args.keys().cloned().collect();
        for filter_arg_key in keys {
            // Format: filterName.argKey
            let split_key = AbstractAnalysisFactory::split_at(
                Self::FILTER_ARG_SEPARATOR,
                Some(&filter_arg_key),
            );
            if split_key.len() == 2 {
                // Skip if no slash
                let filter_name = split_key[0].to_lowercase();
                // Skip if not in "wrappedFilter" arg
                if let Some((_, filter_args)) = wrapped_filter_args
                    .iter_mut()
                    .find(|(name, _)| *name == filter_name)
                {
                    let arg_value = args
                        .remove(&filter_arg_key)
                        .expect("INVARIANT: the key was read from the map");
                    let arg_key = split_key[1].clone();
                    // argKey is guaranteed unique, don't need to check for duplicates
                    filter_args.insert(arg_key, arg_value);
                }
            }
        }
        if args.is_empty() {
            Self::populate_inner_filters(base, wrapped_filter_args)?;
        }
        Ok(())
    }

    /// Equivalent to the private `populateInnerFilters(LinkedHashMap)`: each
    /// wrapped filter, its `-id` suffix stripped, built with
    /// `TokenFilterFactory.forName(filterName, filterArgs)`.
    fn populate_inner_filters(
        base: &mut ConditionalTokenFilterFactory,
        wrapped_filter_args: Vec<(String, HashMap<String, String>)>,
    ) -> Result<()> {
        let mut inner_filters: Vec<Arc<dyn TokenFilterFactory>> = Vec::new();
        for (filter_name, mut filter_args) in wrapped_filter_args {
            // Format: SPIname[-id]
            let filter_name = match filter_name.find(Self::FILTER_NAME_ID_SEPARATOR) {
                // Strip '-id' suffix, if any, prior to SPI lookup
                Some(id_suffix_pos) => filter_name[..id_suffix_pos].to_string(),
                None => filter_name,
            };
            inner_filters.push(token_filter_for_name(&filter_name, &mut filter_args)?);
        }
        base.set_inner_filters(Some(inner_filters));
        Ok(())
    }

    /// Equivalent to `isIgnoreCase()`.
    pub fn is_ignore_case(&self) -> bool {
        self.ignore_case
    }

    /// Equivalent to `getProtectedTerms()`: `None` (Java's `null`) until
    /// [`inform`](ResourceLoaderAware::inform) reads them.
    pub fn get_protected_terms(&self) -> Option<&CharArraySet> {
        self.protected_terms.as_ref()
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `ProtectedTermFilterFactory.class`, with its `NAME` field
    /// and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.ProtectedTermFilterFactory",
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

impl ResourceLoaderAware for ProtectedTermFilterFactory {
    /// Equivalent to the inherited final `ConditionalTokenFilterFactory.inform`,
    /// whose `doInform` is `ProtectedTermFilterFactory.doInform(ResourceLoader)`:
    /// `protectedTerms = getWordSet(loader, termFiles, ignoreCase)`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        let Self {
            base,
            term_files,
            ignore_case,
            protected_terms,
            ..
        } = self;
        base.inform(loader, |loader| {
            *protected_terms =
                AbstractAnalysisFactory::get_word_set(loader, Some(term_files), *ignore_case)?;
            Ok(())
        })
    }
}

impl TokenFilterFactory for ProtectedTermFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.ProtectedTermFilterFactory"
    }

    /// Equivalent to the inherited `ConditionalTokenFilterFactory.create(TokenStream)`,
    /// whose abstract `create(TokenStream, Function)` is
    /// `new ProtectedTermFilter(protectedTerms, input, inner)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        self.base.create(input, |input, inner| {
            Ok(Box::new(new_protected_term_filter(
                self.protected_terms.clone(),
                input,
                inner,
            )?))
        })
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
