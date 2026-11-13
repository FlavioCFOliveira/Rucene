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

//! A general-purpose analyzer built from analysis factories, ported from
//! `org.apache.lucene.analysis.custom.CustomAnalyzer`.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use rucene::analysis::factories::{
    char_filter_for_name, token_filter_for_name, token_filter_lookup_class, tokenizer_for_name,
    AnalysisFactoryClass, AnalysisSPILoader, CharFilterFactory, TokenFilterFactory,
    TokenizerFactory, LUCENE_MATCH_VERSION_PARAM,
};
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, CharTermAttribute, GlobalReuseStrategy, SharedTokenStream,
    TokenStream, TokenStreamComponents,
};
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::java_object_to_string;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::{AttributeSource, Unwrappable};
use rucene::util::resource_loader::{ClasspathResourceLoader, ResourceLoader, ResourceLoaderAware};
use rucene::util::{SetOnce, Version};

use crate::analysis::miscellaneous::conditional_token_filter::{
    new_conditional_token_filter, ConditionalTokenFilterLogic,
};
use crate::analysis::miscellaneous::conditional_token_filter_factory::{
    ConditionalTokenFilterFactory, ConditionalTokenFilterFactorySubclass,
};
use crate::analysis::util::filesystem_resource_loader::FilesystemResourceLoader;
use crate::resources::class_loader_resources;
use crate::spi::{
    conditional_token_filter_factory_classes, ConditionalTokenFilterFactoryConstructor,
};

/// The panic of a hook that returns no error, for the exception Java's
/// factory throws there (unchecked in Java).
fn throw(error: &LuceneError) -> ! {
    rucene::store::java_nio_file::throw_unchecked(
        error.java_simple_name(),
        Some(&error.to_string()),
    )
}

/// A general-purpose Analyzer that can be created with a builder-style API.
/// Under the hood it uses the factory classes `TokenizerFactory`,
/// `TokenFilterFactory`, and `CharFilterFactory`.
///
/// You can create an instance of this Analyzer using the builder by passing
/// the SPI names (as defined by `java.util.ServiceLoader` interface) to it:
///
/// ```text
/// Analyzer ana = CustomAnalyzer.builder(Paths.get("/path/to/config/dir"))
///   .withTokenizer(StandardTokenizerFactory.NAME)
///   .addTokenFilter(LowerCaseFilterFactory.NAME)
///   .addTokenFilter(StopFilterFactory.NAME, "ignoreCase", "false", "words", "stopwords.txt", "format", "wordset")
///   .build();
/// ```
///
/// The parameters passed to components are also used by Apache Solr and are
/// documented on their corresponding factory classes. Refer to documentation
/// of subclasses of `TokenizerFactory`, `TokenFilterFactory`, and
/// `CharFilterFactory`.
///
/// This is the same as the above:
///
/// ```text
/// Analyzer ana = CustomAnalyzer.builder(Paths.get("/path/to/config/dir"))
///   .withTokenizer("standard")
///   .addTokenFilter("lowercase")
///   .addTokenFilter("stop", "ignoreCase", "false", "words", "stopwords.txt", "format", "wordset")
///   .build();
/// ```
///
/// The list of names to be used for components can be looked up through:
/// `TokenizerFactory#availableTokenizers()`,
/// `TokenFilterFactory#availableTokenFilters()`, and
/// `CharFilterFactory#availableCharFilters()`.
///
/// You can create conditional branches in the analyzer by using
/// [`Builder::when`] and [`Builder::when_term`]:
///
/// ```text
/// Analyzer ana = CustomAnalyzer.builder()
///    .withTokenizer("standard")
///    .addTokenFilter("lowercase")
///    .whenTerm(t -> t.length() > 10)
///      .addTokenFilter("reversestring")
///    .endwhen()
///    .build();
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.custom.CustomAnalyzer` (since
/// 5.0.0). Its `Integer` gaps, `null` for the `Analyzer` defaults, are
/// `Option`s.
#[derive(Debug)]
pub struct CustomAnalyzer {
    state: AnalyzerState,
    char_filters: Vec<Arc<dyn CharFilterFactory>>,
    tokenizer: Arc<dyn TokenizerFactory>,
    token_filters: Vec<Arc<dyn TokenFilterFactory>>,
    pos_inc_gap: Option<i32>,
    offset_gap: Option<i32>,
}

impl CustomAnalyzer {
    /// Returns a builder for custom analyzers that loads all resources from
    /// Lucene's classloader. All path names given must be absolute with
    /// package prefixes.
    ///
    /// Equivalent to `CustomAnalyzer.builder()`: `new
    /// ClasspathResourceLoader(CustomAnalyzer.class.getClassLoader())`, the
    /// class loader that holds this module's resources
    /// ([`class_loader_resources`]).
    pub fn builder() -> Builder {
        Self::builder_with_loader(Box::new(ClasspathResourceLoader::new(
            class_loader_resources(),
        )))
    }

    /// Returns a builder for custom analyzers that loads all resources from
    /// the given file system base directory. Place, e.g., stop word files
    /// there. Files that are not in the given directory are loaded from
    /// Lucene's classloader.
    ///
    /// Equivalent to `CustomAnalyzer.builder(Path)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `FilesystemResourceLoader` for a
    /// path that is not a directory.
    pub fn builder_with_config_dir(config_dir: &Path) -> Result<Builder> {
        Ok(Self::builder_with_loader(Box::new(
            FilesystemResourceLoader::with_class_loader(config_dir, class_loader_resources())?,
        )))
    }

    /// Returns a builder for custom analyzers that loads all resources using
    /// the given `ResourceLoader`.
    ///
    /// Equivalent to `CustomAnalyzer.builder(ResourceLoader)`.
    pub fn builder_with_loader(loader: Box<dyn ResourceLoader>) -> Builder {
        Builder::new(loader)
    }

    /// Equivalent to the package-private `CustomAnalyzer(CharFilterFactory[],
    /// TokenizerFactory, TokenFilterFactory[], Integer, Integer)`, whose
    /// `Analyzer()` uses `GLOBAL_REUSE_STRATEGY`.
    fn new(
        char_filters: Vec<Arc<dyn CharFilterFactory>>,
        tokenizer: Arc<dyn TokenizerFactory>,
        token_filters: Vec<Arc<dyn TokenFilterFactory>>,
        pos_inc_gap: Option<i32>,
        offset_gap: Option<i32>,
    ) -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            char_filters,
            tokenizer,
            token_filters,
            pos_inc_gap,
            offset_gap,
        }
    }

    /// Returns the list of char filters that are used in this analyzer.
    ///
    /// Equivalent to `CustomAnalyzer.getCharFilterFactories()`, an
    /// unmodifiable list.
    pub fn get_char_filter_factories(&self) -> &[Arc<dyn CharFilterFactory>] {
        &self.char_filters
    }

    /// Returns the tokenizer that is used in this analyzer.
    ///
    /// Equivalent to `CustomAnalyzer.getTokenizerFactory()`.
    pub fn get_tokenizer_factory(&self) -> &Arc<dyn TokenizerFactory> {
        &self.tokenizer
    }

    /// Returns the list of token filters that are used in this analyzer.
    ///
    /// Equivalent to `CustomAnalyzer.getTokenFilterFactories()`, an
    /// unmodifiable list.
    pub fn get_token_filter_factories(&self) -> &[Arc<dyn TokenFilterFactory>] {
        &self.token_filters
    }
}

impl Analyzer for CustomAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.custom.CustomAnalyzer"
    }

    /// Equivalent to `CustomAnalyzer.toString()`: the simple class name,
    /// then each char filter followed by `,`, the tokenizer, and each token
    /// filter preceded by `,`, in parentheses. The factories do not override
    /// `Object.toString()`, so each is `<binary name>@<identity hash>`, with
    /// the port's identity stand-in (see `java_object_to_string`).
    fn analyzer_to_string(&self) -> String {
        let mut sb = String::from("CustomAnalyzer(");
        for filter in &self.char_filters {
            sb.push_str(&java_object_to_string(
                filter.java_class_name(),
                filter.as_ref(),
            ));
            sb.push(',');
        }
        sb.push_str(&java_object_to_string(
            self.tokenizer.java_class_name(),
            self.tokenizer.as_ref(),
        ));
        for filter in &self.token_filters {
            sb.push(',');
            sb.push_str(&java_object_to_string(
                filter.java_class_name(),
                filter.as_ref(),
            ));
        }
        sb.push(')');
        sb
    }

    /// Equivalent to `CustomAnalyzer.initReader(String, Reader)`: each char
    /// filter's `create`, in order.
    ///
    /// # Panics
    ///
    /// With the exception a char filter factory's `create` throws (the hook
    /// returns no error, and Java's exception is unchecked).
    fn init_reader(&self, _field_name: &str, reader: Box<dyn CharReader>) -> Box<dyn CharReader> {
        let mut reader = reader;
        for char_filter in &self.char_filters {
            reader = char_filter
                .create(reader)
                .unwrap_or_else(|error| throw(&error));
        }
        reader
    }

    /// Equivalent to `CustomAnalyzer.initReaderForNormalization(String,
    /// Reader)`: each char filter's `normalize`, in order.
    ///
    /// # Panics
    ///
    /// As [`init_reader`](Self::init_reader).
    fn init_reader_for_normalization(
        &self,
        _field_name: &str,
        reader: Box<dyn CharReader>,
    ) -> Box<dyn CharReader> {
        let mut reader = reader;
        for char_filter in &self.char_filters {
            reader = char_filter
                .normalize(reader)
                .unwrap_or_else(|error| throw(&error));
        }
        reader
    }

    /// Equivalent to `CustomAnalyzer.createComponents(String)`: the
    /// tokenizer over `attributeFactory(fieldName)`, wrapped by each token
    /// filter in order; the tokenizer's `setReader` is the source.
    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        let tk = Arc::new(AtomicRefCell::new(
            self.tokenizer
                .create_with_factory(self.attribute_factory(field_name))?,
        ));
        let tokenizer_for_source = Arc::clone(&tk);
        let source = Box::new(move |reader: Box<dyn CharReader>| {
            tokenizer_for_source.borrow_mut().set_reader(reader)
        });
        let mut ts: Box<dyn TokenStream> = Box::new(SharedTokenStream::new(tk));
        for filter in &self.token_filters {
            ts = filter.create(ts)?;
        }
        let sink: Arc<AtomicRefCell<dyn TokenStream>> =
            Arc::new(AtomicRefCell::new(ChainStream(ts)));
        Ok(TokenStreamComponents::new(source, sink))
    }

    /// Equivalent to `CustomAnalyzer.normalize(String, TokenStream)`: each
    /// token filter's `normalize`, in order.
    ///
    /// # Panics
    ///
    /// With the exception a token filter factory's `normalize` throws (the
    /// hook returns no error, and Java's exception is unchecked).
    fn normalize_token_stream(
        &self,
        _field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let mut result = input;
        for filter in &self.token_filters {
            result = filter
                .normalize(result)
                .unwrap_or_else(|error| throw(&error));
        }
        result
    }

    /// Equivalent to `CustomAnalyzer.getPositionIncrementGap(String)`: the
    /// gap set on the builder, else `Analyzer`'s default, 0.
    fn get_position_increment_gap(&self, _field_name: &str) -> i32 {
        // use default from Analyzer base class if null
        self.pos_inc_gap.unwrap_or(0)
    }

    /// Equivalent to `CustomAnalyzer.getOffsetGap(String)`: the gap set on
    /// the builder, else `Analyzer`'s default, 1.
    fn get_offset_gap(&self, _field_name: &str) -> i32 {
        // use default from Analyzer base class if null
        self.offset_gap.unwrap_or(1)
    }
}

/// The last stream of the chain, sized so that the components can share it
/// behind an `Arc`: Java's `TokenStream ts`.
#[derive(Debug)]
struct ChainStream(Box<dyn TokenStream>);

impl TokenStream for ChainStream {
    fn increment_token(&mut self) -> Result<bool> {
        self.0.increment_token()
    }

    fn end(&mut self) -> Result<()> {
        self.0.end()
    }

    fn reset(&mut self) -> Result<()> {
        self.0.reset()
    }

    fn close(&mut self) -> Result<()> {
        self.0.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        self.0.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.0.attribute_source_mut()
    }

    fn as_unwrappable(&self) -> Option<&dyn Unwrappable<dyn TokenStream>> {
        self.0.as_unwrappable()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.0.as_any()
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        self.0.as_any_mut()
    }
}

impl fmt::Display for CustomAnalyzer {
    /// `CustomAnalyzer.toString()`; see [`Analyzer::analyzer_to_string`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.analyzer_to_string())
    }
}

/// Builder for [`CustomAnalyzer`].
///
/// See [`CustomAnalyzer::builder`], [`CustomAnalyzer::builder_with_config_dir`]
/// and [`CustomAnalyzer::builder_with_loader`].
///
/// Equivalent to `org.apache.lucene.analysis.custom.CustomAnalyzer$Builder`.
/// Java's overloads by parameter type are methods with suffixes here:
/// `_map` takes the parameter map (Java's `Map<String, String>`, which the
/// factory consumes), `_class` the factory's class (Java's `Class<? extends
/// ...>`, here the [`AnalysisFactoryClass`] a factory's `class()` returns),
/// and the plain form the key/value pairs (Java's `String...`). Each returns
/// the builder, as Java's `return this`.
pub struct Builder {
    loader: Box<dyn ResourceLoader>,
    default_match_version: SetOnce<Version>,
    char_filters: Vec<Arc<dyn CharFilterFactory>>,
    tokenizer: SetOnce<Arc<dyn TokenizerFactory>>,
    token_filters: Vec<Arc<dyn TokenFilterFactory>>,
    pos_inc_gap: SetOnce<i32>,
    offset_gap: SetOnce<i32>,
    components_added: bool,
}

impl fmt::Debug for Builder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builder")
            .field("default_match_version", &self.default_match_version.get())
            .field("char_filters", &self.char_filters)
            .field("tokenizer", &self.tokenizer.get())
            .field("token_filters", &self.token_filters)
            .field("pos_inc_gap", &self.pos_inc_gap.get())
            .field("offset_gap", &self.offset_gap.get())
            .field("components_added", &self.components_added)
            .finish_non_exhaustive()
    }
}

/// A freshly created factory as mutable, for `applyResourceLoader`.
fn unique<T: ?Sized>(factory: &mut Arc<T>) -> &mut T {
    Arc::get_mut(factory).expect("INVARIANT: a factory just created is not shared")
}

impl Builder {
    /// Equivalent to the package-private `Builder(ResourceLoader)`.
    fn new(loader: Box<dyn ResourceLoader>) -> Self {
        Self {
            loader,
            default_match_version: SetOnce::new(),
            char_filters: Vec::new(),
            tokenizer: SetOnce::new(),
            token_filters: Vec::new(),
            pos_inc_gap: SetOnce::new(),
            offset_gap: SetOnce::new(),
            components_added: false,
        }
    }

    /// This match version is passed as default to all tokenizers or filters.
    /// It is used unless you pass the parameter `luceneMatchVersion`
    /// explicitly. It defaults to undefined, so the underlying factory will
    /// (in most cases) use `Version#LATEST`.
    ///
    /// Equivalent to `Builder.withDefaultMatchVersion(Version)`; Java's
    /// `NullPointerException` for `null` has no counterpart.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` "You may only set the default match version
    /// before adding tokenizers, token filters, or char filters." once a
    /// component was added, and `SetOnce`'s `AlreadySetException` when the
    /// version was set before.
    pub fn with_default_match_version(&mut self, version: Version) -> Result<&mut Self> {
        if self.components_added {
            return Err(LuceneError::illegal_state(
                "You may only set the default match version before adding tokenizers, \
                 token filters, or char filters.",
            ));
        }
        self.default_match_version.set(version)?;
        Ok(self)
    }

    /// Sets the position increment gap of the analyzer. The default is
    /// defined in the analyzer base class.
    ///
    /// Equivalent to `Builder.withPositionIncrementGap(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` "posIncGap must be >= 0", and `SetOnce`'s
    /// `AlreadySetException` when the gap was set before.
    pub fn with_position_increment_gap(&mut self, pos_inc_gap: i32) -> Result<&mut Self> {
        if pos_inc_gap < 0 {
            return Err(LuceneError::illegal_argument("posIncGap must be >= 0"));
        }
        self.pos_inc_gap.set(pos_inc_gap)?;
        Ok(self)
    }

    /// Sets the offset gap of the analyzer. The default is defined in the
    /// analyzer base class.
    ///
    /// Equivalent to `Builder.withOffsetGap(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` "offsetGap must be >= 0", and `SetOnce`'s
    /// `AlreadySetException` when the gap was set before.
    pub fn with_offset_gap(&mut self, offset_gap: i32) -> Result<&mut Self> {
        if offset_gap < 0 {
            return Err(LuceneError::illegal_argument("offsetGap must be >= 0"));
        }
        self.offset_gap.set(offset_gap)?;
        Ok(self)
    }

    /// Uses the given tokenizer.
    ///
    /// * `factory` class that is used to create the tokenizer.
    /// * `params` a list of factory string params as key/value pairs. The
    ///   number of parameters must be an even number, as they are pairs.
    ///
    /// Equivalent to `Builder.withTokenizer(Class, String...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `paramsToMap` for an odd number of
    /// params, and as
    /// [`with_tokenizer_class_map`](Self::with_tokenizer_class_map).
    pub fn with_tokenizer_class(
        &mut self,
        factory: &AnalysisFactoryClass<dyn TokenizerFactory>,
        params: &[&str],
    ) -> Result<&mut Self> {
        let mut params = Self::params_to_map(params)?;
        self.with_tokenizer_class_map(factory, &mut params)
    }

    /// Uses the given tokenizer.
    ///
    /// * `factory` class that is used to create the tokenizer.
    /// * `params` the map of parameters to be passed to factory. The map must
    ///   be modifiable.
    ///
    /// Equivalent to `Builder.withTokenizer(Class, Map)`.
    ///
    /// # Errors
    ///
    /// What `AnalysisSPILoader.newFactoryClassInstance` and the factory's
    /// `inform` return, and `SetOnce`'s `AlreadySetException` when a
    /// tokenizer was set before.
    pub fn with_tokenizer_class_map(
        &mut self,
        factory: &AnalysisFactoryClass<dyn TokenizerFactory>,
        params: &mut HashMap<String, String>,
    ) -> Result<&mut Self> {
        let mut tokenizer = AnalysisSPILoader::new_factory_class_instance(
            factory,
            self.apply_default_params(params),
        )?;
        self.apply_resource_loader(unique(&mut tokenizer).as_resource_loader_aware())?;
        self.tokenizer.set(tokenizer)?;
        self.components_added = true;
        Ok(self)
    }

    /// Uses the given tokenizer.
    ///
    /// * `name` is used to look up the factory with
    ///   `TokenizerFactory#forName(String, Map)`. The list of possible names
    ///   can be looked up with `TokenizerFactory#availableTokenizers()`.
    /// * `params` a list of factory string params as key/value pairs. The
    ///   number of parameters must be an even number, as they are pairs.
    ///
    /// Equivalent to `Builder.withTokenizer(String, String...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `paramsToMap` for an odd number of
    /// params, and as
    /// [`with_tokenizer_map`](Self::with_tokenizer_map).
    pub fn with_tokenizer(&mut self, name: &str, params: &[&str]) -> Result<&mut Self> {
        let mut params = Self::params_to_map(params)?;
        self.with_tokenizer_map(name, &mut params)
    }

    /// Uses the given tokenizer.
    ///
    /// * `name` is used to look up the factory with
    ///   `TokenizerFactory#forName(String, Map)`. The list of possible names
    ///   can be looked up with `TokenizerFactory#availableTokenizers()`.
    /// * `params` the map of parameters to be passed to factory. The map must
    ///   be modifiable.
    ///
    /// Equivalent to `Builder.withTokenizer(String, Map)`.
    ///
    /// # Errors
    ///
    /// What `TokenizerFactory.forName` and the factory's `inform` return,
    /// and `SetOnce`'s `AlreadySetException` when a tokenizer was set before.
    pub fn with_tokenizer_map(
        &mut self,
        name: &str,
        params: &mut HashMap<String, String>,
    ) -> Result<&mut Self> {
        let mut tokenizer = tokenizer_for_name(name, self.apply_default_params(params))?;
        self.apply_resource_loader(unique(&mut tokenizer).as_resource_loader_aware())?;
        self.tokenizer.set(tokenizer)?;
        self.components_added = true;
        Ok(self)
    }

    /// Adds the given token filter.
    ///
    /// * `factory` class that is used to create the token filter.
    /// * `params` a list of factory string params as key/value pairs. The
    ///   number of parameters must be an even number, as they are pairs.
    ///
    /// Equivalent to `Builder.addTokenFilter(Class, String...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `paramsToMap` for an odd number of
    /// params, and as
    /// [`add_token_filter_class_map`](Self::add_token_filter_class_map).
    pub fn add_token_filter_class(
        &mut self,
        factory: &AnalysisFactoryClass<dyn TokenFilterFactory>,
        params: &[&str],
    ) -> Result<&mut Self> {
        let mut params = Self::params_to_map(params)?;
        self.add_token_filter_class_map(factory, &mut params)
    }

    /// Adds the given token filter.
    ///
    /// * `factory` class that is used to create the token filter.
    /// * `params` the map of parameters to be passed to factory. The map must
    ///   be modifiable.
    ///
    /// Equivalent to `Builder.addTokenFilter(Class, Map)`.
    ///
    /// # Errors
    ///
    /// What `AnalysisSPILoader.newFactoryClassInstance` and the factory's
    /// `inform` return.
    pub fn add_token_filter_class_map(
        &mut self,
        factory: &AnalysisFactoryClass<dyn TokenFilterFactory>,
        params: &mut HashMap<String, String>,
    ) -> Result<&mut Self> {
        let mut filter = AnalysisSPILoader::new_factory_class_instance(
            factory,
            self.apply_default_params(params),
        )?;
        self.apply_resource_loader(unique(&mut filter).as_resource_loader_aware())?;
        self.token_filters.push(filter);
        self.components_added = true;
        Ok(self)
    }

    /// Adds the given token filter.
    ///
    /// * `name` is used to look up the factory with
    ///   `TokenFilterFactory#forName(String, Map)`. The list of possible
    ///   names can be looked up with `TokenFilterFactory#availableTokenFilters()`.
    /// * `params` a list of factory string params as key/value pairs. The
    ///   number of parameters must be an even number, as they are pairs.
    ///
    /// Equivalent to `Builder.addTokenFilter(String, String...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `paramsToMap` for an odd number of
    /// params, and as
    /// [`add_token_filter_map`](Self::add_token_filter_map).
    pub fn add_token_filter(&mut self, name: &str, params: &[&str]) -> Result<&mut Self> {
        let mut params = Self::params_to_map(params)?;
        self.add_token_filter_map(name, &mut params)
    }

    /// Adds the given token filter.
    ///
    /// * `name` is used to look up the factory with
    ///   `TokenFilterFactory#forName(String, Map)`. The list of possible
    ///   names can be looked up with `TokenFilterFactory#availableTokenFilters()`.
    /// * `params` the map of parameters to be passed to factory. The map must
    ///   be modifiable.
    ///
    /// Equivalent to `Builder.addTokenFilter(String, Map)`.
    ///
    /// # Errors
    ///
    /// What `TokenFilterFactory.forName` and the factory's `inform` return.
    pub fn add_token_filter_map(
        &mut self,
        name: &str,
        params: &mut HashMap<String, String>,
    ) -> Result<&mut Self> {
        let mut filter = token_filter_for_name(name, self.apply_default_params(params))?;
        self.apply_resource_loader(unique(&mut filter).as_resource_loader_aware())?;
        self.token_filters.push(filter);
        self.components_added = true;
        Ok(self)
    }

    /// Equivalent to the private `Builder.addTokenFilter(TokenFilterFactory)`.
    fn add_token_filter_factory(&mut self, factory: Arc<dyn TokenFilterFactory>) -> &mut Self {
        self.token_filters.push(factory);
        self.components_added = true;
        self
    }

    /// Adds the given char filter.
    ///
    /// * `factory` class that is used to create the char filter.
    /// * `params` a list of factory string params as key/value pairs. The
    ///   number of parameters must be an even number, as they are pairs.
    ///
    /// Equivalent to `Builder.addCharFilter(Class, String...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `paramsToMap` for an odd number of
    /// params, and as
    /// [`add_char_filter_class_map`](Self::add_char_filter_class_map).
    pub fn add_char_filter_class(
        &mut self,
        factory: &AnalysisFactoryClass<dyn CharFilterFactory>,
        params: &[&str],
    ) -> Result<&mut Self> {
        let mut params = Self::params_to_map(params)?;
        self.add_char_filter_class_map(factory, &mut params)
    }

    /// Adds the given char filter.
    ///
    /// * `factory` class that is used to create the char filter.
    /// * `params` the map of parameters to be passed to factory. The map must
    ///   be modifiable.
    ///
    /// Equivalent to `Builder.addCharFilter(Class, Map)`.
    ///
    /// # Errors
    ///
    /// What `AnalysisSPILoader.newFactoryClassInstance` and the factory's
    /// `inform` return.
    pub fn add_char_filter_class_map(
        &mut self,
        factory: &AnalysisFactoryClass<dyn CharFilterFactory>,
        params: &mut HashMap<String, String>,
    ) -> Result<&mut Self> {
        let mut filter = AnalysisSPILoader::new_factory_class_instance(
            factory,
            self.apply_default_params(params),
        )?;
        self.apply_resource_loader(unique(&mut filter).as_resource_loader_aware())?;
        self.char_filters.push(filter);
        self.components_added = true;
        Ok(self)
    }

    /// Adds the given char filter.
    ///
    /// * `name` is used to look up the factory with
    ///   `CharFilterFactory#forName(String, Map)`. The list of possible names
    ///   can be looked up with `CharFilterFactory#availableCharFilters()`.
    /// * `params` a list of factory string params as key/value pairs. The
    ///   number of parameters must be an even number, as they are pairs.
    ///
    /// Equivalent to `Builder.addCharFilter(String, String...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `paramsToMap` for an odd number of
    /// params, and as
    /// [`add_char_filter_map`](Self::add_char_filter_map).
    pub fn add_char_filter(&mut self, name: &str, params: &[&str]) -> Result<&mut Self> {
        let mut params = Self::params_to_map(params)?;
        self.add_char_filter_map(name, &mut params)
    }

    /// Adds the given char filter.
    ///
    /// * `name` is used to look up the factory with
    ///   `CharFilterFactory#forName(String, Map)`. The list of possible names
    ///   can be looked up with `CharFilterFactory#availableCharFilters()`.
    /// * `params` the map of parameters to be passed to factory. The map must
    ///   be modifiable.
    ///
    /// Equivalent to `Builder.addCharFilter(String, Map)`.
    ///
    /// # Errors
    ///
    /// What `CharFilterFactory.forName` and the factory's `inform` return.
    pub fn add_char_filter_map(
        &mut self,
        name: &str,
        params: &mut HashMap<String, String>,
    ) -> Result<&mut Self> {
        let mut filter = char_filter_for_name(name, self.apply_default_params(params))?;
        self.apply_resource_loader(unique(&mut filter).as_resource_loader_aware())?;
        self.char_filters.push(filter);
        self.components_added = true;
        Ok(self)
    }

    /// Add a `ConditionalTokenFilterFactory` to the analysis chain
    ///
    /// TokenFilters added by subsequent calls to
    /// [`ConditionBuilder::add_token_filter`] and related functions will only
    /// be used if the current token matches the condition. Consumers must
    /// call [`ConditionBuilder::endwhen`] to return to the normal tokenfilter
    /// chain once conditional filters have been added
    ///
    /// * `name` is used to look up the factory with
    ///   `TokenFilterFactory#forName(String, Map)`
    /// * `params` the parameters to be passed to the factory
    ///
    /// Equivalent to `Builder.when(String, String...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `paramsToMap` for an odd number of
    /// params, and as
    /// [`when_map`](Self::when_map).
    pub fn when(&mut self, name: &str, params: &[&str]) -> Result<ConditionBuilder<'_>> {
        let mut params = Self::params_to_map(params)?;
        self.when_map(name, &mut params)
    }

    /// Add a `ConditionalTokenFilterFactory` to the analysis chain
    ///
    /// TokenFilters added by subsequent calls to
    /// [`ConditionBuilder::add_token_filter`] and related functions will only
    /// be used if the current token matches the condition. Consumers must
    /// call [`ConditionBuilder::endwhen`] to return to the normal tokenfilter
    /// chain once conditional filters have been added
    ///
    /// * `name` is used to look up the factory with
    ///   `TokenFilterFactory#forName(String, Map)`
    /// * `params` the parameters to be passed to the factory. The map must
    ///   be modifiable
    ///
    /// Equivalent to `Builder.when(String, Map)`. Java tests
    /// `ConditionalTokenFilterFactory.class.isAssignableFrom(clazz)` on the
    /// class `TokenFilterFactory.lookupClass(name)` finds; Rust has no class
    /// hierarchy to query, so the subclasses are the classes
    /// [`conditional_token_filter_factory_classes`] registers.
    ///
    /// # Errors
    ///
    /// What `TokenFilterFactory.lookupClass` returns for an unknown name,
    /// `IllegalArgumentException` "TokenFilterFactory `<name>` is not a
    /// ConditionalTokenFilterFactory", and as
    /// [`when_class_map`](Self::when_class_map).
    pub fn when_map(
        &mut self,
        name: &str,
        params: &mut HashMap<String, String>,
    ) -> Result<ConditionBuilder<'_>> {
        let clazz = token_filter_lookup_class(name)?;
        let constructor = {
            let classes = conditional_token_filter_factory_classes()
                .read()
                .expect("INVARIANT: no thread panics while holding the conditional class table");
            match classes.find_class(clazz.get_name()) {
                Ok(constructor) => constructor(),
                Err(_) => {
                    return Err(LuceneError::illegal_argument(format!(
                        "TokenFilterFactory {name} is not a ConditionalTokenFilterFactory"
                    )))
                }
            }
        };
        self.when_class_map(constructor, params)
    }

    /// Add a `ConditionalTokenFilterFactory` to the analysis chain
    ///
    /// TokenFilters added by subsequent calls to
    /// [`ConditionBuilder::add_token_filter`] and related functions will only
    /// be used if the current token matches the condition. Consumers must
    /// call [`ConditionBuilder::endwhen`] to return to the normal tokenfilter
    /// chain once conditional filters have been added
    ///
    /// * `factory` class that is used to create the ConditionalTokenFilter
    /// * `params` the parameters to be passed to the factory
    ///
    /// Equivalent to `Builder.when(Class, String...)`; the class is its
    /// `Map<String, String>` constructor.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `paramsToMap` for an odd number of
    /// params, and what the constructor returns.
    pub fn when_class(
        &mut self,
        factory: ConditionalTokenFilterFactoryConstructor,
        params: &[&str],
    ) -> Result<ConditionBuilder<'_>> {
        let mut params = Self::params_to_map(params)?;
        self.when_class_map(factory, &mut params)
    }

    /// Add a `ConditionalTokenFilterFactory` to the analysis chain
    ///
    /// TokenFilters added by subsequent calls to
    /// [`ConditionBuilder::add_token_filter`] and related functions will only
    /// be used if the current token matches the condition. Consumers must
    /// call [`ConditionBuilder::endwhen`] to return to the normal tokenfilter
    /// chain once conditional filters have been added
    ///
    /// * `factory` class that is used to create the ConditionalTokenFilter
    /// * `params` the parameters to be passed to the factory. The map must
    ///   be modifiable
    ///
    /// Equivalent to `Builder.when(Class, Map)`, `when(newFactoryClassInstance(factory,
    /// applyDefaultParams(params)))`; the class is its `Map<String, String>`
    /// constructor.
    ///
    /// # Errors
    ///
    /// What the constructor returns.
    pub fn when_class_map(
        &mut self,
        factory: ConditionalTokenFilterFactoryConstructor,
        params: &mut HashMap<String, String>,
    ) -> Result<ConditionBuilder<'_>> {
        let factory = factory(self.apply_default_params(params))?;
        Ok(self.when_factory(factory))
    }

    /// Add a `ConditionalTokenFilterFactory` to the analysis chain
    ///
    /// TokenFilters added by subsequent calls to
    /// [`ConditionBuilder::add_token_filter`] and related functions will only
    /// be used if the current token matches the condition. Consumers must
    /// call [`ConditionBuilder::endwhen`] to return to the normal tokenfilter
    /// chain once conditional filters have been added
    ///
    /// Equivalent to `Builder.when(ConditionalTokenFilterFactory)`.
    pub fn when_factory(
        &mut self,
        factory: Box<dyn ConditionalTokenFilterFactorySubclass>,
    ) -> ConditionBuilder<'_> {
        ConditionBuilder::new(factory, self)
    }

    /// Apply subsequent token filters if the current token's term matches a
    /// predicate
    ///
    /// This is the equivalent of:
    ///
    /// ```text
    ///   when(new ConditionalTokenFilterFactory(Collections.emptyMap()) {
    ///      @Override
    ///      protected ConditionalTokenFilter create(TokenStream input, Function<TokenStream, TokenStream> inner) {
    ///        return new ConditionalTokenFilter(input, inner) {
    ///          CharTermAttribute termAtt = addAttribute(CharTermAttribute.class);
    ///          @Override
    ///          protected boolean shouldFilter() {
    ///            return predicate.test(termAtt);
    ///          }
    ///        };
    ///      }
    ///   });
    /// ```
    ///
    /// Equivalent to `Builder.whenTerm(Predicate<CharSequence>)`: the
    /// predicate is handed the term as a string, the `CharSequence` Java's
    /// `CharTermAttribute` is.
    pub fn when_term(&mut self, predicate: TermPredicate) -> ConditionBuilder<'_> {
        let factory = WhenTermFactory {
            base: ConditionalTokenFilterFactory::new(&mut HashMap::new())
                .expect("INVARIANT: an empty argument map is valid"),
            predicate,
        };
        ConditionBuilder::new(Box::new(factory), self)
    }

    /// Builds the analyzer.
    ///
    /// Equivalent to `Builder.build()`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` "You have to set at least a tokenizer." when
    /// no tokenizer was set.
    pub fn build(&self) -> Result<CustomAnalyzer> {
        let Some(tokenizer) = self.tokenizer.get() else {
            return Err(LuceneError::illegal_state(
                "You have to set at least a tokenizer.",
            ));
        };
        Ok(CustomAnalyzer::new(
            self.char_filters.clone(),
            Arc::clone(tokenizer),
            self.token_filters.clone(),
            self.pos_inc_gap.get().copied(),
            self.offset_gap.get().copied(),
        ))
    }

    /// Equivalent to the private `Builder.applyDefaultParams(Map)`.
    fn apply_default_params<'m>(
        &self,
        map: &'m mut HashMap<String, String>,
    ) -> &'m mut HashMap<String, String> {
        if let Some(v) = self.default_match_version.get() {
            map.entry(LUCENE_MATCH_VERSION_PARAM.to_string())
                .or_insert_with(|| v.to_string());
        }
        map
    }

    /// Equivalent to the private `Builder.paramsToMap(String...)`; Java's
    /// `NullPointerException` for a `null` key has no counterpart.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` "Key-value pairs expected, so the number of
    /// params must be even." for an odd number of params.
    fn params_to_map(params: &[&str]) -> Result<HashMap<String, String>> {
        if !params.len().is_multiple_of(2) {
            return Err(LuceneError::illegal_argument(
                "Key-value pairs expected, so the number of params must be even.",
            ));
        }
        // Plus 1 since applyDefaultParams will add a default param later.
        let mut map = HashMap::with_capacity(params.len() / 2 + 1);
        for pair in params.chunks(2) {
            map.insert(pair[0].to_string(), pair[1].to_string());
        }
        Ok(map)
    }

    /// Equivalent to the package-private `Builder.applyResourceLoader(T)`:
    /// a factory that is `ResourceLoaderAware` is informed with the
    /// builder's loader.
    ///
    /// # Errors
    ///
    /// What the factory's `inform` returns.
    fn apply_resource_loader(&self, factory: Option<&mut dyn ResourceLoaderAware>) -> Result<()> {
        if let Some(factory) = factory {
            factory.inform(self.loader.as_ref())?;
        }
        Ok(())
    }
}

/// The predicate [`Builder::when_term`] tests each term with.
pub type TermPredicate = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The anonymous `ConditionalTokenFilterFactory` of [`Builder::when_term`].
struct WhenTermFactory {
    base: ConditionalTokenFilterFactory,
    predicate: TermPredicate,
}

impl fmt::Debug for WhenTermFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WhenTermFactory")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl ConditionalTokenFilterFactorySubclass for WhenTermFactory {
    fn conditional_base_mut(&mut self) -> &mut ConditionalTokenFilterFactory {
        &mut self.base
    }
}

impl ResourceLoaderAware for WhenTermFactory {
    /// The inherited final `ConditionalTokenFilterFactory.inform`, whose
    /// `doInform` does nothing.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        self.base.inform(loader, |_| Ok(()))
    }
}

impl TokenFilterFactory for WhenTermFactory {
    /// The binary name of Java's anonymous class.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.custom.CustomAnalyzer$Builder$1"
    }

    /// The inherited `ConditionalTokenFilterFactory.create(TokenStream)`,
    /// whose `create(TokenStream, Function)` builds the anonymous
    /// `ConditionalTokenFilter` that adds the `CharTermAttribute` and tests
    /// the predicate on it.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        self.base.create(input, |input, inner| {
            let mut filter = new_conditional_token_filter(
                input,
                inner,
                WhenTermLogic {
                    predicate: Arc::clone(&self.predicate),
                },
            )?;
            filter
                .attribute_source_mut()
                .add_attribute_interface::<dyn CharTermAttribute>()?;
            Ok(Box::new(filter))
        })
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}

/// The `shouldFilter()` of [`Builder::when_term`]'s anonymous
/// `ConditionalTokenFilter`: `predicate.test(termAtt)`.
struct WhenTermLogic {
    predicate: TermPredicate,
}

impl fmt::Debug for WhenTermLogic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WhenTermLogic").finish_non_exhaustive()
    }
}

impl ConditionalTokenFilterLogic for WhenTermLogic {
    fn should_filter(&mut self, source: &AttributeSource) -> Result<bool> {
        let term: String = {
            let term_att = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            term_att.buffer()[..term_att.length()].iter().collect()
        };
        Ok((self.predicate)(&term))
    }
}

/// Factory class for a `ConditionalTokenFilter`
///
/// Equivalent to `org.apache.lucene.analysis.custom.CustomAnalyzer$ConditionBuilder`:
/// it holds the builder it returns to (Java's `parent`) for as long as the
/// branch is open. Its `addTokenFilter` overloads are named as
/// [`Builder`]'s are.
pub struct ConditionBuilder<'a> {
    inner_filters: Vec<Arc<dyn TokenFilterFactory>>,
    factory: Box<dyn ConditionalTokenFilterFactorySubclass>,
    parent: &'a mut Builder,
}

impl fmt::Debug for ConditionBuilder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConditionBuilder")
            .field("inner_filters", &self.inner_filters)
            .field("factory", &self.factory)
            .finish_non_exhaustive()
    }
}

impl<'a> ConditionBuilder<'a> {
    /// Equivalent to the private `ConditionBuilder(ConditionalTokenFilterFactory,
    /// Builder)`.
    fn new(
        factory: Box<dyn ConditionalTokenFilterFactorySubclass>,
        parent: &'a mut Builder,
    ) -> Self {
        Self {
            inner_filters: Vec::new(),
            factory,
            parent,
        }
    }

    /// Adds the given token filter.
    ///
    /// * `name` is used to look up the factory with
    ///   `TokenFilterFactory#forName(String, Map)`. The list of possible
    ///   names can be looked up with `TokenFilterFactory#availableTokenFilters()`.
    /// * `params` the map of parameters to be passed to factory. The map must
    ///   be modifiable.
    ///
    /// Equivalent to `ConditionBuilder.addTokenFilter(String, Map)`.
    ///
    /// # Errors
    ///
    /// What `TokenFilterFactory.forName` returns.
    pub fn add_token_filter_map(
        &mut self,
        name: &str,
        params: &mut HashMap<String, String>,
    ) -> Result<&mut Self> {
        self.inner_filters.push(token_filter_for_name(
            name,
            self.parent.apply_default_params(params),
        )?);
        Ok(self)
    }

    /// Adds the given token filter.
    ///
    /// * `name` is used to look up the factory with
    ///   `TokenFilterFactory#forName(String, Map)`. The list of possible
    ///   names can be looked up with `TokenFilterFactory#availableTokenFilters()`.
    /// * `params` a list of factory string params as key/value pairs.
    ///
    /// Equivalent to `ConditionBuilder.addTokenFilter(String, String...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `paramsToMap` for an odd number of
    /// params, and as [`add_token_filter_map`](Self::add_token_filter_map).
    pub fn add_token_filter(&mut self, name: &str, params: &[&str]) -> Result<&mut Self> {
        let mut params = Builder::params_to_map(params)?;
        self.add_token_filter_map(name, &mut params)
    }

    /// Adds the given token filter.
    ///
    /// * `factory` class that is used to create the token filter.
    /// * `params` the map of parameters to be passed to factory. The map must
    ///   be modifiable.
    ///
    /// Equivalent to `ConditionBuilder.addTokenFilter(Class, Map)`.
    ///
    /// # Errors
    ///
    /// What `AnalysisSPILoader.newFactoryClassInstance` returns.
    pub fn add_token_filter_class_map(
        &mut self,
        factory: &AnalysisFactoryClass<dyn TokenFilterFactory>,
        params: &mut HashMap<String, String>,
    ) -> Result<&mut Self> {
        self.inner_filters
            .push(AnalysisSPILoader::new_factory_class_instance(
                factory,
                self.parent.apply_default_params(params),
            )?);
        Ok(self)
    }

    /// Adds the given token filter.
    ///
    /// * `factory` class that is used to create the token filter.
    /// * `params` a list of factory string params as key/value pairs.
    ///
    /// Equivalent to `ConditionBuilder.addTokenFilter(Class, String...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `paramsToMap` for an odd number of
    /// params, and as [`add_token_filter_class_map`](Self::add_token_filter_class_map).
    pub fn add_token_filter_class(
        &mut self,
        factory: &AnalysisFactoryClass<dyn TokenFilterFactory>,
        params: &[&str],
    ) -> Result<&mut Self> {
        let mut params = Builder::params_to_map(params)?;
        self.add_token_filter_class_map(factory, &mut params)
    }

    /// Close the branch and return to the main analysis chain
    ///
    /// Equivalent to `ConditionBuilder.endwhen()`: the inner filters are set
    /// on the factory, which the builder informs and adds.
    ///
    /// # Errors
    ///
    /// What the factory's `inform` returns.
    pub fn endwhen(self) -> Result<&'a mut Builder> {
        let Self {
            inner_filters,
            mut factory,
            parent,
        } = self;
        factory
            .conditional_base_mut()
            .set_inner_filters(Some(inner_filters));
        parent.apply_resource_loader(factory.as_resource_loader_aware())?;
        let factory: Box<dyn TokenFilterFactory> = factory;
        parent.add_token_filter_factory(Arc::from(factory));
        Ok(parent)
    }
}
